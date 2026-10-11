/*
 * lse_emu - LD_PRELOAD library that emulates ARMv8.1 LSE atomic instructions on CPUs that lack them.
 *
 * WHY: libMrcLib.so (WPILib's Driver Station / robot communication library, binary only) is built with the
 * SystemCore toolchain and uses LSE instructions (swp, ldadd, cas, ...) inline, but is published under the name
 * "linuxarm64". A Raspberry Pi 4 (Cortex-A72, ARMv8.0) has no LSE: the first such instruction raises SIGILL. MEASURED on a
 * VMX-pi: SIGILL in uv_library_shutdown() at every process exit (exit code 132, core dump). No source is published,
 * and the library is not open source, so it cannot be rebuilt.
 *
 * WHAT: a SIGILL handler decodes the faulting instruction, performs the same operation with an ordinary atomic
 * (compiled to ldxr/stxr loops for plain ARMv8.0), writes the result registers and continues after the instruction.
 * The operation is atomic with respect to every other thread: it uses real exclusive-monitor sequences on the same
 * address, exactly like code compiled without LSE. Each emulated instruction costs a signal round trip (microseconds),
 * so hot paths in such a library are slower; nothing else in the process is affected.
 *
 * Load it with LD_AUDIT, not LD_PRELOAD (MEASURED with a library whose constructor runs an LSE instruction): the dynamic
 * loader initialises libraries from the back of its list, so a preloaded library is initialised AFTER the libraries the
 * program needs, and the first LSE instruction in one of their constructors (libMrcLib has libuv inside) would crash
 * before the handler exists. An audit library is loaded and initialised before everything else.
 *
 * Does nothing on a CPU that has LSE (HWCAP_ATOMICS), so it can stay in LD_AUDIT everywhere. Any SIGILL that is not
 * one of the supported instructions goes to the previous handler or, if there is none, crashes as usual.
 *
 * Supported: LD<op>[A][L][B|H], ST<op>[L][B|H] (op = add, clr, eor, set, smax, smin, umax, umin), SWP[A][L][B|H],
 * CAS[A][L][B|H]. Not supported (SIGILL stays a crash): CASP, LDAPR and anything else.
 *
 * Build for plain ARMv8.0 (no outline atomics, no LSE in this file): see the Makefile.
 *
 * Environment: VMX_LSE_EMU_DEBUG=1 prints the number of emulated instructions at exit.
 */
#define _GNU_SOURCE
#include <errno.h>
#include <signal.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/auxv.h>
#include <ucontext.h>

#ifndef HWCAP_ATOMICS
#define HWCAP_ATOMICS (1UL << 8)
#endif

static struct sigaction g_old;
static int g_installed;
static unsigned long g_count;  /* number of emulated instructions (statistics only) */

/* ---- atomic helpers ------------------------------------------------------------------------------------------- */

enum { OP_ADD, OP_CLR, OP_EOR, OP_SET, OP_SMAX, OP_SMIN, OP_UMAX, OP_UMIN, OP_SWP };

#define MODEL __ATOMIC_SEQ_CST /* at least as strong as every LSE ordering variant */

#define DEFINE_AMO(NAME, UT, ST)                                                                                      \
  static uint64_t NAME(void* addr, int op, uint64_t value) {                                                          \
    UT* p = (UT*)addr;                                                                                                \
    UT v = (UT)value;                                                                                                 \
    switch (op) {                                                                                                     \
      case OP_ADD: return __atomic_fetch_add(p, v, MODEL);                                                            \
      case OP_CLR: return __atomic_fetch_and(p, (UT)~v, MODEL);                                                       \
      case OP_EOR: return __atomic_fetch_xor(p, v, MODEL);                                                            \
      case OP_SET: return __atomic_fetch_or(p, v, MODEL);                                                             \
      case OP_SWP: return __atomic_exchange_n(p, v, MODEL);                                                           \
      default: break;                                                                                                 \
    }                                                                                                                 \
    UT old = __atomic_load_n(p, __ATOMIC_RELAXED);                                                                    \
    for (;;) {                                                                                                        \
      UT next;                                                                                                        \
      switch (op) {                                                                                                   \
        case OP_SMAX: next = (ST)old > (ST)v ? old : v; break;                                                        \
        case OP_SMIN: next = (ST)old < (ST)v ? old : v; break;                                                        \
        case OP_UMAX: next = old > v ? old : v; break;                                                                \
        default: next = old < v ? old : v; break; /* OP_UMIN */                                                       \
      }                                                                                                               \
      if (__atomic_compare_exchange_n(p, &old, next, 0, MODEL, __ATOMIC_RELAXED)) return old;                         \
    }                                                                                                                 \
  }                                                                                                                   \
  static uint64_t NAME##_cas(void* addr, uint64_t expected, uint64_t desired) {                                       \
    UT e = (UT)expected;                                                                                              \
    __atomic_compare_exchange_n((UT*)addr, &e, (UT)desired, 0, MODEL, MODEL);                                         \
    return e; /* the value found in memory, which equals expected on success */                                        \
  }

DEFINE_AMO(amo8, uint8_t, int8_t)
DEFINE_AMO(amo16, uint16_t, int16_t)
DEFINE_AMO(amo32, uint32_t, int32_t)
DEFINE_AMO(amo64, uint64_t, int64_t)

/* ---- decoder -------------------------------------------------------------------------------------------------- */

