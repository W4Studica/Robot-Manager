# Builds the LD_PRELOAD library, and runs its tests (the tests need no GPIO hardware).
CC ?= gcc
CFLAGS ?= -O2 -Wall -Wextra -fPIC
LIB = libvmx_gpio_isr_shim.so

all: $(LIB)

$(LIB): gpio_isr_shim.c
	$(CC) $(CFLAGS) -shared -o $@ gpio_isr_shim.c -ldl -lpthread

libfake_hal.so: test_fake_hal.c
	$(CC) -O1 -g -Wall -Wextra -fPIC -shared -o $@ test_fake_hal.c

test_gpio_isr_shim: test_gpio_isr_shim.c gpio_isr_shim.c
	$(CC) -O1 -g -Wall -Wextra -Wno-unused-function -o $@ test_gpio_isr_shim.c -ldl -lpthread

test: test_gpio_isr_shim libfake_hal.so
	./test_gpio_isr_shim

clean:
	rm -f $(LIB) test_gpio_isr_shim libfake_hal.so

.PHONY: all test clean
