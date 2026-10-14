#!/usr/bin/env bash
# install.sh - install the W4Studica VMX software. Run it ON the Studica VMX-pi.
#
#     ./install.sh [options]        (run as the login user; sudo is requested when needed)
#
# It lives in the W4Studica/Robot-Manager repo. Clone that repo and the W4Studica/allwpilib
# repo side by side on the robot (only allwpilib's studica_drivers/ and
# simulation/halsim_vmx/src/ are used):
#     ~/Robot-Manager/install.sh
#     ~/allwpilib/
# or point at them with --robot-manager-dir / --allwpilib-dir.
#
# It never opens an SSH connection or stores passwords. An SSH-launched install is handed
# to systemd on the robot before privilege escalation can lose the SSH session context.
#
# What it does (required build dependencies are installed when missing; optional steps have explicit --skip flags):
#   1. robot_manager      /opt/robot_manager, /etc/robot_manager/config.json (only if missing),
#                         systemd service robot_manager (enabled and restarted)
#   2. studica_drivers    built with Studica's Makefile, copied into /usr/local (needs g++, make, VMXPi.h)
#   3. Studica plugin     /opt/halsim_vmx/libhalsim_vmx_studica.so for halsim_vmx (needs g++ >= 11, VMXPi.h)
#   4. GPIO interrupt shim /opt/robot_manager/lib/libvmx_gpio_isr_shim.so (needs gcc and make). Fixes
#                         "RPI GPIO Interrupt Enable: PI_BAD_ISR_INIT" on kernels without GPIO sysfs
#   5. LSE atomics emulation /opt/robot_manager/lib/libvmx_lse_emu.so (needs gcc and make). Lets WPILib's libMrcLib
#                         (SystemCore build, uses ARMv8.1 atomics) run on a Pi 4; robot_manager turns it on by itself
#   6. CPU at full speed /opt/robot_manager/bin/robot_cpu_max.sh + robot_cpu_max.service: governor "performance" and the
#                         minimum frequency raised to the maximum, at every boot (a 20 ms robot loop should not wait for the clock)
#   7. network commands   setupHotspot / toggleNetMode / setupEthernet (like the Studica VMX image), always installed;
#                         --network also applies them: Wi-Fi access point VMX-<team> at 10.XX.YY.2 and the wired port at
#                         172.22.11.2 (needs NetworkManager; refuses when it would cut the ssh session you are using)
#   8. sudoers (opt-in)   lets the login user run the few sudo commands `gradlew deploy` uses, without a password
#
# WPILib/OpenCV native libraries are deployed per robot project by GradleRIO from that project's
# pinned WPILib dependency graph; this installer does not install a separate system OpenCV version.

set -euo pipefail

SELF_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

OPT_DIR=/opt/robot_manager
CONF_DIR=/etc/robot_manager
PLUGIN_DIR=/opt/halsim_vmx
BUILD_DIR=/opt/robot_manager/build
SERVICE=robot_manager

DRY_RUN=0
DETACH_INSTALL=0
RUNNING_DETACHED=0
INSTALL_DEPS=1
WITH_SUDOERS=0
SKIP_DRIVERS=0
UPGRADE_DRIVERS=0
SKIP_PLUGIN=0
SKIP_SHIM=0
SKIP_LSE_EMU=0
SKIP_CPU_MAX=0
APPLY_NETWORK=0
FORCE_NETWORK=0
UNDO_NETWORK=0
CHECK_RUNTIME_DEPS=0
AP_SSID=VMX
AP_TEAM=1234
AP_BAND=2.4
AP_PASSWORD=password
ROBOT_HOME=""
LOGIN_USER=""
# Defaults: this repo is where install.sh is; allwpilib is a sibling checkout (or inside this directory).
if [ -f "$SELF_DIR/robot_manager.py" ]; then RM_DIR="$SELF_DIR"; else RM_DIR="$SELF_DIR/robot_manager"; fi
if [ -d "$SELF_DIR/../allwpilib" ]; then AW_DIR="$SELF_DIR/../allwpilib"; else AW_DIR="$SELF_DIR/allwpilib"; fi

