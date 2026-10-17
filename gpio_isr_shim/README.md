# gpio_isr_shim

`LD_PRELOAD` library for kernels without sysfs GPIO. The pigpio inside the Studica VMX HAL (`libvmxpi_hal_cpp.so`) implements GPIO interrupts through `/sys/class/gpio`. The development VMX (Ubuntu 26.04, kernel `7.0.0-1020-raspi`) has `# CONFIG_GPIO_SYSFS is not set`, so opening the HAL prints

```
RPI GPIO Interrupt Enable:  PI_BAD_ISR_INIT.
```

(three times: `strace` shows the HAL failing to open `/sys/class/gpio/export`).

## What it does

The HAL calls `gpioSetISRFunc` and `gpioSetISRFuncEx` through the PLT, so the shim replaces them, and `gpioTerminate`:

- Edge events come from the **GPIO character device** (`/dev/gpiochip0`, uAPI v2). `gpiochip0` is the SoC GPIO controller on a Pi 4.
- The callback contract is pigpio's: `(gpio, level, tick)`; `level` 0 falling, 1 rising, 2 timeout (no edge for `timeout` ms); `tick` is `gpioTick()` in microseconds. One thread per GPIO. `f == NULL` cancels; setting the same GPIO again replaces. A callback may cancel its own GPIO.
- Return values: 0 on success, `PI_BAD_GPIO` (-3) for a GPIO out of range, `PI_BAD_EDGE` (-122) for a bad edge, `PI_BAD_ISR_INIT` (-123) if the device cannot be opened or the line request fails.
- If `/sys/class/gpio` **exists**, the calls go to the original functions (`VMX_GPIO_SHIM_FORCE=1` always uses this implementation).
- `gpioTerminate` stops the ISR threads first, then calls the original. The original is looked up with `dlsym(RTLD_NEXT)` and, if that fails (the Studica plugin loads the HAL with `dlopen(RTLD_LOCAL)`), by the already loaded library's name (`VMX_GPIO_SHIM_REAL_LIB`, default `libvmxpi_hal_cpp.so`). Without that fallback `gpioTerminate` was never forwarded and the program hung in `exit()`.
- `VMX_GPIO_SHIM_DEBUG=1` logs every call to stderr.

## Build, test, use

```bash
make        # libvmx_gpio_isr_shim.so
make test   # needs no GPIO hardware: a pipe replaces the character device
```

The tests cover rising and falling edges, arguments, timeouts, `userdata`, cancel, replace, independence of GPIOs, error codes, recovery after a failed request, `gpioTerminate`, ordering under bursts, cancelling from inside the callback, and a HAL loaded with `dlopen(RTLD_LOCAL)`. They also pass under ThreadSanitizer, AddressSanitizer and UBSan.

`install.sh` builds it to `/opt/robot_manager/lib/libvmx_gpio_isr_shim.so` and, when `/sys/class/gpio` is absent, puts it in `config.json`:

```json
{ "env": { "LD_PRELOAD": "/opt/robot_manager/lib/libvmx_gpio_isr_shim.so" } }
```

For a manual run use `sudo env LD_PRELOAD=... ./program` (`sudo` clears `LD_PRELOAD`).

## Measured on the robot (2026-10-10, Pi 4, kernel 7.0)

- The `PI_BAD_ISR_INIT` lines are gone. The HAL asks for GPIO 13, 6 and 12, all falling edge, no timeout; all requests succeed.
- Falling edges arrive on GPIO 12 about every 20 ms (50 Hz, matching `VMXPi(true, 50)`; probably the VMX board's "data ready" signal). GPIO 6 and 13 were not seen to fire.
- A complete run exits with code 0, zero SPI write CRC mismatches and failures.

## Limits

- Asking the kernel for a line makes that pin a GPIO input. If the HAL uses the same pin differently the request fails (`EBUSY`) and `PI_BAD_ISR_INIT` returns, as without the shim; `VMX_GPIO_SHIM_DEBUG=1` shows the errno.
- What the HAL uses these interrupts for is unknown; its `comm ready` handshake works.
- Cancelling or replacing a different GPIO from a callback while another thread does the same can deadlock.
- Linux only.
