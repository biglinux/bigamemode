#!/usr/bin/env bash
# Alternating A/B benchmark harness.
#
# Runs one workload repeatedly, alternating between two machine configurations,
# and writes the result into the standard layout. Three decisions in here are
# not incidental:
#
#   Runs alternate (A B A B A B) rather than grouping (A A A B B B). Grouped
#   runs confound the configuration with anything that drifts over the session
#   -- chassis temperature above all -- and on a warm machine that drift is
#   larger than most of the differences worth detecting.
#
#   The first run of each arm is discarded. A cold shader cache and a cold GPU
#   make the first run unlike every run that follows it.
#
#   The machine is driven through the product's own D-Bus daemon, not through
#   sysfs directly, so what is measured is the Booster as shipped rather than a
#   shell approximation of it.
#
# The original machine state is captured before anything changes and restored
# on exit, including on interrupt.
set -uo pipefail

BUS=(busctl --system call com.biglinux.BiGameMode /com/biglinux/BiGameMode com.biglinux.BiGameMode)
RUNS=${RUNS:-3}
OUT_ROOT=${OUT_ROOT:-benchmarks}

log() { LC_ALL=C printf '%s  %s\n' "$(date +%H:%M:%S)" "$*" >&2; }
die() { log "ERROR: $*"; exit 1; }

# ── machine state ────────────────────────────────────────────────────────────

# The discrete GPU, or the only one. Picked by asking which card is not the
# integrated part of the CPU package, never by hardcoding a card number.
render_card() {
    local best="" card driver
    for card in /sys/class/drm/card[0-9]*; do
        [ -e "$card/device/power_dpm_force_performance_level" ] || continue
        driver=$(basename "$(readlink -f "$card/device/driver" 2>/dev/null)" 2>/dev/null)
        [ "$driver" = amdgpu ] || [ "$driver" = nvidia ] || [ "$driver" = i915 ] || \
            [ "$driver" = xe ] || continue
        # An integrated GPU sits on the CPU's own root complex; a discrete one
        # sits behind a bridge. Prefer whichever has dedicated VRAM.
        if [ -r "$card/device/mem_info_vram_total" ]; then
            local vram; vram=$(cat "$card/device/mem_info_vram_total" 2>/dev/null || echo 0)
            if [ -z "$best" ] || [ "$vram" -gt "${best_vram:-0}" ]; then
                best=$(basename "$card"); best_vram=$vram
            fi
        elif [ -z "$best" ]; then
            best=$(basename "$card")
        fi
    done
    echo "$best"
}

CARD=$(render_card)
[ -n "$CARD" ] || die "no GPU with a DPM control was found"

read_state() {
    printf 'governor=%s\n' "$(cat /sys/devices/system/cpu/cpu0/cpufreq/scaling_governor 2>/dev/null || echo '')"
    printf 'epp=%s\n' "$(cat /sys/devices/system/cpu/cpu0/cpufreq/energy_performance_preference 2>/dev/null || echo '')"
    printf 'dpm=%s\n' "$(cat "/sys/class/drm/$CARD/device/power_dpm_force_performance_level" 2>/dev/null || echo '')"
    printf 'profile=%s\n' "$(powerprofilesctl get 2>/dev/null || echo '')"
}

ORIGINAL=$(read_state)

set_governor() { [ -n "$1" ] && "${BUS[@]}" SetCpuGovernor s "$1" >/dev/null 2>&1; }
set_epp()      { [ -n "$1" ] && "${BUS[@]}" SetCpuEpp s "$1" >/dev/null 2>&1; }
set_dpm()      { [ -n "$1" ] && "${BUS[@]}" SetGpuDpmLevel ss "$CARD" "$1" >/dev/null 2>&1; }
set_profile()  { [ -n "$1" ] && powerprofilesctl set "$1" >/dev/null 2>&1; }

restore() {
    log "restoring the machine to how it was found"
    eval "$ORIGINAL"
    set_governor "$governor"; set_epp "$epp"; set_dpm "$dpm"; set_profile "$profile"
    read_state | sed 's/^/  /' >&2
}
trap restore EXIT INT TERM

# ── arms ─────────────────────────────────────────────────────────────────────
#
# Each arm is a function so that the isolation matrix can add arms without the
# harness knowing what they do.

arm_baseline() {
    set_profile balanced
    set_governor powersave
    set_epp balance_performance
    set_dpm auto
}

arm_booster() {
    set_profile performance
    set_governor performance
    set_epp performance
    set_dpm high
}

arm_governor_only() { arm_baseline; set_governor performance; set_epp performance; }
arm_gpu_only()      { arm_baseline; set_dpm high; }

# ── workload ─────────────────────────────────────────────────────────────────