usage() {
  cat <<'EOF'
Usage: ./install.sh [options]      (run on the robot; over SSH it continues as a systemd job)

Options:
  --user NAME              login user that deploys to this robot (default: invoking user or SUDO_USER)
  --robot-home DIR         robot_home for robot_manager (default: /home/<user>)
  --robot-manager-dir DIR  Robot-Manager checkout (default: the directory of install.sh)
  --allwpilib-dir DIR      allwpilib checkout (default: ../allwpilib next to this repo)
  --sudoers                allow the login user passwordless sudo for: systemctl (robot_manager), ldconfig, sync
  --detach                 run installation as a robot-managed systemd job, even outside SSH
  --no-install-deps        do not install missing apt packages; fail with the package list instead
  --skip-drivers           do not build/install studica_drivers
  --upgrade-drivers        replace an already installed studica_drivers (e.g. from the vmx-hal package) with this repo's copy
  --skip-plugin            do not build the Studica backend plugin
  --skip-shim              do not build the GPIO interrupt shim (libvmx_gpio_isr_shim.so)
  --skip-lse-emu           do not build the LSE atomics emulation (libvmx_lse_emu.so)
  --skip-cpu-max           do not pin the CPU at its highest frequency (robot_cpu_max.service)
  --network                also set up the network like the Studica VMX image: the Wi-Fi becomes an access point
                           (VMX-<team>, 10.XX.YY.2) and the wired port is fixed at 172.22.11.2. Without it only the
                           commands (setupHotspot, toggleNetMode, setupEthernet) are installed and nothing is changed.
  --team N                 team number for --network (default 1234 -> SSID VMX-1234, 10.12.34.2)
  --ap-ssid PREFIX         SSID prefix for --network (default VMX)
  --ap-band 2.4|5          Wi-Fi band for --network (default 2.4, channel 6)
  --ap-password PASSWORD   Wi-Fi password for --network (default: the VMX image default; visible in the process list while
                           install.sh runs, so use setupHotspot afterwards for a private one)
  --force-network          apply --network even though this ssh session will end (it comes in over the Wi-Fi or wired port
                           that is changed); reconnect through the wired port (172.22.11.2) or the robot's Wi-Fi
  --undo-network           put the network back (restores the netplan files that --network set aside) and exit
  --check-runtime-deps     inspect the deployed robotCommand and GradleRIO .so files with ldd, then exit
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
    --detach) DETACH_INSTALL=1 ;;
    --_run-detached) RUNNING_DETACHED=1 ;;
    --no-install-deps) INSTALL_DEPS=0 ;;
    --skip-drivers) SKIP_DRIVERS=1 ;;
    --upgrade-drivers) UPGRADE_DRIVERS=1 ;;
    --skip-plugin) SKIP_PLUGIN=1 ;;
    --skip-shim) SKIP_SHIM=1 ;;
    --skip-lse-emu) SKIP_LSE_EMU=1 ;;
    --skip-cpu-max) SKIP_CPU_MAX=1 ;;
    --network) APPLY_NETWORK=1 ;;
    --force-network) FORCE_NETWORK=1 ;;
    --undo-network) UNDO_NETWORK=1 ;;
    --check-runtime-deps) CHECK_RUNTIME_DEPS=1 ;;
    --ap-ssid) [ $# -ge 2 ] || die "--ap-ssid needs a value"; AP_SSID="$2"; shift ;;
    --team) [ $# -ge 2 ] || die "--team needs a value"; AP_TEAM="$2"; shift ;;
    --ap-band) [ $# -ge 2 ] || die "--ap-band needs a value"; AP_BAND="$2"; shift ;;
    --ap-password) [ $# -ge 2 ] || die "--ap-password needs a value"; AP_PASSWORD="$2"; shift ;;
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
  run env DEBIAN_FRONTEND=noninteractive apt-get update \
    || die "apt-get update failed; could not install $reason"
  run env DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends "$@" \
    || die "apt could not install $reason ($*)"
}

