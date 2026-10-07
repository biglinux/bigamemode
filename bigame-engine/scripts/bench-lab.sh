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

# shellcheck source=render-card.sh
. "$(dirname "$0")/render-card.sh"
CARD=${CARD:-$(render_card)}
[ -n "$CARD" ] || die "no GPU was found"

# Quoted with %q: the state is put back with eval, and a value is never code.
read_state() {
    printf 'governor=%q\n' "$(cat /sys/devices/system/cpu/cpu0/cpufreq/scaling_governor 2>/dev/null || echo '')"
    printf 'epp=%q\n' "$(cat /sys/devices/system/cpu/cpu0/cpufreq/energy_performance_preference 2>/dev/null || echo '')"
    printf 'dpm=%q\n' "$(cat "/sys/class/drm/$CARD/device/power_dpm_force_performance_level" 2>/dev/null || echo '')"
    printf 'profile=%q\n' "$(powerprofilesctl get 2>/dev/null || echo '')"
}

ORIGINAL=$(read_state)

# The workload's own settings are state as well. A benchmark needs the frame
# cap lifted and, to test anything GPU-side, the resolution and effects raised;
# none of that is what the user chose, and all of it has to come back. Kept
# beside the results rather than in a temporary directory, so a crashed session
# still leaves a recoverable copy.
STK_CONFIG_BACKUP=""
back_up_workload_config() {
    [ -f "$STK_CONFIG/config.xml" ] || return 0
    STK_CONFIG_BACKUP="$OUT/workload-config.original.xml"
    mkdir -p "$OUT"
    cp "$STK_CONFIG/config.xml" "$STK_CONFIG_BACKUP" 2>/dev/null || STK_CONFIG_BACKUP=""
}
restore_workload_config() {
    [ -n "$STK_CONFIG_BACKUP" ] && [ -f "$STK_CONFIG_BACKUP" ] || return 0
    cp "$STK_CONFIG_BACKUP" "$STK_CONFIG/config.xml" 2>/dev/null \
        && log "workload configuration restored from $STK_CONFIG_BACKUP"
}

set_governor() { [ -n "$1" ] && "${BUS[@]}" SetCpuGovernor s "$1" >/dev/null 2>&1; }
set_epp()      { [ -n "$1" ] && "${BUS[@]}" SetCpuEpp s "$1" >/dev/null 2>&1; }
set_dpm()      { [ -n "$1" ] && has_dpm "$CARD" && "${BUS[@]}" SetGpuDpmLevel ss "$CARD" "$1" >/dev/null 2>&1; }
set_profile()  { [ -n "$1" ] && powerprofilesctl set "$1" >/dev/null 2>&1; }

restore() {
    log "restoring the machine to how it was found"
    eval "$ORIGINAL"
    # Assigned by the eval above, from read_state.
    # shellcheck disable=SC2154
    {
        set_governor "$governor"
        set_epp "$epp"
        set_dpm "$dpm"
        set_profile "$profile"
    }
    read_state | sed 's/^/  /' >&2
    restore_workload_config
}
trap restore EXIT
# A trap that returns lets the session carry on: an interrupt ends it, and
# the EXIT trap puts the machine back once.
trap 'exit 130' INT TERM

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

# Single-knob arms, named for the knob.
arm_cpu_governor()  { arm_baseline; set_governor performance; set_epp performance; }
arm_gpu_dpm_level() {
    has_dpm "$CARD" || die "$CARD has no DPM level to force (its driver is not amdgpu)"
    arm_baseline; set_dpm high
}

# ── workload ─────────────────────────────────────────────────────────────────

STK_ROOT=${STK_ROOT:-}
if [ -z "$STK_ROOT" ]; then
    STK_ROOT=$(find "$HOME" -maxdepth 6 -type f -name supertuxkart -path '*/bin/*' \
                 -printf '%h\n' 2>/dev/null | head -1)
    [ -n "$STK_ROOT" ] && STK_ROOT=$(dirname "$STK_ROOT")
fi
STK_CONFIG="${XDG_CONFIG_HOME:-$HOME/.config}/supertuxkart/config-0.10"

