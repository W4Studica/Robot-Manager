#!/usr/bin/env python3
# Copyright (c) W4Studica contributors. SPDX-License-Identifier: MIT
"""Prepare the Ubuntu VMX storage profile for the next boot, without swapoff."""

import os
from pathlib import Path
import re
import shutil
import stat
import subprocess
import tempfile

GIB = 1024**3
SWAP_SIZE = 4 * GIB
SWAP_PATH = "/var/lib/w4studica/swapfile"
PROFILE = Path(__file__).resolve().parent


def command(*args):
    return subprocess.check_output(args, text=True, env={**os.environ, "LC_ALL": "C"}).strip()


def sync_directory(path):
    fd = os.open(path, os.O_RDONLY | os.O_DIRECTORY)
    try:
        os.fsync(fd)
    finally:
        os.close(fd)


def ensure_directory(path):
    if path.is_symlink():
        raise RuntimeError(f"Refusing symlink directory: {path}")
    if path.is_dir():
        return
    ensure_directory(path.parent)
    path.mkdir()
    sync_directory(path.parent)


def atomic_write(path, text, mode=0o644):
    if path.is_symlink():
        raise RuntimeError(f"Refusing to replace symlink: {path}")
    if path.exists() and path.read_text() == text:
        return
    ensure_directory(path.parent)
    fd, temporary = tempfile.mkstemp(prefix=".w4studica-", dir=path.parent)
    try:
        with os.fdopen(fd, "w") as stream:
            os.fchmod(stream.fileno(), mode)
            stream.write(text)
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary, path)
        sync_directory(path.parent)
    finally:
        if os.path.exists(temporary):
            os.unlink(temporary)


def fstab_profile(text):
    lines = []
    roots = 0
    network_mount = False
    for line in text.splitlines():
        body, separator, comment = line.partition("#")
        fields = body.split()
        if not fields or fields[0].startswith("#"):
            lines.append(line)
            continue
        if len(fields) < 4:
            raise RuntimeError(f"Invalid fstab entry: {line}")
        network_mount |= (
            fields[2] in {"nfs", "nfs4", "cifs", "smb3", "sshfs", "fuse.sshfs"}
            or "_netdev" in fields[3].split(",")
        )
        if fields[2] == "swap":
            if fields[0] != SWAP_PATH:
                lines.append("# w4studica: superseded swap: " + line)
            continue
        if fields[1] == "/":
            roots += 1
            if fields[2] not in {"ext4", "auto"}:
                raise RuntimeError("This storage profile requires an ext4 root filesystem")
            # Do not weaken journaling for speed. Changes take effect at reboot.
            replaced = {
                "atime", "relatime", "strictatime", "lazytime", "nolazytime",
                "noatime", "barrier", "nobarrier", "data", "commit", "errors",
                "noload", "norecovery", "auto_da_alloc", "noauto_da_alloc",
            }
            options = [o for o in fields[3].split(",") if o.split("=", 1)[0] not in replaced]
            options += ["noatime", "nolazytime", "data=ordered", "barrier=1", "auto_da_alloc", "commit=5", "errors=remount-ro"]
            fields[3] = ",".join(options)
            # Keep automatic recovery checks enabled, without forcing a full scan every boot.
            while len(fields) < 6:
                fields.append("0")
            fields[5] = "1"
            line = "\t".join(fields) + (" #" + comment if separator else "")
        lines.append(line)
    if roots != 1:
        raise RuntimeError("Expected exactly one root mount in /etc/fstab; no files changed")
    lines.append(f"{SWAP_PATH}\tnone\tswap\tsw,pri=10,nofail\t0\t0")
    return "\n".join(lines) + "\n", network_mount


def prepare_swap(path, run=command, size=SWAP_SIZE):
    ensure_directory(path.parent)
    if path.is_symlink() or (path.exists() and not path.is_file()):
        raise RuntimeError(f"Refusing non-regular swap file: {path}")
    if path.exists():
        if path.stat().st_size != size or run("blkid", "-p", "-s", "TYPE", "-o", "value", str(path)) != "swap":
            raise RuntimeError(f"Existing managed swap is not a valid {size}-byte swap file; refusing to overwrite it: {path}")
        path.chmod(0o600)
        return
    temporary = path.with_name(".swapfile.new")
    # A power loss can leave our previous staging file. Never touch the active file.
    if temporary.is_symlink() or (temporary.exists() and not temporary.is_file()):
        raise RuntimeError(f"Refusing non-regular swap staging file: {temporary}")
    temporary.unlink(missing_ok=True)
    if shutil.disk_usage(path.parent).free < size + 256 * 1024**2:
        raise RuntimeError("Not enough free disk space for 4 GiB swap plus 256 MiB reserve")
    try:
        fd = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
        with os.fdopen(fd, "wb") as stream:
            # Actually write every block: truncate/fallocate can leave unsuitable swap extents.
            block = bytes(4 * 1024**2)
            remaining = size
            while remaining:
                chunk = block[:min(remaining, len(block))]
                stream.write(chunk)
                remaining -= len(chunk)
            stream.flush()
            os.fsync(stream.fileno())
        run("mkswap", str(temporary))
        with temporary.open("rb") as stream:
            os.fsync(stream.fileno())
        os.replace(temporary, path)
        sync_directory(path.parent)
    finally:
        temporary.unlink(missing_ok=True)


