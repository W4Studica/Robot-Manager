#!/bin/sh
# Restores the first pre-install files and network-service states. Legacy netplan-only backups restore networkd.
# Run it on the robot's screen/keyboard or
# over a connection that survives (the robot's Wi-Fi AP or the wired port go away).
#
#   sudo sh undo_network.sh [BACKUP_DIR]
#
# Without BACKUP_DIR, restore the immutable full backup (or oldest legacy netplan backup).
set -eu

[ "$(id -u)" = 0 ] || { echo "run it with sudo" >&2; exit 1; }
SCRIPT_DIR="$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)"
if [ "$#" = 0 ] && [ -f /var/lib/robot_manager/network-original/state.json ]; then
  exec python3 "$SCRIPT_DIR/network_backup.py" restore
fi

# Compatibility with the old installer's netplan-only backups. Pick the oldest
# original configuration, skipping backups containing only the installer YAML.
BACKUP="${1:-}"
if [ -z "$BACKUP" ]; then
  for directory in /etc/netplan/w4studica-backup-*; do
    [ -d "$directory" ] || continue
    for file in "$directory"/*.yaml; do
      [ -f "$file" ] || continue
      [ "$(basename "$file")" = 01-w4studica-network-manager.yaml ] && continue
      BACKUP="$directory"
      break
    done
    [ -z "$BACKUP" ] || break
  done
fi
[ -n "$BACKUP" ] && [ -d "$BACKUP" ] || { echo "no netplan backup found (/etc/netplan/w4studica-backup-*)" >&2; exit 1; }

echo "restoring $BACKUP"
rm -f /etc/netplan/01-w4studica-network-manager.yaml
for f in "$BACKUP"/*.yaml; do
  [ -f "$f" ] && cp -p "$f" /etc/netplan/ && chmod 600 "/etc/netplan/$(basename "$f")"
done
rm -f /etc/cloud/cloud.cfg.d/99-w4studica-disable-network-config.cfg
rm -f /etc/NetworkManager/system-connections/robot-ap.nmconnection \
      /etc/NetworkManager/system-connections/robot-eth.nmconnection \
      /etc/NetworkManager/system-connections/robot-client.nmconnection

systemctl disable --now NetworkManager.service 2>/dev/null || true
systemctl enable systemd-networkd.service systemd-networkd.socket 2>/dev/null
netplan generate
systemctl start systemd-networkd.socket systemd-networkd.service
netplan apply
echo "done: systemd-networkd is back (the home Wi-Fi from the old netplan file connects again)"
