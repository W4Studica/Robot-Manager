#!/usr/bin/env bash
# install.sh - install the W4Studica VMX software. Run it ON the Studica VMX-pi.
#
#     ./install.sh [options]        (run as the login user; sudo is requested when needed)
#
# Only the W4Studica/Robot-Manager checkout is required on the robot.
# GradleRIO deploys WPILib and the matching Studica runtime from the PC release.
# Developers can opt into local Studica builds with --build-studica and an
# allwpilib checkout selected by --allwpilib-dir (default: a sibling checkout).
#
# It never opens an SSH connection or stores passwords. An SSH-launched install is handed
# to systemd on the robot before privilege escalation can lose the SSH session context.
#
# What it does (required build/runtime dependencies are installed when missing; optional steps have explicit --skip flags):
#   1. robot_manager      /opt/robot_manager, /etc/robot_manager/config.json (adds missing RT settings),
#                         systemd service robot_manager (enabled and restarted); serves RioLog on TCP 1741
#   2. RioLog discovery   Avahi publishes robot.local, the hostname used by WPILib's RioLog client
#   3. VMX HAL SDK         installs vmxpi-hal .deb from the sibling builder's dist/ when headers/libs are missing
#   4. studica_drivers    development only (--build-studica): built and copied into /usr/local
#   5. Studica plugin     development only (--build-studica): built into /opt/halsim_studica
#   6. GPIO interrupt shim /opt/robot_manager/lib/libvmx_gpio_isr_shim.so (needs gcc and make). Fixes
#                         "RPI GPIO Interrupt Enable: PI_BAD_ISR_INIT" on kernels without GPIO sysfs
#   7. LSE atomics emulation /opt/robot_manager/lib/libvmx_lse_emu.so (needs gcc and make). Lets WPILib's libMrcLib
#                         (SystemCore build, uses ARMv8.1 atomics) run on a Pi 4; robot_manager turns it on by itself
#   8. CPU at full speed /opt/robot_manager/bin/robot_cpu_max.sh + robot_cpu_max.service: governor "performance" and the
#                         minimum frequency raised to the maximum, at every boot (a 20 ms robot loop should not wait for the clock)
#   9. network             configures the WorldSkills-1234 Wi-Fi access point and wired port 172.22.11.2;
#                         after NetworkManager starts, selects AP mode and deletes saved Wi-Fi profiles
#  10. sudoers (opt-in)   lets the login user run the few sudo commands `gradlew deploy` uses, without a password
#  11. PREEMPT_RT kernel  linux-raspi-realtime on Ubuntu >= 26.04; requires a manual reboot to activate
#  12. storage/boot       ext4 protection, RAM journal, zram 8 GiB + disk swap 4 GiB (next boot; no OverlayFS)
#  13. boot provisioning retires cleanly completed cloud-init; skips online waiting only for verified local-only robots
#
# WPILib/CameraServer/cscore/OpenCV native libraries are deployed per robot project by GradleRIO from
# that project's pinned dependency graph. The Linux UVC camera interface is provided by the kernel (V4L2);
# no separate camera daemon or system OpenCV package is required.

set -euo pipefail

SELF_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

OPT_DIR=/opt/robot_manager
CONF_DIR=/etc/robot_manager
PLUGIN_DIR=/opt/halsim_studica
BUILD_DIR=/opt/robot_manager/build
SERVICE=robot_manager
MANAGER_BINARY=""

DRY_RUN=0
DETACH_INSTALL=0
RUNNING_DETACHED=0
INSTALL_DEPS=1
WITH_SUDOERS=0
SKIP_DRIVERS=1
UPGRADE_DRIVERS=0
SKIP_PLUGIN=1
SKIP_SHIM=0
SKIP_LSE_EMU=0
SKIP_CPU_MAX=0
SKIP_RT_KERNEL=0
RT_KERNEL_STATUS="not checked"
UNDO_NETWORK=0
CHECK_RUNTIME_DEPS=0
CLEANUP_PACKAGES=0
REBOOT_ON_FINISH=1
AP_SSID=WorldSkills
AP_TEAM=1234
AP_BAND=2.4
AP_PASSWORD=password
VMX_HAL_DEB="${VMX_HAL_DEB:-}"
ADMIN_USER="${ADMIN_USER:-admin}"
ROBOT_USER="${ROBOT_USER:-lvuser}"
ADMIN_PASSWORD="${ADMIN_PASSWORD:-password}"
ROBOT_PASSWORD="${ROBOT_PASSWORD:-password}"
PURGE_OLD_USER="${PURGE_OLD_USER:-}"
CLONE_REPO="${CLONE_REPO:-}"
ROBOT_HOME=""
LOGIN_USER=""
# Defaults: this repo is where install.sh is; allwpilib is a sibling checkout (or inside this directory).
if [ -f "$SELF_DIR/Cargo.toml" ]; then RM_DIR="$SELF_DIR"; else RM_DIR="$SELF_DIR/robot_manager"; fi
if [ -d "$SELF_DIR/../allwpilib" ]; then AW_DIR="$SELF_DIR/../allwpilib"; else AW_DIR="$SELF_DIR/allwpilib"; fi

usage() {
  cat <<'EOF'
Usage: ./install.sh [options]      (run on the robot; over SSH it continues as a systemd job)

Normal installation configures robot RT scheduling and installs the Raspberry Pi
PREEMPT_RT kernel on Ubuntu >= 26.04. Reboot manually after installation to activate it.
Also configures ext4 protection, volatile logs, 8 GiB zram and 4 GiB disk swap
for the next boot. Existing swap is not switched off during installation.
WPILib and Studica libraries arrive with GradleRIO deploy; no allwpilib clone is needed.
Once initial provisioning is cleanly complete, cloud-init is disabled for future boots.
Online waiting is skipped only when no remote storage or unknown online consumers exist.

Options:
  --admin-user NAME        administrator user with sudo rights (default: admin)
  --robot-user NAME        unprivileged robot runner user (default: lvuser)
  --purge-user NAME        purge legacy user account right before reboot
  --purge-old-user         purge current non-admin sudo user before reboot
  --clone-repo [URL]       clone the repository to /tmp/robot_manager and install
  --user NAME              login user that deploys to this robot (default: admin)
  --robot-home DIR         robot_home for robot_manager (default: /home/<robot-user>)
  --robot-manager-dir DIR  Robot-Manager checkout (default: the directory of install.sh)
  --manager-binary FILE    use a prebuilt native robot_manager instead of building with Cargo
  --build-studica          development only: build Studica drivers/backend locally; requires allwpilib
  --allwpilib-dir DIR      development checkout for --build-studica (default: ../allwpilib)
  --vmx-hal-deb FILE       install this vmxpi-hal arm64 package if VMX SDK headers/libraries are missing
  --sudoers                allow the login user passwordless sudo for: systemctl (robot_manager), ldconfig, sync
  --detach                 run installation as a robot-managed systemd job, even outside SSH
  --no-install-deps        do not install missing apt packages; fail with the package list instead
  --skip-drivers           skip local driver building (already the default)
  --upgrade-drivers        development only: build/replace installed drivers, including package-owned copies
  --skip-plugin            skip local backend building (already the default)
  --skip-shim              do not build the GPIO interrupt shim (libvmx_gpio_isr_shim.so)
  --skip-lse-emu           do not build the LSE atomics emulation (libvmx_lse_emu.so)
  --skip-cpu-max           do not pin the CPU at its highest frequency (robot_cpu_max.service)
  --skip-rt-kernel         keep the current kernel; still configure robot RT scheduling
  --undo-network           restore the saved pre-install network configuration and exit
  --check-runtime-deps     inspect the deployed robotCommand and GradleRIO .so files with ldd, then exit
  --cleanup-packages       purge optional Ubuntu image tooling and exit (--dry-run previews the removal plan)
  --no-reboot              do not reboot after successful installation (reboot is default)
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
    --admin-user) [ $# -ge 2 ] || die "--admin-user needs a value"; ADMIN_USER="$2"; LOGIN_USER="$2"; shift ;;
    --robot-user) [ $# -ge 2 ] || die "--robot-user needs a value"; ROBOT_USER="$2"; shift ;;
    --user) [ $# -ge 2 ] || die "--user needs a value"; LOGIN_USER="$2"; ADMIN_USER="$2"; shift ;;
    --purge-user) [ $# -ge 2 ] || die "--purge-user needs a value"; PURGE_OLD_USER="$2"; shift ;;
    --purge-old-user) PURGE_OLD_USER=1 ;;
    --clone-repo)
      if [ $# -ge 2 ] && [[ "$2" != --* ]]; then
        CLONE_REPO="$2"; shift
      else
        CLONE_REPO="https://github.com/W4Studica/Robot-Manager.git"
      fi
      ;;
    --robot-home) [ $# -ge 2 ] || die "--robot-home needs a value"; ROBOT_HOME="$2"; shift ;;
    --robot-manager-dir) [ $# -ge 2 ] || die "--robot-manager-dir needs a value"; RM_DIR="$2"; shift ;;
    --manager-binary) [ $# -ge 2 ] || die "--manager-binary needs a value"; MANAGER_BINARY="$2"; shift ;;
    --allwpilib-dir) [ $# -ge 2 ] || die "--allwpilib-dir needs a value"; AW_DIR="$2"; shift ;;
    --vmx-hal-deb) [ $# -ge 2 ] || die "--vmx-hal-deb needs a value"; VMX_HAL_DEB="$(realpath -m -- "$2")"; shift ;;
    --sudoers) WITH_SUDOERS=1 ;;
    --detach) DETACH_INSTALL=1 ;;
    --_run-detached) RUNNING_DETACHED=1 ;;
    --no-install-deps) INSTALL_DEPS=0 ;;
    --build-studica) SKIP_DRIVERS=0; SKIP_PLUGIN=0 ;;
    --skip-drivers) SKIP_DRIVERS=1 ;;
    --upgrade-drivers) UPGRADE_DRIVERS=1; SKIP_DRIVERS=0 ;;
    --skip-plugin) SKIP_PLUGIN=1 ;;
    --skip-shim) SKIP_SHIM=1 ;;
    --skip-lse-emu) SKIP_LSE_EMU=1 ;;
    --skip-cpu-max) SKIP_CPU_MAX=1 ;;
    --skip-rt-kernel) SKIP_RT_KERNEL=1 ;;
    --undo-network) UNDO_NETWORK=1 ;;
    --check-runtime-deps) CHECK_RUNTIME_DEPS=1 ;;
    --cleanup-packages) CLEANUP_PACKAGES=1 ;;
    --no-reboot) REBOOT_ON_FINISH=0 ;;
    --dry-run) DRY_RUN=1 ;;
    -h|--help) usage; exit 0 ;;
    *) usage >&2; die "unknown argument: $1" ;;
  esac
  shift
