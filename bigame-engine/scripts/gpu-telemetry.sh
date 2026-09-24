#!/usr/bin/env bash
# Sample GPU clock, power, temperature and utilisation while something runs.
#
# Written because a frame rate alone cannot explain itself. When a configuration
# that forces the highest DPM state turns out to be slower than one that lets
# the firmware choose, the clocks are where the reason is: a forced state can
# sit *below* the opportunistic boost the automatic algorithm reaches, or push
# the card into its power limit and throttle.
#
# ## Why this spawns no processes
#
# The first version of this sampler called `cat` once per sensor plus `date` and
# `awk` per sample -- eight processes, four times a second, about 1400 forks
# over a single benchmark run. That is not free. Measured against a session run
# without it, it raised the run-to-run spread of the workload from 1.4% to
# around 9%, which is larger than most differences a benchmark is trying to
# detect. An instrument that perturbs what it measures by more than the effect
# size is not an instrument.
#
# Everything below is therefore a bash builtin: `read` for the sensors,
# `EPOCHREALTIME` for the clock, arithmetic in the shell, and a single
# redirection held open for the whole run rather than reopened per line.
#
# Usage: gpu-telemetry.sh <card> <output.csv> &   then kill the pid to stop.
set -uo pipefail
CARD=${1:?usage: gpu-telemetry.sh <card> <output.csv>}
OUT=${2:?usage: gpu-telemetry.sh <card> <output.csv>}
INTERVAL=${INTERVAL:-0.5}

D="/sys/class/drm/$CARD/device"
for h in "$D"/hwmon/hwmon*; do HW=$h; break; done
: "${HW:=/nonexistent}"

# Read one sysfs value without forking. An unreadable sensor yields the empty
# string, which the CSV carries as an empty field -- "not available" recorded
# rather than a zero that would be mistaken for a measurement.
peek() {
    local __var=$1 __file=$2 __line=''
    read -r __line < "$__file" 2>/dev/null
    printf -v "$__var" '%s' "$__line"
}

# A fork-free timer. `sleep` is not a bash builtin, so calling it once per
# sample would put back a third of the process churn this rewrite removed;
# a read with a timeout on an empty pipe waits just as accurately and forks
# once, at setup, instead of twice a second forever.
exec {timer}<> <(:) 2>/dev/null || timer=""
nap() {
    if [ -n "$timer" ]; then
        read -r -t "$INTERVAL" -u "$timer" _ 2>/dev/null
    else
        sleep "$INTERVAL"
    fi
    return 0
}

# CPU utilisation has to be differenced between samples: /proc/stat reports
# cumulative jiffies since boot, so a single reading says what the machine has
# been doing since it started, not what it is doing now.
prev_busy=0 prev_total=0
cpu_util() {
    local _c user nice sys idle iowait irq softirq steal
    read -r _c user nice sys idle iowait irq softirq steal _ < /proc/stat
    local total=$(( user + nice + sys + idle + iowait + irq + softirq + steal ))
    local busy=$(( total - idle - iowait ))
    local d_total=$(( total - prev_total )) d_busy=$(( busy - prev_busy ))
    prev_total=$total prev_busy=$busy
    if [ "$d_total" -gt 0 ]; then
        printf -v cpu_pct '%d' $(( d_busy * 100 / d_total ))
    else
        cpu_pct=''
    fi
}

START=${EPOCHREALTIME/,/.}
cpu_util   # prime the difference; the first reading is otherwise since boot
exec 3>"$OUT"
printf 'elapsed_s,sclk_hz,mclk_hz,power_uw,temp_mc,busy_pct,vram_used_bytes,cpu_pct,cpu_khz\n' >&3

while :; do
    now=${EPOCHREALTIME/,/.}
    peek sclk  "$HW/freq1_input"
    peek mclk  "$HW/freq2_input"
    peek power "$HW/power1_average"
    peek temp  "$HW/temp1_input"
    peek busy  "$D/gpu_busy_percent"
    peek vram  "$D/mem_info_vram_used"
    peek cpu_khz "/sys/devices/system/cpu/cpu0/cpufreq/scaling_cur_freq"
    cpu_util
    # Elapsed time in shell arithmetic: microseconds, so no floating point and
    # no call out to awk.
    elapsed=$(( ${now/./} - ${START/./} ))
    printf '%d.%06d,%s,%s,%s,%s,%s,%s,%s,%s\n' \
        $(( elapsed / 1000000 )) $(( elapsed % 1000000 )) \
        "$sclk" "$mclk" "$power" "$temp" "$busy" "$vram" "$cpu_pct" "$cpu_khz" >&3
    nap
done
