"""The VMX CPU partition: Core 4 (CPU 3) runs every W4Studica system component,
the robot program owns all other CPUs, and network IRQs stay off the robot CPUs."""
import os
import re
import unittest

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))

# Every unit the installer deploys must declare the reserved core.
SERVICE_FILES = [
    "robot_manager.service",
    "system/hwmon.service",
    "system/irqconf.service",
    "system/mrccomm.service",
    "system/robot_cpu_max.service",
    "system/ttyd.service",
    "system/w4studica-rfkill-unblock.service",
]
RESERVED_CORE = "3"


def read(path):
    with open(os.path.join(ROOT, path), encoding="utf-8") as f:
        return f.read()


class ServicePinTest(unittest.TestCase):
    def test_every_deployed_unit_pins_to_the_reserved_core(self):
        for unit in SERVICE_FILES:
            with self.subTest(unit=unit):
                text = read(unit)
                self.assertIn("[Service]", text)
                self.assertRegex(
                    text,
                    rf"(?m)^CPUAffinity={RESERVED_CORE}$",
                    f"{unit} must pin to the reserved system core",
                )

    def test_no_unit_allows_the_whole_cpu_set(self):
        for unit in SERVICE_FILES:
            with self.subTest(unit=unit):
                text = read(unit)
                for line in text.splitlines():
                    if line.startswith("CPUAffinity="):
                        self.assertNotIn(",", line, f"{unit}: a single core only")
                        self.assertNotIn("-", line, f"{unit}: a single core only")


class SourcePinTest(unittest.TestCase):
    def test_supervisor_constant_is_the_single_source_of_truth(self):
        text = read("src/realtime.rs")
        self.assertRegex(text, r"SUPERVISOR_CORE: usize = 3;")
        # system daemons pin through the constant, not a literal
        self.assertIn("pin_to_core(crate::realtime::SUPERVISOR_CORE)", read("src/hwmon.rs"))
        self.assertIn("pin_to_core(crate::realtime::SUPERVISOR_CORE)", read("src/mrccomm.rs"))

    def test_network_interrupts_avoid_the_robot_cpus(self):
        text = read("src/irqconf.rs")
        mask = re.search(r'NETWORK_CPU_MASK: &str = "(\d+)"', text)
        self.assertIsNotNone(mask)
        self.assertEqual(int(mask.group(1)), 1 << int(RESERVED_CORE))

    def test_installer_deploys_every_system_unit(self):
        text = read("install.sh")
        for unit in SERVICE_FILES:
            name = os.path.basename(unit)
            with self.subTest(unit=name):
                self.assertIn(name, text)


if __name__ == "__main__":
    unittest.main()