done

if [ -n "$CLONE_REPO" ]; then
  info "Cloning $CLONE_REPO to /tmp/robot_manager"
  rm -rf /tmp/robot_manager
  git clone "$CLONE_REPO" /tmp/robot_manager
  cd /tmp/robot_manager
  exec ./install.sh --admin-user "$ADMIN_USER" --robot-user "$ROBOT_USER" \
    ${ROBOT_HOME:+--robot-home "$ROBOT_HOME"} \
    ${PURGE_OLD_USER:+--purge-user "$PURGE_OLD_USER"}
fi
# These modes are standalone actions; avoid silently ignoring another request.
if [ "$((UNDO_NETWORK + CHECK_RUNTIME_DEPS + CLEANUP_PACKAGES))" -gt 1 ]; then
  die "choose only one of --undo-network, --check-runtime-deps and --cleanup-packages"
fi
if [ "$CLEANUP_PACKAGES" = 1 ] && [ "$INSTALL_DEPS" != 1 ]; then
  die "--cleanup-packages cannot be combined with --no-install-deps"
fi

run() {
  if [ "$DRY_RUN" = 1 ]; then
    printf '[dry-run]'; printf ' %q' "$@"; printf '\n'
  else
    "$@"
  fi
}

acquire_install_lock() {
  [ "$DRY_RUN" != 1 ] || return 0
  command -v flock >/dev/null 2>&1 || die "flock (util-linux) is required to serialize installations"
  local lock_file="${1:-/run/robot_manager/install.lock}"
  install -d -m 0755 "$(dirname "$lock_file")"
  # Keep this descriptor open until the installer and its build children exit.
  # Never unlink the lock file: all invocations must lock the same inode.
  exec {INSTALL_LOCK_FD}>"$lock_file"
  flock -n "$INSTALL_LOCK_FD" \
    || die "another installation or maintenance action is running; wait for it to finish (systemctl list-units 'w4studica-install-*')"
}

apt_install_packages() {  # apt_install_packages <reason> <package>...
  local reason="$1"; shift
  [ "$#" -gt 0 ] || return 0
  if [ "$INSTALL_DEPS" != 1 ]; then
    die "$reason are missing. Install these packages and rerun: $*"
  fi
  if [ "$DRY_RUN" != 1 ]; then
    command -v apt-get >/dev/null 2>&1 || die "apt-get is required to install $reason; install packages manually: $*"
  fi
  info "Installing $reason: $*"
  info "Waiting up to 10 minutes for apt/dpkg locks held by automatic updates."
  run env DEBIAN_FRONTEND=noninteractive apt-get -o DPkg::Lock::Timeout=600 update \
    || die "apt-get update failed; could not install $reason"
  run env DEBIAN_FRONTEND=noninteractive apt-get -o DPkg::Lock::Timeout=600 install -y --no-install-recommends --no-remove "$@" \
    || die "apt could not install $reason ($*)"
}

ensure_base_runtime_dependencies() {
  local -a packages=()
  command -v python3 >/dev/null 2>&1 || packages+=(python3)
  dpkg-query -W -f='${Status}' btop 2>/dev/null | grep -q 'install ok installed' || packages+=(btop)
  # WPILib's RioLog client discovers the roboRIO-compatible endpoint as robot.local.
  # Publish that mDNS name even when the VMX image hostname is "vmx".
  dpkg-query -W -f='${Status}' avahi-daemon 2>/dev/null | grep -q 'install ok installed' || packages+=(avahi-daemon)
  # WPILib linuxarm64 and the VMX HAL use the system C/C++ runtimes; libatomic
  # is explicitly required by the VMX plugin link and is not guaranteed on a minimal image.
  dpkg-query -W -f='${Status}' libstdc++6 2>/dev/null | grep -q 'install ok installed' || packages+=(libstdc++6)
  dpkg-query -W -f='${Status}' libgcc-s1 2>/dev/null | grep -q 'install ok installed' || packages+=(libgcc-s1)
  dpkg-query -W -f='${Status}' libatomic1 2>/dev/null | grep -q 'install ok installed' || packages+=(libatomic1)
  dpkg-query -W -f='${Status}' systemd-timesyncd 2>/dev/null | grep -q 'install ok installed' || packages+=(systemd-timesyncd)
  dpkg-query -W -f='${Status}' ttyd 2>/dev/null | grep -q 'install ok installed' || packages+=(ttyd)
  if ! command -v ldd >/dev/null 2>&1 || ! command -v ldconfig >/dev/null 2>&1; then
    packages+=(libc-bin)
  fi
  if [ "${#packages[@]}" -gt 0 ]; then
    apt_install_packages "robot_manager and native-library runtime tools" "${packages[@]}"
  fi
  if [ "$DRY_RUN" != 1 ]; then
    if ! command -v ldd >/dev/null 2>&1 || ! command -v ldconfig >/dev/null 2>&1; then
      die "ldd and ldconfig are required to validate native runtime dependencies"
    fi
  fi
  if [ "$DRY_RUN" != 1 ]; then
    local avahi_conf=/etc/avahi/avahi-daemon.conf
    [ -f "$avahi_conf" ] || die "Avahi config not found at $avahi_conf"
    if ! grep -Eq '^[[:space:]]*host-name[[:space:]]*=[[:space:]]*robot[[:space:]]*$' "$avahi_conf"; then
      if grep -Eq '^[[:space:]]*host-name[[:space:]]*=' "$avahi_conf"; then
        run sed -i -E 's/^[[:space:]]*host-name[[:space:]]*=.*/host-name=robot/' "$avahi_conf"
    else
      run sed -i '/^\[server\]/a host-name=robot' "$avahi_conf"
    fi
    grep -Eq '^[[:space:]]*host-name[[:space:]]*=[[:space:]]*robot[[:space:]]*$' "$avahi_conf" \
      || die "could not configure Avahi to publish robot.local in $avahi_conf"
    fi
    run systemctl enable --now avahi-daemon
    run systemctl restart avahi-daemon
  fi
}

ensure_vmx_hal_sdk() {
  # Headers and libvmxpi_hal_cpp.so are not part of Ubuntu or WPILib. They come
  # from the vmxpi-hal package built from vmxpi-aarch64-hal-builder and are a
  # prerequisite for running the deployed hardware runtime and fetching its SDK,
  # even when the Studica libraries are not built locally.
  if have_vmx; then return 0; fi
  local package="$VMX_HAL_DEB"
  if [ -z "$package" ]; then
    local candidate
    for candidate in "$SELF_DIR/../vmxpi-aarch64-hal-builder/dist"/vmxpi-hal_*_arm64.deb; do
      [ -f "$candidate" ] && { package="$candidate"; break; }
    done
  fi
  if [ -n "$package" ] && [ -f "$package" ]; then
    [ "$INSTALL_DEPS" = 1 ] || die "VMX HAL SDK is missing. Install it manually with: sudo apt-get install '$package'"
    [ "$DRY_RUN" = 1 ] || command -v apt-get >/dev/null 2>&1 || die "apt-get is required to install the VMX HAL SDK package"
    info "Installing VMX HAL SDK package: $package"
    run env DEBIAN_FRONTEND=noninteractive apt-get -o DPkg::Lock::Timeout=600 install -y --no-remove "$package" \
      || die "could not install the VMX HAL SDK package: $package"
  elif [ "$DRY_RUN" = 1 ]; then
    warn "dry run: vmxpi-hal package not found; expected $SELF_DIR/../vmxpi-aarch64-hal-builder/dist/vmxpi-hal_*_arm64.deb"
  else
    die "VMX HAL SDK missing. Build vmxpi-hal_<version>_arm64.deb from vmxpi-aarch64-hal-builder on a PC, copy it to the builder's dist/ beside Robot-Manager, then rerun (or pass --vmx-hal-deb FILE)."
  fi
  if [ "$DRY_RUN" != 1 ]; then
    have_vmx || die "VMX HAL SDK package installed but VMXPi.h/libvmxpi_hal_cpp.so are still missing"
  fi
}

