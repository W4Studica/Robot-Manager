#!/usr/bin/env python3
"""Record the robot OS, SDK and deployed runtime hashes without opening hardware."""

import argparse
from datetime import datetime, timezone
import hashlib
import json
from pathlib import Path
import platform
import re
import subprocess
import sys


def fingerprint(path):
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return {"bytes": path.stat().st_size, "sha256": digest.hexdigest()}


def version(args):
    try:
        result = subprocess.run(args, capture_output=True, text=True, timeout=10)
        return (result.stdout or result.stderr).strip().splitlines()[0]
    except (OSError, subprocess.TimeoutExpired, IndexError):
        return None


def collect(sdk, runtime, image_id, image_sha256, release):
    headers = sdk / "usr/local/include/vmxpi"
    hal = sdk / "usr/local/lib/vmxpi/libvmxpi_hal_cpp.so"
    required = [headers / "VMXPi.h", hal] + [runtime / name for name in (
        "libhalsim_vmx.so", "libhalsim_vmxd.so", "libhalsim_vmx_studica.so",
        "libstudica_drivers.so", "libwpiHal.so", "libwpiHald.so")]
    paths = set(headers.rglob("*.h")) | {hal} | set(runtime.glob("*.so*"))
    return {
        "schemaVersion": 1,
        "capturedAt": datetime.now(timezone.utc).isoformat(),
        "validation": "inventory-only; hardware acceptance must be recorded separately",
        "image": {"id": image_id, "sha256": image_sha256},
        "release": release,
        "os": {"name": platform.freedesktop_os_release().get("PRETTY_NAME"),
               "kernel": platform.release(), "architecture": platform.machine(),
               "libc": list(platform.libc_ver()), "python": platform.python_version(),
               "java": version(["java", "-version"]), "compiler": version(["g++", "--version"]),
               "gpioSysfs": Path("/sys/class/gpio").exists()},
        "files": {str(path): fingerprint(path) for path in sorted(paths) if path.is_file()},
        "missingRequired": [str(path) for path in required if not path.is_file()],
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--sdk", type=Path, default=Path("/"), help="SDK filesystem root (default: installed robot SDK)")
    parser.add_argument("--runtime-dir", type=Path, required=True, help="GradleRIO's deployed third-party/lib directory")
    parser.add_argument("--image-id", required=True, help="Name/version of the fixed OS image")
    parser.add_argument("--image-sha256", help="SHA-256 of the original ISO/IMG; omitted values remain explicitly unknown")
    parser.add_argument("--release", required=True, help="W4Studica release version")
    parser.add_argument("--output", type=Path, help="Write JSON here instead of stdout")
    args = parser.parse_args()
    if args.image_sha256 and not re.fullmatch(r"[a-fA-F0-9]{64}", args.image_sha256):
        parser.error("--image-sha256 must contain 64 hexadecimal characters")
    try:
        record = collect(args.sdk.resolve(), args.runtime_dir.resolve(), args.image_id,
                         args.image_sha256.lower() if args.image_sha256 else None, args.release)
        text = json.dumps(record, indent=2, ensure_ascii=False) + "\n"
        if args.output:
            args.output.write_text(text)
        else:
            print(text, end="")
        return 1 if record["missingRequired"] else 0
    except OSError as exc:
        parser.exit(1, "Inventory failed: " + str(exc) + "\n")


if __name__ == "__main__":
    sys.exit(main())
