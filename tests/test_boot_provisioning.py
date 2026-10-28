import importlib.util
import json
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("boot_profile", ROOT / "system/storage/configure.py")
boot = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(boot)


class CompletedProvisioningTests(unittest.TestCase):
    def fixture(self, root):
        network = root / "etc/cloud/cloud.cfg.d/99-w4studica-disable-network-config.cfg"
        network.parent.mkdir(parents=True)
        network.write_text("network: {config: disabled}\n")
        return root / "etc/cloud/cloud-init.disabled"

    def test_successfully_provisioned_robot_disables_only_future_cloud_init_runs(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            marker = self.fixture(root)
            calls = []

            def run(*args):
                calls.append(args)
                return json.dumps({"status": "done", "extended_status": "done", "errors": [], "stage": None})

            boot.finish_provisioning(root, run)
            self.assertTrue(marker.is_file())
            self.assertEqual(calls, [("cloud-init", "status", "--format", "json")])
            before = marker.read_bytes()
            boot.finish_provisioning(root, lambda *args: self.fail("existing marker must not be rewritten"))
            self.assertEqual(marker.read_bytes(), before)

    def test_running_failed_and_degraded_provisioning_remains_enabled(self):
        for status in [
            {"status": "running"},
            {"status": "error"},
            {"status": "done", "extended_status": "degraded done"},
            {"status": "done", "errors": ["failed ssh setup"]},
            {"status": "done", "recoverable_errors": {"WARNING": ["configuration error"]}},
            {"status": "done", "stage": "modules-final"},
        ]:
            with self.subTest(status=status), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                marker = self.fixture(root)
                boot.finish_provisioning(root, lambda *args: json.dumps(status))
                self.assertFalse(marker.exists())

    def test_unprepared_network_cannot_disable_cloud_init(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            boot.finish_provisioning(root, lambda *args: self.fail("network provisioning is not ready"))
            self.assertFalse((root / "etc/cloud/cloud-init.disabled").exists())

    def test_missing_or_broken_status_command_never_disables_cloud_init(self):
        for error in [FileNotFoundError(), subprocess.CalledProcessError(2, ["cloud-init"])]:
            with self.subTest(error=error), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                marker = self.fixture(root)

                def run(*args):
                    raise error

                boot.finish_provisioning(root, run)
                self.assertFalse(marker.exists())

    def test_invalid_status_json_remains_enabled(self):
        for status in ["not json", "null", "[]", "{}"]:
            with self.subTest(status=status), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                marker = self.fixture(root)
                boot.finish_provisioning(root, lambda *args: status)
                self.assertFalse(marker.exists())


class NetworkWaitTests(unittest.TestCase):
    def fixture(self, root):
        files = {
            "etc/cloud/cloud-init.disabled": "# provisioned\n",
            "etc/fstab": "UUID=root / ext4 defaults 0 1\n",
            "proc/cmdline": "root=UUID=root rw\n",
        }
        for name, value in files.items():
            path = root / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(value)
        return root / "etc/systemd/system/NetworkManager-wait-online.service.d/99-w4studica-local-boot.conf"

    def query(self, *args):
        if args[2] == "remote-fs.target":
            return "remote-fs-pre.target"
        return "cloud-config.service cloud-final.service fwupd-refresh.service open-iscsi.service iscsid.service"

    def test_completed_local_robot_skips_wait_and_reinstall_is_idempotent(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            dropin = self.fixture(root)
            boot.configure_network_wait(root, self.query)
            self.assertIn("ConditionPathExists=/etc/robot_manager/require-network-online", dropin.read_text())
            before = dropin.read_bytes()
            boot.configure_network_wait(root, self.query)
            self.assertEqual(dropin.read_bytes(), before)

    def test_unsafe_or_unknown_network_requirements_restore_normal_waiting(self):
        for scenario in ["cloud", "fstab", "iscsi-node", "iscsi-session", "netboot", "mount-unit", "consumer", "manual", "query-failed"]:
            with self.subTest(scenario=scenario), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                dropin = self.fixture(root)
                dropin.parent.mkdir(parents=True)
                dropin.write_text("previous local-only override")
                if scenario == "cloud":
                    (root / "etc/cloud/cloud-init.disabled").unlink()
                elif scenario == "fstab":
                    with (root / "etc/fstab").open("a") as stream:
                        stream.write("server:/data /data nfs defaults 0 0\n")
                elif scenario in {"iscsi-node", "iscsi-session"}:
                    name = "etc/iscsi/nodes/node" if scenario == "iscsi-node" else "sys/class/iscsi_session/session1"
                    (root / name).mkdir(parents=True)
                elif scenario == "netboot":
                    (root / "proc/cmdline").write_text("root=UUID=root rd.neednet=1\n")
                elif scenario == "manual":
                    marker = root / "etc/robot_manager/require-network-online"
                    marker.parent.mkdir(parents=True)
                    marker.touch()

                def query(*args):
                    if scenario == "mount-unit" and args[2] == "remote-fs.target":
                        return "data.mount"
                    if scenario == "consumer" and args[2] == "network-online.target":
                        return "custom-network-client.service"
                    if scenario == "query-failed":
                        raise subprocess.CalledProcessError(1, ["systemctl"])
                    return self.query(*args)

                boot.configure_network_wait(root, query)
                self.assertFalse(dropin.exists())


if __name__ == "__main__":
    unittest.main()
