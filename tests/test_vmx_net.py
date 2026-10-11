import os
import stat
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "net"))
import vmx_net as net  # noqa: E402

IW_PHY = """\
		Band 1:
			* 2412.0 MHz [1] (20.0 dBm)
		Band 2:
			* 5180.0 MHz [36] (22.0 dBm)
			* 5200.0 MHz [40] (22.0 dBm)
			* 5260.0 MHz [52] (22.0 dBm) (no IR, radar detection)
			* 5500.0 MHz [100] (22.0 dBm) (radar detection)
			* 5700.0 MHz [140] (disabled)
			* 5745.0 MHz [149] (22.0 dBm)
"""


class FakeSystem(net.System):
    """Records the commands and answers them from a table; files go to a temporary directory."""

    def __init__(self, nm_active=True, root=True, ssh_connection=None, route_dev="eth0", wifi_connections=(), tty=False,
                 nmcli_missing=False, profiles=()):
        self.runs = []
        self.nm_active = nm_active
        self.root = root
        self.ssh_connection = ssh_connection
        self.route_dev = route_dev
        self.wifi_connections = list(wifi_connections)
        self.tty = tty
        self.answers = []
        self.nmcli_missing = nmcli_missing
        self.profiles = list(profiles)

    def interfaces(self):
        return ["eth0", "lo", "wlan0"]

    def is_wireless(self, name):
        return name == "wlan0"

    def run(self, args, check=True):
        self.runs.append(list(args))
        if args[0] == "nmcli" and self.nmcli_missing:
            raise FileNotFoundError("nmcli")
        if args[:2] == ["systemctl", "is-active"]:
            return "active\n" if self.nm_active else "inactive\n"
        if args[:5] == ["nmcli", "-t", "-f", "DEVICE,TYPE", "device"]:
            return "lo:loopback\neth0:ethernet\nwlan0:wifi\n"
        if args[:5] == ["nmcli", "-t", "-f", "NAME,UUID", "connection"]:
            return "".join(p + "\n" for p in self.profiles)
        if args[:5] == ["nmcli", "-t", "-f", "NAME,TYPE", "connection"]:
            return "".join("%s:802-11-wireless\n" % n for n in self.wifi_connections) + "Wired connection 1:802-3-ethernet\n"
        if args[:3] == ["ip", "-o", "route"]:
            return "192.168.0.220 dev %s src 192.168.0.222 uid 0\n" % self.route_dev
        if args[:2] == ["iw", "phy"]:
            return IW_PHY
        return ""

    def environ(self, name):
        return self.ssh_connection if name == "SSH_CONNECTION" else None

    def is_root(self):
        return self.root

    def interactive(self):
        return self.tty

    def ask(self, prompt):
        return self.answers.pop(0)

    def ran(self, *prefix):
        return any(r[:len(prefix)] == list(prefix) for r in self.runs)


