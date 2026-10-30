"""The canonical swap/memory policy (RUNTIME_BASELINE.md) must be stated in one
place - system/storage/*.conf - and reused everywhere: systemd units may only
differ where the robot program shares the cgroup."""
import os
import re
import unittest

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))

LEAF_DAEMONS = ["system/hwmon.service", "system/mrccomm.service", "system/ttyd.service"]


def read(path):
    with open(os.path.join(ROOT, path), encoding="utf-8") as f:
        return f.read()


class ProfileFileTest(unittest.TestCase):
    def test_conf_files_carry_the_canonical_values(self):
        canonical = {
            "system/storage/zram.conf": [
                "zram-size = 8192",
                "compression-algorithm = zstd",
                "host-memory-limit = 2048",
                "swap-priority = 100",
                "fs-type = swap",
            ],
            "system/storage/memory.conf": [
                "vm.swappiness = 180",
                "vm.page-cluster = 0",
                "vm.vfs_cache_pressure = 50",
                "vm.watermark_boost_factor = 0",
                "vm.watermark_scale_factor = 125",
            ],
            "system/storage/journald.conf": [
                "Storage=volatile",
                "RuntimeMaxUse=64M",
                "RuntimeMaxFileSize=8M",
                "ForwardToSyslog=no",
            ],
            "system/storage/network-wait.conf": [
                "nm-online -s -q --timeout=15",
                "TimeoutStartSec=20s",
            ],
        }
        for path, keys in canonical.items():
            with self.subTest(path=path):
                text = read(path)
                for key in keys:
                    self.assertIn(key, text)

    def test_the_rust_binary_embeds_the_files_instead_of_retyping_them(self):
        text = read("src/storage.rs")
        for name in ["zram.conf", "journald.conf", "memory.conf", "network-wait.conf", "local-boot.conf"]:
            with self.subTest(name=name):
                self.assertIn(f'include_str!("../system/storage/{name}")', text)
        self.assertNotIn("zram-size =", text, "policy literals must not be duplicated in Rust")
        self.assertNotIn("vm.swappiness", text, "policy literals must not be duplicated in Rust")


class UnitPolicyTest(unittest.TestCase):
    def test_leaf_daemons_share_one_cap_and_may_swap_freely(self):
        for unit in LEAF_DAEMONS:
            with self.subTest(unit=unit):
                text = read(unit)
                self.assertRegex(text, r"(?m)^MemoryHigh=32M$")
                self.assertRegex(text, r"(?m)^MemorySwapMax=infinity$")
                self.assertNotRegex(text, r"(?m)^Memory(Max|Low)=")

    def test_robot_manager_protects_the_robot_instead_of_capping_it(self):
        text = read("robot_manager.service")
        # The robot program runs in this cgroup: a MemoryHigh would throttle it.
        self.assertNotRegex(text, r"(?m)^MemoryHigh=")
        self.assertRegex(text, r"(?m)^MemoryLow=256M$")
        self.assertRegex(text, r"(?m)^MemorySwapMax=infinity$")

    def test_no_other_unit_sets_memory_limits(self):
        for name in os.listdir(os.path.join(ROOT, "system")):
            if not name.endswith(".service") or f"system/{name}" in LEAF_DAEMONS:
                continue
            with self.subTest(unit=name):
                text = read(f"system/{name}")
                self.assertNotIn("MemoryHigh=", text)
                self.assertNotIn("MemoryLow=", text)


class InstallerTextTest(unittest.TestCase):
    def test_installer_describes_the_deployed_policy(self):
        text = read("install.sh")
        self.assertIn("zram 8 GiB (zstd", text)
        self.assertNotIn("zram-size = ram", text)


if __name__ == "__main__":
    unittest.main()