STK_ROOT=${STK_ROOT:-}
if [ -z "$STK_ROOT" ]; then
    STK_ROOT=$(find "$HOME" -maxdepth 6 -type f -name supertuxkart -path '*/bin/*' \
                 -printf '%h\n' 2>/dev/null | head -1)
    [ -n "$STK_ROOT" ] && STK_ROOT=$(dirname "$STK_ROOT")
fi
STK_CONFIG="${XDG_CONFIG_HOME:-$HOME/.config}/supertuxkart/config-0.10"

run_workload() {
    local dest=$1
    mkdir -p "$dest"
    # Telemetry runs for the life of the workload. Without it a frame rate is
    # a fact with no explanation; with it, a slower arm can be traced to the
    # clocks, the power limit or the temperature that produced it.
    local telemetry_pid=""
    if [ -x "$(dirname "$0")/gpu-telemetry.sh" ]; then
        "$(dirname "$0")/gpu-telemetry.sh" "$CARD" "$dest/gpu.csv" &
        telemetry_pid=$!
    fi
    ( cd "$STK_ROOT" && \
      LD_LIBRARY_PATH="$STK_ROOT/lib" \
      SUPERTUXKART_DATADIR="$STK_ROOT" \
      SUPERTUXKART_ASSETS_DIR="$STK_ROOT/data/" \
      timeout 240 ./bin/supertuxkart --benchmark >/dev/null 2>&1 )
    [ -n "$telemetry_pid" ] && kill "$telemetry_pid" 2>/dev/null
    local summary
    summary=$(grep -a "Profiler: Frame count" "$STK_CONFIG/stdout.log" 2>/dev/null | tail -1)
    [ -n "$summary" ] || return 1
    for f in stdout.log stdout.log.perf-report-black_forest.csv \
             stdout.log.profile-black_forest-cpu-0.csv; do
        [ -f "$STK_CONFIG/$f" ] && cp "$STK_CONFIG/$f" "$dest/" 2>/dev/null
    done
    # Frame count 'N', Time (ms) 'M'
    local frames ms
    frames=$(sed "s/.*Frame count '\([0-9]*\)'.*/\1/" <<<"$summary")
    ms=$(sed "s/.*Time (ms) '\([0-9]*\)'.*/\1/" <<<"$summary")
    [ -n "$frames" ] && [ -n "$ms" ] && [ "$ms" -gt 0 ] || return 1
    awk -v f="$frames" -v m="$ms" 'BEGIN{printf "%.4f\n", f/(m/1000)}'
    printf '%s\n' "$summary" > "$dest/summary.txt"
}

# ── main ─────────────────────────────────────────────────────────────────────

[ -n "$STK_ROOT" ] && [ -x "$STK_ROOT/bin/supertuxkart" ] || die "SuperTuxKart was not found"

STAMP=$(date +%Y-%m-%d)
# LABEL distinguishes sessions of the same workload on the same day -- a
# CPU-bound configuration and a GPU-bound one are different experiments and
# must not overwrite each other's evidence.
OUT="$OUT_ROOT/$STAMP-supertuxkart${LABEL:+-$LABEL}"
mkdir -p "$OUT"
log "render GPU: $CARD    workload: $STK_ROOT    output: $OUT"

declare -A RESULTS
ARMS=("$@")
[ ${#ARMS[@]} -gt 0 ] || ARMS=(baseline booster)

# Warm-up, discarded. Its only job is to populate the shader cache and bring
# the GPU to a steady temperature.
log "warm-up run (discarded)"
"arm_${ARMS[0]}"
run_workload "$OUT/.warmup" >/dev/null || die "the warm-up run produced no result"
rm -rf "$OUT/.warmup"

for i in $(seq 1 "$RUNS"); do
    for arm in "${ARMS[@]}"; do
        dir=$(printf '%s/%s/run-%02d' "$OUT" "$arm" "$i")
        "arm_$arm"
        sleep 3   # let the governor and DPM level settle before measuring
        fps=$(LC_ALL=C run_workload "$dir")
        if [ -z "$fps" ]; then
            log "$arm run $i: NO RESULT"
            continue
        fi
        RESULTS[$arm]="${RESULTS[$arm]:-} $fps"
        log "$(printf '%-16s run %d: %8.1f fps' "$arm" "$i" "$fps")"
        printf '%s\n' "$fps" > "$dir/fps.txt"
    done
done

# ── summary ──────────────────────────────────────────────────────────────────

printf '\n' >&2
for arm in "${ARMS[@]}"; do
    printf '%s %s\n' "$arm" "${RESULTS[$arm]:-}"
done > "$OUT/raw.txt"
log "raw results written to $OUT/raw.txt"
