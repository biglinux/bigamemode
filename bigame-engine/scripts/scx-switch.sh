#!/usr/bin/env bash
# Root side of a scheduler benchmark session: one Polkit approval for all of it.
#
# scx_loader authorizes switches per calling process (auth_admin_keep), so a
# harness that switched with busctl would prompt at every run. This runs once
# as root, and applies each requested scheduler the way the product does: it
# rewrites the game's falcond profile (scx_sched, scx_sched_props) and asks
# falcond to reload, so falcond -- the owner of the scheduler -- applies it.
#
# Usage: pkexec scripts/scx-switch.sh <request-fifo> <profile-name>
#
# Each request line is "SCHEDULER MODE" (e.g. "lavd gaming", "none default");
# the reply, written to <request-fifo>.ack, is "ok <kernel ops>" or
# "error <why>". "quit" ends the session. The profile is put back as it was
# on exit, whatever happened.
set -uo pipefail

FIFO=${1:?request fifo}
NAME=${2:?profile name}
PROFILES=/usr/share/falcond/profiles/user

# Everything that reaches a file or a command is allow-listed.
[[ "$NAME" =~ ^[A-Za-z0-9._+\ -]+$ ]] || { echo "bad profile name" >&2; exit 2; }
PROFILE="$PROFILES/$NAME.conf"
[ -f "$PROFILE" ] && [ ! -L "$PROFILE" ] || { echo "no such profile: $PROFILE" >&2; exit 2; }
[ -p "$FIFO" ] || { echo "not a fifo: $FIFO" >&2; exit 2; }

BACKUP=$(mktemp)
cp -p "$PROFILE" "$BACKUP"
restore() {
    cp -p "$BACKUP" "$PROFILE"; rm -f "$BACKUP"
    systemctl kill -s HUP falcond 2>/dev/null
}
trap restore EXIT INT TERM

MODES=" default gaming power latency server "
installed() { [ "$1" = none ] || [ -x "/usr/bin/scx_$1" ]; }

ops() { cat /sys/kernel/sched_ext/root/ops 2>/dev/null || echo none; }

while true; do
    if ! read -r sched mode < "$FIFO"; then
        sleep 0.2; continue
    fi
    [ "$sched" = quit ] && break
    if ! [[ "$sched" =~ ^[a-z0-9_]+$ ]] || ! installed "$sched" || [[ "$MODES" != *" $mode "* ]]; then
        echo "error rejected: $sched $mode" > "$FIFO.ack"; continue
    fi
    sed -i -e "s/^scx_sched = .*/scx_sched = $sched/" \
           -e "s/^scx_sched_props = .*/scx_sched_props = $mode/" "$PROFILE"
    systemctl kill -s HUP falcond
    # Verify against the kernel, not against the request.
    want=$([ "$sched" = none ] && echo none || echo "$sched")
    for _ in $(seq 1 50); do
        now=$(ops)
        if { [ "$want" = none ] && [ "$now" = none ]; } || [[ "$now" == "$want"* ]]; then break; fi
        sleep 0.2
    done
    echo "ok $(ops)" > "$FIFO.ack"
done