compiler_major() {  # compiler_major <compiler>
  "$1" -dumpfullversion -dumpversion 2>/dev/null | cut -d. -f1
}

c_compiler_works() {  # c_compiler_works <compiler>
  command -v "$1" >/dev/null 2>&1 \
    && printf 'int main(void) { return 0; }\n' | "$1" -x c - -o /dev/null >/dev/null 2>&1
}

cpp_compiler_works() {  # cpp_compiler_works <compiler> [standard]
  local standard="${2:-}"
  local -a standard_flag=()
  [ -z "$standard" ] || standard_flag=("-std=$standard")
  command -v "$1" >/dev/null 2>&1 \
    && printf 'int main() { return 0; }\n' | "$1" "${standard_flag[@]}" -x c++ - -o /dev/null >/dev/null 2>&1
}

BUILD_CC=gcc
BUILD_CXX=g++
ensure_build_dependencies() {
  local need_build=0 need_cpp=0 need_modern_cpp=0 cpp_standard=""
  if [ "$SKIP_PLUGIN" != 1 ] || [ "$SKIP_SHIM" != 1 ] || [ "$SKIP_LSE_EMU" != 1 ]; then
    need_build=1
  fi
  if [ "$SKIP_DRIVERS" != 1 ] && { [ "$UPGRADE_DRIVERS" = 1 ] || [ ! -f /usr/local/lib/studica_drivers/libstudica_drivers.so ]; }; then
    need_build=1
    need_cpp=1
  fi
  [ "$need_build" = 1 ] || return 0
  if [ "$SKIP_PLUGIN" != 1 ]; then need_cpp=1; cpp_standard=c++20; fi

  if ! c_compiler_works gcc || ! command -v make >/dev/null 2>&1 \
      || { [ "$need_cpp" = 1 ] && ! cpp_compiler_works g++ "$cpp_standard"; }; then
    if [ "$need_cpp" = 1 ]; then
      apt_install_packages "the C/C++ build tools" build-essential
    else
      apt_install_packages "the C runtime helper build tools" gcc make libc6-dev
    fi
  fi

  local major=""
  if [ "$need_cpp" = 1 ] && command -v g++ >/dev/null 2>&1; then major="$(compiler_major g++)"; fi
  if [ "$SKIP_PLUGIN" != 1 ] && { [ -z "$major" ] || [ "$major" -lt 11 ]; }; then
    need_modern_cpp=1
    # Prefer the system compiler when it is already new enough; otherwise use
    # Ubuntu's versioned GCC 11 package without changing the OS-wide default.
    if command -v gcc-11 >/dev/null 2>&1 && command -v g++-11 >/dev/null 2>&1; then
      BUILD_CC=gcc-11
      BUILD_CXX=g++-11
    elif [ "$DRY_RUN" = 1 ]; then
      BUILD_CC=gcc-11
      BUILD_CXX=g++-11
      apt_install_packages "GCC 11 for the C++20 Studica plugin" build-essential gcc-11 g++-11
    else
      apt_install_packages "GCC 11 for the C++20 Studica plugin" build-essential gcc-11 g++-11
      if ! command -v gcc-11 >/dev/null 2>&1 || ! command -v g++-11 >/dev/null 2>&1; then
        die "GCC 11 packages installed but gcc-11/g++-11 are unavailable"
      fi
      BUILD_CC=gcc-11
      BUILD_CXX=g++-11
    fi
  fi

  if [ "$DRY_RUN" != 1 ]; then
    c_compiler_works "$BUILD_CC" || die "$BUILD_CC cannot compile and link a C program; install build-essential and rerun"
    if [ "$need_cpp" = 1 ]; then
      cpp_compiler_works "$BUILD_CXX" "$cpp_standard" \
        || die "$BUILD_CXX cannot compile and link a C++ program ($cpp_standard); install build-essential and rerun"
    fi
    command -v make >/dev/null 2>&1 || die "make is required to build runtime helpers"
    if [ "$SKIP_PLUGIN" != 1 ]; then
      major="$(compiler_major "$BUILD_CXX")"
      [ -n "$major" ] && [ "$major" -ge 11 ] \
        || die "the Studica backend requires GCC/G++ 11 or newer for C++20; found '${major:-unknown}'"
    fi
  elif [ "$need_modern_cpp" = 0 ]; then
    info "C build tools: $(tool_version "$BUILD_CC" --version); Studica C++ build enabled: $need_cpp"
  fi
  if [ "$DRY_RUN" != 1 ]; then
    echo "  build compiler: $BUILD_CC; make: $(tool_version make --version)"
    if [ "$need_cpp" = 1 ]; then echo "  Studica compiler: $BUILD_CXX"; fi
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
  # shellcheck disable=SC1091
  echo "  OS:      $(. /etc/os-release 2>/dev/null && echo "${PRETTY_NAME:-unknown}")"
  echo "  kernel:  $(uname -r)"
  echo "  model:   ${model:-unknown}"
  echo "  glibc:   $(ldd --version 2>/dev/null | head -1 || echo unknown)"
  echo "  g++:     $(tool_version g++ --version)"
  echo "  python3: $(tool_version python3 --version)"
  echo "  cargo:   $(tool_version cargo --version)"
  if ! command -v systemctl >/dev/null 2>&1; then
    if [ "$DRY_RUN" = 1 ]; then
      warn "systemctl is missing on this host; the dry run cannot validate service operations"
    else
      die "systemd (systemctl) is required on the VMX-pi"
    fi
  fi
  if [ "$(uname -m)" != aarch64 ]; then
    if [ "$DRY_RUN" = 1 ]; then
      warn "dry run is on $(uname -m); real installation/builds require the VMX-pi aarch64 host"
    else
      die "this installer builds VMX aarch64 libraries and must run on aarch64 (found $(uname -m))"
    fi
  fi
}

have_vmx() {
  [ -f /usr/local/include/vmxpi/VMXPi.h ] && [ -f /usr/local/lib/vmxpi/libvmxpi_hal_cpp.so ]
}

gpp_major() { compiler_major "${BUILD_CXX:-g++}"; }

validate_install_inputs() {
  if [ "$DRY_RUN" = 1 ]; then
    [ -f "$RM_DIR/Cargo.toml" ] && [ -f "$RM_DIR/Cargo.lock" ] && [ -f "$RM_DIR/robot_manager.service" ] \
      || warn "dry run: Robot-Manager sources are not present at $RM_DIR"
    have_vmx || warn "dry run: VMX headers/libraries are not on this host; required on the robot for hardware access"
    if [ "$SKIP_PLUGIN" != 1 ] || [ "$SKIP_DRIVERS" != 1 ]; then
      [ -d "$AW_DIR/studica/drivers" ] || warn "dry run: development Studica sources are not present under $AW_DIR"
    fi
    return 0
  fi
  [ -f "$RM_DIR/src/storage.rs" ] || [ -f "$RM_DIR/system/storage/configure.py" ] \
    || die "storage profile sources not found under $RM_DIR/src/storage.rs"
  [ -f "$RM_DIR/Cargo.toml" ] && [ -f "$RM_DIR/Cargo.lock" ] && [ -f "$RM_DIR/robot_manager.service" ] \
    || die "Cargo.toml / Cargo.lock / robot_manager.service not found in $RM_DIR (use --robot-manager-dir)"
  if [ "$SKIP_PLUGIN" != 1 ] || [ "$SKIP_DRIVERS" != 1 ]; then
    [ -f "$AW_DIR/LICENSE.md" ] && [ -f "$AW_DIR/ThirdPartyNotices.txt" ] \
      && [ -f "$AW_DIR/studica/drivers/LICENSE" ] && [ -f "$AW_DIR/studica/drivers/SOURCE.txt" ] \
      || die "development Studica build needs the allwpilib sources and license notices (use --allwpilib-dir)"
  fi

  if [ "$SKIP_PLUGIN" != 1 ]; then
    have_vmx || die "VMX SDK missing: expected /usr/local/include/vmxpi/VMXPi.h and /usr/local/lib/vmxpi/libvmxpi_hal_cpp.so. Install the vmxpi-hal SDK (for example, the package built by vmxpi-aarch64-hal-builder), then rerun or pass --skip-plugin."
    local studica="$AW_DIR/studica"
    [ -f "$studica/src/hardware/StudicaBackend.cpp" ] \
      || die "halsim_studica plugin source not found under $AW_DIR (use --allwpilib-dir or pass --skip-plugin)"
    [ -d "$AW_DIR/studica/drivers" ] \
      || die "studica_drivers source not found under $AW_DIR (use --allwpilib-dir or pass --skip-plugin)"
  fi

  if [ "$SKIP_DRIVERS" != 1 ] && { [ "$UPGRADE_DRIVERS" = 1 ] || [ ! -f /usr/local/lib/studica_drivers/libstudica_drivers.so ]; }; then
    have_vmx || die "VMX SDK missing: expected /usr/local/include/vmxpi/VMXPi.h and /usr/local/lib/vmxpi/libvmxpi_hal_cpp.so. Install the vmxpi-hal SDK (for example, the package built by vmxpi-aarch64-hal-builder), or pass --skip-drivers."
    [ -d "$AW_DIR/studica/drivers" ] \
      || die "studica_drivers source not found under $AW_DIR (use --allwpilib-dir or pass --skip-drivers)"
  fi
  if [ "$SKIP_SHIM" != 1 ]; then
    [ -f "$RM_DIR/gpio_isr_shim/gpio_isr_shim.c" ] \
      || die "GPIO shim source is missing from $RM_DIR (or pass --skip-shim)"
  fi
  if [ "$SKIP_LSE_EMU" != 1 ]; then
    [ -f "$RM_DIR/lse_emu/lse_emu.c" ] \
      || die "LSE emulator source is missing from $RM_DIR (or pass --skip-lse-emu)"
  fi
  [ -f "$RM_DIR/src/network/mod.rs" ] || die "Rust network command source is missing from $RM_DIR"
  [ -f "$RM_DIR/src/network/backup.rs" ] || die "Rust network backup source is missing from $RM_DIR"
  if [ "$WITH_SUDOERS" = 1 ]; then
    command -v visudo >/dev/null 2>&1 || die "visudo is required to validate --sudoers rules"
  fi
}

