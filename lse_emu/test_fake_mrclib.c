/* Stands in for libMrcLib: a library whose constructor runs an LSE instruction before main(), like libuv inside MrcLib.
 * The emulation has to be in place before this runs (loaded early through LD_AUDIT). */
#include <stdint.h>

static uint64_t counter;
static int ran;

__attribute__((constructor)) static void fake_mrclib_init(void) {
  uint64_t one = 1, old;
  __asm__ volatile(".arch_extension lse\n\tldaddal %x[v], %x[r], [%[p]]" : [r] "=&r"(old) : [v] "r"(one), [p] "r"(&counter) : "memory");
  __asm__ volatile(".arch_extension rcpc\n\tldapr %x[r], [%[p]]" : [r] "=r"(old) : [p] "r"(&counter) : "memory");
  ran = (old == 1);
}

int fake_mrclib_constructor_ran(void) { return ran; }
