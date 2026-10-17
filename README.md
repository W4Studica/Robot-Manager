# robot_manager

What the W4Studica software puts on a Studica VMX-pi: a service that runs the robot program, the helpers that make WPILib 2027 run on the VMX's Raspberry Pi 4, the network setup, and the installer. `robot_manager.py` is a single Python 3 script (standard library only) and replaces the roboRIO `robot` service.

## What it does

- Runs `/home/<user>/robotCommand` as **root** and restarts it when it exits. The contract matches 2027: `robotCommand` is a file with one command line, run by `sh` (no shebang, no fixed program name). If it does not exist yet (before the first deploy) the service logs once and waits.
- Restart delay: 1 s, doubling to 10 s after repeated crashes; a program that ran at least 10 s restarts quickly.
- **Stop** (service stop, `SIGTERM`, `SIGINT`): sends `stop_signals` (default `SIGINT`, then `SIGTERM`) to the program's whole process group, splitting `stop_grace_seconds` (default 5 s) between them, then `SIGKILL`. Processes left in the group are cleaned up too. Every stop logs how it ended (`robot program stopped by SIGINT after 0.2s`, or a warning if `SIGKILL` was needed).
  - `SIGINT` goes first because the VMX HAL cleans up pigpio on it, while on `SIGTERM` it also only cleans up. The halsim_vmx extension makes both end the process after that clean-up (`halsim_vmx/DESIGN.md`), so a stop takes about a second. A `SIGKILL` before the clean-up leaks pigpio GPU memory: the next start fails with `initMboxBlock: init mbox zaps failed` until a reboot.
- **RioLog**: the program's stdout and stderr go to the systemd journal and to TCP port 1741 in WPILib NetConsole framing, so VS Code's RioLog works. `install.sh` enables Avahi and publishes `robot.local`; without mDNS the client needs a route to the team address `10.TE.AM.2`. Each client has its own bounded queue (256 frames) and is dropped if it falls behind; at most 8 clients. A failing journal or RioLog never stops the robot output from being read. Settings: `riolog_enabled`, `riolog_port`, `riolog_history`.
- Adds environment variables from the platform profile whose `kernel_regex` matches the kernel. The OS image is fixed, so no profile is used.
- Does not open the VMX and does not run MockDS; both belong to the robot program.

## Helpers for the Pi 4

- **`gpio_isr_shim/`**: `LD_PRELOAD` library that replaces pigpio's `gpioSetISRFunc*` with the GPIO character device, because this kernel has no `/sys/class/gpio` (otherwise `PI_BAD_ISR_INIT`). Details: `gpio_isr_shim/README.md`. When `/sys/class/gpio` is absent, `install.sh` adds it to `env.LD_PRELOAD` in `config.json`.
- **`lse_emu/`**: `LD_AUDIT` library that emulates ARMv8.1 LSE atomics, which WPILib's `libMrcLib.so` uses and a Cortex-A72 lacks (`SIGILL`, exit code 132). `robot_manager` sets `LD_AUDIT` itself when `/proc/cpuinfo` has no `atomics` (`lse_emulation` `auto`/`off`/`on`, `lse_emulation_library`). Details: `lse_emu/README.md`.

## Networking (`net/`)

Command names and use match the Studica VMX image, so a PC can connect as it would at a competition.

| Mode | VMX address | |
|---|---|---|
| Wi-Fi access point (default) | `10.XX.YY.2` (team 1234 gives `10.12.34.2`) | the VMX is the access point and DHCP server |
| Wired (always on) | `172.22.11.2`, fixed | a computer plugged in directly gets `172.22.11.X` |
| Wi-Fi client | from the router | access point off; for updates and packages |

```bash
sudo setupHotspot WorldSkills 1234 2.4 password   # SSID prefix, team, band [password]; prompts for the channel (default 6), or --channel N
sudo toggleNetMode wifi HomeRouter routerpass     # join a router and turn the access point off; `toggleNetMode ap` returns
sudo setupEthernet                                # wired port 172.22.11.2 with DHCP
```

The SSID is `<prefix>-<team>`. 5 GHz uses only the standard channels (default 36); if a 5 GHz access point does not start it falls back to 2.4 GHz channel 6. Wi-Fi power saving is off in the profiles (`powersave=2`): with it on the access point answers in bursts (ping 6 ms to 400 ms) and SSH logins and deploys stall. The commands use NetworkManager keyfiles; a change that would cut the SSH session it runs in is not refused, the robot applies it itself 3 s later (`systemd-run`).

**`install.sh` always sets the network up** (there is no option to skip it, by design): it saves the original configuration under `/var/lib/robot_manager/network-original/` (mode 0700, may contain Wi-Fi passwords; never reinstall over it), installs `network-manager`, `dnsmasq-base`, `iw` and `iproute2` if needed (internet required; if that fails nothing is changed), writes the wired and access-point connection files, replaces the netplan YAML with one that only selects the NetworkManager renderer, disables cloud-init network rewriting, and **8 s after the script ends** switches from systemd-networkd to NetworkManager, runs `setupHotspot WorldSkills 1234 2.4 password`, selects access point mode and deletes saved Wi-Fi client profiles. The SSH session ends; reconnect by cable (set the PC to DHCP, `ssh <user>@172.22.11.2`) or join `WorldSkills-1234` (password `password`) and `ssh <user>@10.12.34.2`.