setup_accounts() {
  info "Configuring accounts: admin='$ADMIN_USER', robot='$ROBOT_USER'"
  local admin_groups="sudo,adm,dialout,plugdev,netdev,kmem"
  local robot_groups="sudo,adm,dialout,i2c,spi,kmem,plugdev,netdev"

  if [ "$DRY_RUN" = 1 ]; then
    echo "  [dry-run] configure admin account '$ADMIN_USER' (groups: $admin_groups)"
    echo "  [dry-run] configure robot account '$ROBOT_USER' (home: $ROBOT_HOME, groups: $robot_groups)"
    return 0
  fi

  # 1. Admin account setup
  if id "$ADMIN_USER" >/dev/null 2>&1; then
    run usermod -a -G "$admin_groups" "$ADMIN_USER" 2>/dev/null || true
  else
    info "Creating admin user '$ADMIN_USER'"
    run useradd -m -s /bin/bash -U -G "$admin_groups" "$ADMIN_USER"
    echo "$ADMIN_USER:$ADMIN_PASSWORD" | chpasswd
  fi

  # Copy SSH authorized_keys from current sudo user if present
  local old_user="${SUDO_USER:-}"
  if [ -n "$old_user" ] && [ "$old_user" != "$ADMIN_USER" ] && [ -f "/home/$old_user/.ssh/authorized_keys" ]; then
    run install -d -m 0700 "/home/$ADMIN_USER/.ssh"
    run cp -f "/home/$old_user/.ssh/authorized_keys" "/home/$ADMIN_USER/.ssh/authorized_keys"
    run chown -R "$ADMIN_USER:$ADMIN_USER" "/home/$ADMIN_USER/.ssh" 2>/dev/null || true
    run chmod 0600 "/home/$ADMIN_USER/.ssh/authorized_keys"
  fi

  # 2. Robot user setup
  if id "$ROBOT_USER" >/dev/null 2>&1; then
    run usermod -a -G "$robot_groups" "$ROBOT_USER" 2>/dev/null || true
  else
    info "Creating robot user '$ROBOT_USER' (home: $ROBOT_HOME)"
    run useradd -m -d "$ROBOT_HOME" -s /bin/bash -U -G "$robot_groups" "$ROBOT_USER"
    echo "$ROBOT_USER:$ROBOT_PASSWORD" | chpasswd
  fi
  run install -d -m 0755 "$ROBOT_HOME"
  run chown "$ROBOT_USER:$ROBOT_USER" "$ROBOT_HOME" 2>/dev/null || true
}
verify_shared_library() {  # verify_shared_library <path> [extra-library-path]
  local path="$1" search_path="${2:-/usr/local/lib/vmxpi}" result
  [ -f "$path" ] || die "expected shared library was not built: $path"
  result="$(LD_LIBRARY_PATH="$search_path${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}" ldd "$path" 2>&1)" \
    || die "could not inspect runtime dependencies for $path: $result"
  if grep -q 'not found' <<<"$result"; then
    echo "$result" >&2
    die "unresolved runtime dependency in $path"
  fi
}

