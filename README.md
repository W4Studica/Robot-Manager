# robot_manager

A service that runs continuously on the VMX-pi and replaces the roboRIO robot-program startup model (`robot` service). It is a single Python 3 script that uses only the standard library (Python 3.10 or later; works with the version included in Ubuntu 22.04).

## What it does

- Runs `/home/<user>/robotCommand` as **root** and restarts it when it exits. The execution contract matches 2027: `robotCommand` is a file containing a single command line. It is run by `sh`, so it does not need a shebang. No fixed program name such as `frcUserProgram` is required.
- If `robotCommand` is missing because the first deploy has not happened yet, logs once and waits for the file to appear.
- Increases the restart delay from 1 second up to 10 seconds after repeated crashes. A program that ran for at least 10 seconds restarts quickly.
- **Shutdown sequence** (service stop or `SIGTERM`/`SIGINT`): sends `stop_signals` to the robot program's **entire process group**, in order (default **`SIGINT` → `SIGTERM`**). It divides `stop_grace_seconds` (default 5 seconds) evenly among the signals and stops waiting once the process-group leader exits. If the grace period expires, sends `SIGKILL` to the group.

  **Why `SIGINT` comes first:** Measured with a test program on a development VMX that initialized the HAL and then waited. The VMX HAL cleans up pigpio and exits in about **0.2 seconds** after `SIGINT` (`Exiting VMX-pi HAL application ... [normal exit]`). `SIGTERM` cleans up pigpio but returns without terminating the process; the main thread remains in its wait loop, as confirmed with `gdb`. Sending only `SIGTERM` therefore uses the full grace period and ends with `SIGKILL`.

  The HAL's `Signal 2 received by PIGPIOClient::signal_func()`, stack output, and `Exiting ... [normal exit]` messages are normal. These messages are understood to appear on older VMX distributions too, although their signal source has not been confirmed from source. The old fork calls `frcKillRobot.sh -t`, a script on the VMX image.

  After the group leader exits, the manager waits for any remaining group processes until the grace period expires, then sends `SIGKILL`. If the program exits on its own (for example, after a crash) but leaves children behind, it sends `SIGTERM` to those children first and `SIGKILL` after the grace period.

  Every shutdown logs a message such as `robot program stopped by SIGINT after 0.2s` or `robot program killed after 5.0s`; it warns if `SIGKILL` was needed, since cleanup code such as pigpio's `gpioTerminate` did not run. The `robot_manager` service waits for its shutdown thread before exiting. Otherwise process-group cleanup is interrupted and systemd sends its own `SIGKILL` after 15 seconds; this behavior was observed on the robot.
- **Platform profiles:** adds environment variables from the profile matching the kernel version (`platform.release()`), such as `HALSIMVMX_DIO_MAP`. This layer can accommodate possible pin-number differences between OS or kernel versions (see below).
- **Does not access the VMX directly.** The robot program opens the VMX (pigpio). MockDS is also outside this service's scope; it belongs in user code.
- **RioLog console:** continuously writes the robot program's stdout/stderr to the systemd journal and forwards it as a WPILib NetConsole-compatible stream on TCP port 1741. The install script enables Avahi and advertises the mDNS name `robot.local`. If mDNS is unavailable on the RioLog client's network, the client must be able to route directly to the team address (`10.TE.AM.2`). Configure this with `riolog_enabled` (default `true`), `riolog_port` (default `1741`), and `riolog_history` (number of recent lines replayed to a new connection; default `200`).

  Each client has a dedicated sender thread and a queue of up to 256 frames. A connection is dropped if it falls behind and its queue fills. Up to 8 clients can connect at once. A new client receives the latest history that fits in its queue. The thread reading robot stdout never waits for socket writes.

## GPIO interrupt shim (`gpio_isr_shim/`)

This `LD_PRELOAD` library fixes `RPI GPIO Interrupt Enable: PI_BAD_ISR_INIT.` on kernels without sysfs GPIO (for example, Ubuntu 26.04 / kernel 7.0). `install.sh` builds it and installs it at `/opt/robot_manager/lib/libvmx_gpio_isr_shim.so`. When `/sys/class/gpio` is absent, the manager preserves any existing `config.json` settings and automatically adds the shim to `env.LD_PRELOAD`:

```json
{ "env": { "LD_PRELOAD": "/opt/robot_manager/lib/libvmx_gpio_isr_shim.so" } }
```

See [gpio_isr_shim/README.md](gpio_isr_shim/README.md) for details and limitations. **Only the callback contract has been tested on a PC; the shim has not been verified on hardware.**