/* x31 as a source is the zero register (value 0), as a destination it discards. Rn == 31 is SP. */
static uint64_t get_x(const ucontext_t* uc, unsigned r) { return r == 31 ? 0 : uc->uc_mcontext.regs[r]; }
static void set_x(ucontext_t* uc, unsigned r, uint64_t v) {
  if (r != 31) uc->uc_mcontext.regs[r] = v;
}
static uint64_t get_base(const ucontext_t* uc, unsigned r) { return r == 31 ? uc->uc_mcontext.sp : uc->uc_mcontext.regs[r]; }

/* Returns 1 if the instruction was one of ours and has been executed. */
static int emulate(ucontext_t* uc, uint32_t insn) {
  const unsigned size = insn >> 30; /* 0 byte, 1 half, 2 word, 3 double word */
  const unsigned rn = (insn >> 5) & 31;
  const unsigned rt = insn & 31;
  const unsigned rs = (insn >> 16) & 31;
  void* addr = (void*)get_base(uc, rn);
  uint64_t result;

  if ((insn & 0x3F200C00u) == 0x38200000u) {
    /* LD<op>, ST<op>, SWP: size 111 0 00 A R 1 Rs o3 opc 00 Rn Rt */
    const unsigned o3 = (insn >> 15) & 1;
    const unsigned opc = (insn >> 12) & 7;
    int op;
    if (o3 == 0) {
      op = (int)opc; /* ADD CLR EOR SET SMAX SMIN UMAX UMIN */
    } else if (opc == 0) {
      op = OP_SWP;
    } else {
      return 0; /* LDAPR and the like */
    }
    const uint64_t value = get_x(uc, rs);
    switch (size) {
      case 0: result = amo8(addr, op, value); break;
      case 1: result = amo16(addr, op, value); break;
      case 2: result = amo32(addr, op, value); break;
      default: result = amo64(addr, op, value); break;
    }
    set_x(uc, rt, result); /* ST<op> has Rt = xzr: the old value is dropped */
    return 1;
  }

  if ((insn & 0x3FA07C00u) == 0x08A07C00u) {
    /* CAS: size 001000 1 L 1 Rs o0 11111 Rn Rt. Rs holds the expected value and receives the old one. */
    const uint64_t expected = get_x(uc, rs);
    const uint64_t desired = get_x(uc, rt);
    switch (size) {
      case 0: result = amo8_cas(addr, expected, desired); break;
      case 1: result = amo16_cas(addr, expected, desired); break;
      case 2: result = amo32_cas(addr, expected, desired); break;
      default: result = amo64_cas(addr, expected, desired); break;
    }
    set_x(uc, rs, result);
    return 1;
  }
  return 0;
}

static void handler(int sig, siginfo_t* info, void* context) {
  ucontext_t* uc = (ucontext_t*)context;
  const uint64_t pc = uc->uc_mcontext.pc;
  uint32_t insn;
  memcpy(&insn, (const void*)pc, sizeof insn);
  if (emulate(uc, insn)) {
    __atomic_fetch_add(&g_count, 1, __ATOMIC_RELAXED);
    uc->uc_mcontext.pc = pc + 4;
    return;
  }
  /* Not ours: hand over to whoever was there before, or crash the normal way. */
  if (g_old.sa_flags & SA_SIGINFO) {
    if (g_old.sa_sigaction) {
      g_old.sa_sigaction(sig, info, context);
      return;
    }
  } else if (g_old.sa_handler != SIG_DFL && g_old.sa_handler != SIG_IGN) {
    g_old.sa_handler(sig);
    return;
  }
  /* Restore the default action and return: the faulting instruction runs again and kills the process. */
  struct sigaction dfl;
  memset(&dfl, 0, sizeof dfl);
  dfl.sa_handler = SIG_DFL;
  sigaction(SIGILL, &dfl, NULL);
}

static void report(void) {
  fprintf(stderr, "lse_emu: emulated %lu LSE instruction(s)\n", __atomic_load_n(&g_count, __ATOMIC_RELAXED));
}

/* For tests: 1 if the emulation is installed (the CPU has no LSE), 0 otherwise. */
int vmx_lse_emu_active(void) { return g_installed; }
unsigned long vmx_lse_emu_count(void) { return __atomic_load_n(&g_count, __ATOMIC_RELAXED); }

static void lse_emu_init(void) {
  if (g_installed) {
    return;
  }
  if (getauxval(AT_HWCAP) & HWCAP_ATOMICS) {
    return; /* the CPU runs these instructions itself */
  }
  struct sigaction sa;
  memset(&sa, 0, sizeof sa);
  sa.sa_sigaction = handler;
  sa.sa_flags = SA_SIGINFO | SA_RESTART;
  sigemptyset(&sa.sa_mask);
  if (sigaction(SIGILL, &sa, &g_old) == 0) {
    g_installed = 1;
    const char* dbg = getenv("VMX_LSE_EMU_DEBUG");
    if (dbg && *dbg == '1') {
      atexit(report);
    }
  }
}

__attribute__((constructor(101))) static void lse_emu_ctor(void) { lse_emu_init(); }

/* LD_AUDIT entry point: the loader calls this right after loading the audit library, before any other object is
 * initialised. Returns the audit interface version it accepts. */
unsigned int la_version(unsigned int version) {
  lse_emu_init();
  return version;
}
