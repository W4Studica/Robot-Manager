#!/bin/sh
# Runs the CPU at its highest frequency: governor "performance" and the minimum frequency raised to the maximum.
#
# A robot program has to react on time, not save power. The default governor scales the clock down when it looks idle and
# back up when load arrives; that delay shows up as jitter in a 20 ms loop.
#
# ROBOT_CPU_SYSFS (default /sys/devices/system/cpu) points at another tree, for the tests.
# Always exits 0: a board without cpufreq must not fail the boot.

CPU_DIR="${ROBOT_CPU_SYSFS:-/sys/devices/system/cpu}"
found=0

for dir in "$CPU_DIR"/cpu[0-9]*/cpufreq; do
  [ -d "$dir" ] || continue
  found=1
  cpu="$(basename "$(dirname "$dir")")"

  if grep -qw performance "$dir/scaling_available_governors" 2>/dev/null; then
    echo performance > "$dir/scaling_governor" 2>/dev/null || echo "$cpu: cannot set the governor" >&2
  else
    echo "$cpu: no 'performance' governor ($(cat "$dir/scaling_available_governors" 2>/dev/null))" >&2
  fi

  # The minimum may not exceed the current maximum, so use that.
  max="$(cat "$dir/scaling_max_freq" 2>/dev/null)"
  if [ -n "$max" ]; then
    echo "$max" > "$dir/scaling_min_freq" 2>/dev/null || echo "$cpu: cannot raise the minimum frequency" >&2
  fi

  echo "$cpu: governor $(cat "$dir/scaling_governor" 2>/dev/null), min $(cat "$dir/scaling_min_freq" 2>/dev/null) max $(cat "$dir/scaling_max_freq" 2>/dev/null) kHz"
done

[ "$found" = 1 ] || echo "no cpufreq in $CPU_DIR: nothing to set" >&2
exit 0