check_deployed_wpilib_runtime() {
  local lib_dir="$ROBOT_HOME/wpilib/third-party/lib"
  local command_file="$ROBOT_HOME/robotCommand"
  local search_path="$lib_dir:/usr/local/lib/vmxpi:$PLUGIN_DIR:$OPT_DIR/lib"
  local count=0 executable library name required_hal command_details active_extensions=""
  local -a libraries=()
  info "WPILib deployed runtime dependencies"
  if [ ! -d "$lib_dir" ] && [ ! -f "$command_file" ]; then
    if [ "$CHECK_RUNTIME_DEPS" = 1 ]; then
      die "no GradleRIO deployment found under $ROBOT_HOME; deploy the robot program first"
    fi
    echo "  no GradleRIO deployment yet; its project-specific libraries will be checked after deploy"
    return 0
  fi
  command -v ldd >/dev/null 2>&1 || die "ldd is required to inspect deployed native dependencies"
  if [ -f "$command_file" ]; then
    local raw_cmd words=() program="" extensions=""
    raw_cmd="$(<"$command_file")"
    eval "words=($raw_cmd)"
    local idx=0
    while [ $idx -lt ${#words[@]} ] && [[ "${words[$idx]}" =~ ^[A-Za-z_][A-Za-z0-9_]*= ]]; do
      if [[ "${words[$idx]}" == HALSIM_EXTENSIONS=* ]]; then
        local ext_val="${words[$idx]#HALSIM_EXTENSIONS=}"
        local IFS=':' ext_parts
        read -ra ext_parts <<< "$ext_val"
        local ext_list=()
        for ep in "${ext_parts[@]}"; do
          [ -n "$ep" ] && ext_list+=("$(basename "$ep")")
        done
        extensions="$(IFS=:; echo "${ext_list[*]}")"
      fi
      idx=$((idx + 1))
    done
    if [ $idx -lt ${#words[@]} ] && [ "${words[$idx]}" = "gdbserver" ]; then
      idx=$((idx + 2))
    fi
    program="${words[$idx]}"
    [[ "$program" = /* ]] || program="$ROBOT_HOME/$program"
    executable="$program"
    if [ -n "$extensions" ]; then
      active_extensions=":$extensions:"
    fi
  fi
  if [ -d "$lib_dir" ]; then
    mapfile -d '' -t libraries < <(find "$lib_dir" -type f \( -name '*.so' -o -name '*.so.*' \) ! -name '*.debug' -print0)
    for library in "${libraries[@]}"; do
      name="$(basename "$library")"
      required_hal=""
      case "$name" in
        libhalsim_studicad.so|libhalsim_vmxd.so) required_hal=libwpiHald.so ;;
        libhalsim_studica.so|libhalsim_vmx.so) required_hal=libwpiHal.so ;;
      esac
      # Release ZIPs contain both extensions, and old deploys can leave the VMX
      # names behind. Skip only the unused build type; an explicitly selected
      # extension must still fail validation when its dependencies are missing.
      if [ -n "$required_hal" ] && [ ! -f "$lib_dir/$required_hal" ] \
          && [[ "$active_extensions" != *":$name:"* ]]; then
        echo "  skipping $name: unused extension build type (no $required_hal)"
        continue
      fi
      verify_shared_library "$library" "$search_path"
      count=$((count + 1))
    done
  fi
  if [ -f "$command_file" ]; then
    verify_shared_library "$executable" "$search_path"
    count=$((count + 1))
  fi
  if [ "$count" -eq 0 ]; then
    if [ "$CHECK_RUNTIME_DEPS" = 1 ]; then
      die "no deployed executable or shared libraries were found under $ROBOT_HOME"
    fi
    echo "  GradleRIO deployment files are not present yet; skipped"
    return 0
  fi
  echo "  checked $count deployed executable/shared librar$( [ "$count" -eq 1 ] && echo "y" || echo "ies" ); no unresolved dependencies"
}

rust_compiler_supported() {
  local version major minor
  version="$(rustc --version 2>/dev/null)" || return 1
  major="$(echo "$version" | sed -n 's/^rustc \([0-9]*\)\.\([0-9]*\).*/\1/p')"
  minor="$(echo "$version" | sed -n 's/^rustc \([0-9]*\)\.\([0-9]*\).*/\2/p')"
  [ -n "$major" ] && [ -n "$minor" ] || return 1
  [ "$major" -gt 1 ] || { [ "$major" -eq 1 ] && [ "$minor" -ge 85 ]; }
}

ensure_rust_build_dependencies() {
  if [ -n "$MANAGER_BINARY" ]; then
    [ -f "$MANAGER_BINARY" ] || die "--manager-binary file not found: $MANAGER_BINARY"
    return
  fi
  if ! command -v cargo >/dev/null 2>&1 || ! rust_compiler_supported; then
    apt_install_packages "Rust build tools for robot_manager" cargo rustc
  fi
  if [ "$DRY_RUN" != 1 ]; then
    command -v cargo >/dev/null 2>&1 && rust_compiler_supported \
      || die "cargo and rustc (Rust >= 1.85) are required; install them or use --manager-binary"
  fi
}

rt_kernel_active() {
  if [ -r /sys/kernel/realtime ]; then
    [ "$(cat /sys/kernel/realtime)" = 1 ]
  else
    grep -qw PREEMPT_RT /proc/version
  fi
}

rt_kernel_platform_supported() {
  local ID="" VERSION_ID="" model=""
  [ -r /etc/os-release ] || return 1
  # shellcheck disable=SC1091
  . /etc/os-release
  [ "$ID" = ubuntu ] && dpkg --compare-versions "$VERSION_ID" ge 26.04 || return 1
  [ "$(uname -m)" = aarch64 ] || return 1
  [ -r /proc/device-tree/model ] || return 1
  model="$(tr -d '\0' < /proc/device-tree/model)"
  [[ "$model" = Raspberry\ Pi* ]]
}

ensure_rt_kernel() {
  info "PREEMPT_RT kernel"
  if rt_kernel_active; then
    RT_KERNEL_STATUS="PREEMPT_RT active"
    echo "  $RT_KERNEL_STATUS ($(uname -r))"
    return
  fi
  if [ "$SKIP_RT_KERNEL" = 1 ]; then
    RT_KERNEL_STATUS="kernel installation skipped; robot scheduling is still configured"
    echo "  $RT_KERNEL_STATUS (--skip-rt-kernel)"
    return
  fi
  if [ "$DRY_RUN" != 1 ] && ! rt_kernel_platform_supported; then
    die "automatic RT kernel installation requires Ubuntu >= 26.04 on a Raspberry Pi (aarch64). Install a compatible PREEMPT_RT kernel manually or use --skip-rt-kernel."
  fi
  # Ubuntu >= 26.04 provides this Pi-specific metapackage in its main archive.
  # Never install the generic ubuntu-realtime kernel on the VMX's Raspberry Pi.
  # https://ubuntu.com/real-time/docs/how-to/enable-real-time-ubuntu/
  if ! dpkg-query -W -f='${Status}' linux-raspi-realtime 2>/dev/null | grep -q 'install ok installed'; then
    apt_install_packages "Raspberry Pi PREEMPT_RT kernel" linux-raspi-realtime
  fi
  RT_KERNEL_STATUS="RT kernel installed; reboot required, then verify /sys/kernel/realtime = 1"
  if [ "$DRY_RUN" = 1 ]; then
    RT_KERNEL_STATUS="would install linux-raspi-realtime if missing; reboot required to activate"
  fi
  warn "$RT_KERNEL_STATUS. The installer does not reboot automatically."
}

configure_realtime() {
  local path="$CONF_DIR/config.json"
  [ -f "$path" ] || return 0
  local content
  content="$(<"$path")"
  if [[ ! "$content" =~ ^[[:space:]]*\{ ]]; then
    echo "$path: configuration must be a JSON object" >&2
    exit 1
  fi
  if grep -q '"realtime"' "$path"; then
    echo "  keeping existing RT policy in $path"
    return 0
  fi
  if [ "$DRY_RUN" = 1 ]; then
    echo "[dry-run] add realtime auto/rr/10 to $path"
    return 0
  fi
  local tmp; tmp="$(mktemp "${path}.XXXXXX")"
  sed '0,/{/s/{/{\n  "realtime": {"mode": "auto", "policy": "rr", "priority": 10},/' "$path" > "$tmp"
  chmod --reference="$path" "$tmp" 2>/dev/null || chmod 0640 "$tmp"
  chown --reference="$path" "$tmp" 2>/dev/null || true
  mv -f "$tmp" "$path"
  echo "  added RT policy auto/rr/10 to $path"
}

install_robot_manager() {
  info "robot_manager (Rust)"
  local binary="$MANAGER_BINARY"
  if [ -z "$binary" ]; then
    run cargo build --release --locked --manifest-path "$RM_DIR/Cargo.toml" --target-dir "$BUILD_DIR/robot_manager"
    binary="$BUILD_DIR/robot_manager/release/robot_manager"
  fi
  if [ "$DRY_RUN" != 1 ]; then
    "$binary" --version || die "robot_manager binary cannot run on this robot"
    "$binary" network --help >/dev/null || die "robot_manager binary is missing Rust network commands"
    "$binary" network-backup --help >/dev/null || die "robot_manager binary is missing Rust network backup"
  fi
  run install -d -m 0755 "$OPT_DIR" "$CONF_DIR"
  run install -d -m 0755 "$OPT_DIR/licenses"
  run install -m 0644 "$RM_DIR/LICENSE" "$RM_DIR/ThirdPartyNotices.txt" "$OPT_DIR/licenses/"
  # Replace atomically: an already running Rust service may still hold the old executable open.
  run install -m 0755 "$binary" "$OPT_DIR/robot_manager.new"
  run mv -f "$OPT_DIR/robot_manager.new" "$OPT_DIR/robot_manager"
  run install -m 0644 "$RM_DIR/robot_manager.service" "/etc/systemd/system/$SERVICE.service"
  run ln -sfn "$SERVICE.service" "/etc/systemd/system/robot.service"

  if [ -f "$CONF_DIR/config.json" ]; then
    configure_realtime
  else
    # The platform profile list is left empty on purpose: the pin-number offset it would carry is not known yet.
    write_file "$CONF_DIR/config.json" 0644 <<EOF
{
  "robot_home": "$ROBOT_HOME",
  "realtime": { "mode": "auto", "policy": "rr", "priority": 10 },
  "env": {},
  "profiles": []
}
EOF
  fi

  run "$OPT_DIR/robot_manager" --config "$CONF_DIR/config.json" --check-config
  run rm -f "$OPT_DIR/robot_manager.py"
  run systemctl daemon-reload
  run systemctl enable "$SERVICE"
  install_systemcore_services
}

install_systemcore_services() {
  info "SystemCore compatibility services (mrccomm, hwmon, irqconf, ttyd, timesyncd)"
  run install -d -m 0755 /etc/systemd/timesyncd.conf.d
  run install -m 0644 "$RM_DIR/system/99-w4studica-timesyncd.conf" /etc/systemd/timesyncd.conf.d/99-w4studica.conf
  run systemctl enable systemd-timesyncd 2>/dev/null || true

  if command -v ttyd >/dev/null 2>&1 || [ "$DRY_RUN" = 1 ]; then
    run install -m 0644 "$RM_DIR/system/ttyd.service" /etc/systemd/system/ttyd.service
    run systemctl enable ttyd.service 2>/dev/null || true
  fi

  run install -m 0644 "$RM_DIR/system/hwmon.service" /etc/systemd/system/hwmon.service
  run systemctl enable hwmon.service 2>/dev/null || true

  run install -m 0644 "$RM_DIR/system/irqconf.service" /etc/systemd/system/irqconf.service
  run systemctl enable irqconf.service 2>/dev/null || true

  run install -m 0644 "$RM_DIR/system/mrccomm.service" /etc/systemd/system/mrccomm.service
  run systemctl enable mrccomm.service 2>/dev/null || true
}
install_studica_notices() {
  if [ "$SKIP_DRIVERS" = 1 ] && [ "$SKIP_PLUGIN" = 1 ]; then return; fi
  run install -d -m 0755 "$OPT_DIR/licenses/studica"
  run install -m 0644 "$AW_DIR/LICENSE.md" "$AW_DIR/ThirdPartyNotices.txt" "$OPT_DIR/licenses/studica/"
  run install -m 0644 "$AW_DIR/studica/drivers/LICENSE" "$AW_DIR/studica/drivers/SOURCE.txt" "$OPT_DIR/licenses/studica/"
}

install_drivers() {
  info "studica_drivers"
  if [ "$SKIP_DRIVERS" = 1 ]; then echo "  supplied by GradleRIO deploy (local build disabled)"; return; fi
  have_vmx || { [ "$DRY_RUN" = 1 ] && { warn "dry run: VMX SDK is absent on this host; skipping the local driver build preview"; return; }; die "VMX SDK not found under /usr/local; cannot build studica_drivers"; }
  # The vmx-hal package already ships (an older) libstudica_drivers. Do not overwrite files a package owns
  # unless asked. The halsim_studica plugin compiles the driver sources itself, so it does not need this library.
  if [ -f /usr/local/lib/studica_drivers/libstudica_drivers.so ] && [ "$UPGRADE_DRIVERS" != 1 ]; then
    echo "  already installed (probably by the vmx-hal package); not replacing it."
    echo "  pass --upgrade-drivers to install this repo's newer copy over it (replaces package-owned files)."
    return
  fi
  # Build in a copy so the checkout is not littered with build output (or root-owned files).
  run rm -rf "$BUILD_DIR/studica_drivers"
  run install -d "$BUILD_DIR"
  run cp -r "$AW_DIR/studica/drivers" "$BUILD_DIR/studica_drivers"
  run make -C "$BUILD_DIR/studica_drivers" CC="$BUILD_CC" CXX="$BUILD_CXX"
  if [ "$DRY_RUN" != 1 ] && ! compgen -G "$BUILD_DIR/studica_drivers/*.hpp" >/dev/null; then
    die "studica_drivers build produced no public headers"
  fi
  # Copy as root directly. The upstream Makefile's install target invokes sudo,
  # which is unnecessary here and is not guaranteed in minimal images.
  run install -d /usr/local/include/studica_drivers /usr/local/lib/studica_drivers
  run install -m 0644 "$BUILD_DIR"/studica_drivers/*.hpp /usr/local/include/studica_drivers/
  run install -m 0755 "$BUILD_DIR/studica_drivers/libstudica_drivers.so" /usr/local/lib/studica_drivers/
  run ldconfig
  if [ "$DRY_RUN" != 1 ]; then verify_shared_library /usr/local/lib/studica_drivers/libstudica_drivers.so; fi
}

install_plugin() {
  info "Studica backend plugin for halsim_studica"
  if [ "$SKIP_PLUGIN" = 1 ]; then echo "  supplied by GradleRIO deploy (local build disabled)"; return; fi
  have_vmx || { [ "$DRY_RUN" = 1 ] && { warn "dry run: VMX SDK is absent on this host; skipping the local plugin build preview"; return; }; die "VMX SDK not found under /usr/local; cannot build the Studica backend plugin"; }
  local major; major="$(gpp_major)"
  if [ "$DRY_RUN" != 1 ]; then
    [ -n "$major" ] && [ "$major" -ge 11 ] || die "$BUILD_CXX >= 11 (C++20) is required, found '${major:-none}'"
  else
    info "Would build the plugin with $BUILD_CXX (GCC/G++ 11 or newer is required)"
  fi

  local drv="$AW_DIR/studica/drivers"
  local studica="$AW_DIR/studica"
  run install -d -m 0755 "$PLUGIN_DIR"
  run "$BUILD_CXX" -std=c++20 -shared -fPIC -O2 \
    -I/usr/local/include/vmxpi -I"$drv" \
    -I"$studica/include" -I"$AW_DIR/hal/src/main/native/include" -I"$AW_DIR/wpiutil/src/main/native/include" \
    "$studica"/src/hardware/*.cpp "$studica"/src/hardware/devices/*.cpp "$studica/src/conversion/DeviceConversions.cpp" "$studica/src/titan/Titan.cpp" \
    "$drv/analog_input.cpp" "$drv/dio.cpp" "$drv/encoder.cpp" "$drv/imu.cpp" "$drv/titan.cpp" \
    -L/usr/local/lib/vmxpi -lvmxpi_hal_cpp -lrt -lpthread -latomic \
    -o "$PLUGIN_DIR/libstudica_hardware.so"
  if [ "$DRY_RUN" != 1 ]; then
    run ldconfig
    verify_shared_library "$PLUGIN_DIR/libstudica_hardware.so"
  fi
}

install_shim() {
  info "GPIO interrupt shim"
  if [ "$SKIP_SHIM" = 1 ]; then echo "  skipped (--skip-shim)"; return; fi
  local src="$RM_DIR/gpio_isr_shim"
  run rm -rf "$BUILD_DIR/gpio_isr_shim"
  run install -d "$BUILD_DIR" "$OPT_DIR/lib"
  run cp -r "$src" "$BUILD_DIR/gpio_isr_shim"
  # The checkout may contain ignored binaries built for the developer PC;
  # never let make reuse them for the robot's architecture.
  run make -C "$BUILD_DIR/gpio_isr_shim" clean
  run make -C "$BUILD_DIR/gpio_isr_shim" CC="$BUILD_CC" test
  run make -C "$BUILD_DIR/gpio_isr_shim" CC="$BUILD_CC"
  run install -m 0755 "$BUILD_DIR/gpio_isr_shim/libvmx_gpio_isr_shim.so" "$OPT_DIR/lib/libvmx_gpio_isr_shim.so"
  if [ "$DRY_RUN" != 1 ]; then verify_shared_library "$OPT_DIR/lib/libvmx_gpio_isr_shim.so"; fi
  if [ ! -d /sys/class/gpio ]; then
    if [ "$DRY_RUN" = 1 ]; then
      echo "  [dry-run] add $OPT_DIR/lib/libvmx_gpio_isr_shim.so to $CONF_DIR/config.json env.LD_PRELOAD"
    else
      if [ -f "$CONF_DIR/config.json" ]; then
        local library="$OPT_DIR/lib/libvmx_gpio_isr_shim.so"
        if ! grep -q "$library" "$CONF_DIR/config.json" 2>/dev/null; then
          local tmp; tmp="$(mktemp "${CONF_DIR}/config.json.XXXXXX")"
          if grep -q '"env":' "$CONF_DIR/config.json"; then
            sed "s|\"env\":[[:space:]]*{[[:space:]]*}|\"env\": { \"LD_PRELOAD\": \"$library\" }|" "$CONF_DIR/config.json" > "$tmp"
          else
            sed '0,/{/s/{/{\n  "env": { "LD_PRELOAD": "'"$library"'" },/' "$CONF_DIR/config.json" > "$tmp"
          fi
          chmod --reference="$CONF_DIR/config.json" "$tmp" 2>/dev/null || chmod 0640 "$tmp"
          chown --reference="$CONF_DIR/config.json" "$tmp" 2>/dev/null || true
          mv -f "$tmp" "$CONF_DIR/config.json"
        fi
        echo "  configured GPIO shim in $CONF_DIR/config.json env.LD_PRELOAD"
      fi
    fi
  fi
}

install_lse_emu() {
  info "LSE atomics emulation"
  if [ "$SKIP_LSE_EMU" = 1 ]; then echo "  skipped (--skip-lse-emu)"; return; fi
  local src="$RM_DIR/lse_emu"
  run rm -rf "$BUILD_DIR/lse_emu"
  run install -d "$BUILD_DIR" "$OPT_DIR/lib"
  run cp -r "$src" "$BUILD_DIR/lse_emu"
  # As above, only use binaries freshly built for the VMX's aarch64 CPU.
  run make -C "$BUILD_DIR/lse_emu" clean
  run make -C "$BUILD_DIR/lse_emu" CC="$BUILD_CC" test
  run make -C "$BUILD_DIR/lse_emu" CC="$BUILD_CC"
  run install -m 0755 "$BUILD_DIR/lse_emu/libvmx_lse_emu.so" "$OPT_DIR/lib/libvmx_lse_emu.so"
  if [ "$DRY_RUN" != 1 ]; then verify_shared_library "$OPT_DIR/lib/libvmx_lse_emu.so"; fi
  if grep -m1 '^Features' /proc/cpuinfo 2>/dev/null | grep -qw atomics; then
    echo "  this CPU has LSE atomics: robot_manager will not load it"
  else
    echo "  this CPU has no LSE atomics (Raspberry Pi 4): robot_manager sets LD_AUDIT=$OPT_DIR/lib/libvmx_lse_emu.so itself"
  fi
}

install_cpu_max() {
  info "CPU at full speed"
  if [ "$SKIP_CPU_MAX" = 1 ]; then echo "  skipped (--skip-cpu-max)"; return; fi
  local src="$RM_DIR/system"
  [ -f "$src/robot_cpu_max.sh" ] && [ -f "$src/robot_cpu_max.service" ] || { warn "$src/robot_cpu_max.* not found; skipping"; return; }
  if ! ls /sys/devices/system/cpu/cpu0/cpufreq >/dev/null 2>&1; then
    echo "  this kernel exposes no cpufreq: nothing to set"; return
  fi

  run install -d "$OPT_DIR/bin"
  run install -m 0755 "$src/robot_cpu_max.sh" "$OPT_DIR/bin/robot_cpu_max.sh"
  run install -m 0644 "$src/robot_cpu_max.service" /etc/systemd/system/robot_cpu_max.service
  # Ubuntu's ondemand.service sets the governor to "ondemand" a minute after every boot and would undo this.
  if systemctl list-unit-files ondemand.service 2>/dev/null | grep -q '^ondemand.service'; then
    run systemctl disable ondemand.service
    run systemctl mask ondemand.service
  fi
  run systemctl daemon-reload
  run systemctl enable robot_cpu_max.service
  run systemctl restart robot_cpu_max.service      # also applies it now
  if [ "$DRY_RUN" != 1 ]; then
    for cpu in /sys/devices/system/cpu/cpu[0-9]*/cpufreq; do
      echo "  $(basename "$(dirname "$cpu")"): $(cat "$cpu/scaling_governor") $(cat "$cpu/scaling_min_freq")-$(cat "$cpu/scaling_max_freq") kHz"
    done
  fi
}

ensure_system_profile_dependencies() {
  local package
  local -a profile_packages=()
  # Swap, fstab and journald configuration do not require a new initramfs.
  # Installing initramfs-tools here would replace Ubuntu's existing dracut.
  for package in systemd-zram-generator util-linux e2fsprogs; do
    dpkg-query -W -f='${Status}' "$package" 2>/dev/null | grep -q 'install ok installed' || profile_packages+=("$package")
  done
  if [ "${#profile_packages[@]}" -gt 0 ]; then
    apt_install_packages "storage and boot configuration tools" "${profile_packages[@]}"
  fi
}

install_system_profile() {
  info "Storage protection and boot configuration (applied at next reboot)"
  info "zram 8 GiB priority 100; disk swap 4 GiB priority 10; volatile logs capped at 64 MiB"
  info "ext4 journal and write barriers retained; no RAM OverlayFS; network wait bounded unless network mounts exist"
  run "$OPT_DIR/robot_manager" storage configure
  # Only disable competing managers for the next boot. swapoff may cause OOM
  # or stall a live robot; persistent syslog writers also stop at the reboot.
  local unit unit_state
  for unit in dphys-swapfile.service zramswap.service zram-config.service rsyslog.service rsyslog.socket; do
    if systemctl list-unit-files "$unit" --no-legend 2>/dev/null | awk '{print $1}' | grep -Fxq "$unit"; then
      unit_state="$(systemctl is-enabled "$unit" 2>/dev/null || true)"
      case "$unit_state" in
        masked|masked-runtime) continue ;;
        enabled|enabled-runtime|linked|linked-runtime|alias) run systemctl disable "$unit" ;;
      esac
      run systemctl mask "$unit"
    fi
  done
  # Existing kernel/initramfs tooling and firmware assets remain OS-managed.
  run systemctl daemon-reload
}

# Makes NetworkManager the network manager (Ubuntu Server ships systemd-networkd + netplan) and sets up the Wi-Fi access
# point and the wired port. Everything is written first, while the ssh session still works; the switch itself runs a few
# seconds after install.sh has finished, because it ends the session.
apply_network() {
  # Preserve the first configuration before apt can start a newly installed NM.
  # Neither the active manager nor saved Wi-Fi profiles are changed here.
  run "$OPT_DIR/bin/network_backup" snapshot

  local missing=0
  command -v nmcli >/dev/null 2>&1 || missing=1
  command -v ip >/dev/null 2>&1 || missing=1
  command -v iw >/dev/null 2>&1 || missing=1
  [ -x /usr/sbin/dnsmasq ] || command -v dnsmasq >/dev/null 2>&1 || missing=1   # dnsmasq-base: NetworkManager's DHCP server
  if [ "$missing" = 1 ]; then
    apt_install_packages "network configuration tools (internet access is required)" network-manager iproute2 dnsmasq-base iw
    if [ "$DRY_RUN" != 1 ]; then
      if ! command -v nmcli >/dev/null 2>&1 || ! command -v iw >/dev/null 2>&1; then
        die "network tools are still unavailable; the network was not changed"
      fi
      command -v ip >/dev/null 2>&1 || die "iproute2 is still unavailable; the network was not changed"
      [ -x /usr/sbin/dnsmasq ] || command -v dnsmasq >/dev/null 2>&1 \
        || die "dnsmasq is still unavailable; the network was not changed"
    fi
  fi

  # The connection files: written for NetworkManager, which has not started yet.
  run install -d -m 0755 /etc/NetworkManager/system-connections
  run "$OPT_DIR/net/vmx_net" setupEthernet --no-activate
  run "$OPT_DIR/net/vmx_net" setupHotspot --no-activate "$AP_SSID" "$AP_TEAM" "$AP_BAND" "$AP_PASSWORD"

  # The original is immutable across reinstalls. Only remove active YAML after
  # dependencies are ready and the snapshot has completed successfully.
  local yaml
  for yaml in /etc/netplan/*.yaml; do
    [ -f "$yaml" ] || continue
    run rm -f "$yaml"
  done
  write_file /etc/netplan/01-w4studica-network-manager.yaml 0600 <<'YAML'
# Installed by W4Studica install.sh: NetworkManager manages the interfaces (Wi-Fi access point, wired 172.22.11.2).
network:
  version: 2
  renderer: NetworkManager
YAML
  # cloud-init would write its own netplan file again at the next boot.
  run install -d /etc/cloud/cloud.cfg.d
  write_file /etc/cloud/cloud.cfg.d/99-w4studica-disable-network-config.cfg 0644 <<'CFG'
# Installed by W4Studica install.sh: the network is configured through NetworkManager, not cloud-init.
network: {config: disabled}
CFG
  run install -d -m 0755 /etc/polkit-1/rules.d /etc/polkit-1/localauthority/50-local.d
  run install -m 0644 "$RM_DIR/system/10-w4studica-network.rules" /etc/polkit-1/rules.d/10-w4studica-network.rules
  run install -m 0644 "$RM_DIR/system/10-w4studica-network.pkla" /etc/polkit-1/localauthority/50-local.d/10-w4studica-network.pkla
  run install -m 0644 "$RM_DIR/system/w4studica-rfkill-unblock.service" /etc/systemd/system/w4studica-rfkill-unblock.service
  run systemctl enable w4studica-rfkill-unblock.service 2>/dev/null || true
  command -v rfkill >/dev/null 2>&1 && run rfkill unblock wifi 2>/dev/null || true
  command -v iw >/dev/null 2>&1 && run iw reg set US 2>/dev/null || true
  command -v nmcli >/dev/null 2>&1 && run nmcli radio wifi on 2>/dev/null || true

  run install -m 0755 "$RM_DIR/system/undo_network.sh" "$OPT_DIR/bin/undo_network.sh"
  run install -m 0755 "$RM_DIR/system/apply_network.sh" "$OPT_DIR/bin/apply_network.sh"
  local team_number=$((10#$AP_TEAM)) address
  address="10.$((team_number / 100)).$((team_number % 100)).2"
  cat <<MSG
  Network prepared. In 8 seconds the robot switches to NetworkManager, applies setupHotspot and selects AP mode.
  Saved Wi-Fi profiles are deleted:
    wired port  172.22.11.2   (a computer on the cable gets 172.22.11.X from the robot)
    access point '$AP_SSID-$AP_TEAM'  ->  $address
  This SSH session may end. Reconnect:
    - cable:  set the PC to DHCP, then   ssh $LOGIN_USER@172.22.11.2
    - Wi-Fi:  join '$AP_SSID-$AP_TEAM' (password: password), then   ssh $LOGIN_USER@$address
  Go back (on the robot's screen and keyboard):   sudo $OPT_DIR/bin/undo_network.sh
MSG
  run systemd-run --collect --unit=w4studica-network-switch --on-active=8 --timer-property=RemainAfterElapse=no \
    --description="W4Studica: switch to NetworkManager" \
    "$OPT_DIR/bin/apply_network.sh" "$AP_SSID" "$AP_TEAM" "$AP_BAND" "$AP_PASSWORD"
}

undo_network() {
  info "Undo installer network setup"
  [ -f "$RM_DIR/system/undo_network.sh" ] || die "$RM_DIR/system/undo_network.sh not found"
  local binary="${MANAGER_BINARY:-$OPT_DIR/robot_manager}"
  # An older installation may have Python-only network helpers. Build the new
  # restore command without replacing its running service or touching the network.
  if [ "$DRY_RUN" != 1 ] && [ -f /var/lib/robot_manager/network-original/state.json ] \
      && ! "$binary" network-backup --help >/dev/null 2>&1; then
    [ -z "$MANAGER_BINARY" ] || die "--manager-binary is missing Rust network backup"
    ensure_base_runtime_dependencies
    ensure_rust_build_dependencies
    run cargo build --release --locked --manifest-path "$RM_DIR/Cargo.toml" --target-dir "$BUILD_DIR/robot_manager"
    binary="$BUILD_DIR/robot_manager/release/robot_manager"
  fi
  run env ROBOT_MANAGER_BINARY="$binary" sh "$RM_DIR/system/undo_network.sh"
}

install_network() {
  info "Network (Wi-Fi access point and fixed wired address)"
  run install -d "$OPT_DIR/net" "$OPT_DIR/bin"
  run ln -sf "$OPT_DIR/robot_manager" "$OPT_DIR/net/vmx_net"
  run ln -sf "$OPT_DIR/robot_manager" "$OPT_DIR/bin/network_backup"
  local cmd
  for cmd in setupHotspot toggleNetMode setupEthernet; do
    run ln -sf "$OPT_DIR/net/vmx_net" "/usr/local/bin/$cmd"
  done
  run rm -f "$OPT_DIR/net/vmx_net.py" "$OPT_DIR/bin/network_backup.py"
  echo "  installed: setupHotspot, toggleNetMode, setupEthernet"

  apply_network
}

cleanup_packages() {
  info "Optional Ubuntu image package cleanup"
  local helper="$RM_DIR/system/package_cleanup.sh"
  [ -f "$helper" ] || die "$helper not found"
  local -a options=()
  [ "$DRY_RUN" != 1 ] || options+=(--dry-run)
  bash "$helper" "${options[@]}"
}

# Last, so robot_manager starts with every library in place: it looks for the LSE emulation library when it starts.
restart_service() {
  info "Restarting $SERVICE"
  run systemctl restart "$SERVICE"
}

install_sudoers() {
  local target_user="${ADMIN_USER:-$LOGIN_USER}"
  info "sudoers for '$target_user'"
  if [ "$WITH_SUDOERS" != 1 ]; then echo "  skipped (pass --sudoers to allow passwordless deploy)"; return; fi
  local systemctl_bin ldconfig_bin sync_bin
  systemctl_bin="$(command -v systemctl)"; ldconfig_bin="$(command -v ldconfig)"; sync_bin="$(command -v sync)"
  local tmp; tmp="$(mktemp)"
  local sudo_users="$target_user"
  if [ -n "$LOGIN_USER" ] && [ "$LOGIN_USER" != "$target_user" ]; then
    sudo_users="$target_user, $LOGIN_USER"
  fi
  cat > "$tmp" <<EOF
# Installed by W4Studica install.sh: what 'gradlew deploy' runs through sudo for the VmxPi target.
$sudo_users ALL=(root) NOPASSWD: $systemctl_bin stop $SERVICE, $systemctl_bin start $SERVICE, $systemctl_bin restart $SERVICE, $systemctl_bin enable $SERVICE, $systemctl_bin stop robot, $systemctl_bin start robot, $systemctl_bin restart robot, $systemctl_bin enable robot, $ldconfig_bin, $sync_bin
# Full passwordless sudo for admin and robot runner accounts (root-equivalent privilege).
$ADMIN_USER ALL=(ALL) NOPASSWD: ALL
$ROBOT_USER ALL=(ALL) NOPASSWD: ALL
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
  service:   systemctl status robot (or $SERVICE)      logs: journalctl -u robot -f
  accounts:  admin = $ADMIN_USER, robot = $ROBOT_USER
  config:    $CONF_DIR/config.json   (robot_home = $ROBOT_HOME)
  RT kernel: $RT_KERNEL_STATUS
  RT policy: $CONF_DIR/config.json realtime (new/legacy default: auto, SCHED_RR, priority 10; existing policies preserved)
  next boot: zram 8 GiB + disk swap 4 GiB; volatile logs (64 MiB); ext4 protection; no OverlayFS
  boot check: systemd-analyze critical-chain $SERVICE.service
  boot policy: completed cloud-init retired; online wait skipped only for verified local-only setups (see above)
  Studica:   libraries supplied by GradleRIO deploy; local builds require --build-studica
  shim:      $OPT_DIR/lib/libvmx_gpio_isr_shim.so   -> LD_PRELOAD (only on kernels without /sys/class/gpio)
  lse emu:   $OPT_DIR/lib/libvmx_lse_emu.so   -> LD_AUDIT, set by robot_manager on CPUs without LSE atomics
  build:     C helpers / Rust manager (missing packages installed by apt; --no-install-deps disables this)
  cpu:       systemctl status robot_cpu_max      (governor performance, at every boot)
  network:   setupHotspot WorldSkills 1234 2.4 password; toggleNetMode ap; saved Wi-Fi profiles are deleted
  terminal:  http://10.TE.AM.2:7681 (ttyd web terminal)
  SystemCore: mrccomm.service (DS port 1110), hwmon.service, irqconf.service
WPILib/OpenCV shared libraries are deployed per project by GradleRIO; this installer checks them but does not select a global OpenCV version.
EOF
}

[ -n "$LOGIN_USER" ] || LOGIN_USER="${ADMIN_USER:-${SUDO_USER:-$(id -un)}}"
[ -n "$ADMIN_USER" ] || ADMIN_USER="$LOGIN_USER"
[ -n "$ROBOT_USER" ] || ROBOT_USER="lvuser"
[ -n "$ROBOT_HOME" ] || ROBOT_HOME="/home/$ROBOT_USER"
if [ "$PURGE_OLD_USER" = 1 ]; then
  prev_candidate="${SUDO_USER:-}"
  if [ -n "$prev_candidate" ] && [ "$prev_candidate" != "$ADMIN_USER" ] && [ "$prev_candidate" != "$ROBOT_USER" ] && [ "$prev_candidate" != "root" ]; then
    PURGE_OLD_USER="$prev_candidate"
  elif id ubuntu >/dev/null 2>&1 && [ "$ADMIN_USER" != "ubuntu" ] && [ "$ROBOT_USER" != "ubuntu" ]; then
    PURGE_OLD_USER="ubuntu"
  else
    PURGE_OLD_USER=""
  fi
fi

# A transient system service survives SSH disconnects, including a disconnect
# caused by installing NetworkManager. Invoke without sudo over SSH so this
# process can detect the SSH environment before privilege escalation.
if [ "$DRY_RUN" != 1 ] && [ "$CHECK_RUNTIME_DEPS" != 1 ] && [ "$RUNNING_DETACHED" != 1 ]; then
  if [ "$DETACH_INSTALL" = 1 ] || [ -n "${SSH_CONNECTION:-}${SSH_CLIENT:-}" ]; then
    command -v systemd-run >/dev/null 2>&1 || die "systemd-run is required to keep an SSH install running after disconnect"
    local_unit="w4studica-install-$(date +%Y%m%d-%H%M%S)-$$.service"
    working_directory="$(pwd -P)"
    run_as_root=()
    if [ "$(id -u)" != 0 ]; then
      command -v sudo >/dev/null 2>&1 || die "sudo is required to start the robot-managed install service"
      run_as_root=(sudo)
    fi
    info "Starting installation on the robot as $local_unit; it will continue if SSH disconnects"
    "${run_as_root[@]}" systemd-run --unit="$local_unit" --no-block \
      --property=Type=exec --property=StandardOutput=journal --property=StandardError=journal \
      --property="WorkingDirectory=$working_directory" \
      "$SELF_DIR/install.sh" --_run-detached "${ORIG_ARGS[@]}" \
      --admin-user "$ADMIN_USER" --robot-user "$ROBOT_USER" --robot-home "$ROBOT_HOME" \
      ${PURGE_OLD_USER:+--purge-user "$PURGE_OLD_USER"} \
      || die "could not start the robot-managed install service"
    echo "Follow progress: sudo journalctl -fu $local_unit"
    echo "Check result:    sudo systemctl status $local_unit --no-pager"
    exit 0
  fi

  if [ "$(id -u)" != 0 ]; then
    command -v sudo >/dev/null 2>&1 || die "must run as root (sudo not found)"
    exec sudo "$SELF_DIR/install.sh" "${ORIG_ARGS[@]}" \
      --admin-user "$ADMIN_USER" --robot-user "$ROBOT_USER" --robot-home "$ROBOT_HOME" \
      ${PURGE_OLD_USER:+--purge-user "$PURGE_OLD_USER"}
  fi
fi

if [ "$CHECK_RUNTIME_DEPS" = 1 ]; then
  check_deployed_wpilib_runtime
  exit 0
fi

acquire_install_lock

if [ "$UNDO_NETWORK" = 1 ]; then
  undo_network
  exit 0
fi

if [ "$CLEANUP_PACKAGES" = 1 ]; then
  cleanup_packages
  exit 0
fi

preflight
setup_accounts
ensure_base_runtime_dependencies
ensure_system_profile_dependencies
ensure_rt_kernel
ensure_vmx_hal_sdk
validate_install_inputs
ensure_build_dependencies
ensure_rust_build_dependencies
install_robot_manager
install_drivers
install_plugin
install_studica_notices
install_shim
install_lse_emu
install_cpu_max
install_system_profile
install_network
run "$OPT_DIR/robot_manager" storage finish-provisioning
run systemctl daemon-reload
install_sudoers
# A library problem in the deployed program is reported, but must not stop the service from being restarted.
( check_deployed_wpilib_runtime ) || warn "deployed libraries have unresolved dependencies (see above); robot_manager is restarted anyway"
restart_service
run sync
summary
if [ -n "$PURGE_OLD_USER" ] && [ "$PURGE_OLD_USER" != "$ADMIN_USER" ] && [ "$PURGE_OLD_USER" != "$ROBOT_USER" ] && [ "$PURGE_OLD_USER" != "root" ]; then
  info "Purging old user '$PURGE_OLD_USER' before reboot"
  run pkill -9 -u "$PURGE_OLD_USER" 2>/dev/null || true
  run userdel -r "$PURGE_OLD_USER" 2>/dev/null || true
fi
if [ "$DRY_RUN" = 1 ]; then
  echo "[dry-run] reboot"
elif [ "$REBOOT_ON_FINISH" = 1 ]; then
  info "Installation complete. Rebooting the robot..."
  run systemctl reboot 2>/dev/null || run reboot 2>/dev/null || true
fi
