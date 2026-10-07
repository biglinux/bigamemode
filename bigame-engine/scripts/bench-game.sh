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
#   GAME=sottr FG_PROCESS=SOTTR.exe MANGOHUD_CSV_DIR=~/.cache/mh LABEL=lsfg \
#       scripts/bench-game.sh fg_off fg_x2
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
# another library, so a stale prefix elsewhere can shadow the one the game
# writes its results to.
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

# The game's window and its focus: xdotool on X11; kdotool under KDE's
# Wayland session, where xdotool sees only XWayland windows and cannot tell
# which window has focus, with the key sent by a virtual keyboard
# (press-key.py) that the compositor delivers to the focused window.
export DISPLAY=${DISPLAY:-:0}
if command -v xdotool >/dev/null && xdotool search --name "$WINDOW_NAME" >/dev/null 2>&1; then
    WINDOWS=xdotool
elif command -v kdotool >/dev/null && [ -w /dev/uinput ]; then
    WINDOWS=kdotool
else
    die "neither xdotool (X11) nor kdotool with a writable /dev/uinput (KDE Wayland) is available"
fi
WINDOW=$("$WINDOWS" search --name "$WINDOW_NAME" 2>/dev/null | head -1)
[ -n "$WINDOW" ] || die "$TITLE is not running"

# Held rather than tapped: the game samples key state once per frame, and an
# instantaneous press/release falls between two samples.
press_rerun() {
    local waited=0
    until [ "$("$WINDOWS" getactivewindow 2>/dev/null)" = "$WINDOW" ]; do
        # A game that has exited will never take focus again.
        "$WINDOWS" getwindowname "$WINDOW" >/dev/null 2>&1 || { log "the game exited"; return 1; }
        [ $waited -eq 0 ] && log "waiting for $TITLE to regain keyboard focus"
        sleep 2; waited=$((waited + 2))
        [ $waited -ge "$TIMEOUT_S" ] && return 1
    done
    if [ "$WINDOWS" = xdotool ]; then
        xdotool keydown "$RERUN_KEY"; sleep 0.25; xdotool keyup "$RERUN_KEY"
    else
        python3 "$HERE/press-key.py" "$RERUN_KEY" 0.25
    fi
}

count() { local n; n=$(grep -c "\[Benchmark\] Benchmark $1" "$GAME_LOG" 2>/dev/null); echo "${n:-0}"; }
stops()  { count stopped; }
starts() { count started; }

# A press can be lost: the game drops input for a moment when it regains
# focus, and under Wayland a native dialog (the Polkit prompt) can hold the
# keyboard while X still reports the game as the active window. So a run
# counts as started only when the game's log says it started; loading the
# benchmark takes up to ~20 s, so a press is repeated only after 45.
start_run() {
    local before tries
    before=$(starts)
    for tries in 1 2 3; do
        press_rerun || return 1
        for _ in $(seq 1 45); do
            [ "$(starts)" -gt "$before" ] && return 0
            sleep 1
        done
        log "the game did not start a run (press $tries); pressing again"
    done
    return 1
}

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

# shellcheck source=render-card.sh
. "$HERE/render-card.sh"
CARD=${CARD:-$(render_card)}
[ -n "$CARD" ] || die "no GPU was found"

# Quoted with %q: the state is put back with eval, and a value is never code.
read_state() {
    printf 'governor=%q\n' "$(cat /sys/devices/system/cpu/cpu0/cpufreq/scaling_governor 2>/dev/null)"
    printf 'epp=%q\n' "$(cat /sys/devices/system/cpu/cpu0/cpufreq/energy_performance_preference 2>/dev/null)"
    printf 'dpm=%q\n' "$(cat "/sys/class/drm/$CARD/device/power_dpm_force_performance_level" 2>/dev/null)"
    printf 'profile=%q\n' "$(powerprofilesctl get 2>/dev/null)"
}
ORIGINAL=$(read_state)

set_governor() { [ -n "$1" ] && "${BUS[@]}" SetCpuGovernor s "$1" >/dev/null 2>&1; }
set_epp()      { [ -n "$1" ] && "${BUS[@]}" SetCpuEpp s "$1" >/dev/null 2>&1; }
set_dpm()      { [ -n "$1" ] && has_dpm "$CARD" && "${BUS[@]}" SetGpuDpmLevel ss "$CARD" "$1" >/dev/null 2>&1; }
set_profile()  { [ -n "$1" ] && powerprofilesctl set "$1" >/dev/null 2>&1; }

