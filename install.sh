#!/usr/bin/env bash
# install.sh - install the W4Studica VMX software. Run it ON the Studica VMX-pi.
#
#     sudo ./install.sh [options]
#
# It lives in the W4Studica/Robot-Manager repo. Clone that repo and the W4Studica/allwpilib
# repo side by side on the robot (only allwpilib's studica_drivers/ and
# simulation/halsim_vmx/src/ are used):
#     ~/Robot-Manager/install.sh
#     ~/allwpilib/
# or point at them with --robot-manager-dir / --allwpilib-dir.
#
# No ssh and no passwords: if it is not started as root it re-runs itself through sudo,
# which asks for the password itself.
#
# What it does (a step is skipped with a message if its requirements are missing):
#   1. robot_manager      /opt/robot_manager, /etc/robot_manager/config.json (only if missing),
#                         systemd service robot_manager (enabled and restarted)
#   2. studica_drivers    built and installed with the Studica Makefile (needs g++, make, VMXPi.h)
#   3. Studica plugin     /opt/halsim_vmx/libhalsim_vmx_studica.so for halsim_vmx (needs g++ >= 11, VMXPi.h)
#   4. GPIO interrupt shim /opt/robot_manager/lib/libvmx_gpio_isr_shim.so (needs gcc and make). Fixes
#                         "RPI GPIO Interrupt Enable: PI_BAD_ISR_INIT" on kernels without GPIO sysfs
#   5. LSE atomics emulation /opt/robot_manager/lib/libvmx_lse_emu.so (needs gcc and make). Lets WPILib's libMrcLib
#                         (SystemCore build, uses ARMv8.1 atomics) run on a Pi 4; robot_manager turns it on by itself
#   6. sudoers (opt-in)   lets the login user run the few sudo commands `gradlew deploy` uses, without a password
#
# NOT VERIFIED on a real VMX-pi: the commands in steps 2 and 3 and the systemd unit were only
# syntax-checked and dry-run on a PC. See allwpilib/simulation/halsim_vmx/HARDWARE_CHECKLIST.md.

set -euo pipefail

SELF_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

OPT_DIR=/opt/robot_manager
CONF_DIR=/etc/robot_manager
PLUGIN_DIR=/opt/halsim_vmx
BUILD_DIR=/opt/robot_manager/build
SERVICE=robot_manager

DRY_RUN=0
WITH_SUDOERS=0
SKIP_DRIVERS=0
UPGRADE_DRIVERS=0
SKIP_PLUGIN=0
SKIP_SHIM=0
SKIP_LSE_EMU=0
ROBOT_HOME=""
LOGIN_USER=""
# Defaults: this repo is where install.sh is; allwpilib is a sibling checkout (or inside this directory).
if [ -f "$SELF_DIR/robot_manager.py" ]; then RM_DIR="$SELF_DIR"; else RM_DIR="$SELF_DIR/robot_manager"; fi
if [ -d "$SELF_DIR/../allwpilib" ]; then AW_DIR="$SELF_DIR/../allwpilib"; else AW_DIR="$SELF_DIR/allwpilib"; fi

usage() {
  cat <<'EOF'
Usage: sudo ./install.sh [options]      (run on the robot)

Options:
  --user NAME              login user that deploys to this robot (default: the user who ran sudo)
  --robot-home DIR         robot_home for robot_manager (default: /home/<user>)
  --robot-manager-dir DIR  Robot-Manager checkout (default: the directory of install.sh)
  --allwpilib-dir DIR      allwpilib checkout (default: ../allwpilib next to this repo)
  --sudoers                allow the login user passwordless sudo for: systemctl (robot_manager), ldconfig, sync
  --skip-drivers           do not build/install studica_drivers
  --upgrade-drivers        replace an already installed studica_drivers (e.g. from the vmx-hal package) with this repo's copy
  --skip-plugin            do not build the Studica backend plugin
  --skip-shim              do not build the GPIO interrupt shim (libvmx_gpio_isr_shim.so)
  --skip-lse-emu           do not build the LSE atomics emulation (libvmx_lse_emu.so)
  --dry-run                print what would be done, change nothing
  -h, --help               this text
EOF
}

die() { echo "install.sh: $*" >&2; exit 1; }
info() { echo "==> $*"; }
warn() { echo "WARNING: $*" >&2; }

# The parsing loop below shifts "$@" away; keep the original arguments for the sudo re-run.
ORIG_ARGS=("$@")

