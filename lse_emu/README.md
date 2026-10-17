# lse_emu

Runs ARMv8.1 LSE atomic instructions (`swp`, `ldadd`, `ldclr`, `ldeor`, `ldset`, `ldsmax/min`, `ldumax/min`, `stadd` and friends, `cas`) on a CPU that lacks them (Raspberry Pi 4, Cortex-A72). WPILib's `libMrcLib.so` (Driver Station networking, binary only) is built with the SystemCore toolchain and uses them in 1,219 places; without this the program dies with `SIGILL` (exit code 132), at the latest when it exits. The reasoning is at the top of `lse_emu.c`.

## Use

Load it with **`LD_AUDIT`**, not `LD_PRELOAD`: a preloaded library is initialised after the libraries the program needs, so their constructors would run LSE instructions before the handler exists.

```bash
LD_AUDIT=/opt/robot_manager/lib/libvmx_lse_emu.so ./program
```

`robot_manager` sets this itself when `/proc/cpuinfo` shows no `atomics`; on a CPU with LSE the library does nothing. `VMX_LSE_EMU_DEBUG=1` prints how many instructions were emulated when the process exits.

## What is emulated

`LD<op>`, `ST<op>`, `SWP` and `CAS` in all sizes and ordering variants, performed as a sequentially consistent atomic compiled to an exclusive load/store loop, so they stay atomic with respect to other threads. **Not** emulated: `CASP`, `LDAPR` and anything else; those still crash with `SIGILL` (or reach a handler installed before). A foreign `SIGILL` is never swallowed.

## Cost

Measured on a Pi 4 (`make bench`): **about 26.8 microseconds per emulated instruction**, roughly 1,200 times a plain atomic add, the same with four threads. The programs tried so far emulate one instruction in total, at exit. A path that executes thousands per second (the Driver Station network code, if used heavily) would cost a large part of a core; count with `VMX_LSE_EMU_DEBUG=1` first. The next step, if needed, is to rewrite the instructions at load time; not done.

## Tests (`make test`)

All 224 instruction forms against a C model, four threads mixing emulated and compiler atomics on the same words, a library constructor that runs an LSE instruction before `main()`, and a plain illegal instruction still killing the process. `install.sh` runs it on the robot (passed on a Pi 4 on 2026-10-10). On a PC: cross compile with the WPILib arm64 toolchain and run under `qemu-aarch64 -cpu cortex-a72 -E LD_AUDIT=...`.
