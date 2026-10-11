#!/usr/bin/env python3
"""VMX networking, the way Studica's VMX image does it: setupHotspot and toggleNetMode.

  setupHotspot [--channel N] [--force] SSID TEAMNUMBER BAND [PASSWORD]
      Wi-Fi access point. SSID is a prefix: SSID "VMX" and team 1234 give the network "VMX-1234". BAND is 2.4 or 5.
      The VMX is 10.XX.YY.2 on it (team 1234 -> 10.12.34.2) and hands out addresses to the computers that join.
      Defaults of the VMX image: setupHotspot VMX 1234 2.4 password   (channel 1)
  toggleNetMode ap | wifi [SSID PASSWORD] [--force]
      ap: Wi-Fi is the access point.  wifi: the access point is off and the VMX joins a router (updates, packages).
  setupEthernet [--force] [IFACE]
      The wired port is always 172.22.11.2 and hands out 172.22.11.X addresses (a computer plugged in directly).
  vmx_net.py status

Everything is done through NetworkManager keyfiles. Nothing is run that could cut a Wi-Fi SSH session without --force.
Standard library only.
"""

import argparse
import os
import re
import subprocess
import sys
import uuid

NM_DIR = os.environ.get("VMX_NM_DIR", "/etc/NetworkManager/system-connections")
AP_NAME = "robot-ap"
CLIENT_NAME = "robot-client"
ETH_NAME = "robot-eth"
ETH_ADDRESS = "172.22.11.2/24"
DEFAULTS = {"ssid": "VMX", "team": 1234, "band": "2.4", "password": "password", "channel": 1}


class NetError(Exception):
    pass


# ---- pure helpers (tested without NetworkManager) ---------------------------------------------------------------------