restore() {
    [ -n "${TELEMETRY_PID:-}" ] && kill "$TELEMETRY_PID" 2>/dev/null
    declare -F scx_stop >/dev/null && scx_stop
    [ -n "${UI_PID:-}" ] && kill -CONT "$UI_PID" 2>/dev/null
    log "restoring the machine to how it was found"
    eval "$ORIGINAL"
    # Assigned by the eval above, from read_state.
    # shellcheck disable=SC2154
    {
        set_profile "$profile"
        set_governor "$governor"
        set_epp "$epp"
        set_dpm "$dpm"
    }
    if [ -n "${TURBO_TOUCHED:-}" ]; then
        "$TURBO_BIN" "$([ "$TURBO_WAS" = active ] && echo on || echo off)" >/dev/null 2>&1 \
            && log "Turbo put back ${TURBO_WAS}"
    fi
    read_state | sed 's/^/  /' >&2
}
trap restore EXIT
# A trap that returns lets the session carry on: an interrupt ends it, and
# the EXIT trap puts the machine back once.
trap 'exit 130' INT TERM

# ── arms ─────────────────────────────────────────────────────────────────────
#
# An arm that isolates one knob is named for it.

# The distribution default: what an untouched machine runs.
arm_baseline()  { set_profile balanced; set_governor powersave; set_epp balance_performance; set_dpm auto; }
# The performance power profile with the GPU left to its firmware -- the
# Booster's plan.
arm_rest()      { set_profile performance; set_governor performance; set_epp performance; set_dpm auto; }
# The same, with the GPU pinned to its highest fixed DPM state.
arm_gpu_dpm_level() {
    has_dpm "$CARD" || die "$CARD has no DPM level to force (its driver is not amdgpu)"
    arm_rest; set_dpm high
}
# The distribution default with only the CPU governor and EPP raised -- the
# CPU knob isolated, for a workload where the CPU is what limits the frame rate.
arm_cpu_governor() { arm_baseline; set_governor performance; set_epp performance; }
# The product's own overhead: the same machine state, with the running
# Big Game Mode UI either polling as usual or frozen with SIGSTOP. Frozen rather
# than closed, so nothing it owns is restored or torn down; SIGCONT resumes it
# exactly where it was, and restore() always sends it.
UI_PID=$(pgrep -x bigame-ui | head -1)
arm_ui_polling() { arm_rest; [ -n "$UI_PID" ] && kill -CONT "$UI_PID"; }
arm_ui_paused()  { arm_rest; [ -n "$UI_PID" ] || die "no bigame-ui is running"; kill -STOP "$UI_PID"; }

# Scheduler arms. The scheduler is falcond's to set, per game, so these do not
# set it themselves: they ask scripts/scx-switch.sh -- started once, as root,
# under a single Polkit approval -- to rewrite the game's falcond profile and
# have falcond reload. What the kernel reports afterwards is recorded with the
# run, so a scheduler that did not take is visible rather than assumed.
SCX_PID=""; SCX_READY=""
scx_start() {
    [ -n "$SCX_PID" ] && return 0
    log "starting the scheduler switcher: approve the Polkit prompt"
    # A coprocess: requests go to its stdin, replies come from its stdout, and
    # if this script dies the pipe closes, which ends the root side and puts
    # the profile back.
    coproc SCX { exec pkexec "$HERE/scx-switch.sh" "${SCX_PROFILE:?set SCX_PROFILE to the falcond profile name of the game}"; }
    local reply=""
    read -r -t "${SCX_AUTH_TIMEOUT_S:-300}" -u "${SCX[0]}" reply
    [ "$reply" = ready ] || die "the scheduler switcher did not start (Polkit refused, timed out, or bad profile)"
    SCX_READY=1
}
scx_set() {
    scx_start
    echo "$1 $2" >&"${SCX[1]}"
    SCX_NOW=""
    read -r -t 60 -u "${SCX[0]}" SCX_NOW || die "the scheduler switcher did not answer"
    case $SCX_NOW in ok\ *) ;; *) die "scheduler switch refused: $SCX_NOW" ;; esac
    log "scheduler: asked $1/$2, kernel reports: ${SCX_NOW#ok }"
}
scx_stop() {
    [ -n "$SCX_PID" ] || return 0
    local pid=$SCX_PID; SCX_PID=""
    [ -n "${SCX[1]:-}" ] && eval "exec ${SCX[1]}>&-"
    # pkexec still waiting for its approval reads no input, so closing the
    # pipe would not end it and the wait below would never return. It has
    # changed nothing yet, and it still runs as this user: stop it.
    [ -n "$SCX_READY" ] || kill "$pid" 2>/dev/null
    wait "$pid" 2>/dev/null
}
# Frame generation arms: the game's lsfg-vk entry, written by Big Game Mode's own
# code (bigame-core's lsfg example), which lsfg-vk reloads while the game runs.
# Whether generation really took is in the data: with MANGOHUD_CSV_DIR set,
# presented frames (MangoHud) against rendered frames (the game's own count).
LSFG_BIN=${LSFG_BIN:-$HERE/../target/debug/examples/lsfg}
fg_set() {
    [ -x "$LSFG_BIN" ] || die "build it first: cargo build -p bigame-core --example lsfg"
    "$LSFG_BIN" "$@" >/dev/null || die "lsfg-vk entry not written: $*"
}
arm_fg_off() { arm_rest; fg_set off "${FG_PROCESS:?set FG_PROCESS to the process name of the game}"; }
arm_fg_x2()  { arm_rest; fg_set set "${FG_PROCESS:?set FG_PROCESS to the process name of the game}" 2; }
arm_fg_x3()  { arm_rest; fg_set set "${FG_PROCESS:?set FG_PROCESS to the process name of the game}" 3; }

