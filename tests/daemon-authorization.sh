#!/usr/bin/env bash
#
# Integration test: the privileged helper must refuse every privileged request
# it cannot authorize, and must refuse it *before* acting.
#
# The helper is started on a private D-Bus instance, as an ordinary user, with
# no Polkit reachable on that bus. That is the fail-closed case, and it is the
# one that matters most: a helper that grants root because its authorization
# service is unavailable is worse than one that stops working.
#
# Requires no root and installs nothing.
#
#   ./tests/daemon-authorization.sh [path-to-bigame-daemon]
#
set -uo pipefail

DAEMON="${1:-bigame-engine/target/debug/bigame-daemon}"
if [[ ! -x "$DAEMON" ]]; then
    echo "not found: $DAEMON" >&2
    echo "build it first: cargo build -p bigame-daemon" >&2
    exit 2
fi

WORK="$(mktemp -d)"
cleanup() {
    [[ -n "${DAEMON_PID:-}" ]] && kill "$DAEMON_PID" 2>/dev/null
    [[ -n "${BUS_PID:-}"    ]] && kill "$BUS_PID" 2>/dev/null
    rm -rf "$WORK"
}
trap cleanup EXIT

cat > "$WORK/bus.conf" <<'EOF'
<!DOCTYPE busconfig PUBLIC "-//freedesktop//DTD D-BUS Bus Configuration 1.0//EN"
 "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">
<busconfig>
  <type>system</type>
  <listen>unix:tmpdir=/tmp</listen>
  <policy context="default">
    <allow own="*"/>
    <allow send_destination="*"/>
    <allow receive_sender="*"/>
    <allow send_type="method_call"/>
    <allow send_type="signal"/>
    <allow send_type="method_return"/>
    <allow send_type="error"/>
  </policy>
</busconfig>
EOF

ADDR="$(dbus-daemon --config-file="$WORK/bus.conf" --print-address --fork --print-pid=3 3>"$WORK/bus.pid")"
BUS_PID="$(cat "$WORK/bus.pid")"

DBUS_SYSTEM_BUS_ADDRESS="$ADDR" "$DAEMON" > "$WORK/daemon.log" 2>&1 &
DAEMON_PID=$!
sleep 2

fail=0
check() {
    local label="$1" expect="$2"; shift 2
    local out
    out="$(busctl --address="$ADDR" call com.biglinux.BiGameMode \
            /com/biglinux/BiGameMode com.biglinux.BiGameMode "$@" 2>&1)"
    if [[ "$out" == *"$expect"* ]]; then
        echo "  ok    $label"
    else
        echo "  FAIL  $label"
        echo "        expected to contain: $expect"
        echo "        got: $out"
        fail=1
    fi
}

echo "Liveness (unauthenticated, must succeed):"
check "Ping" "pong" Ping

echo "Privileged methods (must all be refused):"
check "SaveProfile"        "Access denied" SaveProfile ss "Cyberpunk2077.exe" 'name = "x"'
check "SetCpuGovernor"     "Access denied" SetCpuGovernor s "performance"
check "SetCpuEpp"          "Access denied" SetCpuEpp s "performance"
check "SetGpuDpmLevel"     "Access denied" SetGpuDpmLevel ss "card1" "high"
check "SetVCacheMode"      "Access denied" SetVCacheMode s "cache"
check "ApplyFalcondConfig" "Access denied" ApplyFalcondConfig s "scx_sched = none"
check "DeleteProfile"      "Access denied" DeleteProfile s "Cyberpunk2077.exe"

echo "Audit SEC-02 payloads (path traversal, must be refused and write nothing):"
check "SaveProfile ../etc/cron.d"   "Access denied" \
      SaveProfile ss "../../../../../etc/cron.d/pwn" "evil"
check "SaveProfile ../etc/systemd"  "Access denied" \
      SaveProfile ss "../../../../../etc/systemd/system/pwn.service" "evil"
check "DeleteProfile ../etc/passwd" "Access denied" \
      DeleteProfile s "../../../etc/passwd"

for path in /etc/cron.d/pwn.conf /etc/systemd/system/pwn.service.conf; do
    if [[ -e "$path" ]]; then
        echo "  FAIL  privilege escalation: $path was created"
        fail=1
    else
        echo "  ok    nothing written to $path"
    fi
done

echo
if (( fail )); then
    echo "FAILED"
    echo "--- daemon log ---"
    cat "$WORK/daemon.log"
    exit 1
fi
echo "PASSED — every privileged request refused with Polkit unreachable"