def team_to_address(team):
    """10.TE.AM.2 for team TEAM (the FRC/WSR convention): 1234 -> 10.12.34.2, 4444 -> 10.44.44.2, 254 -> 10.2.54.2."""
    try:
        number = int(team)
    except (TypeError, ValueError):
        raise NetError("the team number must be a number, got %r" % (team,))
    if not 1 <= number <= 9999:
        raise NetError("the team number must be 1-9999, got %d" % number)
    return "10.%d.%d.2" % (number // 100, number % 100)


def ssid_for(prefix, team):
    return "%s-%d" % (prefix, int(team))


def band_to_nm(band):
    """nmcli/keyfile band names: bg (2.4 GHz) or a (5 GHz)."""
    text = str(band).strip().lower().replace("ghz", "").replace("g", "")
    if text in ("2.4", "2"):
        return "bg"
    if text == "5":
        return "a"
    raise NetError("the band must be 2.4 or 5, got %r" % (band,))


def valid_channels(nm_band, allowed_5ghz=None):
    if nm_band == "bg":
        return list(range(1, 12))  # 1-11: the channels every country allows
    return sorted(allowed_5ghz or [])


def check_wpa_password(password):
    if not 8 <= len(password) <= 63:
        raise NetError("a WPA2 password has 8 to 63 characters (it has %d)" % len(password))


def keyfile(sections):
    """NetworkManager keyfile text from [(section, [(key, value), ...]), ...]."""
    lines = []
    for name, items in sections:
        lines.append("[%s]" % name)
        lines.extend("%s=%s" % (k, v) for k, v in items)
        lines.append("")
    return "\n".join(lines)


def ap_keyfile(ssid, team, nm_band, channel, password, interface="wlan0", connection_uuid=None):
    return keyfile([
        ("connection", [("id", AP_NAME), ("uuid", connection_uuid or str(uuid.uuid4())), ("type", "wifi"),
                        ("interface-name", interface), ("autoconnect", "true"), ("autoconnect-priority", "100")]),
        ("wifi", [("mode", "ap"), ("ssid", ssid), ("band", nm_band), ("channel", str(channel))]),
        ("wifi-security", [("key-mgmt", "wpa-psk"), ("psk", password)]),
        ("ipv4", [("method", "shared"), ("address1", team_to_address(team) + "/24")]),
        ("ipv6", [("method", "ignore")]),
    ])


def ethernet_keyfile(interface, connection_uuid=None):
    return keyfile([
        ("connection", [("id", ETH_NAME), ("uuid", connection_uuid or str(uuid.uuid4())), ("type", "ethernet"),
                        ("interface-name", interface), ("autoconnect", "true"), ("autoconnect-priority", "100")]),
        ("ipv4", [("method", "shared"), ("address1", ETH_ADDRESS)]),
        ("ipv6", [("method", "ignore")]),
    ])


def client_keyfile(ssid, password, interface="wlan0", connection_uuid=None):
    sections = [
        ("connection", [("id", CLIENT_NAME), ("uuid", connection_uuid or str(uuid.uuid4())), ("type", "wifi"),
                        ("interface-name", interface), ("autoconnect", "true")]),
        ("wifi", [("mode", "infrastructure"), ("ssid", ssid)]),
    ]
    if password:
        sections.append(("wifi-security", [("key-mgmt", "wpa-psk"), ("psk", password)]))
    sections += [("ipv4", [("method", "auto")]), ("ipv6", [("method", "auto")])]
    return keyfile(sections)


def parse_5ghz_channels(iw_phy_output):
    """Channels of the 5 GHz band that are usable now, from `iw phy` text: entries that are not disabled/radar/no IR."""
    channels = []
    for line in iw_phy_output.splitlines():
        m = re.match(r"\s*\*\s*(\d+)\s*MHz\s*\[(\d+)\](.*)", line)
        if not m:
            continue
        freq, channel, rest = int(m.group(1)), int(m.group(2)), m.group(3)
        if freq < 5000:
            continue
        if any(flag in rest for flag in ("disabled", "radar detection", "no IR")):
            continue
        channels.append(channel)
    return channels


# ---- system access (replaced by fakes in the tests) -------------------------------------------------------------------

class System:
    def run(self, args, check=True):
        result = subprocess.run(args, capture_output=True, text=True, timeout=60)
        if check and result.returncode != 0:
            raise NetError("%s failed: %s" % (" ".join(args[:3]), (result.stderr or result.stdout).strip()))
        return result.stdout

    def write_secret(self, path, text):
        os.makedirs(os.path.dirname(path), exist_ok=True)
        fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
        with os.fdopen(fd, "w", encoding="utf-8") as f:
            f.write(text)
        os.chmod(path, 0o600)

    def exists(self, path):
        return os.path.exists(path)

    def remove(self, path):
        os.remove(path)

    def environ(self, name):
        return os.environ.get(name)

    def is_root(self):
        return os.geteuid() == 0

    def interactive(self):
        return sys.stdin.isatty()

    def ask(self, prompt):
        return input(prompt)


def network_manager_active(system):
    return system.run(["systemctl", "is-active", "NetworkManager"], check=False).strip() == "active"


def wifi_interface(system):
    for line in system.run(["nmcli", "-t", "-f", "DEVICE,TYPE", "device"], check=False).splitlines():
        device, _, kind = line.partition(":")
        if kind == "wifi":
            return device
    return "wlan0"


def ethernet_interface(system):
    for line in system.run(["nmcli", "-t", "-f", "DEVICE,TYPE", "device"], check=False).splitlines():
        device, _, kind = line.partition(":")
        if kind == "ethernet":
            return device
    return "eth0"


def ssh_arrives_through(system, interface):
    """True if the SSH session this is run from comes in over `interface` (so changing it would cut the session)."""
    connection = system.environ("SSH_CONNECTION")
    if not connection:
        return False
    client = connection.split()[0]
    route = system.run(["ip", "-o", "route", "get", client], check=False)
    m = re.search(r"\bdev\s+(\S+)", route)
    return bool(m) and m.group(1) == interface


def guard_ssh(system, interface, force, what):
    if not force and ssh_arrives_through(system, interface):
        raise NetError(
            "you are connected over %s: %s would cut this session (and you could not get back in by Wi-Fi). Connect through "
            "the wired port or a screen and keyboard, or run it again with --force." % (interface, what))


def reload_connection(system, name):
    system.run(["nmcli", "connection", "reload"])
    system.run(["nmcli", "connection", "up", name], check=False)


# ---- commands ---------------------------------------------------------------------------------------------------------

def setup_hotspot(system, ssid_prefix, team, band, password, channel=None, force=False):
    if not system.is_root():
        raise NetError("run it with sudo")
    if not network_manager_active(system):
        raise NetError("NetworkManager is not running; this needs it (see install.sh --network)")
    team_to_address(team)  # validates the team number before anything else uses it
    nm_band = band_to_nm(band)
    check_wpa_password(password)
    interface = wifi_interface(system)
    allowed = parse_5ghz_channels(system.run(["iw", "phy"], check=False)) if nm_band == "a" else None
    channels = valid_channels(nm_band, allowed)
    if not channels:
        raise NetError("no usable 5 GHz channel here (set the country first: sudo iw reg set KR); use band 2.4")
    if channel is None:
        if system.interactive():
            answer = system.ask("channel %s [%d]: " % ("/".join(str(c) for c in channels), channels[0])).strip()
            channel = int(answer) if answer else channels[0]
        else:
            channel = channels[0]
    if channel not in channels:
        raise NetError("channel %d is not valid for %s GHz here; valid: %s" % (channel, band, ", ".join(map(str, channels))))
    guard_ssh(system, interface, force, "turning the Wi-Fi into an access point")

    ssid = ssid_for(ssid_prefix, team)
    path = os.path.join(NM_DIR, AP_NAME + ".nmconnection")
    system.write_secret(path, ap_keyfile(ssid, team, nm_band, channel, password, interface))
    reload_connection(system, AP_NAME)
    return {"ssid": ssid, "address": team_to_address(team), "band": nm_band, "channel": channel, "interface": interface}


def setup_ethernet(system, interface=None, force=False):
    if not system.is_root():
        raise NetError("run it with sudo")
    if not network_manager_active(system):
        raise NetError("NetworkManager is not running")
    interface = interface or ethernet_interface(system)
    guard_ssh(system, interface, force, "fixing the wired port at 172.22.11.2")
    path = os.path.join(NM_DIR, ETH_NAME + ".nmconnection")
    system.write_secret(path, ethernet_keyfile(interface))
    system.run(["nmcli", "connection", "reload"])
    return {"interface": interface, "address": ETH_ADDRESS}


def _wifi_connections(system):
    names = []
    for line in system.run(["nmcli", "-t", "-f", "NAME,TYPE", "connection", "show"], check=False).splitlines():
        name, _, kind = line.rpartition(":")
        if kind in ("802-11-wireless", "wifi"):
            names.append(name)
    return names


def toggle_net_mode(system, mode, ssid=None, password=None, force=False):
    if not system.is_root():
        raise NetError("run it with sudo")
    if not network_manager_active(system):
        raise NetError("NetworkManager is not running")
    interface = wifi_interface(system)
    if mode not in ("ap", "wifi"):
        raise NetError("the mode is ap or wifi")
    path = os.path.join(NM_DIR, AP_NAME + ".nmconnection")
    if mode == "ap":
        if not system.exists(path):
            raise NetError("no access point is set up yet: sudo setupHotspot VMX 1234 2.4 password")
        guard_ssh(system, interface, force, "switching the Wi-Fi to the access point")
        system.run(["nmcli", "connection", "modify", AP_NAME, "connection.autoconnect", "yes"])
        for name in _wifi_connections(system):
            if name != AP_NAME:
                system.run(["nmcli", "connection", "modify", name, "connection.autoconnect", "no"], check=False)
        system.run(["nmcli", "connection", "up", AP_NAME])
        return {"mode": "ap"}

    # wifi: join a router
    guard_ssh(system, interface, force, "leaving the access point")
    if ssid:
        client_path = os.path.join(NM_DIR, CLIENT_NAME + ".nmconnection")
        if password:
            check_wpa_password(password)
        system.write_secret(client_path, client_keyfile(ssid, password, interface))
        system.run(["nmcli", "connection", "reload"])
        client = CLIENT_NAME
    else:
        others = [n for n in _wifi_connections(system) if n != AP_NAME]
        if not others:
            raise NetError("no router is saved: toggleNetMode wifi SSID PASSWORD")
        client = others[0]
    if system.exists(path):
        system.run(["nmcli", "connection", "modify", AP_NAME, "connection.autoconnect", "no"], check=False)
    system.run(["nmcli", "connection", "modify", client, "connection.autoconnect", "yes"], check=False)
    system.run(["nmcli", "connection", "up", client])
    return {"mode": "wifi", "connection": client}


def status(system):
    return system.run(["nmcli", "-t", "-f", "NAME,TYPE,DEVICE,STATE", "connection", "show"], check=False)


# ---- command line -----------------------------------------------------------------------------------------------------

def main(argv=None, system=None):
    system = system or System()
    argv = list(sys.argv[1:] if argv is None else argv)
    name = os.path.basename(sys.argv[0])
    if name in ("setupHotspot", "toggleNetMode", "setupEthernet"):
        argv.insert(0, name)
    parser = argparse.ArgumentParser(prog="vmx_net")
    sub = parser.add_subparsers(dest="command", required=True)

    p = sub.add_parser("setupHotspot", help="make the Wi-Fi an access point")
    p.add_argument("ssid"); p.add_argument("team"); p.add_argument("band"); p.add_argument("password", nargs="?", default=None)
    p.add_argument("--channel", type=int); p.add_argument("--force", action="store_true")

    p = sub.add_parser("toggleNetMode", help="ap | wifi")
    p.add_argument("mode"); p.add_argument("ssid", nargs="?"); p.add_argument("password", nargs="?")
    p.add_argument("--force", action="store_true")

    p = sub.add_parser("setupEthernet", help="the wired port: 172.22.11.2 with a DHCP server")
    p.add_argument("interface", nargs="?"); p.add_argument("--force", action="store_true")

    sub.add_parser("status")
    args = parser.parse_args(argv)
    try:
        if args.command == "setupHotspot":
            password = args.password if args.password is not None else DEFAULTS["password"]
            info = setup_hotspot(system, args.ssid, args.team, args.band, password, args.channel, args.force)
            print("Wi-Fi access point '%s' on %s (channel %d): the VMX is %s" % (
                info["ssid"], info["interface"], info["channel"], info["address"]))
        elif args.command == "toggleNetMode":
            info = toggle_net_mode(system, args.mode, args.ssid, args.password, args.force)
            print("network mode: %s" % info["mode"])
        elif args.command == "setupEthernet":
            info = setup_ethernet(system, args.interface, args.force)
            print("wired port %s: %s with a DHCP server" % (info["interface"], info["address"]))
        else:
            print(status(system), end="")
    except NetError as e:
        print("error: %s" % e, file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