class NetTestCase(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.mkdtemp()
        self._old = net.NM_DIR
        net.NM_DIR = self.dir

    def tearDown(self):
        net.NM_DIR = self._old

    def path(self, name):
        return os.path.join(self.dir, name + ".nmconnection")

    def read(self, name):
        with open(self.path(name), encoding="utf-8") as f:
            return f.read()


class HelperTest(unittest.TestCase):
    def test_team_number_to_address(self):
        self.assertEqual(net.team_to_address(1234), "10.12.34.2")
        self.assertEqual(net.team_to_address("4444"), "10.44.44.2")
        self.assertEqual(net.team_to_address(254), "10.2.54.2")
        self.assertEqual(net.team_to_address(1), "10.0.1.2")
        self.assertEqual(net.team_to_address(9999), "10.99.99.2")

    def test_bad_team_numbers(self):
        for bad in (0, 10000, -5, "abc", None):
            with self.assertRaises(net.NetError, msg=repr(bad)):
                net.team_to_address(bad)

    def test_ssid_is_prefix_dash_team(self):
        self.assertEqual(net.ssid_for("VMX", 1234), "VMX-1234")
        self.assertEqual(net.ssid_for("TestRobot", "4444"), "TestRobot-4444")

    def test_band_names(self):
        self.assertEqual(net.band_to_nm("2.4"), "bg")
        self.assertEqual(net.band_to_nm("5"), "a")
        self.assertEqual(net.band_to_nm("2.4GHz"), "bg")
        with self.assertRaises(net.NetError):
            net.band_to_nm("6")

    def test_password_length(self):
        net.check_wpa_password("password")
        with self.assertRaises(net.NetError):
            net.check_wpa_password("short")
        with self.assertRaises(net.NetError):
            net.check_wpa_password("x" * 64)

    def test_5ghz_channels_skip_disabled_radar_and_no_ir_and_the_2_4_band(self):
        self.assertEqual(net.parse_5ghz_channels(IW_PHY), [36, 40, 149])

    def test_2_4_channels_are_1_to_11(self):
        self.assertEqual(net.valid_channels("bg"), list(range(1, 12)))

    def test_the_default_access_point_keyfile(self):
        text = net.ap_keyfile("VMX-1234", 1234, "bg", 6, "password", "wlan0", connection_uuid="u")
        for line in ("id=robot-ap", "type=wifi", "interface-name=wlan0", "mode=ap", "ssid=VMX-1234", "band=bg", "channel=6",
                     "key-mgmt=wpa-psk", "psk=password", "method=shared", "address1=10.12.34.2/24", "autoconnect=true"):
            self.assertIn(line, text.splitlines())

    def test_the_ethernet_keyfile_is_172_22_11_2_with_a_dhcp_server(self):
        text = net.ethernet_keyfile("eth0", connection_uuid="u")
        self.assertIn("address1=172.22.11.2/24", text.splitlines())
        self.assertIn("method=shared", text.splitlines())
        self.assertIn("interface-name=eth0", text.splitlines())


class HotspotTest(NetTestCase):
    def test_defaults_of_the_vmx_image(self):
        system = FakeSystem()
        info = net.setup_hotspot(system, "VMX", 1234, "2.4", "password")
        self.assertEqual((info["ssid"], info["address"], info["channel"]), ("VMX-1234", "10.12.34.2", 6))
        text = self.read("robot-ap")
        self.assertIn("ssid=VMX-1234", text)
        self.assertIn("address1=10.12.34.2/24", text)
        self.assertTrue(system.ran("nmcli", "connection", "reload"))
        self.assertTrue(system.ran("nmcli", "connection", "up", "uuid", system.read_uuid(self.path("robot-ap"))))

    def test_the_documented_custom_example(self):
        info = net.setup_hotspot(FakeSystem(), "TestRobot", 4444, "2.4", "Checkers")
        self.assertEqual((info["ssid"], info["address"]), ("TestRobot-4444", "10.44.44.2"))
        self.assertIn("psk=Checkers", self.read("robot-ap"))

    def test_the_file_with_the_password_is_private(self):
        net.setup_hotspot(FakeSystem(), "VMX", 1234, "2.4", "password")
        self.assertEqual(stat.S_IMODE(os.stat(self.path("robot-ap")).st_mode), 0o600)

    def test_the_password_is_not_on_any_command_line(self):
        system = FakeSystem()
        net.setup_hotspot(system, "VMX", 1234, "2.4", "s3cretpass")
        self.assertFalse(any("s3cretpass" in " ".join(r) for r in system.runs))

    def test_a_short_password_changes_nothing(self):
        system = FakeSystem()
        with self.assertRaises(net.NetError):
            net.setup_hotspot(system, "VMX", 1234, "2.4", "short")
        self.assertFalse(os.path.exists(self.path("robot-ap")))
        self.assertFalse(system.ran("nmcli", "connection", "up"))

    def test_channel_must_be_valid_for_the_band(self):
        with self.assertRaises(net.NetError):
            net.setup_hotspot(FakeSystem(), "VMX", 1234, "2.4", "password", channel=12)
        info = net.setup_hotspot(FakeSystem(), "VMX", 1234, "2.4", "password", channel=11)
        self.assertEqual(info["channel"], 11)

    def test_the_channel_is_asked_for_on_a_terminal(self):
        system = FakeSystem(tty=True)
        system.answers = ["6"]
        self.assertEqual(net.setup_hotspot(system, "VMX", 1234, "2.4", "password")["channel"], 6)
        system = FakeSystem(tty=True)
        system.answers = [""]   # Enter keeps the default
        self.assertEqual(net.setup_hotspot(system, "VMX", 1234, "2.4", "password")["channel"], 6)

    def test_a_second_setup_keeps_the_connection_uuid(self):
        system = FakeSystem()
        net.setup_hotspot(system, "VMX", 1234, "2.4", "password")
        path = os.path.join(net.NM_DIR, "robot-ap.nmconnection")
        first = system.read_uuid(path)
        net.setup_hotspot(system, "VMX", 1234, "2.4", "password", channel=11)
        self.assertEqual(system.read_uuid(path), first)

    def test_5ghz_uses_only_the_channels_this_country_allows(self):
        info = net.setup_hotspot(FakeSystem(), "VMX", 1234, "5", "password")
        self.assertEqual((info["band"], info["channel"]), ("a", 36))
        with self.assertRaises(net.NetError):
            net.setup_hotspot(FakeSystem(), "VMX", 1234, "5", "password", channel=52)

    def test_needs_root_and_networkmanager(self):
        with self.assertRaises(net.NetError):
            net.setup_hotspot(FakeSystem(root=False), "VMX", 1234, "2.4", "password")
        with self.assertRaises(net.NetError):
            net.setup_hotspot(FakeSystem(nm_active=False), "VMX", 1234, "2.4", "password")

    def test_a_change_that_cuts_the_ssh_session_is_made_by_the_robot_a_few_seconds_later(self):
        system = FakeSystem(ssh_connection="192.168.0.220 5555 192.168.0.222 22", route_dev="wlan0")
        info = net.setup_hotspot(system, "VMX", 1234, "5", "password", channel=36)
        self.assertTrue(info["deferred"])
        self.assertIn("channel=36", self.read("robot-ap"))
        self.assertFalse(any(r[:3] == ["nmcli", "connection", "up"] for r in system.runs))   # not now, in the session
        unit = [r for r in system.runs if r[0] == "systemd-run"]
        self.assertEqual(len(unit), 1)
        self.assertIn("--on-active=3", unit[0])
        self.assertEqual(unit[0][-3:-1], ["/bin/sh", "-c"])
        net_uuid = system.read_uuid(self.path("robot-ap"))
        self.assertEqual(unit[0][-1], "nmcli connection reload; nmcli connection up uuid " + net_uuid)
        # a session over the wired port is not cut: it is applied at once
        other = FakeSystem(ssh_connection="172.22.11.5 5555 172.22.11.2 22", route_dev="eth0")
        self.assertFalse(net.setup_hotspot(other, "VMX", 1234, "2.4", "password")["deferred"])
        self.assertIn(["nmcli", "connection", "up", "uuid", other.read_uuid(self.path("robot-ap"))], other.runs)
        self.assertFalse(any(r[0] == "systemd-run" for r in other.runs))

    def test_an_older_profile_of_the_same_name_is_deleted_and_the_new_one_brought_up_by_uuid(self):
        system = FakeSystem(profiles=["robot-ap:old-uuid", "robot-eth:eth-uuid", "Home:home-uuid"])
        net.setup_hotspot(system, "VMX", 1234, "2.4", "password")
        new = system.read_uuid(self.path("robot-ap"))
        self.assertIn(["nmcli", "connection", "delete", "uuid", "old-uuid"], system.runs)
        self.assertNotIn(["nmcli", "connection", "delete", "uuid", "eth-uuid"], system.runs)
        self.assertNotIn(["nmcli", "connection", "delete", "uuid", "home-uuid"], system.runs)
        self.assertEqual(system.runs[-1], ["nmcli", "connection", "up", "uuid", new])

    def test_the_old_force_flag_is_still_accepted(self):
        self.assertEqual(net.main(["setupHotspot", "--force", "VMX", "1234", "2.4"], FakeSystem()), 0)


class BeforeNetworkManagerTest(NetTestCase):
    """install.sh --network writes the connection files first and lets NetworkManager start afterwards."""

    def test_the_interfaces_are_found_without_nmcli(self):
        system = FakeSystem(nmcli_missing=True)
        self.assertEqual(net.wifi_interface(system), "wlan0")
        self.assertEqual(net.ethernet_interface(system), "eth0")

    def test_no_activate_only_writes_the_files(self):
        system = FakeSystem(nm_active=False, nmcli_missing=True,
                            ssh_connection="192.168.0.220 5555 192.168.0.222 22", route_dev="wlan0")
        net.setup_ethernet(system, apply=False)
        info = net.setup_hotspot(system, "VMX", 1234, "2.4", "password", apply=False)
        self.assertEqual(info["address"], "10.12.34.2")
        self.assertIn("address1=172.22.11.2/24", self.read("robot-eth"))
        self.assertIn("ssid=VMX-1234", self.read("robot-ap"))
        # no connection is touched, no NetworkManager check and no SSH guard (the switch-over is install.sh's job).
        # Asking nmcli which devices exist is fine: it fails when nmcli is not installed yet.
        self.assertFalse(any(r[:2] == ["nmcli", "connection"] for r in system.runs))
        self.assertFalse(any(r[:2] == ["systemctl", "is-active"] for r in system.runs))

    def test_the_command_line_flag(self):
        system = FakeSystem(nm_active=False, nmcli_missing=True)
        self.assertEqual(net.main(["setupHotspot", "--no-activate", "VMX", "1234", "2.4"], system), 0)
        self.assertEqual(net.main(["setupEthernet", "--no-activate"], system), 0)
        self.assertTrue(os.path.exists(self.path("robot-ap")))
        self.assertTrue(os.path.exists(self.path("robot-eth")))

    def test_without_the_flag_networkmanager_is_still_required(self):
        with self.assertRaises(net.NetError):
            net.setup_hotspot(FakeSystem(nm_active=False), "VMX", 1234, "2.4", "password")


class SshDeviceTest(NetTestCase):
    def test_the_device_the_session_comes_in_over(self):
        self.assertEqual(net.ssh_device(FakeSystem(ssh_connection="192.168.0.220 1 192.168.0.222 22", route_dev="wlan0")), "wlan0")
        self.assertEqual(net.ssh_device(FakeSystem(ssh_connection="172.22.11.5 1 172.22.11.2 22", route_dev="eth0")), "eth0")

    def test_no_ssh_session_means_no_device(self):
        self.assertEqual(net.ssh_device(FakeSystem()), "")

    def test_the_command_prints_it(self):
        import io
        import contextlib
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            net.main(["sshdevice"], FakeSystem(ssh_connection="192.168.0.220 1 192.168.0.222 22", route_dev="wlan0"))
        self.assertEqual(out.getvalue().strip(), "wlan0")

    def test_the_real_lookup_reads_the_variable_from_a_parent_process(self):
        # what sudo does: the child has no SSH_CONNECTION, its parent does
        import subprocess
        code = ("import sys; sys.path.insert(0, %r); import vmx_net; print(vmx_net.System().environ('SSH_CONNECTION'))"
                % os.path.join(os.path.dirname(__file__), "..", "net"))
        env = dict(os.environ, SSH_CONNECTION="10.1.2.3 1 10.1.2.4 22")
        # the trailing "; true" keeps the shell as the parent (a shell would otherwise exec the last command)
        script = "unset SSH_CONNECTION; python3 -c \"%s\"; true" % code.replace('"', '\\"')
        out = subprocess.run(["sh", "-c", script], env=env, capture_output=True, text=True, timeout=20)
        self.assertEqual(out.stdout.strip(), "10.1.2.3 1 10.1.2.4 22", out.stderr)


class EthernetTest(NetTestCase):
    def test_wired_port_gets_the_fixed_address(self):
        info = net.setup_ethernet(FakeSystem())
        self.assertEqual((info["interface"], info["address"]), ("eth0", "172.22.11.2/24"))
        self.assertIn("address1=172.22.11.2/24", self.read("robot-eth"))

    def test_a_wired_ssh_session_is_cut_by_the_robot_itself_later(self):
        wired_ssh = dict(ssh_connection="192.168.0.220 5555 192.168.0.222 22", route_dev="eth0")
        system = FakeSystem(**wired_ssh)
        self.assertTrue(net.setup_ethernet(system)["deferred"])
        self.assertTrue(any(r[0] == "systemd-run" for r in system.runs))
        # a session over Wi-Fi does not touch the wired port
        self.assertFalse(net.setup_ethernet(FakeSystem(ssh_connection="192.168.0.220 5555 192.168.0.222 22", route_dev="wlan0"))["deferred"])

    def test_an_interface_can_be_named(self):
        net.setup_ethernet(FakeSystem(), "end0")
        self.assertIn("interface-name=end0", self.read("robot-eth"))


class ToggleTest(NetTestCase):
    def test_ap_needs_the_access_point_to_exist(self):
        with self.assertRaises(net.NetError):
            net.toggle_net_mode(FakeSystem(), "ap")

    def test_ap_mode_turns_the_other_wifi_networks_off(self):
        net.setup_hotspot(FakeSystem(), "VMX", 1234, "2.4", "password")
        system = FakeSystem(wifi_connections=["robot-ap", "HomeRouter"])
        net.toggle_net_mode(system, "ap")
        self.assertTrue(system.ran("nmcli", "connection", "modify", "robot-ap", "connection.autoconnect", "yes"))
        self.assertTrue(system.ran("nmcli", "connection", "modify", "HomeRouter", "connection.autoconnect", "no"))
        self.assertTrue(system.ran("nmcli", "connection", "up", "robot-ap"))

    def test_wifi_mode_joins_a_new_router(self):
        net.setup_hotspot(FakeSystem(), "VMX", 1234, "2.4", "password")
        system = FakeSystem()
        net.toggle_net_mode(system, "wifi", "HomeRouter", "routerpass")
        text = self.read("robot-client")
        self.assertIn("ssid=HomeRouter", text)
        self.assertIn("psk=routerpass", text)
        self.assertEqual(stat.S_IMODE(os.stat(self.path("robot-client")).st_mode), 0o600)
        self.assertTrue(system.ran("nmcli", "connection", "modify", "robot-ap", "connection.autoconnect", "no"))
        self.assertTrue(system.ran("nmcli", "connection", "up", "robot-client"))
        self.assertFalse(any("routerpass" in " ".join(r) for r in system.runs))

    def test_wifi_mode_uses_a_saved_router(self):
        system = FakeSystem(wifi_connections=["robot-ap", "HomeRouter"])
        net.toggle_net_mode(system, "wifi")
        self.assertTrue(system.ran("nmcli", "connection", "up", "HomeRouter"))

    def test_wifi_mode_without_any_router_says_what_to_do(self):
        with self.assertRaises(net.NetError) as ctx:
            net.toggle_net_mode(FakeSystem(wifi_connections=["robot-ap"]), "wifi")
        self.assertIn("SSID PASSWORD", str(ctx.exception))

    def test_unknown_mode(self):
        with self.assertRaises(net.NetError):
            net.toggle_net_mode(FakeSystem(), "bridge")

    def test_both_directions_are_deferred_over_a_wifi_ssh_session(self):
        net.setup_hotspot(FakeSystem(), "VMX", 1234, "2.4", "password")
        wifi_ssh = dict(ssh_connection="192.168.0.220 5555 192.168.0.222 22", route_dev="wlan0")
        system = FakeSystem(**wifi_ssh)
        self.assertTrue(net.toggle_net_mode(system, "ap")["deferred"])
        self.assertFalse(any(r[:3] == ["nmcli", "connection", "up"] for r in system.runs))
        system = FakeSystem(wifi_connections=["Home"], **wifi_ssh)
        self.assertTrue(net.toggle_net_mode(system, "wifi")["deferred"])
        self.assertIn("nmcli connection up Home", [r for r in system.runs if r[0] == "systemd-run"][0][-1])


class CommandLineTest(NetTestCase):
    def test_setup_hotspot_with_the_default_password_when_none_is_given(self):
        system = FakeSystem()
        self.assertEqual(net.main(["setupHotspot", "VMX", "1234", "2.4"], system), 0)
        self.assertIn("psk=password", self.read("robot-ap"))

    def test_errors_are_a_message_and_exit_code_1(self):
        self.assertEqual(net.main(["setupHotspot", "VMX", "abc", "2.4"], FakeSystem()), 1)
        self.assertFalse(os.path.exists(self.path("robot-ap")))


if __name__ == "__main__":
    unittest.main()