while [ $# -gt 0 ]; do
  case "$1" in
    --user) [ $# -ge 2 ] || die "--user needs a value"; LOGIN_USER="$2"; shift ;;
    --robot-home) [ $# -ge 2 ] || die "--robot-home needs a value"; ROBOT_HOME="$2"; shift ;;
    --robot-manager-dir) [ $# -ge 2 ] || die "--robot-manager-dir needs a value"; RM_DIR="$2"; shift ;;
    --allwpilib-dir) [ $# -ge 2 ] || die "--allwpilib-dir needs a value"; AW_DIR="$2"; shift ;;
    --sudoers) WITH_SUDOERS=1 ;;
    --skip-drivers) SKIP_DRIVERS=1 ;;
    --upgrade-drivers) UPGRADE_DRIVERS=1 ;;
    --skip-plugin) SKIP_PLUGIN=1 ;;
    --skip-shim) SKIP_SHIM=1 ;;
    --skip-lse-emu) SKIP_LSE_EMU=1 ;;
    --dry-run) DRY_RUN=1 ;;
    -h|--help) usage; exit 0 ;;
    *) usage >&2; die "unknown argument: $1" ;;
  esac
  shift
done

run() {
  if [ "$DRY_RUN" = 1 ]; then
    printf '[dry-run]'; printf ' %q' "$@"; printf '\n'
  else
    "$@"
  fi
}

write_file() {  # write_file <path> <mode>   (content on stdin)
  local path="$1" mode="$2"
  if [ "$DRY_RUN" = 1 ]; then
    echo "[dry-run] write $path (mode $mode):"; sed 's/^/[dry-run]   /'
  else
    cat > "$path"; chmod "$mode" "$path"
  fi
}

tool_version() {  # tool_version <tool> <flag>: first line of its version output, or "none"
  if command -v "$1" >/dev/null 2>&1; then "$@" 2>&1 | head -1; else echo none; fi
}

preflight() {
  info "Robot environment"
  local model=""
  [ -r /proc/device-tree/model ] && model="$(tr -d '\0' < /proc/device-tree/model)"
  echo "  OS:      $(. /etc/os-release 2>/dev/null && echo "${PRETTY_NAME:-unknown}")"
  echo "  kernel:  $(uname -r)"
  echo "  model:   ${model:-unknown}"
  echo "  glibc:   $(ldd --version 2>/dev/null | head -1 || echo unknown)"
  echo "  g++:     $(tool_version g++ --version)"
  echo "  java:    $(tool_version java -version)"
  echo "  python3: $(tool_version python3 --version)"
  command -v python3 >/dev/null 2>&1 || die "python3 is required (robot_manager is a Python script)"
  command -v systemctl >/dev/null 2>&1 || die "systemd (systemctl) is required"
}

have_vmx() { [ -f /usr/local/include/vmxpi/VMXPi.h ] && [ -d /usr/local/lib/vmxpi ]; }

gpp_major() { g++ -dumpversion 2>/dev/null | cut -d. -f1; }

install_robot_manager() {
  info "robot_manager"
  [ -f "$RM_DIR/robot_manager.py" ] && [ -f "$RM_DIR/robot_manager.service" ] \
    || die "robot_manager.py / robot_manager.service not found in $RM_DIR (use --robot-manager-dir)"
  run install -d -m 0755 "$OPT_DIR" "$CONF_DIR"
  run install -m 0755 "$RM_DIR/robot_manager.py" "$OPT_DIR/robot_manager.py"
  run install -m 0644 "$RM_DIR/robot_manager.service" "/etc/systemd/system/$SERVICE.service"

  if [ -f "$CONF_DIR/config.json" ]; then
    echo "  keeping existing $CONF_DIR/config.json"
  else
    # The platform profile list is left empty on purpose: the pin-number offset it would carry is not known yet.
    write_file "$CONF_DIR/config.json" 0644 <<EOF
{
  "robot_home": "$ROBOT_HOME",
  "env": {},
  "profiles": []
}
EOF
  fi

  run systemctl daemon-reload
  run systemctl enable "$SERVICE"
}

