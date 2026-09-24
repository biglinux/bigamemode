#!/usr/bin/env bash
# Alternating A/B session against a game's own built-in benchmark.
#
# The Crystal Dynamics titles end their benchmark on a results screen that
# offers "[R] Run benchmark" again. That makes one launch enough for a whole
# session: the game stays loaded, its shader and file caches stay warm, and
# every run after the first is taken under identical conditions except for
# the one thing the arm changes. Relaunching per run would add a cold start to
# every measurement.
#
# What this does per run:
#   1. apply the arm through the product's own D-Bus daemon
#   2. press the rerun key -- only while the game holds keyboard focus, so a
#      keystroke can never land in whatever else the person is doing
#   3. wait for the game's log to say the benchmark stopped
#   4. collect the frametimes and summary files the game wrote
#
# Arms rotate between rounds (ABC, BCA, CAB) so that no configuration always
# runs first, when the card is coolest, or last, when it is warmest.
#
# The first run of the session is a warm-up and is discarded. Machine state is
# captured before anything changes and restored on exit, including on
# interrupt.
#
# Usage:
#   GAME=sottr RUNS=4 LABEL=dpm scripts/bench-game.sh rest gpu_dpm_level baseline
#
# The game must already be running and sitting on its benchmark results
# screen (or running its first benchmark pass, which becomes the warm-up).
set -uo pipefail
export LC_ALL=C

BUS=(busctl --system call com.biglinux.BiGameMode /com/biglinux/BiGameMode com.biglinux.BiGameMode)
RUNS=${RUNS:-3}
OUT_ROOT=${OUT_ROOT:-benchmarks}
GAME=${GAME:-sottr}
SETTLE_S=${SETTLE_S:-20}
TIMEOUT_S=${TIMEOUT_S:-600}
HERE=$(cd "$(dirname "$0")" && pwd)

log() { printf '%s  %s\n' "$(date +%H:%M:%S)" "$*" >&2; }
die() { log "ERROR: $*"; exit 1; }

# ── the game ─────────────────────────────────────────────────────────────────

# The prefix in the library that holds the game's manifest. Not simply the first
# compatdata/<id> found: Steam leaves the old one behind when a game moves to
# another library, and on this machine the home library still has a stale
# prefix for a game that now lives, and writes its results, on another disk.
steam_prefix() {
    local id=$1 lib
    while read -r lib; do
        [ -f "$lib/steamapps/appmanifest_$id.acf" ] && [ -d "$lib/steamapps/compatdata/$id/pfx" ] \
            && { echo "$lib/steamapps/compatdata/$id/pfx"; return; }
    done < <(grep -o '"path"[[:space:]]*"[^"]*"' "$HOME/.local/share/Steam/steamapps/libraryfolders.vdf" \
               | sed 's/.*"\([^"]*\)"$/\1/')
}

case "$GAME" in
    sottr)
        APP_ID=750920
        TITLE="Shadow of the Tomb Raider"
        WINDOW_NAME="^Shadow of the Tomb Raider"
        PFX=$(steam_prefix $APP_ID)
        RESULT_DIR="$PFX/drive_c/users/steamuser/Documents/Shadow of the Tomb Raider"
        GAME_LOG="$RESULT_DIR/Shadow of the Tomb Raider.log"
        RERUN_KEY=r
        ;;
    *) die "unknown GAME '$GAME'" ;;
esac
[ -n "${PFX:-}" ] && [ -d "$RESULT_DIR" ] || die "$TITLE's Proton prefix was not found"

export DISPLAY=${DISPLAY:-:0}
WINDOW=$(xdotool search --name "$WINDOW_NAME" 2>/dev/null | head -1)
[ -n "$WINDOW" ] || die "$TITLE is not running"

# Held rather than tapped: the game samples key state once per frame, and an
# instantaneous press/release from XTEST falls between two samples.
press_rerun() {
    local waited=0
    until [ "$(xdotool getactivewindow 2>/dev/null)" = "$WINDOW" ]; do
        [ $waited -eq 0 ] && log "waiting for $TITLE to regain keyboard focus"
        sleep 2; waited=$((waited + 2))
        [ $waited -ge "$TIMEOUT_S" ] && return 1
    done
    xdotool keydown "$RERUN_KEY"; sleep 0.25; xdotool keyup "$RERUN_KEY"
}

count() { local n; n=$(grep -c "\[Benchmark\] Benchmark $1" "$GAME_LOG" 2>/dev/null); echo "${n:-0}"; }
stops()  { count stopped; }
starts() { count started; }

wait_for_stop() {
    local before=$1 waited=0
    while [ "$(stops)" -le "$before" ]; do
        sleep 5; waited=$((waited + 5))
        [ $waited -ge "$TIMEOUT_S" ] && return 1
        pgrep -f "SOTTR.exe" >/dev/null || { log "the game exited"; return 1; }
    done
    sleep 3   # the files are written just after the log line
}

# ── machine state ────────────────────────────────────────────────────────────

CARD=${CARD:-$(for c in /sys/class/drm/card[0-9]*; do
    [ -r "$c/device/mem_info_vram_total" ] && [ -e "$c/device/power_dpm_force_performance_level" ] \
        && printf '%s %s\n' "$(cat "$c/device/mem_info_vram_total")" "$(basename "$c")"
done | sort -n | tail -1 | cut -d' ' -f2)}
[ -n "$CARD" ] || die "no GPU with a DPM control was found"