## LSE atomic instruction emulation (`lse_emu/`)

WPILib's `linuxarm64` `libMrcLib.so` (Driver Station/robot communications; source unavailable) is built with the SystemCore toolchain (Buildroot GCC 14.3) and uses ARMv8.1 LSE atomic instructions such as `swp`, `ldadd`, and `cas` throughout the binary (1,219 occurrences). The Pi 4 (Cortex-A72, ARMv8.0) does not support them, causing **`SIGILL`**. On the robot, `uv_library_shutdown()` consistently crashed during process exit with status 132 and a core dump. Other WPILib libraries and our code use only libgcc helper functions (`__aarch64_*`) that check CPU support before use, so they are unaffected.

`lse_emu/` catches `SIGILL`, emulates the instruction with equivalent regular atomic operations (`ldxr/stxr`), and resumes at the next instruction. Atomicity is preserved, at the cost of one signal's overhead per emulated instruction. It must be loaded with **`LD_AUDIT`, not `LD_PRELOAD`**: the loader initializes libraries from the end of the list, so a preloaded library initializes after MrcLib's constructor. This ordering was measured.

If `/proc/cpuinfo`'s `Features` does not contain `atomics`, `robot_manager` sets `LD_AUDIT` itself (`install.sh` installs `libvmx_lse_emu.so`). Configure `"lse_emulation"` as `"auto"` (default), `"off"`, or `"on"`, and optionally set `"lse_emulation_library"`. An existing `LD_AUDIT` in `env` is left unchanged. See [lse_emu/README.md](lse_emu/README.md) for details.

## Forced shutdown (`SIGKILL`) and pigpio

`robot_manager` stops robot programs with `SIGINT` → `SIGTERM` → `SIGKILL` (`stop_signals`). If a program is killed with `SIGKILL`—because it did not exit within `stop_grace_seconds`, or someone ran `kill -9`—pigpio cannot release GPU memory. The memory leak accumulates until a later start fails with `initMboxBlock: init mbox zaps failed`; this was measured on the robot. Rebooting clears it. Motors still stop in this case: `TitanEnableGuard`/`StudicaTitan` in `allwpilib/halsim_vmx` does not rely on repeated enable frames, so the Titan stops itself within 200 ms when the program exits.

## Networking (`net/`, following the Studica VMX image)

The network setup lets a development PC connect directly to the robot, as at a competition. Command names and usage match those on the Studica VMX image.

| Mode | VMX address | Description |
|---|---|---|
| **Wi-Fi AP** (default) | **`10.XX.YY.2`** (team 1234 → `10.12.34.2`; team 4444 → `10.44.44.2`) | VMX acts as the access point and DHCP server. Install defaults: SSID `WorldSkills-1234`, password `password`, 2.4 GHz, channel 6. |
| **Wired** (always enabled) | **`172.22.11.2`** (fixed) | A directly connected PC receives a `172.22.11.XX` address from the VMX's DHCP server. Wired is preferred. |
| Wi-Fi client | Assigned by the router | Turns off the AP and connects to a router; useful for updates and package downloads. |

```bash
sudo setupHotspot VMX 1234 2.4 password          # SSID prefix, team number, band [password]; prompts for channel (default 6); use --channel N to set it
sudo setupHotspot TestRobot 4444 2.4 Checkers    # TestRobot-4444, 10.44.44.2
sudo toggleNetMode wifi HomeRouter routerpass    # Connect to router and turn off AP; later, `toggleNetMode wifi` reconnects
sudo toggleNetMode ap                            # Return to AP mode
sudo setupEthernet                               # Wired 172.22.11.2 (including DHCP server)
```

During a default installation, `install.sh` configures the hotspot as `WorldSkills 1234 2.4 password`, switches to NetworkManager, and runs `toggleNetMode ap`. The installer removes saved Wi-Fi client profiles, so the robot remains in AP mode when installation finishes. These installer values are fixed.

**Network changes made during installation** (Ubuntu Server uses systemd-networkd and netplan rather than NetworkManager):