install_drivers() {
  info "studica_drivers"
  if [ "$SKIP_DRIVERS" = 1 ]; then echo "  skipped (--skip-drivers)"; return; fi
  have_vmx || { warn "VMXPi.h / vmxpi_hal_cpp not found under /usr/local; skipping studica_drivers"; return; }
  # The vmx-hal package already ships (an older) libstudica_drivers. Do not overwrite files a package owns
  # unless asked. The halsim_vmx plugin compiles the driver sources itself, so it does not need this library.
  if [ -f /usr/local/lib/studica_drivers/libstudica_drivers.so ] && [ "$UPGRADE_DRIVERS" != 1 ]; then
    echo "  already installed (probably by the vmx-hal package); not replacing it."
    echo "  pass --upgrade-drivers to install this repo's newer copy over it (replaces package-owned files)."
    return
  fi
  if ! command -v g++ >/dev/null 2>&1 || ! command -v make >/dev/null 2>&1; then
    warn "g++ and make are required; skipping studica_drivers (sudo apt install -y g++ make)"; return
  fi
  [ -d "$AW_DIR/studica_drivers" ] || { warn "$AW_DIR/studica_drivers not found (use --allwpilib-dir); skipping studica_drivers"; return; }

  # Build in a copy so the checkout is not littered with build output (or root-owned files).
  run rm -rf "$BUILD_DIR/studica_drivers"
  run install -d "$BUILD_DIR"
  run cp -r "$AW_DIR/studica_drivers" "$BUILD_DIR/studica_drivers"
  run make -C "$BUILD_DIR/studica_drivers"
  run make -C "$BUILD_DIR/studica_drivers" install
}

install_plugin() {
  info "Studica backend plugin for halsim_vmx"
  if [ "$SKIP_PLUGIN" = 1 ]; then echo "  skipped (--skip-plugin)"; return; fi
  have_vmx || { warn "VMXPi.h / vmxpi_hal_cpp not found under /usr/local; skipping the plugin"; return; }
  command -v g++ >/dev/null 2>&1 || { warn "g++ is required; skipping the plugin (sudo apt install -y g++ make)"; return; }
  local major; major="$(gpp_major)"
  if [ -z "$major" ] || [ "$major" -lt 11 ]; then
    warn "g++ >= 11 (C++20) is required, found '${major:-none}'; skipping the plugin"; return
  fi

  local drv="$AW_DIR/studica_drivers"
  local hv="$AW_DIR/simulation/halsim_vmx/src"
  [ -f "$hv/studica/native/cpp/StudicaBackend.cpp" ] && [ -d "$drv" ] \
    || { warn "halsim_vmx sources not found under $AW_DIR (use --allwpilib-dir); skipping the plugin"; return; }

  run install -d -m 0755 "$PLUGIN_DIR"
  run g++ -std=c++20 -shared -fPIC -O2 \
    -I/usr/local/include/vmxpi -I"$drv" \
    -I"$hv/main/native/include" -I"$hv/studica/native/include" \
    "$hv/studica/native/cpp/StudicaBackend.cpp" \
    "$drv/analog_input.cpp" "$drv/dio.cpp" "$drv/encoder.cpp" "$drv/imu.cpp" \
    -L/usr/local/lib/vmxpi -lvmxpi_hal_cpp -lrt -lpthread -latomic \
    -o "$PLUGIN_DIR/libhalsim_vmx_studica.so"
}

install_shim() {
  info "GPIO interrupt shim"
  if [ "$SKIP_SHIM" = 1 ]; then echo "  skipped (--skip-shim)"; return; fi
  local src="$RM_DIR/gpio_isr_shim"
  [ -f "$src/gpio_isr_shim.c" ] || { warn "$src/gpio_isr_shim.c not found; skipping the shim"; return; }
  if ! command -v gcc >/dev/null 2>&1 || ! command -v make >/dev/null 2>&1; then
    warn "gcc and make are required; skipping the shim (sudo apt install -y g++ make)"; return
  fi

  run rm -rf "$BUILD_DIR/gpio_isr_shim"
  run install -d "$BUILD_DIR" "$OPT_DIR/lib"
  run cp -r "$src" "$BUILD_DIR/gpio_isr_shim"
  run make -C "$BUILD_DIR/gpio_isr_shim" test
  run make -C "$BUILD_DIR/gpio_isr_shim"
  run install -m 0755 "$BUILD_DIR/gpio_isr_shim/libvmx_gpio_isr_shim.so" "$OPT_DIR/lib/libvmx_gpio_isr_shim.so"
  if [ ! -d /sys/class/gpio ]; then
    echo "  this kernel has no /sys/class/gpio, so the HAL needs the shim. Add to $CONF_DIR/config.json:"
    echo "    \"env\": { \"LD_PRELOAD\": \"$OPT_DIR/lib/libvmx_gpio_isr_shim.so\" }"
  fi
}