# Turbo as the Home button switches it: the turbo example, through the helper
# and Polkit. falcond applies the running game's profile when it starts and
# puts the machine back when it stops, so these arms alternate within one
# launch; the state Turbo was found in comes back at the end.
TURBO_BIN=${TURBO_BIN:-$HERE/../target/release/examples/turbo}
TURBO_WAS=$(systemctl is-active falcond 2>/dev/null)
turbo_set() {
    [ -x "$TURBO_BIN" ] || die "build it first: cargo build --release -p bigame-core --example turbo"
    TURBO_TOUCHED=1
    "$TURBO_BIN" "$1" >/dev/null 2>&1 || die "Turbo $1 failed"
}
arm_turbo_off() { turbo_set off; }
# falcond looks at running processes again every 9 s.
arm_turbo_on()  { turbo_set on; sleep 12; }

arm_scx_none()    { arm_rest; scx_set none default; }
arm_scx_lavd()    { arm_rest; scx_set lavd gaming; }
arm_scx_bpfland() { arm_rest; scx_set bpfland gaming; }

# ── session ──────────────────────────────────────────────────────────────────

# More arms, for a matrix of its own: a file of arm_<name> functions.
if [ -n "${ARMS_FILE:-}" ]; then
    # shellcheck source=/dev/null
    . "$ARMS_FILE" || die "could not read $ARMS_FILE"
fi

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
    if [ -n "${MANGOHUD_CSV_DIR:-}" ]; then
        for f in "$MANGOHUD_CSV_DIR"/*.csv; do
            [ "$f" -nt "$since" ] && cp -p "$f" "$dest/"
        done
    fi
    ls "$dest"/*_frametimes_*.txt >/dev/null 2>&1
}

# Presented frames: MangoHud's logging key, pressed once the run has started.
# MangoHud's own log_duration ends the log (set it in the MangoHud
# configuration together with output_folder = MANGOHUD_CSV_DIR).
mangohud_log() {
    [ -n "${MANGOHUD_CSV_DIR:-}" ] || return 0
    sleep "${MANGOHUD_LOG_DELAY_S:-8}"   # past the scene's loading frames
    [ "$(xdotool getactivewindow 2>/dev/null)" = "$WINDOW" ] || { log "no focus: MangoHud log not started"; return 0; }
    xdotool keydown Shift_L keydown F2; sleep 0.25; xdotool keyup F2 keyup Shift_L
}

# The Polkit prompt comes before the warm-up, so a refusal costs nothing.
for arm in "${ARMS[@]}"; do case $arm in scx_*) scx_start; break ;; esac; done

# Warm-up: whatever pass is running or last finished is discarded. If the game
# is idle on its results screen, start one.
if [ "$(starts)" -le "$(stops)" ]; then
    log "warm-up run (discarded)"
    before=$(stops); start_run || die "could not start the warm-up"
else
    log "a pass is already running; it is the warm-up (discarded)"
    before=$(stops)
fi
wait_for_stop "$before" || die "the warm-up did not finish"

N=${#ARMS[@]}; INCOMPLETE=""
for round in $(seq 1 "$RUNS"); do
    for k in $(seq 0 $((N - 1))); do
        arm=${ARMS[$(( (k + round - 1) % N ))]}
        dir=$(printf '%s/%s/run-%02d' "$OUT" "$arm" "$round")
        mkdir -p "$dir"
        "arm_$arm"
        sleep "$SETTLE_S"   # let clocks, governor and temperature settle
        read_state > "$dir/state.txt"
        printf 'sched_ext=%s\n' "$(cat /sys/kernel/sched_ext/root/ops 2>/dev/null || echo none)" >> "$dir/state.txt"
        stamp="$dir/.start"; touch "$stamp"
        "$HERE/gpu-telemetry.sh" "$CARD" "$dir/gpu.csv" & TELEMETRY_PID=$!
        before=$(stops)
        start_run || { log "$arm run $round: the game would not start a run"; INCOMPLETE=1; break 2; }
        mangohud_log
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
if [ -n "${INCOMPLETE:-}" ]; then
    log "session INCOMPLETE -- stopped at $arm run $round; results so far: $OUT"
    exit 1
fi
log "session complete: $OUT"
