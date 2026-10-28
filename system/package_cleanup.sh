#!/usr/bin/env bash
# Remove optional Ubuntu image tooling from a dedicated VMX robot.
# This list deliberately excludes boot/storage, firmware, networking, SSH,
# time sync, compilers and shared libraries used by the deployed robot program.
set -euo pipefail

DRY_RUN=0
case "${1:-}" in
  --dry-run) DRY_RUN=1; shift ;;
  --help|-h) echo 'package_cleanup.sh [--dry-run]'; exit 0 ;;
esac
[ "$#" = 0 ] || { echo "unexpected argument: $1" >&2; exit 1; }

candidates=(
  # Ubuntu crash reporting and server support reports.
  apport apport-core-dump-handler apport-symptoms python3-apport python3-problem-report
  landscape-common motd-news-config sos
  # Installers/guest agents for virtualization, not VMX hardware drivers.
  lxd-installer lxd-agent-loader open-vm-tools
  # Additional package-management frontends; apt/dpkg remain available.
  packagekit packagekit-tools gir1.2-packagekitglib-1.0 appstream
  command-not-found python3-commandnotfound
)
fail() { echo "package cleanup: $*" >&2; exit 1; }

if ! command -v dpkg-query >/dev/null 2>&1 || ! command -v apt-get >/dev/null 2>&1; then
  if [ "$DRY_RUN" = 1 ]; then
    printf 'Optional package candidates (APT database unavailable on this host):\n  %s\n' "${candidates[*]}"
    exit 0
  fi
  fail 'apt-get and dpkg-query are required on the VMX robot'
fi
if [ "$DRY_RUN" != 1 ]; then
  [ "$(id -u)" = 0 ] || fail 'run on the robot with sudo'
  [ "$(uname -m)" = aarch64 ] || fail 'cleanup is intended for the aarch64 VMX robot'
fi

installed=()
for package in "${candidates[@]}"; do
  if status="$(dpkg-query -W -f='${Status}' "$package" 2>/dev/null)" && [ "$status" = 'install ok installed' ]; then
    installed+=("$package")
  fi
done
if [ "${#installed[@]}" = 0 ]; then
  echo 'No optional image packages from the cleanup list are installed.'
  exit 0
fi
printf 'Optional image packages selected for removal:\n  %s\n' "${installed[*]}"

# Force autoremoval off even if enabled in the user's apt.conf. Unpackaged
# VMX/WPILib libraries are invisible to APT's dependency graph.
apt_options=(-o DPkg::Lock::Timeout=600 -o APT::Get::AutomaticRemove=false -o DPkg::ConfigurePending=false --no-download)
umask 077
work="$(mktemp -d /tmp/w4studica-package-cleanup.XXXXXX)"
trap 'rm -rf -- "$work"' EXIT
printf '%s\n' "${installed[@]}" > "$work/guard.allowed"

if ! LC_ALL=C apt-get "${apt_options[@]}" --simulate purge "${installed[@]}" > "$work/plan" 2>&1; then
  cat "$work/plan"
  fail 'APT could not prepare a removal plan; no packages were changed'
fi
cat "$work/plan"
removals=0
while read -r action package rest; do
  case "$action" in
    Remv|Purg)
      grep -Fxq -- "${package%%:*}" "$work/guard.allowed" \
        || fail "APT also wants to remove $package, outside the cleanup list; no packages were changed"
      removals=$((removals + 1))
      ;;
    Inst|Conf) fail 'APT wants to install/configure packages; no packages were changed' ;;
  esac
done < "$work/plan"
[ "$removals" -gt 0 ] || { echo 'APT has no packages to remove.'; exit 0; }
if [ "$DRY_RUN" = 1 ]; then
  echo 'Dry run complete: no packages were changed.'
  exit 0
fi

# Recheck the actual transaction under APT's lock. A package database change
# between simulation and execution must not expand the removal scope.
# Protocol: https://manpages.debian.org/bookworm/apt/apt.conf.5.en.html
cat > "$work/guard" <<'GUARD'
#!/bin/sh
set -eu
fail() { echo "package cleanup guard: $*" >&2; exit 1; }
IFS= read -r version || fail 'missing APT transaction'
[ "$version" = 'VERSION 2' ] || fail 'unsupported APT hook protocol'
header_done=0
while IFS= read -r line; do
  [ -n "$line" ] || { header_done=1; break; }
done
[ "$header_done" = 1 ] || fail 'incomplete APT transaction header'
while read -r package old_version direction new_version action extra; do
  [ -n "$package" ] && [ -z "$extra" ] && [ "$action" = '**REMOVE**' ] \
    || fail 'unexpected package operation'
  grep -Fxq -- "$package" "$0.allowed" || fail "refusing to remove $package"
done
GUARD
chmod 0700 "$work/guard"
LC_ALL=C DEBIAN_FRONTEND=noninteractive apt-get "${apt_options[@]}" \
  -o "DPkg::Pre-Install-Pkgs::=$work/guard" \
  -o "DPkg::Tools::Options::$work/guard::Version=2" \
  -o "DPkg::Tools::Options::$work/guard::InfoFD=0" \
  purge -y "${installed[@]}" || fail 'APT cleanup failed; check the APT output above'
echo 'Optional image package cleanup complete.'
