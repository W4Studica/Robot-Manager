# Robot Manager

Used by [W4Studica](https://github.com/W4Studica) on Studica VMX-pi (Raspberry Pi 4, Linux ARM64)
to run WPILib 2027 C++ robot programs deployed through GradleRIO. Provides process supervision,
RioLog, network configuration and VMX runtime helpers.

One binary (`/opt/robot_manager/robot_manager`) implements the supervisor and every helper;
its subcommands are also installed as the `/usr/local/bin` symlinks `setupHotspot`,
`toggleNetMode`, `setupEthernet`, `vmx_net` and `network_backup`.

## Accounts and directories (created by `install.sh`)

| Account | Purpose | Home | sudo |
|---|---|---|---|
| `lvuser` | Deploy login and owner of the robot files (GradleRIO uploads here; password `password`) | `/home/lvuser` | full NOPASSWD with `--sudoers` |
| `admin` | Interactive SSH management (password `password`, receives the previous user's SSH keys) | `/home/admin` | full NOPASSWD with `--sudoers` |

The robot program itself runs as **root** from `/home/lvuser/robotCommand`
(`/opt/robot_manager/robot_manager` supervises it; the VMX HAL needs `/dev/mem`).

## Services (all pinned to Core 4 = CPU 3; the robot program owns CPU 0-2)

| Unit | Role | Ports |
|---|---|---|
| `robot_manager.service` (alias `robot`) | Supervisor: runs `robotCommand`, RioLog server, alert republisher | 1741 (RioLog TCP), 1743 (alert lines, loopback) |
| `mrccomm.service` | FRC Driver Station daemon: control packets in, status out, watchdog | 1110 / 1150 (DS UDP), 1742 (relay, loopback) |
| `hwmon.service` | CPU temperature, throttling/under-voltage, RAM and swap monitoring | — |
| `irqconf.service` | Moves network IRQs to CPU 3 | — |
| `ttyd.service` | Web terminal (`http://10.TE.AM.2:7681`) | 7681 |
| `robot_cpu_max.service` | Keeps every CPU at its maximum frequency | — |
| `w4studica-rfkill-unblock.service` | Unblocks Wi-Fi before NetworkManager at boot | — |
| `systemd-timesyncd` (drop-in) | NTP for the RTC-less Pi | — |

The Driver Station state reaches the robot program through
`simulation/halsim_ds_relay` in allwpilib: `mrccomm` forwards over loopback
(port 1742), the extension applies it to the simulation HAL and returns the
battery voltage. The joystick decoding follows the documented Driver Station
layout and still needs a check against a real Driver Station.
`hwmon` and `mrccomm` events are republished into RioLog through port 1743.

## Commands

```sh
sudo ./install.sh                 # full robot setup (see --help; --sudoers enables deploys)
./install.sh --dry-run            # preview without changing anything
install.sh --undo-network         # restore the saved pre-network configuration
/opt/robot_manager/robot_manager storage verify [--strict]   # compare deployed vs shipped policy
/opt/robot_manager/robot_manager hwmon [--json]
/opt/robot_manager/robot_manager inventory --runtime-dir /home/lvuser/wpilib/third-party/lib \
    --image-id '<image>' --release '<version>' --output runtime-inventory.json
```

`RUNTIME_BASELINE.md` records the hardware/runtime baseline and the canonical
swap and memory policy; `system/storage/*.conf` is the single source of truth
for that policy (embedded into the binary at compile time).

## Layout

- `install.sh` — robot-side installer (accounts, services, network, storage profile)
- `src/` — the Rust supervisor and every subcommand (`network`, `storage`, `inventory`, `hwmon`, `irqconf`, `mrccomm`, `config`)
- `lse_emu/`, `gpio_isr_shim/` — C helpers for the Cortex-A72 (ARMv8.1 atomics) and sysfs-less GPIO kernels
- `system/` — systemd units, polkit rules and the storage profile files
- `tests/` — Rust integration tests plus the installer's Python tests

## License

[MIT](LICENSE). Dependency notices: [ThirdPartyNotices.txt](ThirdPartyNotices.txt).
