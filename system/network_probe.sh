#!/bin/sh
# Prints how this robot's network is set up, to decide how to turn the Wi-Fi into the robot's access point.
# READ ONLY: it changes nothing. It never prints a Wi-Fi password (netplan files are listed by name and renderer only).

section() { printf '\n== %s\n' "$1"; }
have() { command -v "$1" >/dev/null 2>&1; }

section "OS / kernel"
# shellcheck disable=SC1091
. /etc/os-release 2>/dev/null && echo "$PRETTY_NAME"
uname -r
[ -r /proc/device-tree/model ] && tr -d '\0' < /proc/device-tree/model && echo

section "interfaces (ip -br)"
ip -br link
ip -br addr
echo "-- routes"; ip route

section "how you are connected right now"
echo "SSH_CONNECTION=${SSH_CONNECTION:-<not over ssh>}"
for path in /sys/class/net/*; do
  [ -e "$path" ] || continue
  dev="${path##*/}"
  case "$dev" in eth*|en*|wl*) ;; *) continue ;; esac
  echo "$dev: carrier=$(cat "$path/carrier" 2>/dev/null) operstate=$(cat "$path/operstate" 2>/dev/null)"
done

section "services (active / enabled)"
for u in NetworkManager systemd-networkd wpa_supplicant hostapd dnsmasq systemd-resolved systemd-networkd-wait-online; do
  printf '%-34s %s / %s\n' "$u" "$(systemctl is-active $u 2>/dev/null)" "$(systemctl is-enabled $u 2>/dev/null)"
done

section "packages"
for p in network-manager netplan.io hostapd dnsmasq dnsmasq-base iw wireless-regdb rfkill; do
  printf '%-18s %s\n' "$p" "$(dpkg -s $p 2>/dev/null | sed -n 's/^Version: //p' | head -1)"
done

section "netplan (file names and renderer only)"
ls -l /etc/netplan 2>/dev/null
for f in /etc/netplan/*.yaml; do
  [ -f "$f" ] || continue
  echo "-- $f"
  grep -E '^\s*(renderer|version):|^\s{2,4}(ethernets|wifis|bridges):' "$f"
  printf 'has a wifis: section? '; grep -q '^\s*wifis:' "$f" && echo yes || echo no
done

section "NetworkManager"
if have nmcli; then
  nmcli -t -f DEVICE,TYPE,STATE,CONNECTION device 2>&1
  echo "-- connections (names and types)"; nmcli -t -f NAME,TYPE,DEVICE connection show 2>&1
else
  echo "nmcli not installed"
fi

section "Wi-Fi"
if have iw; then
  iw dev 2>&1
  echo "-- supported interface modes (AP must be listed)"
  iw list 2>/dev/null | sed -n '/Supported interface modes/,/Band/p' | head -14
  echo "-- combinations (can it be AP and client at once?)"
  iw list 2>/dev/null | sed -n '/valid interface combinations/,/Device supports/p' | head -12
  echo "-- regulatory domain"; iw reg get 2>&1
else
  echo "iw not installed"
fi
have rfkill && { echo "-- rfkill"; rfkill list 2>&1; }

section "current Wi-Fi power save"
for path in /sys/class/net/wl*; do
  [ -e "$path" ] || continue
  have iw && iw dev "${path##*/}" get power_save 2>&1
done

section "done"
echo "Nothing was changed. Paste everything above."
