#!/usr/bin/env bash
# Sample GPU clock, power, temperature and utilisation while something runs.
#
# Written because a frame rate alone cannot explain itself. When a configuration
# that forces the highest DPM state turns out to be slower than one that lets
# the firmware choose, the clocks are where the reason is: a forced state can
# sit *below* the opportunistic boost the automatic algorithm reaches, or push
# the card into its power limit and throttle.
#
# Usage: gpu-telemetry.sh <card> <output.csv> &   then kill the pid to stop.
set -uo pipefail
CARD=${1:?usage: gpu-telemetry.sh <card> <output.csv>}
OUT=${2:?usage: gpu-telemetry.sh <card> <output.csv>}
INTERVAL=${INTERVAL:-0.25}

D="/sys/class/drm/$CARD/device"
HW=$(echo "$D"/hwmon/hwmon* | awk '{print $1}')

read_or_blank() { cat "$1" 2>/dev/null | head -1 || true; }

echo "elapsed_s,sclk_hz,mclk_hz,power_uw,temp_mc,busy_pct,vram_used_bytes" > "$OUT"
START=$(date +%s.%N)
while :; do
    now=$(date +%s.%N)
    printf '%s,%s,%s,%s,%s,%s,%s\n' \
        "$(awk -v a="$now" -v b="$START" 'BEGIN{printf "%.2f", a-b}')" \
        "$(read_or_blank "$HW/freq1_input")" \
        "$(read_or_blank "$HW/freq2_input")" \
        "$(read_or_blank "$HW/power1_average")" \
        "$(read_or_blank "$HW/temp1_input")" \
        "$(read_or_blank "$D/gpu_busy_percent")" \
        "$(read_or_blank "$D/mem_info_vram_used")" \
        >> "$OUT"
    sleep "$INTERVAL"
done