1. Saves the initial network configuration and service state under `/var/lib/robot_manager/network-original/`. While keeping the current connection active, installs required tools such as `network-manager`, `dnsmasq-base`, and `iw` (Internet required). If dependency installation fails, it does not proceed to switch connections or remove profiles.
2. Writes wired `172.22.11.2` and AP connection files to `/etc/NetworkManager/system-connections/robot-*.nmconnection` with mode 0600.
3. After the backup is complete, replaces the active netplan YAML with a file that changes only the renderer to NetworkManager. Disables cloud-init network rewriting. Reinstalls do not overwrite the original backup.
4. **Eight seconds after the script exits**, switches to NetworkManager (stops `systemd-networkd`, starts `NetworkManager`), runs `setupHotspot WorldSkills 1234 2.4 password` and `toggleNetMode ap`, then deletes saved Wi-Fi connection profiles. The robot's systemd job continues if SSH disconnects.
5. Reconnect by **Ethernet** (set the PC to DHCP to receive `172.22.11.X`) and run `ssh <user>@172.22.11.2`, or connect to the **Wi-Fi AP** `WorldSkills-1234` (password `password`) and run `ssh <user>@10.12.34.2`.

**Rollback:** From the robot's display and keyboard, run `sudo /opt/robot_manager/bin/undo_network.sh` (or `install.sh --undo-network`). It restores the original netplan, NetworkManager connection files, cloud-init configuration, and network-service enablement/running state. If NetworkManager was in use originally, it restores that state. The backup remains after rollback, allowing repeated restores. The backup directory has mode 0700 and may contain Wi-Fi credentials; do not share it.

If an older installation has only `/etc/netplan/w4studica-backup-*`, the installer migrates the oldest actual original YAML into the new backup. This uses the older installation's networkd assumptions; Wi-Fi profiles that were already deleted cannot be restored. If only the installer's YAML exists and there is no original, a new install stops with an error. `undo_network.sh BACKUP_DIR` can also use a specified legacy netplan backup directory.

`setupHotspot`, `toggleNetMode`, and `setupEthernet` do not refuse changes that disconnect the current SSH session. The robot applies them itself through `systemd-run` after 3 seconds, so they complete after the session drops (`--force` is unnecessary and accepted only for compatibility). Reconnect on the new network. `--no-activate` for `setupHotspot`/`setupEthernet` writes connection files without activating them, for use when `install.sh` prepares settings in advance. The setup uses NetworkManager's `shared` mode.

**Tested on a PC only:** 39 unit tests with a mocked NetworkManager and `install.sh --dry-run`. **Not yet run on a robot.** Inspect the robot's current configuration with the read-only `system/network_probe.sh`. On the development robot (2026-10-11), systemd-networkd and netplan were present; NetworkManager, hostapd, dnsmasq, and iw were absent.

Backup/restore and package-failure paths are regression-tested using temporary files and fake service commands. Actual network switching and rollback after reinstall still need verification on the fixed image. Follow [RUNTIME_BASELINE.md](RUNTIME_BASELINE.md) for recording the OS/SDK/runtime combination.

## Maximum CPU performance (`system/`)

`install.sh` installs `robot_cpu_max.service`, which sets the CPU governor to `performance` and raises the minimum frequency to the maximum on every boot. This prevents a 20 ms loop from waiting for the clock to ramp up. Ubuntu's `ondemand.service` would revert this after one minute, so it is disabled. Use `--skip-cpu-max` to skip this setup. No overclocking is performed.

## Relationship to deploy

GradleRIO deploys **directly over SSH**, as in 2027. `robot_manager` does not participate in deploy; it only runs the deployed program. Atomic replacement/rollback and version reporting will be considered if hardware testing reveals deploy problems such as partial uploads or crash loops.

## Configuration (`config.json`)

Copy `config.example.json` and edit it. The default path is `/etc/robot_manager/config.json`.

```json
{
  "robot_home": "/home/<user>",
  "robot_command": "/home/<user>/robotCommand",
  "env": { "KEY": "value" },
  "profiles": [
    { "name": "...", "kernel_regex": "^6\\.", "env": { "HALSIMVMX_DIO_MAP": "0:12,1:13" } }
  ]
}
```

- `robot_home` (required): must match the GradleRIO `VmxPi` deploy directory (`/home/<username>`). This is the robot program's working directory.
- `robot_command` (optional): defaults to `<robot_home>/robotCommand`.
- `stop_grace_seconds` (optional): total time in seconds for the shutdown signals; must be positive. Defaults to 5 seconds and is divided evenly among the signals.
- `stop_signals` (optional): ordered shutdown signals. Allowed values: `SIGINT`, `SIGTERM`, `SIGHUP`, `SIGQUIT`, `SIGUSR1`, and `SIGUSR2`. Defaults to `["SIGINT", "SIGTERM"]`.
- Environment precedence: service environment < `env` < `env` from the matching profile. All values are converted to strings.
- Profiles are checked in order; only the first profile whose `kernel_regex` matches is applied.

