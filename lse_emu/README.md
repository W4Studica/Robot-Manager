# LSE Emulator

Used by Robot Manager on the Studica VMX-pi's Raspberry Pi 4 to run WPILib libraries that require LSE atomic instructions and RCpc acquire loads on its Cortex-A72 CPU.

The library catches unsupported instructions through `SIGILL`, performs the operation with baseline ARMv8.0 instructions, and resumes the program. It is loaded through `LD_AUDIT` before dependency constructors run. Robot Manager checks both `atomics` and `ilrcpc` CPU flags in auto mode; the library independently checks Linux HWCAP before installing its handler.

Supported instructions:

- LSE add, clear, xor, set, signed/unsigned min/max, swap and compare-and-swap, including byte/halfword and ordering variants.
- No-offset `LDAPR`, `LDAPRB` and `LDAPRH`. Loads use the stronger `LDAR` acquire ordering and zero-extend narrow results.

`CASP`, post-index `LDAPR`, `LDAPUR` and other unsupported instructions still follow the previous `SIGILL` handler or terminate normally. This is a limited compatibility layer, not general ARMv8.2/8.3 CPU emulation. Each handled instruction incurs signal overhead; measure performance on the robot.

## References

The RCpc support is an original implementation informed by [agy-termux-lse's `LDAPR` to `LDAR` patch](https://github.com/krecod/agy-termux-lse/blob/main/patch_engine.py) and [Arm's discussion of RCpc versus RCsc acquire ordering](https://developer.arm.com/community/arm-community-blogs/b/architectures-and-processors-blog/posts/when-a-barrier-does-not-block-the-pitfalls-of-partial-order). No external source code is incorporated. We retain compiler atomic builtins, early audit loading and foreign-signal handling rather than the external project's binary patching and Termux launcher.

## Validation

On ARM64, run `make test`. The suite covers the existing LSE forms, RCpc load widths/zero extension, register aliasing, discarded results, errno preservation, instructions in a dependency constructor, mixed multithreaded atomics and unhandled illegal instructions.

For cross execution, build with an ARM64 `CC`, then run the executable through QEMU with `-L <sysroot> -E LD_AUDIT=<absolute library path>`. Run on `cortex-a72` to exercise emulation and `max` to exercise native instructions. Set the audit variable inside the emulated process rather than preloading an ARM64 library into the host QEMU executable. QEMU verifies behavior, not real-device latency.

## License

See the [project license status](../README.md#license).
