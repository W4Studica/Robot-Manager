/* How much does one emulated LSE instruction cost? Run with the emulation loaded (make bench).
 * Compares a loop of emulated `ldaddal` / `casal` / `swpal` with the same work done by an ordinary exclusive load/store
 * loop (what the compiler generates for plain ARMv8.0), single threaded and with all CPUs doing it at once. */
#define _GNU_SOURCE
#include <pthread.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <sys/auxv.h>
#include <time.h>
#include <unistd.h>

static double now(void) {
  struct timespec ts;
  clock_gettime(CLOCK_MONOTONIC, &ts);
  return ts.tv_sec + ts.tv_nsec * 1e-9;
}

static void lse_add(uint64_t* p) {
  uint64_t one = 1, old;
  __asm__ volatile(".arch_extension lse\n\tldaddal %x[v], %x[r], [%[p]]" : [r] "=&r"(old) : [v] "r"(one), [p] "r"(p) : "memory");
}
static void lse_cas(uint64_t* p) {
  uint64_t cur = *p, s = cur;
  __asm__ volatile(".arch_extension lse\n\tcasal %x[s], %x[t], [%[p]]" : [s] "+&r"(s) : [t] "r"(cur + 1), [p] "r"(p) : "memory");
}
static void lse_swp(uint64_t* p) {
  uint64_t one = 1, old;
  __asm__ volatile(".arch_extension lse\n\tswpal %x[v], %x[r], [%[p]]" : [r] "=&r"(old) : [v] "r"(one), [p] "r"(p) : "memory");
}
static void plain_add(uint64_t* p) { __atomic_fetch_add(p, 1, __ATOMIC_SEQ_CST); }

typedef void (*op_fn)(uint64_t*);
struct job { op_fn fn; long n; double seconds; uint64_t word; };

static void* run(void* arg) {
  struct job* j = arg;
  const double t0 = now();
  for (long i = 0; i < j->n; ++i) j->fn(&j->word);
  j->seconds = now() - t0;
  return NULL;
}

static double per_op_ns(op_fn fn, long n, int threads) {
  struct job jobs[64];
  pthread_t t[64];
  for (int i = 0; i < threads; ++i) {
    jobs[i] = (struct job){fn, n, 0, 0};
  }
  for (int i = 0; i < threads; ++i) pthread_create(&t[i], NULL, run, &jobs[i]);
  for (int i = 0; i < threads; ++i) pthread_join(t[i], NULL);
  double worst = 0;
  for (int i = 0; i < threads; ++i) if (jobs[i].seconds > worst) worst = jobs[i].seconds;
  return worst / (double)n * 1e9;
}

int main(int argc, char** argv) {
  const long n = argc > 1 ? atol(argv[1]) : 200000;
  const int cpus = (int)sysconf(_SC_NPROCESSORS_ONLN);
  const int has_lse = (getauxval(AT_HWCAP) & (1UL << 8)) != 0;
  printf("CPU %s LSE; %ld operations per measurement, %d CPUs\n", has_lse ? "has" : "has no", n, cpus);
  if (has_lse) printf("(the LSE instructions run natively here: the numbers show the hardware, not the emulation)\n");

  const struct { const char* name; op_fn fn; } ops[] = {{"ldaddal", lse_add}, {"casal", lse_cas}, {"swpal", lse_swp}};
  const double base = per_op_ns(plain_add, n * 5, 1);
  printf("%-26s %10.1f ns each\n", "plain atomic add (no LSE)", base);
  for (size_t i = 0; i < sizeof ops / sizeof ops[0]; ++i) {
    const double one = per_op_ns(ops[i].fn, n, 1);
    const double all = per_op_ns(ops[i].fn, n, cpus);
    printf("%-26s %10.1f ns each (1 thread)  %10.1f ns each (%d threads at once)  = %.0fx plain\n", ops[i].name, one, all, cpus, one / base);
  }
  return 0;
}
