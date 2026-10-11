import os
import stat
import subprocess
import tempfile
import unittest

ROOT = os.path.join(os.path.dirname(__file__), "..")
CPU_MAX = os.path.join(ROOT, "system", "robot_cpu_max.sh")


def write(path, text):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="utf-8") as f:
        f.write(text)


def read(path):
    with open(path, "r", encoding="utf-8") as f:
        return f.read().strip()


class CpuMaxTest(unittest.TestCase):
    """robot_cpu_max.sh against a fake /sys/devices/system/cpu."""

    def setUp(self):
        self.dir = tempfile.mkdtemp()

    def cpu(self, n, governors="ondemand performance schedutil", governor="ondemand", cur_min="600000", cur_max="1500000"):
        base = os.path.join(self.dir, "cpu%d" % n, "cpufreq")
        write(os.path.join(base, "scaling_available_governors"), governors + "\n")
        write(os.path.join(base, "scaling_governor"), governor + "\n")
        write(os.path.join(base, "scaling_min_freq"), cur_min + "\n")
        write(os.path.join(base, "scaling_max_freq"), cur_max + "\n")
        return base

    def run_script(self):
        return subprocess.run(["/bin/sh", CPU_MAX], env={"ROBOT_CPU_SYSFS": self.dir, "PATH": os.environ["PATH"]},
                              capture_output=True, text=True, timeout=20)

    def test_sets_performance_and_raises_the_minimum_on_every_cpu(self):
        bases = [self.cpu(i) for i in range(4)]
        result = self.run_script()
        self.assertEqual(result.returncode, 0, result.stderr)
        for base in bases:
            self.assertEqual(read(os.path.join(base, "scaling_governor")), "performance")
            self.assertEqual(read(os.path.join(base, "scaling_min_freq")), "1500000")
            self.assertEqual(read(os.path.join(base, "scaling_max_freq")), "1500000")

    def test_a_lowered_maximum_is_respected(self):
        base = self.cpu(0, cur_max="1000000")
        self.run_script()
        self.assertEqual(read(os.path.join(base, "scaling_min_freq")), "1000000")
        self.assertEqual(read(os.path.join(base, "scaling_max_freq")), "1000000")

    def test_a_cpu_without_the_governor_is_left_alone_and_the_script_still_succeeds(self):
        base = self.cpu(0, governors="powersave", governor="powersave")
        result = self.run_script()
        self.assertEqual(result.returncode, 0)
        self.assertEqual(read(os.path.join(base, "scaling_governor")), "powersave")
        self.assertIn("no 'performance' governor", result.stderr)

    def test_no_cpufreq_at_all_is_not_an_error(self):
        os.makedirs(os.path.join(self.dir, "cpu0"))   # a CPU without a cpufreq directory
        result = self.run_script()
        self.assertEqual(result.returncode, 0)
        self.assertIn("nothing to set", result.stderr)

    def test_the_script_is_executable_and_posix_sh(self):
        self.assertTrue(os.stat(CPU_MAX).st_mode & stat.S_IXUSR)
        with open(CPU_MAX, encoding="utf-8") as f:
            self.assertEqual(f.readline().strip(), "#!/bin/sh")


if __name__ == "__main__":
    unittest.main()
