import os
from pathlib import Path
import shlex
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
DEFINITIONS = (ROOT / "install.sh").read_text().split('\n[ -n "$LOGIN_USER" ] || LOGIN_USER=', 1)[0]


class DeployedRuntimeTests(unittest.TestCase):
    def check(self, hal, active, *, broken=False):
        with tempfile.TemporaryDirectory(prefix="runtime check ") as directory:
            root = Path(directory)
            libs = root / "wpilib/third-party/lib"
            libs.mkdir(parents=True)
            names = ["libhalsim_studica.so", "libhalsim_studicad.so", "libhalsim_vmx.so", "libhalsim_vmxd.so", hal]
            if broken:
                names.append("libbroken.so")
            for name in names:
                (libs / name).touch()
            executable = root / "deploy/robot executable"
            executable.parent.mkdir()
            executable.touch()
            (root / "robotCommand").write_text(
                "HALSIM_EXTENSIONS=" + shlex.quote(str(libs / active)) + " " + shlex.quote(str(executable)) + "\n"
            )
            harness = root / "check.sh"
            harness.write_text(DEFINITIONS + f'''
ROBOT_HOME={shlex.quote(directory)}
CHECK_RUNTIME_DEPS=1
ldd() {{ die "host ldd must never run on these fixtures"; }}
verify_shared_library() {{
  printf 'CHECK %s\\n' "$(basename "$1")"
  case "$(basename "$1")" in
    libhalsim_studicad.so|libhalsim_vmxd.so)
      [ -f "$ROBOT_HOME/wpilib/third-party/lib/libwpiHald.so" ] || die 'missing debug HAL' ;;
    libhalsim_studica.so|libhalsim_vmx.so)
      [ -f "$ROBOT_HOME/wpilib/third-party/lib/libwpiHal.so" ] || die 'missing release HAL' ;;
    libbroken.so) die 'missing real dependency' ;;
  esac
}}
check_deployed_wpilib_runtime
''')
            return subprocess.run(["bash", str(harness)], capture_output=True, text=True, env=os.environ, timeout=10)

    def test_release_deploy_skips_both_old_and_new_unused_debug_extensions(self):
        result = self.check("libwpiHal.so", "libhalsim_studica.so")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("skipping libhalsim_studicad.so", result.stdout)
        self.assertIn("skipping libhalsim_vmxd.so", result.stdout)
        self.assertIn("CHECK libhalsim_studica.so", result.stdout)
        self.assertIn("CHECK robot executable", result.stdout)

    def test_debug_deploy_skips_unused_release_extensions(self):
        result = self.check("libwpiHald.so", "libhalsim_studicad.so")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("skipping libhalsim_studica.so", result.stdout)
        self.assertIn("skipping libhalsim_vmx.so", result.stdout)
        self.assertIn("CHECK libhalsim_studicad.so", result.stdout)

    def test_explicitly_selected_broken_debug_extension_is_never_skipped(self):
        for name in ["libhalsim_studicad.so", "libhalsim_vmxd.so"]:
            with self.subTest(name=name):
                result = self.check("libwpiHal.so", name)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("missing debug HAL", result.stderr)

    def test_real_unresolved_dependencies_still_fail_validation(self):
        result = self.check("libwpiHal.so", "libhalsim_studica.so", broken=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("missing real dependency", result.stderr)

    def test_storage_dependencies_never_replace_the_installed_initramfs_provider(self):
        with tempfile.TemporaryDirectory() as directory:
            harness = Path(directory) / "dependencies.sh"
            harness.write_text(DEFINITIONS + '''
dpkg-query() { echo 'unknown ok not-installed'; }
apt_install_packages() { printf '%s\\n' "$@"; }
ensure_system_profile_dependencies
''')
            result = subprocess.run(["bash", str(harness)], capture_output=True, text=True, env=os.environ, timeout=10)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn("systemd-zram-generator", result.stdout)
            for package in ["initramfs-tools", "dracut", "dracut-core"]:
                self.assertNotIn(package, result.stdout)


if __name__ == "__main__":
    unittest.main()
