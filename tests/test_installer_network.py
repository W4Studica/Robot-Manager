import os
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]


class InstallNetworkOrderTest(unittest.TestCase):
    def run_network(self, fail=False):
        source = (ROOT / "install.sh").read_text()
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

    def test_install_links_commands_and_retires_old_python_scripts(self):
        definitions = (ROOT / "install.sh").read_text().split('\n[ -n "$LOGIN_USER" ] || LOGIN_USER=', 1)[0]
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            opt = root / "opt"
            (opt / "net").mkdir(parents=True)
            (opt / "bin").mkdir()
            (root / "commands").mkdir()
            binary = opt / "robot_manager"
            binary.write_text("prebuilt Rust executable")
            (opt / "net/vmx_net.py").write_text("old network script")
            (opt / "bin/network_backup.py").write_text("old backup script")
            harness = r'''
DRY_RUN=0
OPT_DIR="$TEST_NETWORK_ROOT/opt"
COMMAND_DIR="$TEST_NETWORK_ROOT/commands"
run() {
  if [ "$1" = ln ] && [[ "$4" = /usr/local/bin/* ]]; then
    command ln "$2" "$3" "$COMMAND_DIR/${4##*/}"
  else
    "$@"
  fi
}
apply_network() { echo PREPARED; }
install_network
'''
            script = root / "install-links.sh"
            script.write_text(definitions + harness)
            result = subprocess.run(["bash", str(script)], env={**os.environ, "TEST_NETWORK_ROOT": str(root)}, capture_output=True, text=True, timeout=10)
            self.assertEqual(result.returncode, 0, result.stderr)
            for name in ("setupHotspot", "setupEthernet", "toggleNetMode"):
                self.assertEqual((root / "commands" / name).resolve(), binary)
            self.assertEqual((opt / "bin/network_backup").resolve(), binary)
            self.assertFalse((opt / "net/vmx_net.py").exists())
            self.assertFalse((opt / "bin/network_backup.py").exists())
            self.assertIn("PREPARED", result.stdout)

    def test_package_failure_never_reaches_network_mutations(self):
        result = self.run_network(fail=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertNotIn("setupEthernet", result.stdout)
        self.assertNotIn("WRITE", result.stdout)
        self.assertNotIn("<stop>", result.stdout)


if __name__ == "__main__":
    unittest.main()
