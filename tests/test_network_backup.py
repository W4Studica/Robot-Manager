import importlib.util
import os
from pathlib import Path
import stat
import subprocess
import tempfile
import unittest

SCRIPT = Path(__file__).resolve().parents[1] / "system/network_backup.py"
spec = importlib.util.spec_from_file_location("network_backup", SCRIPT)
backup = importlib.util.module_from_spec(spec)
spec.loader.exec_module(backup)


class NetworkBackupTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.calls = []
        self.original_manager = "NetworkManager.service"

    def run_command(self, args, check=True):
        self.calls.append(args)
        if args[1] == "is-active":
            return "active" if args[-1] == self.original_manager else "inactive"
        if args[1] == "is-enabled":
            return "enabled" if args[-1] == self.original_manager else "disabled"
        return ""

    def write(self, name, text, mode=0o600):
        path = self.root / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text)
        path.chmod(mode)
        return path

    def test_reinstall_and_repeated_restore_keep_the_first_original(self):
        original = self.write(backup.NETPLAN + "/home.yaml", "network: {version: 2, renderer: NetworkManager}\n")
        wifi = self.write(backup.NM + "/home.nmconnection", "[wifi]\nssid=home\n")
        cloud = self.write(backup.CLOUD, "original cloud configuration\n")
        state = backup.snapshot(self.root, self.run_command)
        self.assertEqual(stat.S_IMODE(state.stat().st_mode), 0o700)
        original.unlink()
        wifi.unlink()
        self.write(backup.NETPLAN + "/" + backup.MANAGED, "installer config")
        self.write(backup.NM + "/robot-ap.nmconnection", "installer ap")
        cloud.write_text("installer cloud configuration")
        # A reinstall must not replace the original with the installed settings.
        backup.snapshot(self.root, self.run_command)
        for _ in range(2):
            backup.restore(self.root, self.run_command)
            self.assertIn("NetworkManager", original.read_text())
            self.assertEqual(wifi.read_text(), "[wifi]\nssid=home\n")
            self.assertEqual(stat.S_IMODE(wifi.stat().st_mode), 0o600)
            self.assertEqual(cloud.read_text(), "original cloud configuration\n")
            self.assertFalse((self.root / backup.NETPLAN / backup.MANAGED).exists())
        self.assertIn(["systemctl", "start", "NetworkManager.service"], self.calls)
        self.assertNotIn(["systemctl", "start", "systemd-networkd.service"], self.calls)
        self.assertIn(["nmcli", "connection", "reload"], self.calls)

    def test_networkd_and_absent_cloud_config_are_restored(self):
        self.original_manager = "systemd-networkd.service"
        self.write(backup.NETPLAN + "/home.yaml", "network: {version: 2, renderer: networkd}")
        backup.snapshot(self.root, self.run_command)
        self.write(backup.CLOUD, "installer cloud config")
        backup.restore(self.root, self.run_command)
        self.assertFalse((self.root / backup.CLOUD).exists())
        self.assertIn(["systemctl", "start", "systemd-networkd.service"], self.calls)
        self.assertNotIn(["systemctl", "start", "NetworkManager.service"], self.calls)

    def test_legacy_migration_skips_installer_only_reinstall_backups(self):
        self.write(backup.NETPLAN + "/" + backup.MANAGED, "installer")
        self.write(backup.NETPLAN + "/w4studica-backup-1/home.yaml", "original")
        self.write(backup.NETPLAN + "/w4studica-backup-2/" + backup.MANAGED, "installer")
        self.write(backup.NM + "/robot-ap.nmconnection", "installer ap")
        state = backup.snapshot(self.root, self.run_command)
        self.assertEqual((state / "netplan/home.yaml").read_text(), "original")
        self.assertFalse((state / "netplan" / backup.MANAGED).exists())
        backup.restore(self.root, self.run_command)
        self.assertFalse((self.root / backup.NM / "robot-ap.nmconnection").exists())

    def test_incomplete_backup_is_not_accepted_by_reinstall(self):
        state = backup.snapshot(self.root, self.run_command)
        (state / "nm").rmdir()
        with self.assertRaisesRegex(RuntimeError, "Incomplete"):
            backup.snapshot(self.root, self.run_command)

    def test_no_original_backup_fails_before_changing_network(self):
        managed = self.write(backup.NETPLAN + "/" + backup.MANAGED, "installer")
        with self.assertRaisesRegex(RuntimeError, "Original netplan"):
            backup.snapshot(self.root, self.run_command)
        self.assertEqual(managed.read_text(), "installer")
        self.assertEqual(self.calls, [])


class InstallNetworkOrderTest(unittest.TestCase):
    def run_network(self, fail=False):
        source = (SCRIPT.parents[1] / "install.sh").read_text()
        # Execute the actual installer functions with side effects replaced by a
        # command trace. No host /etc files, packages, or services are changed.
        definitions = source.split('\n[ -n "$LOGIN_USER" ] || LOGIN_USER=', 1)[0]
        harness = r'''
DRY_RUN=0
ready=0
command() { if [ "$2" = iw ] && [ "$ready" = 0 ]; then return 1; fi; return 0; }
run() { printf 'RUN'; printf ' <%s>' "$@"; printf '\n'; }
write_file() { cat >/dev/null; printf 'WRITE <%s>\n' "$1"; }
apt_install_packages() { echo DEPS; ready=1; FAIL_DEPENDENCIES; }
apply_network
'''.replace("FAIL_DEPENDENCIES", "return 1" if fail else "return 0")
        with tempfile.TemporaryDirectory() as directory:
            script = Path(directory) / "install-test.sh"
            script.write_text(definitions + harness)
            return subprocess.run(["bash", str(script)], env={**os.environ, "SSH_CONNECTION": ""},
                                  capture_output=True, text=True, timeout=10)

    def test_dependencies_precede_configuration_and_no_early_disconnect(self):
        result = self.run_network()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertLess(result.stdout.index("DEPS"), result.stdout.index("setupEthernet"))
        self.assertNotIn("<stop>", result.stdout)
        self.assertNotIn("<rm> <-f> </etc/NetworkManager", result.stdout)

    def test_package_failure_never_reaches_network_mutations(self):
        result = self.run_network(fail=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertNotIn("setupEthernet", result.stdout)
        self.assertNotIn("WRITE", result.stdout)
        self.assertNotIn("<stop>", result.stdout)


if __name__ == "__main__":
    unittest.main()