def configure(root=Path("/"), run=command):
    def target(name):
        return root / name.lstrip("/")

    release = target("/etc/os-release").read_text()
    if not re.search(r'^ID=[\"\']?ubuntu[\"\']?$', release, re.MULTILINE):
        raise RuntimeError("This profile supports Ubuntu only")
    fstab = target("/etc/fstab")
    if fstab.is_symlink():
        raise RuntimeError("Refusing symlink /etc/fstab")
    before = fstab.read_text()
    after, network_mount = fstab_profile(before)
    ensure_directory(target("/var/lib/w4studica"))
    if run("findmnt", "-n", "-o", "FSTYPE", "--target", "/") != "ext4":
        raise RuntimeError("Root is not ext4; refusing to change mount options")
    device = run("findmnt", "-n", "-o", "SOURCE", "--target", "/")
    features = run("tune2fs", "-l", device)
    if not any("has_journal" in line.split() for line in features.splitlines() if line.startswith("Filesystem features:")):
        raise RuntimeError("Root ext4 has no journal; enable it offline before using this profile")
    mounted_options = run("findmnt", "-n", "-o", "OPTIONS", "--target", "/").split(",")
    if any(option in {"data=writeback", "data=journal"} for option in mounted_options):
        raise RuntimeError("Root journal data mode cannot be changed by remount; configure it offline before installation")
    boot_options = target("/proc/cmdline").read_text().split()
    if any(v in boot_options for v in ["systemd.zram=0", "systemd.zram=no", "systemd.zram=false", "systemd.zram=off"]):
        raise RuntimeError("Kernel command line disables zram; remove systemd.zram=0 before installation")
    for option in boot_options:
        if option.startswith("rootflags="):
            flags = option.removeprefix("rootflags=").split(",")
            if any(f in {"noload", "norecovery", "data=writeback", "data=journal"} for f in flags):
                raise RuntimeError("Conflicting rootflags in kernel command line; fix them before changing ext4 mount policy")
    # Swap on other filesystems (e.g. Btrfs) needs additional filesystem-specific preparation.
    if run("findmnt", "-n", "-o", "FSTYPE", "--target", str(target("/var/lib"))) not in {"ext4", "xfs"}:
        raise RuntimeError("Swap directory must be on ext4 or XFS")
    backup = target("/var/lib/w4studica/fstab.before-storage-profile")
    if not backup.exists():
        atomic_write(backup, before, 0o600)
    prepare_swap(target(SWAP_PATH), run)
    for source, destination in [
        ("zram.conf", "/etc/systemd/zram-generator.conf.d/99-w4studica.conf"),
        ("journald.conf", "/etc/systemd/journald.conf.d/99-w4studica.conf"),
        ("memory.conf", "/etc/sysctl.d/99-w4studica-memory.conf"),
    ]:
        atomic_write(target(destination), (PROFILE / source).read_text())
    wait = target("/etc/systemd/system/NetworkManager-wait-online.service.d/99-w4studica.conf")
    if network_mount:
        # Network-backed filesystems must retain their administrator's startup policy.
        if wait.exists() and not wait.is_symlink():
            wait.unlink()
            sync_directory(wait.parent)
        print("Network mounts present: keeping the existing network wait policy")
    else:
        atomic_write(wait, (PROFILE / "network-wait.conf").read_text())
    atomic_write(fstab, after, stat.S_IMODE(fstab.stat().st_mode))
    print("Prepared: zram 8 GiB priority 100; disk swap 4 GiB priority 10")
    print("Existing swap files are preserved; old fstab swap entries are disabled at next boot")
    print("Prepared: volatile journal (64 MiB); ext4 journal/barriers retained; no OverlayFS")
    print("Reboot required; running swap and root mount were not changed")


if __name__ == "__main__":
    try:
        if os.geteuid() != 0:
            raise RuntimeError("Run this helper through install.sh as root")
        configure()
    except (OSError, RuntimeError, subprocess.CalledProcessError) as error:
        raise SystemExit(f"storage profile: {error}") from error