Undo, from the robot's screen and keyboard: `sudo /opt/robot_manager/bin/undo_network.sh` (or `install.sh --undo-network`). It restores the original netplan, NetworkManager files, cloud-init configuration and service states; the backup stays, so it can be repeated. `system/network_probe.sh` prints the current configuration without changing anything.

## CPU

`install.sh` installs `robot_cpu_max.service`, which sets the governor to `performance` and the minimum frequency to the maximum at every boot, so a 20 ms loop does not wait for the clock; Ubuntu's `ondemand.service` is disabled. `--skip-cpu-max` skips it. No overclocking.

## Configuration (`/etc/robot_manager/config.json`)

Copy `config.example.json` and edit it.

```json
{
  "robot_home": "/home/<user>",
  "env": { "KEY": "value" },
  "profiles": [ { "name": "...", "kernel_regex": "^6\\.", "env": { "KEY": "value" } } ]
}
```

- `robot_home` (required): the GradleRIO `VmxPi` deploy directory (`/home/<username>`), also the program's working directory.
- `robot_command`: defaults to `<robot_home>/robotCommand`.
- `stop_grace_seconds`: total seconds for the stop signals (default 5, divided evenly).
- `stop_signals`: ordered list from `SIGINT`, `SIGTERM`, `SIGHUP`, `SIGQUIT`, `SIGUSR1`, `SIGUSR2` (default `["SIGINT", "SIGTERM"]`).
- Environment precedence: service environment, then `env`, then the LSE `LD_AUDIT` (only if `env` does not set it), then the first matching profile.

## Installation

Clone this repository and `W4Studica/allwpilib` side by side on the robot and run the installer (`sudo` is requested; over SSH it continues as a robot-side systemd job, so installing NetworkManager cannot kill it; follow it with the printed `journalctl -fu <unit>`):

```bash
git clone https://github.com/W4Studica/Robot-Manager.git
git clone https://github.com/W4Studica/allwpilib.git
cd Robot-Manager
./install.sh --dry-run --user <deploy-user>      # review first
./install.sh --user <deploy-user> --sudoers
```

It installs `robot_manager` and its service, `studica_drivers`, the Studica backend plugin (`/opt/halsim_vmx/`, needs g++ 11 or newer; GCC 11 is installed if the default is older), the GPIO shim, the LSE emulation, the CPU service, the network commands and setup, and (with `--sudoers`) passwordless `sudo` for the deploy login, limited to `systemctl stop|start|enable robot_manager`, `ldconfig` and `sync`. Missing apt packages are installed (`--no-install-deps` lists them and exits). The service is restarted last.

The VMX SDK (`VMXPi.h`, `libvmxpi_hal_cpp.so`) is not part of Ubuntu or WPILib. It comes with the robot image, or from `../vmxpi-aarch64-hal-builder/dist/vmxpi-hal_*_arm64.deb` (or `--vmx-hal-deb FILE`). Use `--skip-drivers`, `--skip-plugin`, `--skip-shim`, `--skip-lse-emu` or `--skip-cpu-max` to leave parts out; see `./install.sh --help`.

WPILib, CameraServer and OpenCV libraries are deployed per project by GradleRIO to `/home/<user>/wpilib/third-party/lib`, not installed globally. `./install.sh --check-runtime-deps --user <user>` runs `ldd` on the deployed program and libraries and reports unresolved system libraries (the debug extension `libhalsim_vmxd.so` is skipped in a release deploy). During a full install this check only warns.

Manual install:

```bash
sudo mkdir -p /opt/robot_manager /etc/robot_manager
sudo cp robot_manager.py /opt/robot_manager/
sudo cp config.example.json /etc/robot_manager/config.json   # edit robot_home
sudo cp robot_manager.service /etc/systemd/system/
sudo systemctl daemon-reload && sudo systemctl enable --now robot_manager
journalctl -u robot_manager -f
```

GradleRIO's `VmxPi` target deploys with `sudo systemctl stop/start/enable robot_manager` and `sudo ldconfig`, so the SSH user needs those without a password.

`RUNTIME_BASELINE.md` records the OS, SDK and runtime combination that was tested; `system/runtime_inventory.py` collects it on the robot.

## Tests

```bash
python3 -m unittest discover -s tests
```

They cover profiles and precedence, running a command without a shebang, environment passing, restarts, waiting for a missing file, process-tree stop and `SIGKILL` escalation, RioLog framing and clients, the network commands with a mocked NetworkManager, network backup and restore, the CPU script, the runtime inventory and the apt handling. Hardware results: `allwpilib/simulation/halsim_vmx/HARDWARE_CHECKLIST.md`.
