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

Cost: one signal round trip (microseconds) per emulated instruction. Fine for start-up, shutdown and occasional use;
a library that executes millions of such instructions per second would slow down.

## Tests (`make test`)

* all 224 instruction forms against a C model (memory result, returned old value, no write past the operand),
* four threads hammering emulated `ldaddal`/`casal`/`stadd` mixed with compiler atomics on the same words,
* a library constructor that executes an LSE instruction before `main()` (the case that needs `LD_AUDIT`),
* a plain illegal instruction still kills the process with `SIGILL`.

On the PC: cross compile with the WPILib arm64 toolchain and run under `qemu-aarch64 -cpu cortex-a72` (no LSE) with
`-E LD_AUDIT=...`; with `-cpu max` (LSE) the same test runs natively. **Not run on a real Pi 4 yet** (install.sh runs
`make test` on the robot, which is that run).
