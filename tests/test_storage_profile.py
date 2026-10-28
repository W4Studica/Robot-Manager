import importlib.util
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("storage_profile", ROOT / "system/storage/configure.py")
storage = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(storage)
DEFINITIONS = (ROOT / "install.sh").read_text().split('\n[ -n "$LOGIN_USER" ] || LOGIN_USER=', 1)[0]


class StorageProfileTests(unittest.TestCase):
    def test_fstab_keeps_boot_and_comments_and_retires_other_swaps(self):
        before = "# robot\nUUID=root / ext4 defaults,data=writeback,nobarrier,noauto_da_alloc,lazytime 0 0 # root\nUUID=boot /boot/firmware vfat defaults 0 2\n/swap.img none swap sw 0 0\n"
        after, network = storage.fstab_profile(before)
        self.assertFalse(network)
        self.assertIn("UUID=boot /boot/firmware vfat defaults 0 2", after)
        self.assertIn("# w4studica: superseded swap: /swap.img", after)
        self.assertIn("data=ordered,barrier=1,auto_da_alloc,commit=5,errors=remount-ro\t0\t1 # root", after)
        self.assertNotIn("data=writeback", after)
        self.assertNotIn("nobarrier", after)
        self.assertEqual(storage.fstab_profile(after)[0], after)
        active = [line for line in after.splitlines() if line and not line.startswith("#") and line.split()[2] == "swap"]
        self.assertEqual(active, [f"{storage.SWAP_PATH}\tnone\tswap\tsw,pri=10,nofail\t0\t0"])

    def test_fstab_rejects_missing_duplicate_and_non_ext4_root(self):
        for text in ["", "UUID=x / btrfs defaults 0 1\n", "UUID=a / ext4 defaults 0 1\nUUID=b / ext4 defaults 0 1\n"]:
            with self.subTest(text=text), self.assertRaises(RuntimeError):
                storage.fstab_profile(text)

    def test_network_filesystems_keep_their_startup_wait_policy(self):
        for mount in ["server:/data /data nfs defaults 0 0", "//server/data /data cifs defaults 0 0", "UUID=data /data ext4 defaults,_netdev 0 2"]:
            self.assertTrue(storage.fstab_profile("UUID=root / ext4 defaults 0 1\n" + mount + "\n")[1])

    def test_new_swap_is_fully_allocated_private_and_published_after_mkswap(self):
        with tempfile.TemporaryDirectory() as directory:
            swap = Path(directory) / "swapfile"
            calls = []

            def run(*args):
                calls.append(args)
                self.assertFalse(swap.exists())
                staging = Path(args[1])
                self.assertEqual(staging.read_bytes(), bytes(8192))
                self.assertEqual(staging.stat().st_mode & 0o777, 0o600)
                return ""

            storage.prepare_swap(swap, run, 8192)
            self.assertEqual(swap.stat().st_size, 8192)
            self.assertEqual(calls[0][0], "mkswap")
            self.assertFalse(swap.with_name(".swapfile.new").exists())

    def test_failed_swap_format_never_publishes_a_swap_file(self):
        with tempfile.TemporaryDirectory() as directory:
            swap = Path(directory) / "swapfile"
            with self.assertRaises(RuntimeError):
                storage.prepare_swap(swap, lambda *args: (_ for _ in ()).throw(RuntimeError("format failed")), 8192)
            self.assertFalse(swap.exists())
            self.assertFalse(swap.with_name(".swapfile.new").exists())

    def test_existing_swap_is_never_reformatted_or_resized(self):
        with tempfile.TemporaryDirectory() as directory:
            swap = Path(directory) / "swapfile"
            swap.write_bytes(b"original" * 1024)
            before = swap.read_bytes()
            calls = []
            storage.prepare_swap(swap, lambda *args: calls.append(args) or "swap", len(before))
            self.assertEqual(calls[0][0], "blkid")
            self.assertEqual(swap.read_bytes(), before)
            with self.assertRaises(RuntimeError):
                storage.prepare_swap(swap, size=len(before) + 1)
            with self.assertRaises(RuntimeError):
                storage.prepare_swap(swap, lambda *args: "ext4", len(before))
            self.assertEqual(swap.read_bytes(), before)

    def test_interrupted_staging_is_recreated_and_symlinks_are_refused(self):
        with tempfile.TemporaryDirectory() as directory:
            swap = Path(directory) / "swapfile"
            stage = swap.with_name(".swapfile.new")
            stage.write_bytes(b"incomplete")
            storage.prepare_swap(swap, lambda *args: "", 8192)
            swap.unlink()
            victim = Path(directory) / "victim"
            victim.write_text("keep")
            stage.symlink_to(victim)
            with self.assertRaises(RuntimeError):
                storage.prepare_swap(swap, size=8192)
            self.assertEqual(victim.read_text(), "keep")

    def test_low_disk_space_does_not_create_swap(self):
        with tempfile.TemporaryDirectory() as directory:
            swap = Path(directory) / "swapfile"
            with patch.object(storage.shutil, "disk_usage", return_value=type("Usage", (), {"free": 0})()):
                with self.assertRaisesRegex(RuntimeError, "free disk"):
                    storage.prepare_swap(swap, size=8192)
            self.assertFalse(swap.exists())

    def fixture(self, root):
        for path, text in {"etc/os-release": "ID=ubuntu\n", "etc/fstab": "UUID=root / ext4 defaults 0 1\n/swap.img none swap sw 0 0\n", "proc/cmdline": "root=UUID=root rw\n"}.items():
            p = root / path
            p.parent.mkdir(parents=True, exist_ok=True)
            p.write_text(text)

    def query(self, *args):
        if args[0] == "findmnt":
            return "ext4" if args[3] == "FSTYPE" else "/dev/fake-root"
        if args[0] == "tune2fs":
            return "Filesystem features: has_journal ext_attr metadata_csum\n"
        if args[0] == "blkid":
            return "swap"
        self.assertEqual(args[0], "mkswap")
        return ""

    def test_full_profile_is_idempotent_and_preserves_the_original_backup(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.fixture(root)
            original = (root / "etc/fstab").read_text()
            real_prepare = storage.prepare_swap
            with patch.object(storage, "prepare_swap", side_effect=lambda path, run: real_prepare(path, run, 8192)):
                storage.configure(root, self.query)
                after = (root / "etc/fstab").read_text()
                storage.configure(root, self.query)
            self.assertEqual((root / "etc/fstab").read_text(), after)
            self.assertEqual((root / "var/lib/w4studica/fstab.before-storage-profile").read_text(), original)
            self.assertIn("zram-size = 8192", (root / "etc/systemd/zram-generator.conf.d/99-w4studica.conf").read_text())
            self.assertIn("Storage=volatile", (root / "etc/systemd/journald.conf.d/99-w4studica.conf").read_text())
            self.assertIn("--timeout=15", (root / "etc/systemd/system/NetworkManager-wait-online.service.d/99-w4studica.conf").read_text())
            self.assertFalse((root / "etc/initramfs-tools").exists())

    def test_no_journal_and_disabled_zram_fail_before_replacing_fstab(self):
        for no_journal in [True, False]:
            with self.subTest(no_journal=no_journal), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                self.fixture(root)
                before = (root / "etc/fstab").read_bytes()
                if not no_journal:
                    (root / "proc/cmdline").write_text("systemd.zram=0\n")

                def query(*args):
                    return "Filesystem features: ext_attr\n" if no_journal and args[0] == "tune2fs" else self.query(*args)

                with self.assertRaises(RuntimeError):
                    storage.configure(root, query)
                self.assertEqual((root / "etc/fstab").read_bytes(), before)
                self.assertFalse((root / storage.SWAP_PATH.lstrip("/")).exists())

    def test_swap_creation_failure_keeps_the_old_boot_configuration(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.fixture(root)
            before = (root / "etc/fstab").read_bytes()
            with patch.object(storage, "prepare_swap", side_effect=OSError("disk write failed")):
                with self.assertRaises(OSError):
                    storage.configure(root, self.query)
            self.assertEqual((root / "etc/fstab").read_bytes(), before)
            self.assertFalse((root / "etc/systemd/zram-generator.conf.d/99-w4studica.conf").exists())

    def test_storage_settings_preserve_existing_initramfs_resume_configuration(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.fixture(root)
            resume = root / "etc/initramfs-tools/conf.d/resume"
            resume.parent.mkdir(parents=True)
            resume.write_text("RESUME=none\n# existing OS policy\n")
            with patch.object(storage, "prepare_swap"):
                storage.configure(root, self.query)
            self.assertEqual(resume.read_text(), "RESUME=none\n# existing OS policy\n")

    def test_mounted_nondefault_journal_mode_is_not_changed_at_next_remount(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.fixture(root)
            before = (root / "etc/fstab").read_bytes()

            def query(*args):
                if args[0] == "findmnt" and args[3] == "OPTIONS":
                    return "rw,relatime,data=journal"
                return self.query(*args)

            with self.assertRaisesRegex(RuntimeError, "cannot be changed by remount"):
                storage.configure(root, query)
            self.assertEqual((root / "etc/fstab").read_bytes(), before)

    def test_conflicting_initramfs_root_flags_cannot_break_the_next_remount(self):
        for flag in ["data=writeback", "data=journal", "noload"]:
            with self.subTest(flag=flag), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                self.fixture(root)
                before = (root / "etc/fstab").read_bytes()
                (root / "proc/cmdline").write_text("rootflags=" + flag)
                with self.assertRaisesRegex(RuntimeError, "Conflicting rootflags"):
                    storage.configure(root, self.query)
                self.assertEqual((root / "etc/fstab").read_bytes(), before)

    def test_network_mount_removes_only_our_previous_wait_override(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.fixture(root)
            with (root / "etc/fstab").open("a") as stream:
                stream.write("server:/data /data nfs defaults 0 0\n")
            dropin = root / "etc/systemd/system/NetworkManager-wait-online.service.d"
            dropin.mkdir(parents=True)
            (dropin / "99-w4studica.conf").write_text("old override")
            custom = dropin / "50-custom.conf"
            custom.write_text("administrator policy")
            with patch.object(storage, "prepare_swap"):
                storage.configure(root, self.query)
            self.assertFalse((dropin / "99-w4studica.conf").exists())
            self.assertEqual(custom.read_text(), "administrator policy")

    def test_symlink_root_config_or_swap_cannot_overwrite_another_file(self):
        with tempfile.TemporaryDirectory() as directory:
            victim = Path(directory) / "victim"
            victim.write_text("keep")
            link = Path(directory) / "link"
            link.symlink_to(victim)
            with self.assertRaises(RuntimeError):
                storage.atomic_write(link, "replace")
            with self.assertRaises(RuntimeError):
                storage.prepare_swap(link, size=8192)
            self.assertEqual(victim.read_text(), "keep")

    def test_installer_preview_has_no_swapoff_remount_or_reboot(self):
        with tempfile.TemporaryDirectory() as directory:
            harness = Path(directory) / "installer-test.sh"
            harness.write_text(DEFINITIONS + f'''
RM_DIR={ROOT}
DRY_RUN=1
systemctl() {{ printf '%s enabled\\n' "$2"; }}
install_system_profile
''')
            result = subprocess.run(["bash", str(harness)], capture_output=True, text=True, env=os.environ, timeout=10)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn("[dry-run] python3", result.stdout)
            self.assertNotIn("update-initramfs", result.stdout)
            self.assertNotIn("[dry-run] dracut", result.stdout)
            self.assertIn("systemctl mask zramswap.service", result.stdout)
            for forbidden in ["[dry-run] swapoff", "[dry-run] swapon", "[dry-run] reboot", "[dry-run] mount", "--now"]:
                self.assertNotIn(forbidden, result.stdout)


if __name__ == "__main__":
    unittest.main()
