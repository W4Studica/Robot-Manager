# lse_emu

Runs ARMv8.1 LSE atomic instructions (`swp`, `ldadd`, `ldclr`, `ldeor`, `ldset`, `ldsmax/min`, `ldumax/min`, `stadd`..., `cas`)
on a CPU that lacks them (Raspberry Pi 4). Why: see the top of `lse_emu.c` and the section in the main README.

## Use

Load it with **`LD_AUDIT`** (not `LD_PRELOAD`: a preloaded library is initialised after the libraries the program needs, so
their constructors would run LSE instructions before the handler exists):

```bash
LD_AUDIT=/opt/robot_manager/lib/libvmx_lse_emu.so ./program
```

`robot_manager` sets this itself when `/proc/cpuinfo` shows no `atomics`. On a CPU with LSE the library does nothing.
`VMX_LSE_EMU_DEBUG=1` prints how many instructions were emulated when the process exits.

## What is emulated

`LD<op>`, `ST<op>`, `SWP` and `CAS` in all sizes (byte, half, word, double word) and all ordering variants (`A`, `L`, `AL`).
The operation is performed with a sequentially consistent atomic compiled to an exclusive load/store loop, so it is atomic
with respect to every other thread. **Not** emulated: `CASP`, `LDAPR` and anything else; those still crash with `SIGILL`
(or go to a handler that was installed before). A foreign `SIGILL` is never swallowed.

Cost, MEASURED on a Raspberry Pi 4 (`make bench`, kernel 7.0): **about 26.8 microseconds per emulated instruction, roughly 1,200
times a plain atomic add (21.6 ns)**, the same with four threads at once. Fine for start-up, shutdown and occasional use. The
programs tried so far (simulation HAL, sensors, Titan) emulate **one** instruction in total, at exit. A path that executes
thousands per second (the Driver Station network code inside libMrcLib, if it is ever used) would cost a large part of a core:
count them with `VMX_LSE_EMU_DEBUG=1` first. If that is too slow the next step is to rewrite the instructions when the
library is loaded (branch to a short load/store-exclusive sequence), which removes the signal, at the price of patching code
in memory and handling every register alias correctly. Not done.

## Cost (`make bench`)

`make bench` prints how long one emulated `ldaddal` / `casal` / `swpal` takes compared with a plain atomic add, on one thread and
on all CPUs at once. Run it on the robot (under qemu the numbers mean nothing).

## Tests (`make test`)

* all 224 instruction forms against a C model (memory result, returned old value, no write past the operand),
* four threads hammering emulated `ldaddal`/`casal`/`stadd` mixed with compiler atomics on the same words,
* a library constructor that executes an LSE instruction before `main()` (the case that needs `LD_AUDIT`),
* a plain illegal instruction still kills the process with `SIGILL`.

On the PC: cross compile with the WPILib arm64 toolchain and run under `qemu-aarch64 -cpu cortex-a72` (no LSE) with
`-E LD_AUDIT=...`; with `-cpu max` (LSE) the same test runs natively. **Not run on a real Pi 4 yet** (install.sh runs
`make test` on the robot, which is that run: 224 forms and the thread test passed on a Pi 4 on 2026-10-10).
