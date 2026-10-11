# Builds the LSE emulation library and runs its tests. Needs an aarch64 compiler (native on the VMX, or a cross compiler
# with a qemu-aarch64 for CC / RUN). The library must not contain LSE instructions itself, hence -march=armv8-a.
CC ?= gcc
RUN ?=
CFLAGS ?= -O2 -Wall -Wextra
LIB = libvmx_lse_emu.so
LIBFLAGS = $(CFLAGS) -march=armv8-a -mno-outline-atomics -fPIC

all: $(LIB)

$(LIB): lse_emu.c
	$(CC) $(LIBFLAGS) -shared -o $@ lse_emu.c

libfake_mrclib.so: test_fake_mrclib.c
	$(CC) -O1 -Wall -Wextra -fPIC -shared -o $@ test_fake_mrclib.c

test_lse_emu: test_lse_emu.c libfake_mrclib.so $(LIB)
	$(CC) -O1 -Wall -Wextra -o $@ test_lse_emu.c -L. -lfake_mrclib -Wl,-rpath,'$$ORIGIN' -lpthread

# Loaded with LD_AUDIT, like on the robot (see lse_emu.c for why not LD_PRELOAD). The audit path must be absolute.
test: test_lse_emu
	LD_AUDIT=$(CURDIR)/$(LIB) $(RUN) ./test_lse_emu

bench_lse_emu: bench_lse_emu.c
	$(CC) -O2 -Wall -Wextra -march=armv8-a -mno-outline-atomics -o $@ bench_lse_emu.c -lpthread

# Cost of one emulated instruction (run it on the robot; the numbers under qemu mean nothing).
bench: bench_lse_emu $(LIB)
	LD_AUDIT=$(CURDIR)/$(LIB) $(RUN) ./bench_lse_emu

clean:
	rm -f $(LIB) libfake_mrclib.so test_lse_emu bench_lse_emu

.PHONY: all test bench clean
