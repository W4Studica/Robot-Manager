import os
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
DEFINITIONS = (ROOT / "install.sh").read_text().split('\n[ -n "$LOGIN_USER" ] || LOGIN_USER=', 1)[0]


class RustInstallerTest(unittest.TestCase):
    def preview(self, prebuilt=False):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            binary = root / "prebuilt"
            binary.write_text("test binary")
            harness = f'''
DRY_RUN=1
RM_DIR='{ROOT}'
OPT_DIR='{root}/opt'
CONF_DIR='{root}/etc'
BUILD_DIR='{root}/build'
MANAGER_BINARY='{'{}'.format(binary) if prebuilt else ''}'
install_robot_manager
'''
            script = root / "preview.sh"
            script.write_text(DEFINITIONS + harness)
            result = subprocess.run(["bash", str(script)], env=os.environ, capture_output=True, text=True, timeout=10)
            self.assertEqual(result.returncode, 0, result.stderr)
            return result.stdout

    def test_build_is_locked_and_install_is_atomic(self):
        text = self.preview()
        self.assertIn("cargo build --release --locked", text)
        self.assertIn("robot_manager.new", text)
        self.assertIn("mv -f", text)
        self.assertIn("--check-config", text)
        self.assertNotRegex(text, r"install -m 0755 [^\n]*robot_manager\.py")

    def test_prebuilt_binary_skips_cargo(self):
        text = self.preview(True)
        self.assertNotIn("cargo build", text)
        self.assertIn("prebuilt", text)

    def test_service_executes_rust_binary_directly(self):
        unit = (ROOT / "robot_manager.service").read_text()
        self.assertIn("ExecStart=/opt/robot_manager/robot_manager --config", unit)
        self.assertNotIn("python", unit)

if __name__ == "__main__":
    unittest.main()
