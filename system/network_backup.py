#!/usr/bin/env python3
"""Keep the first pre-install network configuration, including private NM profiles.

Only configuration is backed up; no credentials are printed. Tests pass a temporary
root and a fake command runner, so they never change the development PC's network.
"""

import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile

STATE = "var/lib/robot_manager/network-original"
NETPLAN = "etc/netplan"
NM = "etc/NetworkManager/system-connections"
CLOUD = "etc/cloud/cloud.cfg.d/99-w4studica-disable-network-config.cfg"
MANAGED = "01-w4studica-network-manager.yaml"
UNITS = ("NetworkManager.service", "systemd-networkd.service", "systemd-networkd.socket",
         "systemd-networkd-wait-online.service")
MANAGED_PROFILES = ("robot-ap.nmconnection", "robot-eth.nmconnection", "robot-client.nmconnection")


def command(args, check=True):
    return subprocess.run(args, check=check, capture_output=True, text=True).stdout.strip()


def load_state(backup):
    state = json.loads((backup / "state.json").read_text())
    if state.get("schemaVersion") != 1 or set(state.get("services", {})) != set(UNITS):
        raise RuntimeError("Invalid network backup metadata")
    for status in state["services"].values():
        if not isinstance(status, dict) or not isinstance(status.get("enabled"), str) or type(status.get("active")) is not bool:
            raise RuntimeError("Invalid network service state")
    if not (backup / "netplan").is_dir() or not (backup / "nm").is_dir():
        raise RuntimeError("Incomplete network backup")
    files = state.get("files", {})
    for directory in ("netplan", "nm"):
        if sorted(p.name for p in (backup / directory).iterdir()) != files.get(directory):
            raise RuntimeError("Incomplete network backup: " + directory)
    if (backup / "cloud.cfg").is_file() != files.get("cloud"):
        raise RuntimeError("Incomplete network backup: cloud configuration")
    return state


def snapshot(root=Path("/"), run=command):
    destination = root / STATE
    if (destination / "state.json").is_file():
        load_state(destination)
        return destination
    if destination.exists():
        raise RuntimeError("Incomplete network backup: " + str(destination))
    source = root / NETPLAN
    legacy = False
    if (source / MANAGED).exists():
        # Older installers backed up the installer YAML on every rerun. Select
        # the first backup containing real original files, never that YAML alone.
        candidates = sorted(source.glob("w4studica-backup-*"))
        source = next((p for p in candidates if any(f.name != MANAGED for f in p.glob("*.yaml"))), None)
        if source is None:
            raise RuntimeError("Original netplan backup is missing; refusing to snapshot installer settings")
        legacy = True
    services = {unit: {"enabled": run(["systemctl", "is-enabled", unit], check=False),
                       "active": run(["systemctl", "is-active", unit], check=False) == "active"}
                for unit in UNITS}
    if legacy:
        # Recover the old installer's documented systemd-networkd baseline.
        # Profiles it already deleted cannot be reconstructed.
        services = {unit: {"enabled": "enabled" if unit.startswith("systemd-networkd") else "disabled",
                           "active": unit in ("systemd-networkd.service", "systemd-networkd.socket")}
                    for unit in UNITS}
    destination.parent.mkdir(parents=True, exist_ok=True)
    temporary = Path(tempfile.mkdtemp(prefix=".network-original-", dir=destination.parent))
    try:
        (temporary / "netplan").mkdir()
        for file in source.glob("*.yaml"):
            shutil.copy2(file, temporary / "netplan" / file.name)
        (temporary / "nm").mkdir()
        for file in (root / NM).glob("*"):
            if file.is_file() and not (legacy and file.name in MANAGED_PROFILES):
                shutil.copy2(file, temporary / "nm" / file.name)
        cloud = root / CLOUD
        if cloud.is_file() and not legacy:
            shutil.copy2(cloud, temporary / "cloud.cfg")
        files = {name: sorted(p.name for p in (temporary / name).iterdir()) for name in ("netplan", "nm")}
        files["cloud"] = (temporary / "cloud.cfg").is_file()
        state = {"schemaVersion": 1, "services": services, "legacy": legacy, "files": files}
        (temporary / "state.json").write_text(json.dumps(state, indent=2) + "\n")
        temporary.rename(destination)
    finally:
        if temporary.exists():
            shutil.rmtree(temporary)
    return destination


def restore(root=Path("/"), run=command):
    backup = root / STATE
    state = load_state(backup)
    # Prevent a pending installer job from switching back after undo finishes.
    run(["systemctl", "stop", "w4studica-network-switch.timer"], check=False)
    run(["systemctl", "stop", "w4studica-network-switch.service"], check=False)
    netplan = root / NETPLAN
    netplan.mkdir(parents=True, exist_ok=True)
    (netplan / MANAGED).unlink(missing_ok=True)
    for file in (backup / "netplan").glob("*.yaml"):
        shutil.copy2(file, netplan / file.name)
    nm = root / NM
    nm.mkdir(parents=True, exist_ok=True)
    for name in MANAGED_PROFILES:
        (nm / name).unlink(missing_ok=True)
    for file in (backup / "nm").iterdir():
        shutil.copy2(file, nm / file.name)
    cloud = root / CLOUD
    if (backup / "cloud.cfg").exists():
        cloud.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(backup / "cloud.cfg", cloud)
    else:
        cloud.unlink(missing_ok=True)
    run(["netplan", "generate"])
    for unit, status in state["services"].items():
        if not status["active"]:
            run(["systemctl", "stop", unit], check=False)
    for unit, status in state["services"].items():
        enabled = status["enabled"]
        if enabled.startswith("masked"):
            run(["systemctl", "mask"] + (["--runtime"] if enabled == "masked-runtime" else []) + [unit])
        elif enabled in ("enabled", "enabled-runtime", "disabled", "not-found", ""):
            run(["systemctl", "unmask", unit], check=False)
            action = "enable" if enabled.startswith("enabled") else "disable"
            args = ["systemctl", action]
            if enabled == "enabled-runtime":
                args.append("--runtime")
            run(args + [unit], check=enabled != "not-found" and enabled != "")
    for unit, status in state["services"].items():
        if status["active"]:
            run(["systemctl", "start", unit])
    if state["services"]["NetworkManager.service"]["active"]:
        run(["nmcli", "connection", "reload"])
    run(["netplan", "apply"])
    # Keep the original backup intact: a second undo or install must use it too.
    return backup


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("snapshot", "restore"))
    args = parser.parse_args()
    if os.geteuid() != 0:
        parser.error("run on the robot with sudo")
    try:
        path = (snapshot if args.action == "snapshot" else restore)()
        print("Network " + args.action + ": " + str(path))
    except (OSError, ValueError, RuntimeError, subprocess.CalledProcessError) as exc:
        parser.exit(1, "Network " + args.action + " failed: " + str(exc) + "\n")


if __name__ == "__main__":
    main()
