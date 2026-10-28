import os
from pathlib import Path
import subprocess
import tempfile
import unittest


INSTALLER = Path(__file__).resolve().parents[1] / "install.sh"


class AptLockWaitTest(unittest.TestCase):
    def test_package_install_waits_for_automatic_update_locks(self):
        source = INSTALLER.read_text()
        definitions = source.split('\n[ -n "$LOGIN_USER" ] || LOGIN_USER=', 1)[0]
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            fake_bin = root / "bin"
            fake_bin.mkdir()
            log = root / "apt.log"
            fake_apt = fake_bin / "apt-get"
            fake_apt.write_text('#!/bin/sh\nprintf "%s\\n" "$*" >> "$APT_LOG"\n')
            fake_apt.chmod(0o755)
            script = root / "test.sh"
            script.write_text(definitions + '\nINSTALL_DEPS=1\nDRY_RUN=0\napt_install_packages test-package test-package\n')
            result = subprocess.run(["bash", str(script)], env={
                **os.environ, "PATH": str(fake_bin) + os.pathsep + os.environ["PATH"], "APT_LOG": str(log)
            }, capture_output=True, text=True, timeout=10)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(log.read_text().splitlines(), [
                "-o DPkg::Lock::Timeout=600 update",
                "-o DPkg::Lock::Timeout=600 install -y --no-install-recommends --no-remove test-package",
            ])
            self.assertIn("Waiting up to 10 minutes", result.stdout)


if __name__ == "__main__":
    unittest.main()
