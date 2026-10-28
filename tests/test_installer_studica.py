import os
from pathlib import Path
import shlex
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
DEFINITIONS = (ROOT / "install.sh").read_text().split('\n[ -n "$LOGIN_USER" ] || LOGIN_USER=', 1)[0]


class StudicaInstallerTests(unittest.TestCase):
    def run_function(self, script, *arguments):
        with tempfile.TemporaryDirectory() as directory:
            harness = Path(directory) / "installer-test.sh"
            package = Path(directory) / "vmxpi-hal.deb"
            package.touch()
            harness.write_text(DEFINITIONS + f'''
RM_DIR={shlex.quote(str(ROOT))}
AW_DIR={shlex.quote(directory + '/absent-allwpilib')}
VMX_HAL_DEB={shlex.quote(str(package))}
''' + script)
            return subprocess.run(["bash", str(harness), *arguments], capture_output=True,
                                  text=True, env=os.environ, timeout=10)

    def test_default_install_validates_without_allwpilib_and_does_not_build_studica(self):
        result = self.run_function('''
run() { die "unexpected mutation: $*"; }
validate_install_inputs
install_drivers
install_plugin
install_studica_notices
echo "VALIDATED"
''')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("VALIDATED", result.stdout)
        self.assertEqual(result.stdout.count("supplied by GradleRIO deploy"), 2)
        self.assertNotIn("absent-allwpilib", result.stderr)

    def test_development_build_requires_allwpilib_sources(self):
        result = self.run_function("validate_install_inputs\n", "--build-studica")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("development Studica build needs the allwpilib sources", result.stderr)

    def test_upgrade_drivers_explicitly_enables_only_driver_build(self):
        result = self.run_function('echo "$SKIP_DRIVERS $SKIP_PLUGIN $UPGRADE_DRIVERS"\n', "--upgrade-drivers")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout.strip(), "0 1 1")

    def test_development_build_respects_subsequent_skip_options(self):
        result = self.run_function('echo "$SKIP_DRIVERS $SKIP_PLUGIN"\n', "--build-studica", "--skip-plugin")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout.strip(), "0 1")

    def test_default_install_still_provisions_the_original_vmx_hal(self):
        result = self.run_function('''
SDK_READY=0
have_vmx() { [ "$SDK_READY" = 1 ]; }
apt-get() { die "host apt must never run"; }
run() { printf 'RUN'; printf ' %s' "$@"; printf '\\n'; SDK_READY=1; }
ensure_vmx_hal_sdk
''')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("Installing VMX HAL SDK package:", result.stdout)
        self.assertIn("apt-get -o DPkg::Lock::Timeout=600 install -y", result.stdout)

    def test_missing_sdk_after_package_install_is_an_error_even_without_local_builds(self):
        result = self.run_function('''
have_vmx() { return 1; }
apt-get() { die "host apt must never run"; }
run() { :; }
ensure_vmx_hal_sdk
''')
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("VMXPi.h/libvmxpi_hal_cpp.so are still missing", result.stderr)

    def test_installed_sdk_does_not_invoke_package_manager(self):
        result = self.run_function('''
have_vmx() { return 0; }
run() { die "unexpected mutation: $*"; }
ensure_vmx_hal_sdk
''')
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_default_runtime_helpers_do_not_require_a_cpp_compiler(self):
        result = self.run_function('''
c_compiler_works() { return 0; }
cpp_compiler_works() { die "unexpected C++ compiler probe"; }
make() { :; }
compiler_major() { die "unexpected C++ version probe"; }
apt_install_packages() { die "unexpected package install"; }
tool_version() { echo fake; }
ensure_build_dependencies
''')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertNotIn("Studica compiler:", result.stdout)

    def test_missing_c_helpers_install_only_c_toolchain_packages(self):
        result = self.run_function('''
DRY_RUN=1
c_compiler_works() { return 1; }
cpp_compiler_works() { die "unexpected C++ compiler probe"; }
apt_install_packages() { printf '%s\\n' "$@"; }
tool_version() { echo fake; }
ensure_build_dependencies
''')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("gcc\nmake\nlibc6-dev", result.stdout)
        self.assertNotIn("build-essential", result.stdout)


if __name__ == "__main__":
    unittest.main()
