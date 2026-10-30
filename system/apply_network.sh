#!/bin/sh
# Runs as a robot-managed systemd service after install.sh prepares NetworkManager.
# It must remain independent of the SSH session that launched the installer.
set -eu

SSID_PREFIX="${1:-WorldSkills}"
TEAM="${2:-1234}"
BAND="${3:-2.4}"
PASSWORD="${4:-password}"

netplan generate

# Do not let NetworkManager auto-connect to an old router before the AP is ready.
systemctl stop NetworkManager.service 2>/dev/null || true
for profile in /etc/NetworkManager/system-connections/*; do
    [ -f "$profile" ] || continue
    [ "$(basename "$profile")" = robot-ap.nmconnection ] && continue
    if grep -q '^\[wifi\]' "$profile"; then
        rm -f "$profile"
    fi
done

systemctl disable --now systemd-networkd.service systemd-networkd.socket systemd-networkd-wait-online.service 2>/dev/null || true
systemctl enable --now NetworkManager.service

i=0
until systemctl is-active --quiet NetworkManager.service; do
    i=$((i + 1))
    [ "$i" -lt 30 ] || { echo "NetworkManager did not start" >&2; exit 1; }
    sleep 1
done
rfkill unblock wifi 2>/dev/null || true
command -v iw >/dev/null 2>&1 && iw reg set US 2>/dev/null || true
nmcli radio wifi on 2>/dev/null || true

# SSH_CONNECTION is intentionally cleared so these commands execute synchronously here,
# in the robot's service, instead of scheduling another operation tied to an SSH shell.
env -u SSH_CONNECTION -u SSH_CLIENT /usr/local/bin/setupHotspot "$SSID_PREFIX" "$TEAM" "$BAND" "$PASSWORD"
env -u SSH_CONNECTION -u SSH_CLIENT /usr/local/bin/toggleNetMode ap

# Remove every saved Wi-Fi client connection while preserving our AP profile.
AP_UUID="$(nmcli -g UUID connection show robot-ap)"
nmcli -t -f UUID,TYPE connection show \
    | awk -F: -v ap_uuid="$AP_UUID" '$1 != ap_uuid && $2 ~ /^(wifi|802-11-wireless)$/ { print $1 }' \
    | while IFS= read -r uuid; do
        [ -n "$uuid" ] && nmcli connection delete uuid "$uuid"
    done

echo "AP mode is active; saved Wi-Fi profiles were removed"
