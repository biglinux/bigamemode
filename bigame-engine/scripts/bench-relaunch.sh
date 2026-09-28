#!/usr/bin/env bash
# One launch per run, for arms that need the game closed to change: its files
# (OptiScaler in or out), its graphics API, frame generation (lsfg-vk starts
# generating only for a game that starts with an entry).
#
# Every run: the game is closed, the arm is applied (arm_<name> from ARMS_FILE,
# with the game closed), the game is started through Steam, its benchmark is
# opened from the main menu, the first pass warms up (shaders, caches) and
# bench-game.sh measures the next one. Arms rotate between rounds. The runs
# land in one session, in the layout bench_native_report reads.
#
# Under KDE's Wayland session, with press-key.py and kdotool, like
# bench-game.sh. The menu path is Shadow of the Tomb Raider's: Options,
# Video and graphics, [R] benchmark.
#
# Usage: ARMS_FILE=arms.sh RUNS=2 LABEL=name scripts/bench-relaunch.sh arm_a arm_b
set -uo pipefail
export LC_ALL=C

HERE=$(cd "$(dirname "$0")" && pwd)
RUNS=${RUNS:-2}
OUT_ROOT=${OUT_ROOT:-benchmarks}
APP_ID=750920
WINDOW_NAME="^Shadow of the Tomb Raider"
PFX="$HOME/.local/share/Steam/steamapps/compatdata/$APP_ID/pfx"
GAME_LOG="$PFX/drive_c/users/steamuser/Documents/Shadow of the Tomb Raider/Shadow of the Tomb Raider.log"

log() { printf '%s  %s\n' "$(date +%H:%M:%S)" "$*" >&2; }
die() { log "ERROR: $*"; exit 1; }
key() { python3 "$HERE/press-key.py" "$1" "${2:-0.15}"; }

[ -n "${ARMS_FILE:-}" ] || die "set ARMS_FILE to a file of arm_<name> functions"
# shellcheck source=/dev/null
. "$ARMS_FILE" || die "could not read $ARMS_FILE"
ARMS=("$@")
[ ${#ARMS[@]} -gt 0 ] || die "name the arms"
for arm in "${ARMS[@]}"; do declare -F "arm_$arm" >/dev/null || die "unknown arm '$arm'"; done
command -v steam >/dev/null && command -v kdotool >/dev/null || die "needs steam and kdotool"

starts() { local n; n=$(grep -c '\[Benchmark\] Benchmark started' "$GAME_LOG" 2>/dev/null); echo "${n:-0}"; }

close_game() {
    pgrep -x SOTTR.exe >/dev/null || return 0
    pkill -TERM -x SOTTR.exe
    for _ in $(seq 1 60); do pgrep -x SOTTR.exe >/dev/null || break; sleep 1; done
    pgrep -x SOTTR.exe >/dev/null && pkill -KILL -x SOTTR.exe
    # Steam takes a moment to see the game gone; a launch before that is
    # refused as "already running".
    sleep 15
}

# Launch, pass the launcher, open the benchmark and start the warm-up pass.
start_game() {
    local window="" _i
    steam -applaunch "$APP_ID" >/dev/null 2>&1 &
    for _i in $(seq 1 90); do
        window=$(kdotool search --name "$WINDOW_NAME" 2>/dev/null | head -1)
        [ -n "$window" ] && break
        sleep 2
    done
    [ -n "$window" ] || { log "the launcher did not appear"; return 1; }
    sleep 4
    kdotool windowactivate "$window" 2>/dev/null
    sleep 1
    key enter
    # The game's own window is titled with its build ("v1.0 build …").
    for _i in $(seq 1 90); do
        window=$(kdotool search --name "$WINDOW_NAME v" 2>/dev/null | head -1)
        [ -n "$window" ] && break
        sleep 2
    done
    [ -n "$window" ] || { log "the game did not start"; return 1; }
    # Splash screens, then the main menu, "Continue" selected.
    sleep "${MENU_WAIT_S:-60}"
    local before tries
    before=$(starts)
    for tries in 1 2 3; do
        [ "$(kdotool getactivewindow 2>/dev/null)" = "$window" ] || kdotool windowactivate "$window" 2>/dev/null
        sleep 1
        key down; key down; key down; key down; key enter   # Options
        sleep 3
        key down; key down; key down; key enter              # Video and graphics
        sleep 4
        key r 0.25                                           # benchmark
        for _ in $(seq 1 45); do
            [ "$(starts)" -gt "$before" ] && return 0
            sleep 1
        done
        log "the benchmark did not start (try $tries); back to the main menu"
        key esc; sleep 2; key esc; sleep 3
    done
    return 1
}

OUT="$OUT_ROOT/$(date +%Y-%m-%d)-sottr${LABEL:+-$LABEL}"
mkdir -p "$OUT"
log "arms: ${ARMS[*]}    runs: $RUNS    output: $OUT"
MEASURE=$(mktemp -d)
trap 'close_game; rm -rf "$MEASURE"' EXIT INT TERM
printf 'arm_measure() { :; }\n' > "$MEASURE/arms.sh"

for i in $(seq 1 "$RUNS"); do
    # Rotated per round: A B, B A, …
    order=("${ARMS[@]:$(( (i - 1) % ${#ARMS[@]} ))}" "${ARMS[@]:0:$(( (i - 1) % ${#ARMS[@]} ))}")
    for arm in "${order[@]}"; do
        log "round $i: $arm"
        close_game
        "arm_$arm" || die "arm $arm could not be applied"
        start_game || die "could not reach the benchmark for $arm"
        rm -rf "$MEASURE/out"
        GAME=sottr RUNS=1 OUT_ROOT="$MEASURE/out" LABEL=run ARMS_FILE="$MEASURE/arms.sh" \
            "$HERE/bench-game.sh" measure || die "the measured pass of $arm failed"
        dest=$(printf '%s/%s/run-%02d' "$OUT" "$arm" "$i")
        mkdir -p "$(dirname "$dest")"
        mv "$MEASURE"/out/*/measure/run-01 "$dest" || die "no run was recorded for $arm"
        log "round $i: $arm recorded in $dest"
    done
done
log "session complete: $OUT"
