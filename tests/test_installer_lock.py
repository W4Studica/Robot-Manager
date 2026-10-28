import os
from pathlib import Path
import shlex
import subprocess
import tempfile
import time
import unittest

ROOT = Path(__file__).resolve().parents[1]
DEFINITIONS = (ROOT / "install.sh").read_text().split('\n[ -n "$LOGIN_USER" ] || LOGIN_USER=', 1)[0]


class InstallLockTests(unittest.TestCase):
    def test_parallel_install_is_rejected_and_lock_is_released_on_exit(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            lock = root / "run/install.lock"
            ready = root / "ready"
            first = root / "first.sh"
            first.write_text(DEFINITIONS + f'''
acquire_install_lock {shlex.quote(str(lock))}
touch {shlex.quote(str(ready))}
read -r release_lock
''')
            second = root / "second.sh"
            second.write_text(DEFINITIONS + f'\nacquire_install_lock {shlex.quote(str(lock))}\necho ACQUIRED\n')
            holder = subprocess.Popen(["bash", str(first)], stdin=subprocess.PIPE,
                                      stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
            try:
                deadline = time.monotonic() + 5
                while not ready.exists() and holder.poll() is None and time.monotonic() < deadline:
                    time.sleep(0.01)
                self.assertTrue(ready.exists(), "first installer did not acquire its lock")
                blocked = subprocess.run(["bash", str(second)], capture_output=True, text=True, timeout=5)
                self.assertNotEqual(blocked.returncode, 0)
                self.assertIn("another installation or maintenance action is running", blocked.stderr)
                self.assertNotIn("ACQUIRED", blocked.stdout)
                holder.communicate("release\n", timeout=5)
                self.assertEqual(holder.returncode, 0)
                retry = subprocess.run(["bash", str(second)], capture_output=True, text=True, timeout=5)
                self.assertEqual(retry.returncode, 0, retry.stderr)
                self.assertIn("ACQUIRED", retry.stdout)
            finally:
                if holder.poll() is None:
                    holder.kill()
                    holder.communicate()

    def test_dry_run_never_creates_a_lock(self):
        with tempfile.TemporaryDirectory() as directory:
            lock = Path(directory) / "run/install.lock"
            script = Path(directory) / "dry-run.sh"
            script.write_text(DEFINITIONS + f'\nDRY_RUN=1\nacquire_install_lock {shlex.quote(str(lock))}\n')
            result = subprocess.run(["bash", str(script)], capture_output=True, text=True, timeout=5)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertFalse(lock.parent.exists())

    def test_reinstall_does_not_disable_already_masked_or_static_services(self):
        with tempfile.TemporaryDirectory() as directory:
            script = Path(directory) / "units.sh"
            script.write_text(DEFINITIONS + f'''
DRY_RUN=1
RM_DIR={shlex.quote(str(ROOT))}
systemctl() {{
  case "$1:$2" in
    list-unit-files:rsyslog.service) echo 'rsyslog.service masked' ;;
    is-enabled:rsyslog.service) echo masked ;;
    list-unit-files:zramswap.service) echo 'zramswap.service static' ;;
    is-enabled:zramswap.service) echo static ;;
  esac
}}
install_system_profile
''')
            result = subprocess.run(["bash", str(script)], capture_output=True, text=True, timeout=5)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertNotIn("[dry-run] systemctl disable", result.stdout)
            self.assertNotIn("[dry-run] systemctl mask rsyslog.service", result.stdout)
            self.assertIn("[dry-run] systemctl mask zramswap.service", result.stdout)


if __name__ == "__main__":
    unittest.main()