# How an arm may change the way the game starts, reset before every arm:
# variables for it, or the whole command Big Game Mode's launcher would run
# (see use_launch_plan). The product is what is measured.
WORKLOAD_ENV=()
WORKLOAD_PLAN=()

# The command Big Game Mode's launcher builds for SuperTuxKart with the launch
# settings in <config-dir>/bigame-mode/video.toml -- render offload on a hybrid
# laptop, Gamescope, vkBasalt -- printed by the launch_plan example.
use_launch_plan() {
    local dir plan line
    # Absolute: a relative XDG_CONFIG_HOME is not one (the XDG rules), so
    # Big Game Mode would read the user's own settings instead of the arm's.
    dir=$(cd "$1" 2>/dev/null && pwd) || die "no configuration directory $1"
    plan="$(dirname "$0")/../target/release/examples/launch_plan"
    [ -x "$plan" ] || die "build the launch plan tool first: cargo build --release -p bigame-core --examples"
    [ -z "$STK_ROOT" ] || die "a launch plan needs the installed supertuxkart, not a build under $STK_ROOT"
    WORKLOAD_ENV=() WORKLOAD_PLAN=()
    while IFS= read -r line; do
        case $line in
            "env "*) WORKLOAD_ENV+=("${line#env }") ;;
            "program "* | "arg "*) WORKLOAD_PLAN+=("${line#* }") ;;
        esac
    done < <(XDG_CONFIG_HOME=$dir "$plan" "$(command -v supertuxkart)" --benchmark)
    [ ${#WORKLOAD_PLAN[@]} -gt 0 ] || die "the launch plan printed no command"
    log "  starts as: ${WORKLOAD_PLAN[*]}"
}

start_workload() {
    if [ ${#WORKLOAD_PLAN[@]} -gt 0 ]; then
        env "${WORKLOAD_ENV[@]}" timeout 240 "${WORKLOAD_PLAN[@]}"
    elif [ -n "$STK_ROOT" ]; then
        ( cd "$STK_ROOT" && \
          env LD_LIBRARY_PATH="$STK_ROOT/lib" \
              SUPERTUXKART_DATADIR="$STK_ROOT" \
              SUPERTUXKART_ASSETS_DIR="$STK_ROOT/data/" \
              "${WORKLOAD_ENV[@]}" timeout 240 ./bin/supertuxkart --benchmark )
    else
        env "${WORKLOAD_ENV[@]}" timeout 240 supertuxkart --benchmark
    fi
}

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
    start_workload >/dev/null 2>&1
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

{ [ -n "$STK_ROOT" ] && [ -x "$STK_ROOT/bin/supertuxkart" ]; } || command -v supertuxkart >/dev/null \
    || die "SuperTuxKart was not found"

# More arms, for a matrix of its own: a file of arm_<name> functions, which
# may call use_launch_plan or set WORKLOAD_ENV.
if [ -n "${ARMS_FILE:-}" ]; then
    # shellcheck source=/dev/null
    . "$ARMS_FILE" || die "could not read $ARMS_FILE"
fi

STAMP=$(date +%Y-%m-%d)
# LABEL distinguishes sessions of the same workload on the same day -- a
# CPU-bound configuration and a GPU-bound one are different experiments and
# must not overwrite each other's evidence.
OUT="$OUT_ROOT/$STAMP-supertuxkart${LABEL:+-$LABEL}"
mkdir -p "$OUT"
log "render GPU: $CARD    workload: ${STK_ROOT:-$(command -v supertuxkart)}    output: $OUT"
back_up_workload_config

declare -A RESULTS
ARMS=("$@")
[ ${#ARMS[@]} -gt 0 ] || ARMS=(baseline booster)

# Warm-up, discarded. Its only job is to populate the shader cache and bring
# the GPU to a steady temperature.
log "warm-up run (discarded)"
WORKLOAD_ENV=() WORKLOAD_PLAN=()
"arm_${ARMS[0]}"
run_workload "$OUT/.warmup" >/dev/null || die "the warm-up run produced no result"
rm -rf "$OUT/.warmup"

for i in $(seq 1 "$RUNS"); do
    for arm in "${ARMS[@]}"; do
        dir=$(printf '%s/%s/run-%02d' "$OUT" "$arm" "$i")
        WORKLOAD_ENV=() WORKLOAD_PLAN=()
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