install_lse_emu() {
  info "LSE atomics emulation"
  if [ "$SKIP_LSE_EMU" = 1 ]; then echo "  skipped (--skip-lse-emu)"; return; fi
  local src="$RM_DIR/lse_emu"
  [ -f "$src/lse_emu.c" ] || { warn "$src/lse_emu.c not found; skipping the LSE emulation"; return; }
  if ! command -v gcc >/dev/null 2>&1 || ! command -v make >/dev/null 2>&1; then
    warn "gcc and make are required; skipping the LSE emulation (sudo apt install -y g++ make)"; return
  fi

  run rm -rf "$BUILD_DIR/lse_emu"
  run install -d "$BUILD_DIR" "$OPT_DIR/lib"
  run cp -r "$src" "$BUILD_DIR/lse_emu"
  run make -C "$BUILD_DIR/lse_emu" test
  run make -C "$BUILD_DIR/lse_emu"
  run install -m 0755 "$BUILD_DIR/lse_emu/libvmx_lse_emu.so" "$OPT_DIR/lib/libvmx_lse_emu.so"
  if grep -m1 '^Features' /proc/cpuinfo 2>/dev/null | grep -qw atomics; then
    echo "  this CPU has LSE atomics: robot_manager will not load it"
  else
    echo "  this CPU has no LSE atomics (Raspberry Pi 4): robot_manager sets LD_AUDIT=$OPT_DIR/lib/libvmx_lse_emu.so itself"
  fi
}

# Last, so robot_manager starts with every library in place: it looks for the LSE emulation library when it starts.
restart_service() {
  info "Restarting $SERVICE"
  run systemctl restart "$SERVICE"
}

install_sudoers() {
  info "sudoers for '$LOGIN_USER'"
  if [ "$WITH_SUDOERS" != 1 ]; then echo "  skipped (pass --sudoers to allow passwordless deploy)"; return; fi
  local systemctl_bin ldconfig_bin sync_bin
  systemctl_bin="$(command -v systemctl)"; ldconfig_bin="$(command -v ldconfig)"; sync_bin="$(command -v sync)"
  local tmp; tmp="$(mktemp)"
  cat > "$tmp" <<EOF
# Installed by W4Studica install.sh: what 'gradlew deploy' runs through sudo for the VmxPi target.
$LOGIN_USER ALL=(root) NOPASSWD: $systemctl_bin stop $SERVICE, $systemctl_bin start $SERVICE, $systemctl_bin enable $SERVICE, $ldconfig_bin, $sync_bin
EOF
  if [ "$DRY_RUN" = 1 ]; then
    echo "[dry-run] write /etc/sudoers.d/w4studica-deploy (mode 0440):"; sed 's/^/[dry-run]   /' "$tmp"; rm -f "$tmp"; return
  fi
  # Never install a sudoers file that does not parse: it could lock sudo out.
  if visudo -cf "$tmp" >/dev/null; then
    install -m 0440 "$tmp" /etc/sudoers.d/w4studica-deploy
    echo "  installed /etc/sudoers.d/w4studica-deploy"
  else
    rm -f "$tmp"; die "generated sudoers file did not pass visudo; nothing was installed"
  fi
  rm -f "$tmp"
}

summary() {
  cat <<EOF

==> Done$( [ "$DRY_RUN" = 1 ] && echo " (dry run: nothing was changed)" )
  service:   systemctl status $SERVICE      logs: journalctl -u $SERVICE -f
  config:    $CONF_DIR/config.json   (robot_home = $ROBOT_HOME)
  plugin:    $PLUGIN_DIR/libhalsim_vmx_studica.so   -> HALSIMVMX_BACKEND
  shim:      $OPT_DIR/lib/libvmx_gpio_isr_shim.so   -> LD_PRELOAD (only on kernels without /sys/class/gpio)
  lse emu:   $OPT_DIR/lib/libvmx_lse_emu.so   -> LD_AUDIT, set by robot_manager on CPUs without LSE atomics
Next: allwpilib/simulation/halsim_vmx/HARDWARE_CHECKLIST.md (section 1 onward). Nothing here has been run on a VMX yet.
EOF
}

# Not root: re-run through sudo (it asks for the password itself). A dry run needs no root.
if [ "$DRY_RUN" != 1 ] && [ "$(id -u)" != 0 ]; then
  command -v sudo >/dev/null 2>&1 || die "must run as root (sudo not found)"
  exec sudo "$0" "${ORIG_ARGS[@]}"
fi

[ -n "$LOGIN_USER" ] || LOGIN_USER="${SUDO_USER:-$(id -un)}"
[ -n "$ROBOT_HOME" ] || ROBOT_HOME="/home/$LOGIN_USER"

preflight
install_robot_manager
install_drivers
install_plugin
install_shim
install_lse_emu
install_sudoers
restart_service
summary