ensure_base_runtime_dependencies() {
  local -a packages=()
  command -v python3 >/dev/null 2>&1 || packages+=(python3)
  if ! command -v ldd >/dev/null 2>&1 || ! command -v ldconfig >/dev/null 2>&1; then
    packages+=(libc-bin)
  fi
  if [ "${#packages[@]}" -gt 0 ]; then
    apt_install_packages "robot_manager and native-library runtime tools" "${packages[@]}"
  fi
  if [ "$DRY_RUN" != 1 ]; then
    command -v python3 >/dev/null 2>&1 || die "python3 is required (robot_manager is a Python script)"
    if ! command -v ldd >/dev/null 2>&1 || ! command -v ldconfig >/dev/null 2>&1; then
      die "ldd and ldconfig are required to validate native runtime dependencies"
    fi
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
  local need_build=0 need_modern_cpp=0 cpp_standard=""
  if [ "$SKIP_PLUGIN" != 1 ] || [ "$SKIP_SHIM" != 1 ] || [ "$SKIP_LSE_EMU" != 1 ]; then
    need_build=1
  fi
  if [ "$SKIP_DRIVERS" != 1 ] && { [ "$UPGRADE_DRIVERS" = 1 ] || [ ! -f /usr/local/lib/studica_drivers/libstudica_drivers.so ]; }; then
    need_build=1
  fi
  [ "$need_build" = 1 ] || return 0
  [ "$SKIP_PLUGIN" = 1 ] || cpp_standard=c++20

  if ! c_compiler_works gcc || ! cpp_compiler_works g++ "$cpp_standard" || ! command -v make >/dev/null 2>&1; then
    apt_install_packages "the C/C++ build tools" build-essential
  fi

  local major=""
  if command -v g++ >/dev/null 2>&1; then major="$(compiler_major g++)"; fi
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
    if [ -n "$cpp_standard" ]; then
      cpp_compiler_works "$BUILD_CXX" "$cpp_standard" \
        || die "$BUILD_CXX cannot compile and link a C++20 program; install build-essential and rerun"
    else
      cpp_compiler_works "$BUILD_CXX" \
        || die "$BUILD_CXX cannot compile and link a C++ program; install build-essential and rerun"
    fi
    command -v make >/dev/null 2>&1 || die "make is required to build Studica components"
    if [ "$SKIP_PLUGIN" != 1 ]; then
      major="$(compiler_major "$BUILD_CXX")"
      [ -n "$major" ] && [ "$major" -ge 11 ] \
        || die "the Studica backend requires GCC/G++ 11 or newer for C++20; found '${major:-unknown}'"
    fi
  elif [ "$need_modern_cpp" = 0 ]; then
    info "C/C++ build tools: $(tool_version "$BUILD_CXX" --version)"
  fi
  if [ "$DRY_RUN" != 1 ]; then
    echo "  build compilers: $BUILD_CC / $BUILD_CXX; make: $(tool_version make --version)"
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
    [ -f "$RM_DIR/robot_manager.py" ] && [ -f "$RM_DIR/robot_manager.service" ] \
      || warn "dry run: Robot-Manager sources are not present at $RM_DIR"
    have_vmx || warn "dry run: VMX headers/libraries are not on this host; required on the robot for Studica builds"
    [ -f "$AW_DIR/simulation/halsim_vmx/src/studica/native/cpp/StudicaBackend.cpp" ] \
      || warn "dry run: halsim_vmx source is not present under $AW_DIR"
    return 0
  fi
  [ -f "$RM_DIR/robot_manager.py" ] && [ -f "$RM_DIR/robot_manager.service" ] \
    || die "robot_manager.py / robot_manager.service not found in $RM_DIR (use --robot-manager-dir)"

  if [ "$SKIP_PLUGIN" != 1 ]; then
    have_vmx || die "VMX SDK missing: expected /usr/local/include/vmxpi/VMXPi.h and /usr/local/lib/vmxpi/libvmxpi_hal_cpp.so. Install the vmxpi-hal SDK (for example, the package built by vmxpi-aarch64-hal-builder), then rerun or pass --skip-plugin."
    local hv="$AW_DIR/simulation/halsim_vmx/src"
    [ -f "$hv/studica/native/cpp/StudicaBackend.cpp" ] \
      || die "halsim_vmx plugin source not found under $AW_DIR (use --allwpilib-dir or pass --skip-plugin)"
    [ -d "$AW_DIR/studica_drivers" ] \
      || die "studica_drivers source not found under $AW_DIR (use --allwpilib-dir or pass --skip-plugin)"
  fi

  if [ "$SKIP_DRIVERS" != 1 ] && { [ "$UPGRADE_DRIVERS" = 1 ] || [ ! -f /usr/local/lib/studica_drivers/libstudica_drivers.so ]; }; then
    have_vmx || die "VMX SDK missing: expected /usr/local/include/vmxpi/VMXPi.h and /usr/local/lib/vmxpi/libvmxpi_hal_cpp.so. Install the vmxpi-hal SDK (for example, the package built by vmxpi-aarch64-hal-builder), or pass --skip-drivers."
    [ -d "$AW_DIR/studica_drivers" ] \
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
  [ -f "$RM_DIR/net/vmx_net.py" ] || die "network command source is missing from $RM_DIR"
  if [ "$WITH_SUDOERS" = 1 ]; then
    command -v visudo >/dev/null 2>&1 || die "visudo is required to validate --sudoers rules"
  fi
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
  local count=0 executable result library
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
  if [ -d "$lib_dir" ]; then
    mapfile -d '' -t libraries < <(find "$lib_dir" -type f \( -name '*.so' -o -name '*.so.*' \) ! -name '*.debug' -print0)
    for library in "${libraries[@]}"; do
      verify_shared_library "$library" "$search_path"
      count=$((count + 1))
    done
  fi
  if [ -f "$command_file" ]; then
    executable="$(python3 - "$command_file" "$ROBOT_HOME" <<'PY'
import os
import re
import shlex
import sys

with open(sys.argv[1], encoding="utf-8") as command_file:
    words = shlex.split(command_file.read())
index = 0
while index < len(words) and re.match(r"^[A-Za-z_][A-Za-z0-9_]*=", words[index]):
    index += 1
if index < len(words) and words[index] == "gdbserver":
    index += 2  # gdbserver and its :port argument
if index >= len(words):
    raise SystemExit("robotCommand does not contain a program path")
program = words[index]
if not os.path.isabs(program):
    program = os.path.join(sys.argv[2], program)
print(program)
PY
)" || die "could not read the executable from $command_file"
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

