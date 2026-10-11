#!/bin/sh
# Puts the network back the way it was before `install.sh --network`: the netplan files that were set aside are restored,
# systemd-networkd takes the interfaces again and NetworkManager is stopped. Run it on the robot's screen/keyboard or
# over a connection that survives (the robot's Wi-Fi AP or the wired port go away).
#
#   sudo sh undo_network.sh [BACKUP_DIR]
#
# BACKUP_DIR defaults to the newest /etc/netplan/w4studica-backup-*.
set -u

BACKUP="${1:-$(find /etc/netplan -maxdepth 1 -name "w4studica-backup-*" 2>/dev/null | sort | tail -1)}"
[ "$(id -u)" = 0 ] || { echo "run it with sudo" >&2; exit 1; }
[ -n "$BACKUP" ] && [ -d "$BACKUP" ] || { echo "no netplan backup found (/etc/netplan/w4studica-backup-*)" >&2; exit 1; }

echo "restoring $BACKUP"
rm -f /etc/netplan/01-w4studica-network-manager.yaml
for f in "$BACKUP"/*.yaml; do
  [ -f "$f" ] && mv "$f" /etc/netplan/ && chmod 600 "/etc/netplan/$(basename "$f")"
done
rmdir "$BACKUP" 2>/dev/null
rm -f /etc/cloud/cloud.cfg.d/99-w4studica-disable-network-config.cfg
rm -f /etc/NetworkManager/system-connections/robot-ap.nmconnection \
      /etc/NetworkManager/system-connections/robot-eth.nmconnection \
      /etc/NetworkManager/system-connections/robot-client.nmconnection

systemctl disable --now NetworkManager.service 2>/dev/null
systemctl enable systemd-networkd.service systemd-networkd.socket 2>/dev/null
netplan generate
systemctl start systemd-networkd.socket systemd-networkd.service
netplan apply
echo "done: systemd-networkd is back (the home Wi-Fi from the old netplan file connects again)"
