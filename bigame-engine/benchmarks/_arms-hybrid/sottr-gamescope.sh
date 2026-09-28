# shellcheck shell=bash
# Gamescope arms for Shadow of the Tomb Raider, read by bench-relaunch.sh:
# the Recommended AI Graphics state (DX12, OptiScaler FSR 3.1 from XeSS
# Quality) with and without the Gamescope wrapper BiGame-mode writes into the
# game's Steam launch options (the steam_gamescope example, as a profile with
# Gamescope "Always" does). Launch options can be written only with Steam
# closed; it is started again as the desktop starts it, with the session's
# environment.
# shellcheck source=sottr-upscaling.sh
. "$(dirname "${BASH_SOURCE[0]}")/sottr-upscaling.sh"
STEAM_GAMESCOPE="$ARMS_DIR/../../target/release/examples/steam_gamescope"

steam_closed() {
    local _i
    steam -shutdown >/dev/null 2>&1
    for _i in $(seq 1 60); do pgrep -x steam >/dev/null || return 0; sleep 1; done
    return 1
}
steam_started() {
    local _i
    systemd-run --user --quiet --collect steam -silent
    for _i in $(seq 1 90); do pgrep -x steamwebhelper >/dev/null && { sleep 20; return 0; }; sleep 1; done
    return 1
}
with_gamescope() {
    arm_optiscaler || return 1
    steam_closed || return 1
    "$STEAM_GAMESCOPE" SOTTR.exe "$1" >&2 || return 1
    steam_started
}
arm_plain() { with_gamescope off; }
arm_gamescope() { with_gamescope on; }