install_robot_manager() {
  info "robot_manager"
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
  have_vmx || { [ "$DRY_RUN" = 1 ] && { warn "dry run: VMX SDK is absent on this host; skipping the local driver build preview"; return; }; die "VMX SDK not found under /usr/local; cannot build studica_drivers"; }
  # The vmx-hal package already ships (an older) libstudica_drivers. Do not overwrite files a package owns
  # unless asked. The halsim_vmx plugin compiles the driver sources itself, so it does not need this library.
  if [ -f /usr/local/lib/studica_drivers/libstudica_drivers.so ] && [ "$UPGRADE_DRIVERS" != 1 ]; then
    echo "  already installed (probably by the vmx-hal package); not replacing it."
    echo "  pass --upgrade-drivers to install this repo's newer copy over it (replaces package-owned files)."
    return
  fi
  # Build in a copy so the checkout is not littered with build output (or root-owned files).
  run rm -rf "$BUILD_DIR/studica_drivers"
  run install -d "$BUILD_DIR"
  run cp -r "$AW_DIR/studica_drivers" "$BUILD_DIR/studica_drivers"
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
  info "Studica backend plugin for halsim_vmx"
  if [ "$SKIP_PLUGIN" = 1 ]; then echo "  skipped (--skip-plugin)"; return; fi
  have_vmx || { [ "$DRY_RUN" = 1 ] && { warn "dry run: VMX SDK is absent on this host; skipping the local plugin build preview"; return; }; die "VMX SDK not found under /usr/local; cannot build the Studica backend plugin"; }
  local major; major="$(gpp_major)"
  if [ "$DRY_RUN" != 1 ]; then
    [ -n "$major" ] && [ "$major" -ge 11 ] || die "$BUILD_CXX >= 11 (C++20) is required, found '${major:-none}'"
  else
    info "Would build the plugin with $BUILD_CXX (GCC/G++ 11 or newer is required)"
  fi

  local drv="$AW_DIR/studica_drivers"
  local hv="$AW_DIR/simulation/halsim_vmx/src"
  run install -d -m 0755 "$PLUGIN_DIR"
  run "$BUILD_CXX" -std=c++20 -shared -fPIC -O2 \
    -I/usr/local/include/vmxpi -I"$drv" \
    -I"$hv/main/native/include" -I"$hv/studica/native/include" \
    "$hv/studica/native/cpp/StudicaBackend.cpp" \
    "$drv/analog_input.cpp" "$drv/dio.cpp" "$drv/encoder.cpp" "$drv/imu.cpp" \
    -L/usr/local/lib/vmxpi -lvmxpi_hal_cpp -lrt -lpthread -latomic \
    -o "$PLUGIN_DIR/libhalsim_vmx_studica.so"
  if [ "$DRY_RUN" != 1 ]; then
    run ldconfig
    verify_shared_library "$PLUGIN_DIR/libhalsim_vmx_studica.so"
  fi
}

