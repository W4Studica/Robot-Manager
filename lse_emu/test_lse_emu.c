/* Tests for lse_emu: every LSE instruction form it supports, checked against a plain C model, then a multi-thread
 * atomicity test and a "foreign SIGILL still crashes" test. GENERATED once from a script, edit by hand afterwards.
 * Runs on ARMv8 Linux: with LSE and RCpc hardware the instructions execute natively (emulation inactive). */
#define _GNU_SOURCE
#include <errno.h>
#include <pthread.h>
#include <signal.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/resource.h>
#include <sys/wait.h>
#include <sys/auxv.h>
#include <unistd.h>

extern int fake_mrclib_constructor_ran(void);

static int failures;
#define CHECK(cond, ...)                                                                                              \
  do {                                                                                                                \
    if (!(cond)) {                                                                                                    \
      ++failures;                                                                                                     \
      printf("FAIL: ");                                                                                               \
      printf(__VA_ARGS__);                                                                                            \
      printf("\n");                                                                                                   \
    }                                                                                                                 \
  } while (0)

static void t_ldaddb_1(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tldaddb %w[v], %w[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_ldaddh_2(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tldaddh %w[v], %w[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_ldadd_4(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tldadd %w[v], %w[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_ldadd_8(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tldadd %x[v], %x[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_ldaddab_1(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tldaddab %w[v], %w[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_ldaddah_2(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tldaddah %w[v], %w[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_ldadda_4(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tldadda %w[v], %w[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_ldadda_8(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tldadda %x[v], %x[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_ldaddlb_1(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tldaddlb %w[v], %w[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_ldaddlh_2(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tldaddlh %w[v], %w[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_ldaddl_4(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tldaddl %w[v], %w[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_ldaddl_8(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tldaddl %x[v], %x[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_ldaddalb_1(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tldaddalb %w[v], %w[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_ldaddalh_2(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tldaddalh %w[v], %w[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_ldaddal_4(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tldaddal %w[v], %w[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_ldaddal_8(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tldaddal %x[v], %x[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_staddb_1(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tstaddb %w[v], [%[p]]" : : [v] "r"(v), [p] "r"(p) : "memory"); *o = 0;
}

static void t_staddh_2(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tstaddh %w[v], [%[p]]" : : [v] "r"(v), [p] "r"(p) : "memory"); *o = 0;
}

static void t_stadd_4(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tstadd %w[v], [%[p]]" : : [v] "r"(v), [p] "r"(p) : "memory"); *o = 0;
}

static void t_stadd_8(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tstadd %x[v], [%[p]]" : : [v] "r"(v), [p] "r"(p) : "memory"); *o = 0;
}

static void t_staddlb_1(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tstaddlb %w[v], [%[p]]" : : [v] "r"(v), [p] "r"(p) : "memory"); *o = 0;
}

static void t_staddlh_2(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tstaddlh %w[v], [%[p]]" : : [v] "r"(v), [p] "r"(p) : "memory"); *o = 0;
}

static void t_staddl_4(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tstaddl %w[v], [%[p]]" : : [v] "r"(v), [p] "r"(p) : "memory"); *o = 0;
}

static void t_staddl_8(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tstaddl %x[v], [%[p]]" : : [v] "r"(v), [p] "r"(p) : "memory"); *o = 0;
}

static void t_ldclrb_1(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tldclrb %w[v], %w[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_ldclrh_2(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tldclrh %w[v], %w[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_ldclr_4(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tldclr %w[v], %w[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_ldclr_8(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tldclr %x[v], %x[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_ldclrab_1(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tldclrab %w[v], %w[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_ldclrah_2(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tldclrah %w[v], %w[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_ldclra_4(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tldclra %w[v], %w[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_ldclra_8(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tldclra %x[v], %x[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_ldclrlb_1(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tldclrlb %w[v], %w[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_ldclrlh_2(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tldclrlh %w[v], %w[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_ldclrl_4(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tldclrl %w[v], %w[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_ldclrl_8(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tldclrl %x[v], %x[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_ldclralb_1(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tldclralb %w[v], %w[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_ldclralh_2(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tldclralh %w[v], %w[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_ldclral_4(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tldclral %w[v], %w[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_ldclral_8(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tldclral %x[v], %x[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_stclrb_1(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tstclrb %w[v], [%[p]]" : : [v] "r"(v), [p] "r"(p) : "memory"); *o = 0;
}

static void t_stclrh_2(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tstclrh %w[v], [%[p]]" : : [v] "r"(v), [p] "r"(p) : "memory"); *o = 0;
}

static void t_stclr_4(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tstclr %w[v], [%[p]]" : : [v] "r"(v), [p] "r"(p) : "memory"); *o = 0;
}

static void t_stclr_8(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tstclr %x[v], [%[p]]" : : [v] "r"(v), [p] "r"(p) : "memory"); *o = 0;
}

static void t_stclrlb_1(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tstclrlb %w[v], [%[p]]" : : [v] "r"(v), [p] "r"(p) : "memory"); *o = 0;
}

static void t_stclrlh_2(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tstclrlh %w[v], [%[p]]" : : [v] "r"(v), [p] "r"(p) : "memory"); *o = 0;
}

static void t_stclrl_4(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tstclrl %w[v], [%[p]]" : : [v] "r"(v), [p] "r"(p) : "memory"); *o = 0;
}

static void t_stclrl_8(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tstclrl %x[v], [%[p]]" : : [v] "r"(v), [p] "r"(p) : "memory"); *o = 0;
}

static void t_ldeorb_1(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tldeorb %w[v], %w[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_ldeorh_2(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tldeorh %w[v], %w[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_ldeor_4(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tldeor %w[v], %w[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_ldeor_8(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tldeor %x[v], %x[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_ldeorab_1(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tldeorab %w[v], %w[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_ldeorah_2(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tldeorah %w[v], %w[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_ldeora_4(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tldeora %w[v], %w[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_ldeora_8(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tldeora %x[v], %x[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_ldeorlb_1(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tldeorlb %w[v], %w[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_ldeorlh_2(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tldeorlh %w[v], %w[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_ldeorl_4(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tldeorl %w[v], %w[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_ldeorl_8(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tldeorl %x[v], %x[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_ldeoralb_1(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tldeoralb %w[v], %w[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_ldeoralh_2(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tldeoralh %w[v], %w[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_ldeoral_4(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tldeoral %w[v], %w[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_ldeoral_8(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tldeoral %x[v], %x[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_steorb_1(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tsteorb %w[v], [%[p]]" : : [v] "r"(v), [p] "r"(p) : "memory"); *o = 0;
}

static void t_steorh_2(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tsteorh %w[v], [%[p]]" : : [v] "r"(v), [p] "r"(p) : "memory"); *o = 0;
}

static void t_steor_4(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tsteor %w[v], [%[p]]" : : [v] "r"(v), [p] "r"(p) : "memory"); *o = 0;
}

static void t_steor_8(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tsteor %x[v], [%[p]]" : : [v] "r"(v), [p] "r"(p) : "memory"); *o = 0;
}

static void t_steorlb_1(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tsteorlb %w[v], [%[p]]" : : [v] "r"(v), [p] "r"(p) : "memory"); *o = 0;
}

static void t_steorlh_2(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tsteorlh %w[v], [%[p]]" : : [v] "r"(v), [p] "r"(p) : "memory"); *o = 0;
}

static void t_steorl_4(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tsteorl %w[v], [%[p]]" : : [v] "r"(v), [p] "r"(p) : "memory"); *o = 0;
}

static void t_steorl_8(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tsteorl %x[v], [%[p]]" : : [v] "r"(v), [p] "r"(p) : "memory"); *o = 0;
}

static void t_ldsetb_1(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tldsetb %w[v], %w[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_ldseth_2(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tldseth %w[v], %w[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_ldset_4(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tldset %w[v], %w[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_ldset_8(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tldset %x[v], %x[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_ldsetab_1(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tldsetab %w[v], %w[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_ldsetah_2(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tldsetah %w[v], %w[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_ldseta_4(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tldseta %w[v], %w[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_ldseta_8(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tldseta %x[v], %x[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_ldsetlb_1(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tldsetlb %w[v], %w[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_ldsetlh_2(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tldsetlh %w[v], %w[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_ldsetl_4(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tldsetl %w[v], %w[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_ldsetl_8(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tldsetl %x[v], %x[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_ldsetalb_1(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tldsetalb %w[v], %w[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_ldsetalh_2(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tldsetalh %w[v], %w[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_ldsetal_4(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tldsetal %w[v], %w[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_ldsetal_8(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tldsetal %x[v], %x[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_stsetb_1(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tstsetb %w[v], [%[p]]" : : [v] "r"(v), [p] "r"(p) : "memory"); *o = 0;
}

static void t_stseth_2(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tstseth %w[v], [%[p]]" : : [v] "r"(v), [p] "r"(p) : "memory"); *o = 0;
}

static void t_stset_4(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tstset %w[v], [%[p]]" : : [v] "r"(v), [p] "r"(p) : "memory"); *o = 0;
}

static void t_stset_8(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tstset %x[v], [%[p]]" : : [v] "r"(v), [p] "r"(p) : "memory"); *o = 0;
}

static void t_stsetlb_1(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tstsetlb %w[v], [%[p]]" : : [v] "r"(v), [p] "r"(p) : "memory"); *o = 0;
}

static void t_stsetlh_2(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tstsetlh %w[v], [%[p]]" : : [v] "r"(v), [p] "r"(p) : "memory"); *o = 0;
}

static void t_stsetl_4(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tstsetl %w[v], [%[p]]" : : [v] "r"(v), [p] "r"(p) : "memory"); *o = 0;
}

static void t_stsetl_8(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tstsetl %x[v], [%[p]]" : : [v] "r"(v), [p] "r"(p) : "memory"); *o = 0;
}

static void t_ldsmaxb_1(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tldsmaxb %w[v], %w[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_ldsmaxh_2(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tldsmaxh %w[v], %w[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_ldsmax_4(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tldsmax %w[v], %w[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_ldsmax_8(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tldsmax %x[v], %x[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_ldsmaxab_1(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tldsmaxab %w[v], %w[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_ldsmaxah_2(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tldsmaxah %w[v], %w[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_ldsmaxa_4(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tldsmaxa %w[v], %w[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_ldsmaxa_8(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tldsmaxa %x[v], %x[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_ldsmaxlb_1(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tldsmaxlb %w[v], %w[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_ldsmaxlh_2(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tldsmaxlh %w[v], %w[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_ldsmaxl_4(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tldsmaxl %w[v], %w[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_ldsmaxl_8(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tldsmaxl %x[v], %x[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_ldsmaxalb_1(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tldsmaxalb %w[v], %w[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_ldsmaxalh_2(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tldsmaxalh %w[v], %w[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_ldsmaxal_4(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tldsmaxal %w[v], %w[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_ldsmaxal_8(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tldsmaxal %x[v], %x[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_stsmaxb_1(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tstsmaxb %w[v], [%[p]]" : : [v] "r"(v), [p] "r"(p) : "memory"); *o = 0;
}

static void t_stsmaxh_2(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tstsmaxh %w[v], [%[p]]" : : [v] "r"(v), [p] "r"(p) : "memory"); *o = 0;
}

static void t_stsmax_4(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tstsmax %w[v], [%[p]]" : : [v] "r"(v), [p] "r"(p) : "memory"); *o = 0;
}

static void t_stsmax_8(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tstsmax %x[v], [%[p]]" : : [v] "r"(v), [p] "r"(p) : "memory"); *o = 0;
}

static void t_stsmaxlb_1(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tstsmaxlb %w[v], [%[p]]" : : [v] "r"(v), [p] "r"(p) : "memory"); *o = 0;
}

static void t_stsmaxlh_2(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tstsmaxlh %w[v], [%[p]]" : : [v] "r"(v), [p] "r"(p) : "memory"); *o = 0;
}

static void t_stsmaxl_4(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tstsmaxl %w[v], [%[p]]" : : [v] "r"(v), [p] "r"(p) : "memory"); *o = 0;
}

static void t_stsmaxl_8(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tstsmaxl %x[v], [%[p]]" : : [v] "r"(v), [p] "r"(p) : "memory"); *o = 0;
}

static void t_ldsminb_1(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tldsminb %w[v], %w[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_ldsminh_2(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tldsminh %w[v], %w[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_ldsmin_4(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tldsmin %w[v], %w[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_ldsmin_8(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tldsmin %x[v], %x[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_ldsminab_1(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tldsminab %w[v], %w[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_ldsminah_2(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tldsminah %w[v], %w[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_ldsmina_4(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tldsmina %w[v], %w[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_ldsmina_8(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tldsmina %x[v], %x[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_ldsminlb_1(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tldsminlb %w[v], %w[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_ldsminlh_2(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tldsminlh %w[v], %w[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_ldsminl_4(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tldsminl %w[v], %w[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_ldsminl_8(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tldsminl %x[v], %x[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_ldsminalb_1(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tldsminalb %w[v], %w[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_ldsminalh_2(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tldsminalh %w[v], %w[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_ldsminal_4(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tldsminal %w[v], %w[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_ldsminal_8(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tldsminal %x[v], %x[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_stsminb_1(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tstsminb %w[v], [%[p]]" : : [v] "r"(v), [p] "r"(p) : "memory"); *o = 0;
}

static void t_stsminh_2(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tstsminh %w[v], [%[p]]" : : [v] "r"(v), [p] "r"(p) : "memory"); *o = 0;
}

static void t_stsmin_4(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tstsmin %w[v], [%[p]]" : : [v] "r"(v), [p] "r"(p) : "memory"); *o = 0;
}

static void t_stsmin_8(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tstsmin %x[v], [%[p]]" : : [v] "r"(v), [p] "r"(p) : "memory"); *o = 0;
}

static void t_stsminlb_1(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tstsminlb %w[v], [%[p]]" : : [v] "r"(v), [p] "r"(p) : "memory"); *o = 0;
}

static void t_stsminlh_2(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tstsminlh %w[v], [%[p]]" : : [v] "r"(v), [p] "r"(p) : "memory"); *o = 0;
}

static void t_stsminl_4(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tstsminl %w[v], [%[p]]" : : [v] "r"(v), [p] "r"(p) : "memory"); *o = 0;
}

static void t_stsminl_8(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tstsminl %x[v], [%[p]]" : : [v] "r"(v), [p] "r"(p) : "memory"); *o = 0;
}

static void t_ldumaxb_1(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tldumaxb %w[v], %w[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_ldumaxh_2(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tldumaxh %w[v], %w[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_ldumax_4(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tldumax %w[v], %w[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_ldumax_8(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tldumax %x[v], %x[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_ldumaxab_1(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tldumaxab %w[v], %w[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_ldumaxah_2(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tldumaxah %w[v], %w[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_ldumaxa_4(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tldumaxa %w[v], %w[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_ldumaxa_8(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tldumaxa %x[v], %x[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_ldumaxlb_1(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tldumaxlb %w[v], %w[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_ldumaxlh_2(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tldumaxlh %w[v], %w[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_ldumaxl_4(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tldumaxl %w[v], %w[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_ldumaxl_8(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tldumaxl %x[v], %x[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_ldumaxalb_1(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tldumaxalb %w[v], %w[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_ldumaxalh_2(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tldumaxalh %w[v], %w[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_ldumaxal_4(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tldumaxal %w[v], %w[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_ldumaxal_8(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tldumaxal %x[v], %x[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_stumaxb_1(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tstumaxb %w[v], [%[p]]" : : [v] "r"(v), [p] "r"(p) : "memory"); *o = 0;
}

static void t_stumaxh_2(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tstumaxh %w[v], [%[p]]" : : [v] "r"(v), [p] "r"(p) : "memory"); *o = 0;
}

static void t_stumax_4(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tstumax %w[v], [%[p]]" : : [v] "r"(v), [p] "r"(p) : "memory"); *o = 0;
}

static void t_stumax_8(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tstumax %x[v], [%[p]]" : : [v] "r"(v), [p] "r"(p) : "memory"); *o = 0;
}

static void t_stumaxlb_1(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tstumaxlb %w[v], [%[p]]" : : [v] "r"(v), [p] "r"(p) : "memory"); *o = 0;
}

static void t_stumaxlh_2(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tstumaxlh %w[v], [%[p]]" : : [v] "r"(v), [p] "r"(p) : "memory"); *o = 0;
}

static void t_stumaxl_4(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tstumaxl %w[v], [%[p]]" : : [v] "r"(v), [p] "r"(p) : "memory"); *o = 0;
}

static void t_stumaxl_8(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tstumaxl %x[v], [%[p]]" : : [v] "r"(v), [p] "r"(p) : "memory"); *o = 0;
}

static void t_lduminb_1(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tlduminb %w[v], %w[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_lduminh_2(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tlduminh %w[v], %w[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_ldumin_4(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tldumin %w[v], %w[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_ldumin_8(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tldumin %x[v], %x[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_lduminab_1(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tlduminab %w[v], %w[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_lduminah_2(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tlduminah %w[v], %w[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_ldumina_4(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tldumina %w[v], %w[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_ldumina_8(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tldumina %x[v], %x[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_lduminlb_1(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tlduminlb %w[v], %w[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_lduminlh_2(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tlduminlh %w[v], %w[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_lduminl_4(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tlduminl %w[v], %w[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_lduminl_8(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tlduminl %x[v], %x[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_lduminalb_1(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tlduminalb %w[v], %w[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_lduminalh_2(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tlduminalh %w[v], %w[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_lduminal_4(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tlduminal %w[v], %w[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_lduminal_8(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tlduminal %x[v], %x[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_stuminb_1(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tstuminb %w[v], [%[p]]" : : [v] "r"(v), [p] "r"(p) : "memory"); *o = 0;
}

static void t_stuminh_2(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tstuminh %w[v], [%[p]]" : : [v] "r"(v), [p] "r"(p) : "memory"); *o = 0;
}

static void t_stumin_4(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tstumin %w[v], [%[p]]" : : [v] "r"(v), [p] "r"(p) : "memory"); *o = 0;
}

static void t_stumin_8(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tstumin %x[v], [%[p]]" : : [v] "r"(v), [p] "r"(p) : "memory"); *o = 0;
}

static void t_stuminlb_1(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tstuminlb %w[v], [%[p]]" : : [v] "r"(v), [p] "r"(p) : "memory"); *o = 0;
}

static void t_stuminlh_2(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tstuminlh %w[v], [%[p]]" : : [v] "r"(v), [p] "r"(p) : "memory"); *o = 0;
}

static void t_stuminl_4(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tstuminl %w[v], [%[p]]" : : [v] "r"(v), [p] "r"(p) : "memory"); *o = 0;
}

static void t_stuminl_8(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tstuminl %x[v], [%[p]]" : : [v] "r"(v), [p] "r"(p) : "memory"); *o = 0;
}

static void t_swpb_1(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tswpb %w[v], %w[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_swph_2(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tswph %w[v], %w[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_swp_4(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tswp %w[v], %w[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_swp_8(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tswp %x[v], %x[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_swpab_1(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tswpab %w[v], %w[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_swpah_2(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tswpah %w[v], %w[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_swpa_4(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tswpa %w[v], %w[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_swpa_8(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tswpa %x[v], %x[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_swplb_1(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tswplb %w[v], %w[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_swplh_2(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tswplh %w[v], %w[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_swpl_4(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tswpl %w[v], %w[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_swpl_8(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tswpl %x[v], %x[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_swpalb_1(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tswpalb %w[v], %w[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_swpalh_2(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tswpalh %w[v], %w[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_swpal_4(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tswpal %w[v], %w[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void t_swpal_8(void* p, uint64_t v, uint64_t* o) {
  uint64_t r = 0; (void)r;
  __asm__ volatile(".arch_extension lse\n\tswpal %x[v], %x[r], [%[p]]" : [r] "=&r"(r) : [v] "r"(v), [p] "r"(p) : "memory"); *o = r;
}

static void c_casb_1(void* p, uint64_t expected, uint64_t desired, uint64_t* o) {
  uint64_t s = expected;
  __asm__ volatile(".arch_extension lse\n\tcasb %w[s], %w[t], [%[p]]" : [s] "+&r"(s) : [t] "r"(desired), [p] "r"(p) : "memory");
  *o = s;
}

static void c_cash_2(void* p, uint64_t expected, uint64_t desired, uint64_t* o) {
  uint64_t s = expected;
  __asm__ volatile(".arch_extension lse\n\tcash %w[s], %w[t], [%[p]]" : [s] "+&r"(s) : [t] "r"(desired), [p] "r"(p) : "memory");
  *o = s;
}

static void c_cas_4(void* p, uint64_t expected, uint64_t desired, uint64_t* o) {
  uint64_t s = expected;
  __asm__ volatile(".arch_extension lse\n\tcas %w[s], %w[t], [%[p]]" : [s] "+&r"(s) : [t] "r"(desired), [p] "r"(p) : "memory");
  *o = s;
}

static void c_cas_8(void* p, uint64_t expected, uint64_t desired, uint64_t* o) {
  uint64_t s = expected;
  __asm__ volatile(".arch_extension lse\n\tcas %x[s], %x[t], [%[p]]" : [s] "+&r"(s) : [t] "r"(desired), [p] "r"(p) : "memory");
  *o = s;
}

static void c_casab_1(void* p, uint64_t expected, uint64_t desired, uint64_t* o) {
  uint64_t s = expected;
  __asm__ volatile(".arch_extension lse\n\tcasab %w[s], %w[t], [%[p]]" : [s] "+&r"(s) : [t] "r"(desired), [p] "r"(p) : "memory");
  *o = s;
}

static void c_casah_2(void* p, uint64_t expected, uint64_t desired, uint64_t* o) {
  uint64_t s = expected;
  __asm__ volatile(".arch_extension lse\n\tcasah %w[s], %w[t], [%[p]]" : [s] "+&r"(s) : [t] "r"(desired), [p] "r"(p) : "memory");
  *o = s;
}

static void c_casa_4(void* p, uint64_t expected, uint64_t desired, uint64_t* o) {
  uint64_t s = expected;
  __asm__ volatile(".arch_extension lse\n\tcasa %w[s], %w[t], [%[p]]" : [s] "+&r"(s) : [t] "r"(desired), [p] "r"(p) : "memory");
  *o = s;
}

static void c_casa_8(void* p, uint64_t expected, uint64_t desired, uint64_t* o) {
  uint64_t s = expected;
  __asm__ volatile(".arch_extension lse\n\tcasa %x[s], %x[t], [%[p]]" : [s] "+&r"(s) : [t] "r"(desired), [p] "r"(p) : "memory");
  *o = s;
}

static void c_caslb_1(void* p, uint64_t expected, uint64_t desired, uint64_t* o) {
  uint64_t s = expected;
  __asm__ volatile(".arch_extension lse\n\tcaslb %w[s], %w[t], [%[p]]" : [s] "+&r"(s) : [t] "r"(desired), [p] "r"(p) : "memory");
  *o = s;
}

static void c_caslh_2(void* p, uint64_t expected, uint64_t desired, uint64_t* o) {
  uint64_t s = expected;
  __asm__ volatile(".arch_extension lse\n\tcaslh %w[s], %w[t], [%[p]]" : [s] "+&r"(s) : [t] "r"(desired), [p] "r"(p) : "memory");
  *o = s;
}

static void c_casl_4(void* p, uint64_t expected, uint64_t desired, uint64_t* o) {
  uint64_t s = expected;
  __asm__ volatile(".arch_extension lse\n\tcasl %w[s], %w[t], [%[p]]" : [s] "+&r"(s) : [t] "r"(desired), [p] "r"(p) : "memory");
  *o = s;
}

static void c_casl_8(void* p, uint64_t expected, uint64_t desired, uint64_t* o) {
  uint64_t s = expected;
  __asm__ volatile(".arch_extension lse\n\tcasl %x[s], %x[t], [%[p]]" : [s] "+&r"(s) : [t] "r"(desired), [p] "r"(p) : "memory");
  *o = s;
}

static void c_casalb_1(void* p, uint64_t expected, uint64_t desired, uint64_t* o) {
  uint64_t s = expected;
  __asm__ volatile(".arch_extension lse\n\tcasalb %w[s], %w[t], [%[p]]" : [s] "+&r"(s) : [t] "r"(desired), [p] "r"(p) : "memory");
  *o = s;
}

static void c_casalh_2(void* p, uint64_t expected, uint64_t desired, uint64_t* o) {
  uint64_t s = expected;
  __asm__ volatile(".arch_extension lse\n\tcasalh %w[s], %w[t], [%[p]]" : [s] "+&r"(s) : [t] "r"(desired), [p] "r"(p) : "memory");
  *o = s;
}

static void c_casal_4(void* p, uint64_t expected, uint64_t desired, uint64_t* o) {
  uint64_t s = expected;
  __asm__ volatile(".arch_extension lse\n\tcasal %w[s], %w[t], [%[p]]" : [s] "+&r"(s) : [t] "r"(desired), [p] "r"(p) : "memory");
  *o = s;
}

static void c_casal_8(void* p, uint64_t expected, uint64_t desired, uint64_t* o) {
  uint64_t s = expected;
  __asm__ volatile(".arch_extension lse\n\tcasal %x[s], %x[t], [%[p]]" : [s] "+&r"(s) : [t] "r"(desired), [p] "r"(p) : "memory");
  *o = s;
}

typedef void (*amo_fn)(void*, uint64_t, uint64_t*);
typedef void (*cas_fn)(void*, uint64_t, uint64_t, uint64_t*);
static const struct { const char* name; int bytes; int op; amo_fn fn; int has_result; } kAmo[] = {
    {"ldaddb", 1, 0, t_ldaddb_1, 1},
    {"ldaddh", 2, 0, t_ldaddh_2, 1},
    {"ldadd", 4, 0, t_ldadd_4, 1},
    {"ldadd", 8, 0, t_ldadd_8, 1},
    {"ldaddab", 1, 0, t_ldaddab_1, 1},
    {"ldaddah", 2, 0, t_ldaddah_2, 1},
    {"ldadda", 4, 0, t_ldadda_4, 1},
    {"ldadda", 8, 0, t_ldadda_8, 1},
    {"ldaddlb", 1, 0, t_ldaddlb_1, 1},
    {"ldaddlh", 2, 0, t_ldaddlh_2, 1},
    {"ldaddl", 4, 0, t_ldaddl_4, 1},
    {"ldaddl", 8, 0, t_ldaddl_8, 1},
    {"ldaddalb", 1, 0, t_ldaddalb_1, 1},
    {"ldaddalh", 2, 0, t_ldaddalh_2, 1},
    {"ldaddal", 4, 0, t_ldaddal_4, 1},
    {"ldaddal", 8, 0, t_ldaddal_8, 1},
    {"staddb", 1, 0, t_staddb_1, 0},
    {"staddh", 2, 0, t_staddh_2, 0},
    {"stadd", 4, 0, t_stadd_4, 0},
    {"stadd", 8, 0, t_stadd_8, 0},
    {"staddlb", 1, 0, t_staddlb_1, 0},
    {"staddlh", 2, 0, t_staddlh_2, 0},
    {"staddl", 4, 0, t_staddl_4, 0},
    {"staddl", 8, 0, t_staddl_8, 0},
    {"ldclrb", 1, 1, t_ldclrb_1, 1},
    {"ldclrh", 2, 1, t_ldclrh_2, 1},
    {"ldclr", 4, 1, t_ldclr_4, 1},
    {"ldclr", 8, 1, t_ldclr_8, 1},
    {"ldclrab", 1, 1, t_ldclrab_1, 1},
    {"ldclrah", 2, 1, t_ldclrah_2, 1},
    {"ldclra", 4, 1, t_ldclra_4, 1},
    {"ldclra", 8, 1, t_ldclra_8, 1},
    {"ldclrlb", 1, 1, t_ldclrlb_1, 1},
    {"ldclrlh", 2, 1, t_ldclrlh_2, 1},
    {"ldclrl", 4, 1, t_ldclrl_4, 1},
    {"ldclrl", 8, 1, t_ldclrl_8, 1},
    {"ldclralb", 1, 1, t_ldclralb_1, 1},
    {"ldclralh", 2, 1, t_ldclralh_2, 1},
    {"ldclral", 4, 1, t_ldclral_4, 1},
    {"ldclral", 8, 1, t_ldclral_8, 1},
    {"stclrb", 1, 1, t_stclrb_1, 0},
    {"stclrh", 2, 1, t_stclrh_2, 0},
    {"stclr", 4, 1, t_stclr_4, 0},
    {"stclr", 8, 1, t_stclr_8, 0},
    {"stclrlb", 1, 1, t_stclrlb_1, 0},
    {"stclrlh", 2, 1, t_stclrlh_2, 0},
    {"stclrl", 4, 1, t_stclrl_4, 0},
    {"stclrl", 8, 1, t_stclrl_8, 0},
    {"ldeorb", 1, 2, t_ldeorb_1, 1},
    {"ldeorh", 2, 2, t_ldeorh_2, 1},
    {"ldeor", 4, 2, t_ldeor_4, 1},
    {"ldeor", 8, 2, t_ldeor_8, 1},
    {"ldeorab", 1, 2, t_ldeorab_1, 1},
    {"ldeorah", 2, 2, t_ldeorah_2, 1},
    {"ldeora", 4, 2, t_ldeora_4, 1},
    {"ldeora", 8, 2, t_ldeora_8, 1},
    {"ldeorlb", 1, 2, t_ldeorlb_1, 1},
    {"ldeorlh", 2, 2, t_ldeorlh_2, 1},
    {"ldeorl", 4, 2, t_ldeorl_4, 1},
    {"ldeorl", 8, 2, t_ldeorl_8, 1},
    {"ldeoralb", 1, 2, t_ldeoralb_1, 1},
    {"ldeoralh", 2, 2, t_ldeoralh_2, 1},
    {"ldeoral", 4, 2, t_ldeoral_4, 1},
    {"ldeoral", 8, 2, t_ldeoral_8, 1},
    {"steorb", 1, 2, t_steorb_1, 0},
    {"steorh", 2, 2, t_steorh_2, 0},
    {"steor", 4, 2, t_steor_4, 0},
    {"steor", 8, 2, t_steor_8, 0},
    {"steorlb", 1, 2, t_steorlb_1, 0},
    {"steorlh", 2, 2, t_steorlh_2, 0},
    {"steorl", 4, 2, t_steorl_4, 0},
    {"steorl", 8, 2, t_steorl_8, 0},
    {"ldsetb", 1, 3, t_ldsetb_1, 1},
    {"ldseth", 2, 3, t_ldseth_2, 1},
    {"ldset", 4, 3, t_ldset_4, 1},
    {"ldset", 8, 3, t_ldset_8, 1},
    {"ldsetab", 1, 3, t_ldsetab_1, 1},
    {"ldsetah", 2, 3, t_ldsetah_2, 1},
    {"ldseta", 4, 3, t_ldseta_4, 1},
    {"ldseta", 8, 3, t_ldseta_8, 1},
    {"ldsetlb", 1, 3, t_ldsetlb_1, 1},
    {"ldsetlh", 2, 3, t_ldsetlh_2, 1},
    {"ldsetl", 4, 3, t_ldsetl_4, 1},
    {"ldsetl", 8, 3, t_ldsetl_8, 1},
    {"ldsetalb", 1, 3, t_ldsetalb_1, 1},
    {"ldsetalh", 2, 3, t_ldsetalh_2, 1},
    {"ldsetal", 4, 3, t_ldsetal_4, 1},
    {"ldsetal", 8, 3, t_ldsetal_8, 1},
    {"stsetb", 1, 3, t_stsetb_1, 0},
    {"stseth", 2, 3, t_stseth_2, 0},
    {"stset", 4, 3, t_stset_4, 0},
    {"stset", 8, 3, t_stset_8, 0},
    {"stsetlb", 1, 3, t_stsetlb_1, 0},
    {"stsetlh", 2, 3, t_stsetlh_2, 0},
    {"stsetl", 4, 3, t_stsetl_4, 0},
    {"stsetl", 8, 3, t_stsetl_8, 0},
    {"ldsmaxb", 1, 4, t_ldsmaxb_1, 1},
    {"ldsmaxh", 2, 4, t_ldsmaxh_2, 1},
    {"ldsmax", 4, 4, t_ldsmax_4, 1},
    {"ldsmax", 8, 4, t_ldsmax_8, 1},
    {"ldsmaxab", 1, 4, t_ldsmaxab_1, 1},
    {"ldsmaxah", 2, 4, t_ldsmaxah_2, 1},
    {"ldsmaxa", 4, 4, t_ldsmaxa_4, 1},
    {"ldsmaxa", 8, 4, t_ldsmaxa_8, 1},
    {"ldsmaxlb", 1, 4, t_ldsmaxlb_1, 1},
    {"ldsmaxlh", 2, 4, t_ldsmaxlh_2, 1},
    {"ldsmaxl", 4, 4, t_ldsmaxl_4, 1},
    {"ldsmaxl", 8, 4, t_ldsmaxl_8, 1},
    {"ldsmaxalb", 1, 4, t_ldsmaxalb_1, 1},
    {"ldsmaxalh", 2, 4, t_ldsmaxalh_2, 1},
    {"ldsmaxal", 4, 4, t_ldsmaxal_4, 1},
    {"ldsmaxal", 8, 4, t_ldsmaxal_8, 1},
    {"stsmaxb", 1, 4, t_stsmaxb_1, 0},
    {"stsmaxh", 2, 4, t_stsmaxh_2, 0},
    {"stsmax", 4, 4, t_stsmax_4, 0},
    {"stsmax", 8, 4, t_stsmax_8, 0},
    {"stsmaxlb", 1, 4, t_stsmaxlb_1, 0},
    {"stsmaxlh", 2, 4, t_stsmaxlh_2, 0},
    {"stsmaxl", 4, 4, t_stsmaxl_4, 0},
    {"stsmaxl", 8, 4, t_stsmaxl_8, 0},
    {"ldsminb", 1, 5, t_ldsminb_1, 1},
    {"ldsminh", 2, 5, t_ldsminh_2, 1},
    {"ldsmin", 4, 5, t_ldsmin_4, 1},
    {"ldsmin", 8, 5, t_ldsmin_8, 1},
    {"ldsminab", 1, 5, t_ldsminab_1, 1},
    {"ldsminah", 2, 5, t_ldsminah_2, 1},
    {"ldsmina", 4, 5, t_ldsmina_4, 1},
    {"ldsmina", 8, 5, t_ldsmina_8, 1},
    {"ldsminlb", 1, 5, t_ldsminlb_1, 1},
    {"ldsminlh", 2, 5, t_ldsminlh_2, 1},
    {"ldsminl", 4, 5, t_ldsminl_4, 1},
    {"ldsminl", 8, 5, t_ldsminl_8, 1},
    {"ldsminalb", 1, 5, t_ldsminalb_1, 1},
    {"ldsminalh", 2, 5, t_ldsminalh_2, 1},
    {"ldsminal", 4, 5, t_ldsminal_4, 1},
    {"ldsminal", 8, 5, t_ldsminal_8, 1},
    {"stsminb", 1, 5, t_stsminb_1, 0},
    {"stsminh", 2, 5, t_stsminh_2, 0},
    {"stsmin", 4, 5, t_stsmin_4, 0},
    {"stsmin", 8, 5, t_stsmin_8, 0},
    {"stsminlb", 1, 5, t_stsminlb_1, 0},
    {"stsminlh", 2, 5, t_stsminlh_2, 0},
    {"stsminl", 4, 5, t_stsminl_4, 0},
    {"stsminl", 8, 5, t_stsminl_8, 0},
    {"ldumaxb", 1, 6, t_ldumaxb_1, 1},
    {"ldumaxh", 2, 6, t_ldumaxh_2, 1},
    {"ldumax", 4, 6, t_ldumax_4, 1},
    {"ldumax", 8, 6, t_ldumax_8, 1},
    {"ldumaxab", 1, 6, t_ldumaxab_1, 1},
    {"ldumaxah", 2, 6, t_ldumaxah_2, 1},
    {"ldumaxa", 4, 6, t_ldumaxa_4, 1},
    {"ldumaxa", 8, 6, t_ldumaxa_8, 1},
    {"ldumaxlb", 1, 6, t_ldumaxlb_1, 1},
    {"ldumaxlh", 2, 6, t_ldumaxlh_2, 1},
    {"ldumaxl", 4, 6, t_ldumaxl_4, 1},
    {"ldumaxl", 8, 6, t_ldumaxl_8, 1},
    {"ldumaxalb", 1, 6, t_ldumaxalb_1, 1},
    {"ldumaxalh", 2, 6, t_ldumaxalh_2, 1},
    {"ldumaxal", 4, 6, t_ldumaxal_4, 1},
    {"ldumaxal", 8, 6, t_ldumaxal_8, 1},
    {"stumaxb", 1, 6, t_stumaxb_1, 0},
    {"stumaxh", 2, 6, t_stumaxh_2, 0},
    {"stumax", 4, 6, t_stumax_4, 0},
    {"stumax", 8, 6, t_stumax_8, 0},
    {"stumaxlb", 1, 6, t_stumaxlb_1, 0},
    {"stumaxlh", 2, 6, t_stumaxlh_2, 0},
    {"stumaxl", 4, 6, t_stumaxl_4, 0},
    {"stumaxl", 8, 6, t_stumaxl_8, 0},
    {"lduminb", 1, 7, t_lduminb_1, 1},
    {"lduminh", 2, 7, t_lduminh_2, 1},
    {"ldumin", 4, 7, t_ldumin_4, 1},
    {"ldumin", 8, 7, t_ldumin_8, 1},
    {"lduminab", 1, 7, t_lduminab_1, 1},
    {"lduminah", 2, 7, t_lduminah_2, 1},
    {"ldumina", 4, 7, t_ldumina_4, 1},
    {"ldumina", 8, 7, t_ldumina_8, 1},
    {"lduminlb", 1, 7, t_lduminlb_1, 1},
    {"lduminlh", 2, 7, t_lduminlh_2, 1},
    {"lduminl", 4, 7, t_lduminl_4, 1},
    {"lduminl", 8, 7, t_lduminl_8, 1},
    {"lduminalb", 1, 7, t_lduminalb_1, 1},
    {"lduminalh", 2, 7, t_lduminalh_2, 1},
    {"lduminal", 4, 7, t_lduminal_4, 1},
    {"lduminal", 8, 7, t_lduminal_8, 1},
    {"stuminb", 1, 7, t_stuminb_1, 0},
    {"stuminh", 2, 7, t_stuminh_2, 0},
    {"stumin", 4, 7, t_stumin_4, 0},
    {"stumin", 8, 7, t_stumin_8, 0},
    {"stuminlb", 1, 7, t_stuminlb_1, 0},
    {"stuminlh", 2, 7, t_stuminlh_2, 0},
    {"stuminl", 4, 7, t_stuminl_4, 0},
    {"stuminl", 8, 7, t_stuminl_8, 0},
    {"swpb", 1, 8, t_swpb_1, 1},
    {"swph", 2, 8, t_swph_2, 1},
    {"swp", 4, 8, t_swp_4, 1},
    {"swp", 8, 8, t_swp_8, 1},
    {"swpab", 1, 8, t_swpab_1, 1},
    {"swpah", 2, 8, t_swpah_2, 1},
    {"swpa", 4, 8, t_swpa_4, 1},
    {"swpa", 8, 8, t_swpa_8, 1},
    {"swplb", 1, 8, t_swplb_1, 1},
    {"swplh", 2, 8, t_swplh_2, 1},
    {"swpl", 4, 8, t_swpl_4, 1},
    {"swpl", 8, 8, t_swpl_8, 1},
    {"swpalb", 1, 8, t_swpalb_1, 1},
    {"swpalh", 2, 8, t_swpalh_2, 1},
    {"swpal", 4, 8, t_swpal_4, 1},
    {"swpal", 8, 8, t_swpal_8, 1},
};
static const struct { const char* name; int bytes; cas_fn fn; } kCas[] = {
    {"casb", 1, c_casb_1},
    {"cash", 2, c_cash_2},
    {"cas", 4, c_cas_4},
    {"cas", 8, c_cas_8},
    {"casab", 1, c_casab_1},
    {"casah", 2, c_casah_2},
    {"casa", 4, c_casa_4},
    {"casa", 8, c_casa_8},
    {"caslb", 1, c_caslb_1},
    {"caslh", 2, c_caslh_2},
    {"casl", 4, c_casl_4},
    {"casl", 8, c_casl_8},
    {"casalb", 1, c_casalb_1},
    {"casalh", 2, c_casalh_2},
    {"casal", 4, c_casal_4},
    {"casal", 8, c_casal_8},
};

static uint64_t mask_of(int bytes) { return bytes == 8 ? ~0ull : ((1ull << (bytes * 8)) - 1); }
static int64_t sext(uint64_t v, int bytes) {
  const int shift = 64 - bytes * 8;
  return (int64_t)(v << shift) >> shift;
}

/* op: 0 add 1 clr 2 eor 3 set 4 smax 5 smin 6 umax 7 umin 8 swp */
static uint64_t model(int op, int bytes, uint64_t old, uint64_t v) {
  const uint64_t m = mask_of(bytes);
  old &= m;
  v &= m;
  switch (op) {
    case 0: return (old + v) & m;
    case 1: return old & ~v & m;
    case 2: return (old ^ v) & m;
    case 3: return (old | v) & m;
    case 4: return sext(old, bytes) > sext(v, bytes) ? old : v;
    case 5: return sext(old, bytes) < sext(v, bytes) ? old : v;
    case 6: return old > v ? old : v;
    case 7: return old < v ? old : v;
    default: return v;
  }
}

static void test_forms(void) {
  const uint64_t patterns[] = {0x0123456789abcdefull, 0xfffffffffffffff0ull, 0x8000000000000001ull, 0x7fffffffffffffffull,
                               0x00000000000000ffull};
  const uint64_t operands[] = {0x1ull, 0x80ull, 0x8000ull, 0x80000000ull, 0xfedcba9876543210ull, 0xffffffffffffffffull};
  for (size_t i = 0; i < sizeof kAmo / sizeof kAmo[0]; ++i) {
    for (size_t a = 0; a < sizeof patterns / sizeof patterns[0]; ++a) {
      for (size_t b = 0; b < sizeof operands / sizeof operands[0]; ++b) {
        /* Eight bytes of memory; the instruction touches only the first `bytes`: the rest must survive. */
        _Alignas(8) uint8_t mem[16];
        memset(mem, 0xA5, sizeof mem);
        memcpy(mem, &patterns[a], 8);
        uint64_t before;
        memcpy(&before, mem, 8);
        uint64_t old = 0xdeadbeefdeadbeefull;
        kAmo[i].fn(mem, operands[b], &old);
        const uint64_t m = mask_of(kAmo[i].bytes);
        uint64_t expect_mem = (before & ~m) | model(kAmo[i].op, kAmo[i].bytes, before, operands[b]);
        uint64_t after;
        memcpy(&after, mem, 8);
        CHECK(after == expect_mem, "%s memory: got %016llx expected %016llx (mem %016llx, v %016llx)", kAmo[i].name,
              (unsigned long long)after, (unsigned long long)expect_mem, (unsigned long long)before,
              (unsigned long long)operands[b]);
        if (kAmo[i].has_result) {
          CHECK(old == (before & m), "%s old value: got %016llx expected %016llx", kAmo[i].name, (unsigned long long)old,
                (unsigned long long)(before & m));
        }
        CHECK(mem[8] == 0xA5 && mem[15] == 0xA5, "%s wrote past the operand", kAmo[i].name);
      }
    }
  }
  for (size_t i = 0; i < sizeof kCas / sizeof kCas[0]; ++i) {
    const uint64_t m = mask_of(kCas[i].bytes);
    for (int success = 0; success < 2; ++success) {
      _Alignas(8) uint8_t mem[16];
      memset(mem, 0xA5, sizeof mem);
      const uint64_t start = 0x1122334455667788ull;
      memcpy(mem, &start, 8);
      uint64_t expected = success ? (start & m) : ((start & m) ^ 1);
      const uint64_t desired = 0xa1b2c3d4e5f60718ull & m;
      uint64_t old = 0;
      kCas[i].fn(mem, expected, desired, &old);
      uint64_t after;
      memcpy(&after, mem, 8);
      const uint64_t want = success ? ((start & ~m) | desired) : start;
      CHECK(after == want, "%s %s: memory %016llx expected %016llx", kCas[i].name, success ? "hit" : "miss",
            (unsigned long long)after, (unsigned long long)want);
      CHECK(old == (start & m), "%s %s: returned %016llx expected %016llx", kCas[i].name, success ? "hit" : "miss",
            (unsigned long long)old, (unsigned long long)(start & m));
      CHECK(mem[8] == 0xA5, "%s wrote past the operand", kCas[i].name);
    }
  }
}

/* ---- atomicity: emulated and compiler atomics on the same counters from several threads ---- */
#define THREADS 4
#define ITER 20000
static uint64_t g_add, g_cas, g_mixed;

static void* worker(void* arg) {
  const long id = (long)arg;
  for (int i = 0; i < ITER; ++i) {
    uint64_t one = 1, old;
    __asm__ volatile(".arch_extension lse\n\tldaddal %x[v], %x[r], [%[p]]" : [r] "=&r"(old) : [v] "r"(one), [p] "r"(&g_add) : "memory");
    for (;;) { /* increment with a CAS loop */
      uint64_t cur = __atomic_load_n(&g_cas, __ATOMIC_RELAXED), s = cur;
      __asm__ volatile(".arch_extension lse\n\tcasal %x[s], %x[t], [%[p]]" : [s] "+&r"(s) : [t] "r"(cur + 1), [p] "r"(&g_cas) : "memory");
      if (s == cur) break;
    }
    if (id % 2 == 0) {
      __asm__ volatile(".arch_extension lse\n\tstadd %x[v], [%[p]]" : : [v] "r"(one), [p] "r"(&g_mixed) : "memory");
    } else {
      __atomic_fetch_add(&g_mixed, 1, __ATOMIC_SEQ_CST);
    }
  }
  return NULL;
}

static void test_threads(void) {
  pthread_t t[THREADS];
  for (long i = 0; i < THREADS; ++i) pthread_create(&t[i], NULL, worker, (void*)i);
  for (int i = 0; i < THREADS; ++i) pthread_join(t[i], NULL);
  CHECK(g_add == (uint64_t)THREADS * ITER, "ldaddal lost updates: %llu", (unsigned long long)g_add);
  CHECK(g_cas == (uint64_t)THREADS * ITER, "casal lost updates: %llu", (unsigned long long)g_cas);
  CHECK(g_mixed == (uint64_t)THREADS * ITER, "emulated + compiler atomics lost updates: %llu", (unsigned long long)g_mixed);
}

/* ---- an instruction that is not ours must still crash the process ---- */
static void test_foreign_sigill(void) {
  const pid_t pid = fork();
  if (pid == 0) {
    struct rlimit rl = {0, 0};
    setrlimit(RLIMIT_CORE, &rl);
    __asm__ volatile(".inst 0x00000000"); /* udf #0 */
    _exit(0);
  }
  int status = 0;
  waitpid(pid, &status, 0);
  CHECK(WIFSIGNALED(status) && WTERMSIG(status) == SIGILL, "a plain illegal instruction should still kill the process with SIGILL (status %d)", status);
}

/* RCpc loads must zero-extend narrow values and preserve errno and memory. */
static void test_rcpc_loads(void) {
  const uint64_t patterns[] = {0, UINT64_MAX, UINT64_C(0xFEDCBA9876543281)};
  for (unsigned i = 0; i < sizeof patterns / sizeof patterns[0]; ++i) {
    uint64_t value = patterns[i], result;
    errno = EDOM;
#define CHECK_LOAD(OP, REG, MASK)                                                                                     \
    result = UINT64_MAX;                                                                                              \
    __asm__ volatile(".arch_extension rcpc\n\t" OP " " REG "[r], [%[p]]"                                        \
                     : [r] "=r"(result) : [p] "r"(&value) : "memory");                                             \
    CHECK(result == (value & (MASK)), OP " result/zero extension is wrong");                                        \
    CHECK(errno == EDOM, OP " changed errno");                                                                      \
    CHECK(value == patterns[i], OP " changed memory")
    CHECK_LOAD("ldaprb", "%w", UINT8_MAX);
    CHECK_LOAD("ldaprh", "%w", UINT16_MAX);
    CHECK_LOAD("ldapr", "%w", UINT32_MAX);
    CHECK_LOAD("ldapr", "%x", UINT64_MAX);
#undef CHECK_LOAD
    /* Rn == Rt: the address must be captured before replacing the register. */
    result = (uintptr_t)&value;
    __asm__ volatile(".arch_extension rcpc\n\tldapr %x[r], [%x[r]]" : [r] "+r"(result) : : "memory");
    CHECK(result == value, "ldapr with Rn == Rt is wrong");
    __asm__ volatile(".arch_extension rcpc\n\tldapr xzr, [%[p]]" : : [p] "r"(&value) : "memory");
    CHECK(errno == EDOM, "ldapr discard changed errno");
  }
}

int main(void) {
  /* The emulation lives in the audit namespace, invisible to the program: whether it is needed follows from the CPU. */
  const unsigned long required = (1UL << 8) | (1UL << 15);
  const int active = (getauxval(AT_HWCAP) & required) != required;
  printf("LSE/RCpc compatibility handler %s\n", active ? "needed (LD_AUDIT)" : "inactive (native instructions)");
  CHECK(fake_mrclib_constructor_ran(), "a library constructor that executes LSE before main() did not run");
  test_forms();
  test_rcpc_loads();
  test_threads();
  if (active) test_foreign_sigill();
  if (failures) {
    printf("%d FAILURE(S)\n", failures);
    return 1;
  }
  printf("all lse_emu tests passed (%zu LSE forms and RCpc loads)\n", sizeof kAmo / sizeof kAmo[0] + sizeof kCas / sizeof kCas[0]);
  return 0;
}
