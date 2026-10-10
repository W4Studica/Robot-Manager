/* Stand-in for libvmxpi_hal_cpp.so in the tests: defines the pigpio functions the shim forwards to. */
#include <stdint.h>

static int g_terminate_calls;

void gpioTerminate(void) { g_terminate_calls++; }
uint32_t gpioTick(void) { return 424242u; }
int fake_hal_terminate_calls(void) { return g_terminate_calls; }
