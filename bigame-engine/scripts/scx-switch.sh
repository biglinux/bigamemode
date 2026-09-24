#!/usr/bin/env bash
# Root side of a scheduler benchmark session: one Polkit approval for all of it.
#
# scx_loader authorizes switches per calling process (auth_admin_keep), so a
# harness that switched with busctl would prompt at every run. This runs once
# as root, and applies each requested scheduler the way the product does: it
# rewrites the game's falcond profile (scx_sched, scx_sched_props) and asks
# falcond to reload, so falcond -- the owner of the scheduler -- applies it.
#
# Usage: pkexec scripts/scx-switch.sh <profile-name>
#
# Requests arrive on stdin, one per line: "SCHEDULER MODE" (e.g. "lavd gaming",
# "none default"). Each gets one line on stdout: "ok <kernel ops>" or
# "error <why>"; "ready" is written once, before the first request. The
# session ends at end of input -- which includes the harness dying -- and the
# profile is put back as it was, whatever happened. Nothing the caller names
# is ever opened for writing: the only path written is the profile, which is
# checked below.
set -uo pipefail

NAME=${1:?profile name}
PROFILES=/usr/share/falcond/profiles/user

# Everything that reaches a file or a command is allow-listed.
[[ "$NAME" =~ ^[A-Za-z0-9._+\ -]+$ ]] || { echo "bad profile name" >&2; exit 2; }
PROFILE="$PROFILES/$NAME.conf"
[ -f "$PROFILE" ] && [ ! -L "$PROFILE" ] || { echo "no such profile: $PROFILE" >&2; exit 2; }
grep -q '^scx_sched = ' "$PROFILE" && grep -q '^scx_sched_props = ' "$PROFILE" \
    || { echo "$PROFILE has no scx_sched/scx_sched_props lines to rewrite" >&2; exit 2; }

BACKUP=$(mktemp)
cp -p "$PROFILE" "$BACKUP"
restore() {
    cp -p "$BACKUP" "$PROFILE"; rm -f "$BACKUP"
    systemctl kill --kill-whom=main -s HUP falcond 2>/dev/null
}
trap restore EXIT
trap 'exit 1' INT TERM HUP

installed() { [ "$1" = none ] || [ -x "/usr/bin/scx_$1" ]; }
ops() { cat /sys/kernel/sched_ext/root/ops 2>/dev/null || echo none; }

echo ready
while read -r sched mode; do
    if ! [[ "$sched" =~ ^[a-z0-9_]+$ ]] || ! installed "$sched" \
        || ! [[ "${mode:-}" =~ ^(default|gaming|power|latency|server)$ ]]; then
        echo "error rejected: $sched ${mode:-}"; continue
    fi
    sed -i -e "s/^scx_sched = .*/scx_sched = $sched/" \
           -e "s/^scx_sched_props = .*/scx_sched_props = $mode/" "$PROFILE"
    systemctl kill --kill-whom=main -s HUP falcond
    # Verify against the kernel, not against the request.
    for _ in $(seq 1 50); do
        now=$(ops)
        if { [ "$sched" = none ] && [ "$now" = none ]; } || [[ "$now" == "$sched"* ]]; then break; fi
        sleep 0.2
    done
    echo "ok $(ops)"
done
