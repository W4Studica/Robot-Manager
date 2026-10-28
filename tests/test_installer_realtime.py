import json
import os
from pathlib import Path
import shlex
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
DEFINITIONS = (ROOT / "install.sh").read_text().split('\n[ -n "$LOGIN_USER" ] || LOGIN_USER=', 1)[0]


class RealtimeInstallerTest(unittest.TestCase):
    def run_function(self, script, directory):
        harness = Path(directory) / "installer-test.sh"
        harness.write_text(DEFINITIONS + "\n" + script)
        return subprocess.run(
            ["bash", str(harness)], capture_output=True, text=True,
            env=os.environ, timeout=10,
        )

    def kernel(self, *, active=False, installed=False, supported=True,
               skip=False, dry_run=False, dependencies=True, apt_failure=False):
        with tempfile.TemporaryDirectory() as directory:
            return self.run_function(f'''
rt_kernel_active() {{ {'true' if active else 'false'}; }}
rt_kernel_platform_supported() {{ {'true' if supported else 'false'}; }}
dpkg-query() {{ echo '{'install ok installed' if installed else 'unknown ok not-installed'}'; }}
# Trace package operations without invoking the host package manager.
apt-get() {{ return 99; }}
run() {{
  printf 'RUN'; printf ' %s' "$@"; printf '\\n'
  {'return 1' if apt_failure else 'return 0'}
}}
DRY_RUN={int(dry_run)}
INSTALL_DEPS={int(dependencies)}
SKIP_RT_KERNEL={int(skip)}
ensure_rt_kernel
echo "STATUS=$RT_KERNEL_STATUS"
''', directory)

    def test_regular_kernel_installs_only_pi_rt_metapackage(self):
        result = self.kernel(dry_run=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("apt-get -o DPkg::Lock::Timeout=600 update", result.stdout)
        self.assertIn("install -y --no-install-recommends --no-remove linux-raspi-realtime", result.stdout)
        self.assertNotIn("ubuntu-realtime", result.stdout)
        self.assertNotIn("RUN reboot", result.stdout)
        self.assertIn("reboot required", result.stdout)

    def test_existing_rt_kernel_needs_no_package_install(self):
        result = self.kernel(active=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("PREEMPT_RT active", result.stdout)
        self.assertNotIn("RUN", result.stdout)

    def test_install_reports_reboot_requirement(self):
        result = self.kernel()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("install -y --no-install-recommends --no-remove linux-raspi-realtime", result.stdout)
        self.assertIn("RT kernel installed; reboot required", result.stdout)

    def test_apt_failure_does_not_claim_kernel_installed(self):
        result = self.kernel(apt_failure=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("apt-get update failed", result.stderr)
        self.assertNotIn("RT kernel installed", result.stdout)

    def test_installed_package_does_not_claim_running_kernel_is_rt(self):
        result = self.kernel(installed=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertNotIn("RUN", result.stdout)
        self.assertIn("reboot required", result.stdout)
        self.assertNotIn("PREEMPT_RT active", result.stdout)

    def test_unsupported_platform_stops_before_package_changes(self):
        result = self.kernel(supported=False)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Ubuntu >= 26.04", result.stderr)
        self.assertNotIn("RUN", result.stdout)

    def test_explicit_skip_allows_manually_managed_kernel(self):
        result = self.kernel(skip=True, supported=False)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("kernel installation skipped", result.stdout)
        self.assertNotIn("RUN", result.stdout)

    def test_no_install_deps_reports_missing_kernel(self):
        result = self.kernel(dependencies=False)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("linux-raspi-realtime", result.stderr)
        self.assertNotIn("RUN", result.stdout)

    def migrate(self, config, *, dry_run=False):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "config.json"
            before = json.dumps(config) + "\n"
            path.write_text(before)
            path.chmod(0o640)
            result = self.run_function(f'''
CONF_DIR={shlex.quote(directory)}
DRY_RUN={int(dry_run)}
configure_realtime
''', directory)
            return result, before, path.read_text(), path.stat().st_mode & 0o777

    def test_legacy_config_gains_rt_and_preserves_deploy_settings(self):
        config = {"robot_home": "/home/robot", "env": {"LD_PRELOAD": "/custom.so"},
                  "profiles": [{"name": "custom"}]}
        result, _, after, mode = self.migrate(config)
        self.assertEqual(result.returncode, 0, result.stderr)
        updated = json.loads(after)
        self.assertEqual(updated.pop("realtime"), {"mode": "auto", "policy": "rr", "priority": 10})
        self.assertEqual(updated, config)
        self.assertEqual(mode, 0o640)

    def test_existing_rt_policy_is_preserved_byte_for_byte(self):
        for policy in ({"mode": "off"}, {"mode": "on", "policy": "fifo", "priority": 25}):
            with self.subTest(policy=policy):
                result, before, after, _ = self.migrate({"realtime": policy})
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(after, before)

    def test_dry_run_keeps_existing_config_unchanged(self):
        result, before, after, _ = self.migrate({"robot_home": "/home/robot"}, dry_run=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("[dry-run] add realtime", result.stdout)
        self.assertEqual(after, before)

    def test_non_object_config_fails_without_rewriting(self):
        result, before, after, _ = self.migrate([])
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("must be a JSON object", result.stderr)
        self.assertEqual(after, before)

    def test_service_keeps_supervisor_normal_and_allows_child_rt(self):
        unit = (ROOT / "robot_manager.service").read_text()
        self.assertIn("CPUSchedulingPolicy=other", unit)
        self.assertIn("LimitRTPRIO=99", unit)


if __name__ == "__main__":
    unittest.main()