install_shim() {
  info "GPIO interrupt shim"
  if [ "$SKIP_SHIM" = 1 ]; then echo "  skipped (--skip-shim)"; return; fi
  local src="$RM_DIR/gpio_isr_shim"
  run rm -rf "$BUILD_DIR/gpio_isr_shim"
  run install -d "$BUILD_DIR" "$OPT_DIR/lib"
  run cp -r "$src" "$BUILD_DIR/gpio_isr_shim"
  run make -C "$BUILD_DIR/gpio_isr_shim" CC="$BUILD_CC" test
  run make -C "$BUILD_DIR/gpio_isr_shim" CC="$BUILD_CC"
  run install -m 0755 "$BUILD_DIR/gpio_isr_shim/libvmx_gpio_isr_shim.so" "$OPT_DIR/lib/libvmx_gpio_isr_shim.so"
  if [ "$DRY_RUN" != 1 ]; then verify_shared_library "$OPT_DIR/lib/libvmx_gpio_isr_shim.so"; fi
  if [ ! -d /sys/class/gpio ]; then
    echo "  this kernel has no /sys/class/gpio, so the HAL needs the shim. Add to $CONF_DIR/config.json:"
    echo "    \"env\": { \"LD_PRELOAD\": \"$OPT_DIR/lib/libvmx_gpio_isr_shim.so\" }"
  fi
}

