"""Exercise the real cleanup scripts with an isolated package DB and fake APT.

No test invokes the host's package manager, touches /etc, or removes OS packages.
"""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
HELPER = ROOT / "system/package_cleanup.sh"


class PackageCleanupTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="vmx-cleanup-test-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.bin = self.root / "bin"
        self.bin.mkdir()
        self.packages = self.root / "installed"
        self.packages.write_text("apport\nopen-vm-tools\nopenssh-server\nlibstdc++6\nlinux-image-raspi\nchrony\n")
        self.log = self.root / "apt.jsonl"
        self.mutations = self.root / "mutations"
        self.env = {
            **os.environ,
            "PATH": str(self.bin) + os.pathsep + os.environ["PATH"],
            "TEST_PACKAGES": str(self.packages),
            "TEST_APT_LOG": str(self.log),
            "TEST_MUTATIONS": str(self.mutations),
            "TEST_PLAN": "Remv apport [1]\nRemv open-vm-tools [1]\n",
            "TEST_TRANSACTION": "VERSION 2\nAPT::Get::AutomaticRemove=false\n\napport 1 > - **REMOVE**\nopen-vm-tools 1 > - **REMOVE**\n",
            "SSH_CONNECTION": "",
            "SSH_CLIENT": "",
        }
        self.executable("id", "#!/bin/sh\necho 0\n")
        self.executable("uname", "#!/bin/sh\necho aarch64\n")
        self.executable("dpkg-query", f'''#!{sys.executable}
import os, pathlib, sys
packages = pathlib.Path(os.environ["TEST_PACKAGES"]).read_text().splitlines()
if sys.argv[-1] not in packages:
    sys.exit(1)
print("install ok installed", end="")
''')
        self.executable("apt-get", f'''#!{sys.executable}
import json, os, pathlib, subprocess, sys
args = sys.argv[1:]
with open(os.environ["TEST_APT_LOG"], "a") as log:
    log.write(json.dumps(args) + "\\n")
if "--simulate" in args:
    print(os.environ["TEST_PLAN"], end="")
    sys.exit(int(os.environ.get("TEST_SIM_EXIT", "0")))
hook = next(a.split("=", 1)[1] for a in args if a.startswith("DPkg::Pre-Install-Pkgs::="))
assert f"DPkg::Tools::Options::{{hook}}::Version=2" in args
assert f"DPkg::Tools::Options::{{hook}}::InfoFD=0" in args
result = subprocess.run([hook], input=os.environ["TEST_TRANSACTION"], text=True)
if result.returncode:
    sys.exit(result.returncode)
if os.environ.get("TEST_APPLY_EXIT"):
    sys.exit(int(os.environ["TEST_APPLY_EXIT"]))
pathlib.Path(os.environ["TEST_MUTATIONS"]).write_text("fake purge only")
# Model installed-state changes so a repeated cleanup is also exercised.
path = pathlib.Path(os.environ["TEST_PACKAGES"])
removed = set(args[args.index("-y") + 1:])
path.write_text("".join(p + "\\n" for p in path.read_text().splitlines() if p not in removed))
''')

    def executable(self, name, text):
        path = self.bin / name
        path.write_text(text)
        path.chmod(0o755)

    def run_cleanup(self, *args, installer=False, **env):
        command = ["bash", str(ROOT / "install.sh" if installer else HELPER)]
        if installer:
            command += ["--cleanup-packages", "--user", "ubuntu"]
        result = subprocess.run(command + list(args), env={**self.env, **env}, capture_output=True, text=True, timeout=10)
        return result

    def calls(self):
        return [json.loads(line) for line in self.log.read_text().splitlines()] if self.log.exists() else []

    def test_dry_run_simulates_without_mutating(self):
        result = self.run_cleanup("--dry-run")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("no packages were changed", result.stdout)
        self.assertEqual(len(self.calls()), 1)
        self.assertFalse(self.mutations.exists())
        self.assertEqual(self.packages.read_text().splitlines()[:2], ["apport", "open-vm-tools"])
        self.assertNotIn("openssh-server", self.calls()[0])

    def test_explicit_cleanup_purges_only_candidates_and_is_repeatable(self):
        result = self.run_cleanup()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(self.mutations.exists())
        self.assertEqual(self.packages.read_text().splitlines(), ["openssh-server", "libstdc++6", "linux-image-raspi", "chrony"])
        calls = self.calls()
        self.assertEqual(len(calls), 2)
        for call in calls:
            self.assertIn("APT::Get::AutomaticRemove=false", call)
            self.assertIn("DPkg::Lock::Timeout=600", call)
            self.assertNotIn("autoremove", call)
        hook = next(a.split("=", 1)[1] for a in calls[-1] if a.startswith("DPkg::Pre-Install-Pkgs::="))
        self.assertFalse(Path(hook).parent.exists(), "temporary guard must be cleaned up")
        result = self.run_cleanup()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.calls(), calls)

    def test_no_installed_candidates_never_invokes_apt(self):
        self.packages.write_text("openssh-server\nlibstdc++6\n")
        result = self.run_cleanup()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.calls(), [])

    def test_unexpected_removals_in_simulation_abort_before_purge(self):
        for package in ("openssh-server", "ubuntu-server", "linux-firmware-raspi", "apport-extra"):
            with self.subTest(package=package):
                result = self.run_cleanup(TEST_PLAN=f"Remv apport [1]\nRemv {package} [1]\n")
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("outside the cleanup list", result.stderr)
                self.assertFalse(self.mutations.exists())
        self.assertTrue(all("--simulate" in call for call in self.calls()))

    def test_installs_and_configures_in_simulation_are_rejected(self):
        for action in ("Inst", "Conf"):
            result = self.run_cleanup(TEST_PLAN=f"Remv apport [1]\n{action} replacement [1]\n")
            self.assertNotEqual(result.returncode, 0)
            self.assertFalse(self.mutations.exists())
        self.assertTrue(all("--simulate" in call for call in self.calls()))

    def test_simulation_failure_never_purges(self):
        result = self.run_cleanup(TEST_SIM_EXIT="100")
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse(self.mutations.exists())
        self.assertEqual(len(self.calls()), 1)

    def test_actual_transaction_is_checked_again_after_simulation(self):
        result = self.run_cleanup(TEST_TRANSACTION="VERSION 2\n\napport 1 > - **REMOVE**\nopenssh-server 1 > - **REMOVE**\n")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("refusing to remove openssh-server", result.stderr)
        self.assertEqual(len(self.calls()), 2)
        self.assertFalse(self.mutations.exists())

    def test_actual_unpacks_configures_and_invalid_protocol_are_rejected(self):
        for transaction in (
            "VERSION 2\n\napport 1 = 1 **CONFIGURE**\n",
            "VERSION 2\n\napport 1 < 2 /tmp/apport.deb\n",
            "VERSION 3\n\napport 1 > - **REMOVE**\n",
            "VERSION 2\nAPT::key=value\n",
            "VERSION 2\n\napport malformed\n",
        ):
            result = self.run_cleanup(TEST_TRANSACTION=transaction)
            self.assertNotEqual(result.returncode, 0)
            self.assertFalse(self.mutations.exists())

    def test_apt_failure_is_propagated(self):
        result = self.run_cleanup(TEST_APPLY_EXIT="100")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("APT cleanup failed", result.stderr)
        self.assertFalse(self.mutations.exists())

    def test_installer_cleanup_is_standalone_without_builds_or_network_setup(self):
        result = self.run_cleanup("--dry-run", installer=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("Optional Ubuntu image package cleanup", result.stdout)
        self.assertNotIn("setupHotspot", result.stdout)
        self.assertNotIn("cargo build", result.stdout)
        self.assertFalse(self.mutations.exists())

    def test_cleanup_cannot_silently_ignore_another_action(self):
        for option in ("--undo-network", "--check-runtime-deps", "--no-install-deps"):
            result = self.run_cleanup(option, installer=True)
            self.assertNotEqual(result.returncode, 0)
        self.assertEqual(self.calls(), [])

    def test_apply_requires_root_on_arm64_but_preview_does_not(self):
        self.executable("id", "#!/bin/sh\necho 1000\n")
        result = self.run_cleanup()
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(self.calls(), [])
        self.assertEqual(self.run_cleanup("--dry-run").returncode, 0)
        self.executable("id", "#!/bin/sh\necho 0\n")
        self.executable("uname", "#!/bin/sh\necho x86_64\n")
        self.assertNotEqual(self.run_cleanup().returncode, 0)
        self.assertFalse(self.mutations.exists())


if __name__ == "__main__":
    unittest.main()