> **Profile values are empty.** The possibility that pin-number offsets vary by OS/kernel comes only from user recollection; the version and values are unknown. On the development VMX (Ubuntu 26.04, kernel 7.0), GPIO sysfs is disabled, so pigpio interrupts fail with `PI_BAD_ISR_INIT`; this is not an offset issue. Verify whether pin I/O needs an offset with a pin-toggle test before adding any values. The example profile does not match a real kernel version.

## Installation (VMX-pi)

### Using the script (`install.sh`)

Clone this repository and `W4Studica/allwpilib` side by side on the robot, then run the installer. When `./install.sh` is started over SSH, it hands installation to a temporary systemd service on the robot, so work continues if installing NetworkManager or switching networks drops SSH. `sudo` requests the required permissions at startup. To follow progress in the terminal, use the printed `journalctl -fu <unit>` command. `--dry-run` and `--check-runtime-deps` run in the foreground.

```bash
git clone https://github.com/W4Studica/Robot-Manager.git
git clone https://github.com/W4Studica/allwpilib.git
cd Robot-Manager
./install.sh --dry-run --user <deploy-user>      # Review the planned changes first
./install.sh --user <deploy-user> --sudoers
```

For a detached background install from a non-SSH terminal, use `./install.sh --detach`. When it finishes, check the result with `sudo systemctl status <unit> --no-pager`.

The installer installs `robot_manager`, builds/installs `studica_drivers`, builds `libhalsim_vmx_studica.so` under `/opt/halsim_vmx/`, and can configure passwordless sudo with `--sudoers`. It installs missing build/runtime packages with apt. It checks for `build-essential`, Python 3, and C/C++ runtime packages (`libstdc++6`, `libgcc-s1`, `libatomic1`). The Studica plugin requires C++20; if the default `g++` is older than 11, the installer installs and uses GCC 11.

The VMX SDK headers and library (`VMXPi.h`, `libvmxpi_hal_cpp.so`) are not part of Ubuntu or WPILib packages. To install automatically, place the `vmxpi-hal_<version>_arm64.deb` built by `vmxpi-aarch64-hal-builder` in `../vmxpi-aarch64-hal-builder/dist/`, next to Robot-Manager. Otherwise pass `--vmx-hal-deb /path/to/vmxpi-hal_arm64.deb`. If the package is missing and a build is needed, the installer stops with a specific error unless `--skip-plugin` or `--skip-drivers` is also supplied.

The robot project deploys WPILib/CameraServer/cscore/OpenCV native libraries through GradleRIO. `install.sh` checks every deployed `.so` and `robotCommand` with `ldd`. OS runtime packages such as `libatomic1` are installed with apt. USB cameras use Linux kernel V4L2/UVC, so no separate camera-server daemon or system OpenCV install is required. Use `--no-install-deps` to skip package installation; the installer lists missing packages and exits. Use `--skip-*` options to intentionally omit components.

WPILib/OpenCV are not installed globally at arbitrary versions on the robot OS. GradleRIO deploys native libraries linked by the robot project, together with that project's WPILib dependency version, to `/home/<user>/wpilib/third-party/lib`. Projects using OpenCV must include its native dependency through GradleRIO. After deployment, run `./install.sh --check-runtime-deps --user <user>` to inspect the executable and shared libraries in that directory. Only missing **system** libraries are installed through OS packages. The installer also checks shared libraries built on the robot for the Studica plugin, drivers, GPIO shim, and LSE emulator.

The installer also installs `network-manager`, `iproute2`, `dnsmasq-base`, and `iw` when needed for network setup. Installing packages refreshes apt indexes and requires Internet access. See `./install.sh --help` for the remaining options.

### Manual installation

```bash
sudo mkdir -p /opt/robot_manager /etc/robot_manager
sudo cp robot_manager.py /opt/robot_manager/
sudo cp config.example.json /etc/robot_manager/config.json   # Edit robot_home
sudo cp robot_manager.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now robot_manager
journalctl -u robot_manager -f
```

GradleRIO `VmxPi` defaults `serviceName` to `robot_manager`, so `./gradlew deploy` stops and starts this service. The SSH user used for deploy must be able to run `systemctl` and `ldconfig` with `sudo` without a password.

## Tests

```bash
python3 -m unittest discover -s tests -v
```

The 13 tests cover profile selection/precedence, running a command without a shebang, environment-variable passing, crash restarts, waiting for a missing command file, process-tree shutdown, forcing down a program that ignores `SIGTERM`, and terminating a real executable with `SIGTERM`.

## Status

Tested on a PC only. The systemd unit and behavior on a physical VMX-pi are **not verified**.
