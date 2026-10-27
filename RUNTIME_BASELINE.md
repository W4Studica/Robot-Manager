# Fixed VMX runtime baseline

The current hardware evidence applies to one development VMX, not every ARM64
Linux image. Keep the OS image, SDK and W4Studica runtime together as a versioned
combination. An inventory records identity; it does not establish compatibility.

| Component | Recorded development baseline |
|---|---|
| Board | Raspberry Pi 4 Model B Rev 1.5, Cortex-A72 (no LSE atomics) |
| OS | Ubuntu 26.04.1, kernel `7.0.0-1020-raspi` |
| Runtime | glibc 2.43, Java 25, Python 3.14 |
| WPILib upstream baseline | `2027.0.0-alpha-7` |
| allwpilib upstream commit | `83df3ee3c` |
| GradleRIO upstream commit | `0ea36d4` |
| Studica drivers source | ROS2 `f97edec`, copied without driver edits |
| Required adaptations | GPIO character-device shim; `LD_AUDIT` LSE emulation |
| ISO identity / SHA-256 | The image is distributed by download link (the site's VMX-Pi Setup page). Its name and SHA-256 are not recorded here yet; publish the SHA-256 next to the link |
| SDK / deployed library SHA-256 | Must be captured on the robot for each release |

Baseline facts above come from the existing 2026-10-10/11 hardware record. A newer
binary release requires its own acceptance record. Do not infer that the upstream
source tag identifies the OS image, SDK or compiled binary contents.

On the **robot**, after deploying the project, record the actual combination:

```sh
python3 system/runtime_inventory.py \
  --runtime-dir /home/ubuntu/wpilib/third-party/lib \
  --image-id '<fixed image version>' \
  --image-sha256 '<SHA-256 of the ISO/IMG recorded on the PC>' \
  --release '<deployed W4Studica release version>' \
  --output runtime-inventory.json
```

This reads OS versions, SDK headers and native libraries without opening VMX or
reading network profiles or passwords. It exits nonzero if required files are
missing (the VMX headers and HAL, the Studica plugin, one `libhalsim_studica[d].so` and one `libwpiHal[d].so`; the build type
decides which of each pair is deployed, and `libstudica_drivers.so` is optional). If the image digest is unknown, omit `--image-sha256`; JSON records `null`
rather than inventing a digest. The original ISO can be hashed on the **PC** with
`sha256sum /path/to/image.iso`. Do not upload the image, HAL or SDK with the record.

Keep the inventory with release acceptance results: C++/Java startup, DIO/analog/
encoder/IMU checks, disabled/E-stop transitions, control-loop and DS packet stalls,
service restart, process termination, network switch and repeated network undo.
Record measured stop times and explicit unverified items. Compare the SDK and
runtime hashes when investigating a regression, even if version labels match.
