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

## Swap and memory policy

The profile files under `system/storage/` are the single source of truth: the
Rust `robot_manager` embeds them at compile time (`include_str!`) and
`install.sh` deploys them through `robot_manager storage configure` at the next
reboot. Nothing is retyped anywhere else.

| Component | Setting | Value | Why |
|---|---|---|---|
| zram (`zram-generator`) | logical size | 8192 MiB | compressed swap first, matches the documented baseline |
| zram | compression | zstd | denser than lzo-rle for idle service pages |
| zram | host limit | 2048 MiB | zram alone can never exhaust a 2 GiB or 4 GiB board |
| zram | swap priority | 100 | always preferred over the disk swap file |
| disk swap | file | `/var/lib/w4studica/swapfile`, 4 GiB, `sw,pri=10,nofail` | overflow for zram; never the first choice |
| sysctl | `vm.swappiness` | 180 | push idle anonymous pages to compressed swap instead of dropping cache |
| sysctl | `vm.page-cluster` | 0 | zram is random access; no swap readahead |
| sysctl | `vm.vfs_cache_pressure` | 50 | keep dentry/inode cache on a slow SD card |
| sysctl | watermark boost / scale | 0 / 125 | steady background reclaim, no bursty kswapd wakeups |
| journald | `Storage` / caps | volatile / 64 MiB / 8 MiB per file | no SD-card log writes, bounded RAM use |
| leaf daemons (`hwmon`, `mrccomm`, `ttyd`) | `MemoryHigh` + `MemorySwapMax` | 32 MiB + infinity | their idle pages are pushed into zram |
| `robot_manager.service` | `MemoryLow` + `MemorySwapMax` | 256 MiB + infinity | the robot program shares this cgroup: protect its hot pages, never set `MemoryHigh` here |

Installing the profile rewrites `/etc/fstab` swap entries (every other swap file
is commented out), disables `dphys-swapfile`/`zramswap`/`zram-config` and
`rsyslog`, and takes effect at the next reboot; a running swap is never touched.