install_lse_emu() {
  info "LSE atomics emulation"
  if [ "$SKIP_LSE_EMU" = 1 ]; then echo "  skipped (--skip-lse-emu)"; return; fi
  local src="$RM_DIR/lse_emu"
  run rm -rf "$BUILD_DIR/lse_emu"
  run install -d "$BUILD_DIR" "$OPT_DIR/lib"
  run cp -r "$src" "$BUILD_DIR/lse_emu"
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

ssh_device() { python3 "$OPT_DIR/net/vmx_net.py" sshdevice 2>/dev/null || true; }

# Makes NetworkManager the network manager (Ubuntu Server ships systemd-networkd + netplan) and sets up the Wi-Fi access
# point and the wired port. Everything is written first, while the ssh session still works; the switch itself runs a few
# seconds after install.sh has finished, because it ends the session.
apply_network() {
  local dev; dev="$(ssh_device)"
  if [ -n "$dev" ] && [ "$FORCE_NETWORK" != 1 ]; then
    die "this ssh session comes in over $dev, which --network changes: it will end. Reconnect through the wired port (172.22.11.2) or the robot's Wi-Fi '$AP_SSID-$AP_TEAM' (the access point), then pass --force-network. (A screen and keyboard are the safest way; --undo-network goes back.)"
  fi

  local missing=0
  command -v nmcli >/dev/null 2>&1 || missing=1
  command -v iw >/dev/null 2>&1 || missing=1
  [ -x /usr/sbin/dnsmasq ] || command -v dnsmasq >/dev/null 2>&1 || missing=1   # dnsmasq-base: NetworkManager's DHCP server
  if [ "$missing" = 1 ]; then
    apt_install_packages "network configuration tools (internet access is required)" network-manager dnsmasq-base iw
    if [ "$DRY_RUN" != 1 ]; then
      if ! command -v nmcli >/dev/null 2>&1 || ! command -v iw >/dev/null 2>&1; then
        die "network tools are still unavailable; the network was not changed"
      fi
      [ -x /usr/sbin/dnsmasq ] || command -v dnsmasq >/dev/null 2>&1 \
        || die "dnsmasq is still unavailable; the network was not changed"
    fi
  fi

  # The connection files: written for NetworkManager, which has not started yet.
  run install -d -m 0755 /etc/NetworkManager/system-connections
  run "$OPT_DIR/net/vmx_net.py" setupEthernet --no-activate
  run "$OPT_DIR/net/vmx_net.py" setupHotspot --no-activate "$AP_SSID" "$AP_TEAM" "$AP_BAND" "$AP_PASSWORD"

  # netplan: hand every interface to NetworkManager. The old files are set aside, not deleted (--undo-network).
  local backup; backup="/etc/netplan/w4studica-backup-$(date +%Y%m%d-%H%M%S)"
  if ls /etc/netplan/*.yaml >/dev/null 2>&1; then
    run install -d -m 0700 "$backup"
    run sh -c "mv /etc/netplan/*.yaml '$backup'/"
  fi
  write_file /etc/netplan/01-w4studica-network-manager.yaml 0600 <<'YAML'
# Installed by W4Studica install.sh --network: NetworkManager manages the interfaces (Wi-Fi access point, wired 172.22.11.2).
network:
  version: 2
  renderer: NetworkManager
YAML
  # cloud-init would write its own netplan file again at the next boot.
  run install -d /etc/cloud/cloud.cfg.d
  write_file /etc/cloud/cloud.cfg.d/99-w4studica-disable-network-config.cfg 0644 <<'CFG'
# Installed by W4Studica install.sh --network: the network is configured through NetworkManager, not cloud-init.
network: {config: disabled}
CFG
  run install -m 0755 "$RM_DIR/system/undo_network.sh" "$OPT_DIR/bin/undo_network.sh"

  local address; address="$(python3 -c "import sys; t=int(sys.argv[1]); print('10.%d.%d.2' % (t//100, t%100))" "$AP_TEAM")"
  local pw="the one you gave"; [ "$AP_PASSWORD" = password ] && pw="password"
  cat <<MSG
  Network prepared. In 8 seconds the robot switches to NetworkManager:
    wired port  172.22.11.2   (a computer on the cable gets 172.22.11.X from the robot)
    Wi-Fi       access point '$AP_SSID-$AP_TEAM'  ->  $address   (the home Wi-Fi is not used any more)
  THIS SSH SESSION ENDS. Reconnect:
    - cable:  set the PC to DHCP, then   ssh $LOGIN_USER@172.22.11.2
    - Wi-Fi:  join '$AP_SSID-$AP_TEAM' (password: $pw), then   ssh $LOGIN_USER@$address
  Go back (on the robot's screen and keyboard):   sudo $OPT_DIR/bin/undo_network.sh
MSG
  run systemd-run --unit=w4studica-network-switch --on-active=8 --description="W4Studica: switch to NetworkManager" \
    /bin/sh -c 'netplan generate && systemctl disable --now systemd-networkd.service systemd-networkd.socket systemd-networkd-wait-online.service; systemctl enable --now NetworkManager.service'
}

undo_network() {
  info "Undo --network"
  [ -f "$RM_DIR/system/undo_network.sh" ] || die "$RM_DIR/system/undo_network.sh not found"
  run sh "$RM_DIR/system/undo_network.sh"
}

install_network() {
  info "Network (Wi-Fi access point and fixed wired address)"
  local src="$RM_DIR/net/vmx_net.py"
  [ -f "$src" ] || { warn "$src not found; skipping the network commands"; return; }
  run install -d "$OPT_DIR/net" "$OPT_DIR/bin"
  run install -m 0755 "$src" "$OPT_DIR/net/vmx_net.py"
  local cmd
  for cmd in setupHotspot toggleNetMode setupEthernet; do
    run ln -sf "$OPT_DIR/net/vmx_net.py" "/usr/local/bin/$cmd"
  done
  echo "  installed: setupHotspot, toggleNetMode, setupEthernet"

  if [ "$APPLY_NETWORK" != 1 ]; then
    echo "  network left as it is (add --network to apply it, see --help)"
    return
  fi
  apply_network
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
  build:     $BUILD_CC / $BUILD_CXX and make (missing packages installed by apt; --no-install-deps disables this)
  cpu:       systemctl status robot_cpu_max      (governor performance, at every boot)
  network:   setupHotspot SSID TEAM BAND [PASSWORD], toggleNetMode ap|wifi, setupEthernet   (add --network to apply)
  runtime:   ./install.sh --check-runtime-deps --user $LOGIN_USER (after GradleRIO deploy)
WPILib/OpenCV shared libraries are deployed per project by GradleRIO; this installer checks them but does not select a global OpenCV version.
Next: allwpilib/simulation/halsim_vmx/HARDWARE_CHECKLIST.md.
EOF
}

[ -n "$LOGIN_USER" ] || LOGIN_USER="${SUDO_USER:-$(id -un)}"
[ -n "$ROBOT_HOME" ] || ROBOT_HOME="/home/$LOGIN_USER"

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
      --user "$LOGIN_USER" --robot-home "$ROBOT_HOME" \
      || die "could not start the robot-managed install service"
    echo "Follow progress: sudo journalctl -fu $local_unit"
    echo "Check result:    sudo systemctl status $local_unit --no-pager"
    exit 0
  fi

  if [ "$(id -u)" != 0 ]; then
    command -v sudo >/dev/null 2>&1 || die "must run as root (sudo not found)"
    exec sudo "$SELF_DIR/install.sh" "${ORIG_ARGS[@]}" --user "$LOGIN_USER" --robot-home "$ROBOT_HOME"
  fi
fi

if [ "$CHECK_RUNTIME_DEPS" = 1 ]; then
  check_deployed_wpilib_runtime
  exit 0
fi

if [ "$UNDO_NETWORK" = 1 ]; then
  undo_network
  exit 0
fi

preflight
ensure_base_runtime_dependencies
validate_install_inputs
ensure_build_dependencies
install_robot_manager
install_drivers
install_plugin
install_shim
install_lse_emu
install_cpu_max
install_network
install_sudoers
check_deployed_wpilib_runtime
restart_service
summary