read_state() {
    printf 'governor=%s\n' "$(cat /sys/devices/system/cpu/cpu0/cpufreq/scaling_governor 2>/dev/null)"
    printf 'epp=%s\n' "$(cat /sys/devices/system/cpu/cpu0/cpufreq/energy_performance_preference 2>/dev/null)"
    printf 'dpm=%s\n' "$(cat "/sys/class/drm/$CARD/device/power_dpm_force_performance_level" 2>/dev/null)"
    printf 'profile=%s\n' "$(powerprofilesctl get 2>/dev/null)"
}
ORIGINAL=$(read_state)

set_governor() { [ -n "$1" ] && "${BUS[@]}" SetCpuGovernor s "$1" >/dev/null 2>&1; }
set_epp()      { [ -n "$1" ] && "${BUS[@]}" SetCpuEpp s "$1" >/dev/null 2>&1; }
set_dpm()      { [ -n "$1" ] && "${BUS[@]}" SetGpuDpmLevel ss "$CARD" "$1" >/dev/null 2>&1; }
set_profile()  { [ -n "$1" ] && powerprofilesctl set "$1" >/dev/null 2>&1; }

restore() {
    [ -n "${TELEMETRY_PID:-}" ] && kill "$TELEMETRY_PID" 2>/dev/null
    log "restoring the machine to how it was found"
    eval "$ORIGINAL"
    set_profile "$profile"; set_governor "$governor"; set_epp "$epp"; set_dpm "$dpm"
    read_state | sed 's/^/  /' >&2
}
trap restore EXIT INT TERM

# ── arms ─────────────────────────────────────────────────────────────────────
#
# Names match the knobs' calibration keys where an arm isolates one knob, so a
# result can feed the planner without translation.

# The distribution default: what an untouched machine runs.
arm_baseline()  { set_profile balanced; set_governor powersave; set_epp balance_performance; set_dpm auto; }
# The performance power profile with the GPU left to its firmware -- what the
# calibrated Booster now produces.
arm_rest()      { set_profile performance; set_governor performance; set_epp performance; set_dpm auto; }
# The same, with the GPU pinned to its highest fixed DPM state -- what the
# Booster did before it consulted measurements.
arm_gpu_dpm_level() { arm_rest; set_dpm high; }

# ── session ──────────────────────────────────────────────────────────────────

ARMS=("$@")
[ ${#ARMS[@]} -gt 0 ] || ARMS=(rest gpu_dpm_level)
for arm in "${ARMS[@]}"; do declare -F "arm_$arm" >/dev/null || die "unknown arm '$arm'"; done

OUT="$OUT_ROOT/$(date +%Y-%m-%d)-$GAME${LABEL:+-$LABEL}"
mkdir -p "$OUT"
log "$TITLE  window $WINDOW  GPU $CARD  output $OUT"
printf '%s\n' "$ORIGINAL" > "$OUT/state-before.txt"

collect() {
    local dest=$1 since=$2 f
    mkdir -p "$dest"
    for f in "$RESULT_DIR"/*.txt; do
        [ "$f" -nt "$since" ] && cp -p "$f" "$dest/"
    done
    ls "$dest"/*_frametimes_*.txt >/dev/null 2>&1
}

# Warm-up: whatever pass is running or last finished is discarded. If the game
# is idle on its results screen, start one.
if [ "$(starts)" -le "$(stops)" ]; then
    log "warm-up run (discarded)"
    before=$(stops); press_rerun || die "could not start the warm-up"
    sleep 10
else
    log "a pass is already running; it is the warm-up (discarded)"
    before=$(stops)
fi
wait_for_stop "$before" || die "the warm-up did not finish"

N=${#ARMS[@]}
for round in $(seq 1 "$RUNS"); do
    for k in $(seq 0 $((N - 1))); do
        arm=${ARMS[$(( (k + round - 1) % N ))]}
        dir=$(printf '%s/%s/run-%02d' "$OUT" "$arm" "$round")
        mkdir -p "$dir"
        "arm_$arm"
        sleep "$SETTLE_S"   # let clocks, governor and temperature settle
        read_state > "$dir/state.txt"
        stamp="$dir/.start"; touch "$stamp"
        "$HERE/gpu-telemetry.sh" "$CARD" "$dir/gpu.csv" & TELEMETRY_PID=$!
        before=$(stops)
        press_rerun || { log "$arm run $round: focus never returned"; break 2; }
        if ! wait_for_stop "$before"; then
            log "$arm run $round: NO RESULT"
            kill "$TELEMETRY_PID" 2>/dev/null; TELEMETRY_PID=""
            continue
        fi
        kill "$TELEMETRY_PID" 2>/dev/null; TELEMETRY_PID=""
        if collect "$dir" "$stamp"; then
            log "$(printf '%-10s run %d: %s' "$arm" "$round" \
                  "$(grep -a -m1 'Average FPS' "$dir"/SOTTR_*[0-9].txt 2>/dev/null | tr -s ' \t' ' ')")"
        else
            log "$arm run $round: the game wrote no result files"
        fi
        rm -f "$stamp"
    done
done
log "session complete: $OUT"
