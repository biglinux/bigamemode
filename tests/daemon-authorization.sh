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
# The bus runs the shipped bus policy, data/com.biglinux.BiGameMode.conf,
# included as it is into the system bus's own defaults (method calls denied
# unless a policy allows them). Only the rule that lets root own the name is
# repeated for the user running the test, who is not root.
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
BUS_POLICY="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)/data/com.biglinux.BiGameMode.conf"
if [[ ! -f "$BUS_POLICY" ]]; then
    echo "not found: $BUS_POLICY" >&2
    exit 2
fi

NAME=com.biglinux.BiGameMode
OBJECT=/com/biglinux/BiGameMode
# gdbus prints the error's name and message, so a refusal by the bus can be
# told from one by the helper; untranslated, so the text can be matched.
export LC_ALL=C

WORK="$(mktemp -d)"
cleanup() {
    [[ -n "${DAEMON_PID:-}" ]] && kill "$DAEMON_PID" 2>/dev/null
    [[ -n "${BUS_PID:-}"    ]] && kill "$BUS_PID" 2>/dev/null
    rm -rf "$WORK"
}
trap cleanup EXIT

# The defaults are those of the system bus (/usr/share/dbus-1/system.conf).
cat > "$WORK/bus.conf" <<EOF
<!DOCTYPE busconfig PUBLIC "-//freedesktop//DTD D-BUS Bus Configuration 1.0//EN"
 "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">
<busconfig>
  <type>system</type>
  <listen>unix:tmpdir=/tmp</listen>
  <auth>EXTERNAL</auth>
  <policy context="default">
    <allow user="*"/>
    <deny own="*"/>
    <deny send_type="method_call"/>
    <allow send_type="signal"/>
    <allow send_requested_reply="true" send_type="method_return"/>
    <allow send_requested_reply="true" send_type="error"/>
    <allow receive_type="method_call"/>
    <allow receive_type="method_return"/>
    <allow receive_type="error"/>
    <allow receive_type="signal"/>
    <allow send_destination="org.freedesktop.DBus"
           send_interface="org.freedesktop.DBus"/>
    <allow send_destination="org.freedesktop.DBus"
           send_interface="org.freedesktop.DBus.Introspectable"/>
  </policy>
  <!-- Stands in for the policy's user="root": the test runs as $(id -un). -->
  <policy user="$(id -un)">
    <allow own="$NAME"/>
  </policy>
  <include>$BUS_POLICY</include>
</busconfig>
EOF

ADDR="$(dbus-daemon --config-file="$WORK/bus.conf" --print-address --fork --print-pid=3 3>"$WORK/bus.pid")"
BUS_PID="$(cat "$WORK/bus.pid")"

DBUS_SYSTEM_BUS_ADDRESS="$ADDR" "$DAEMON" > "$WORK/daemon.log" 2>&1 &
DAEMON_PID=$!
# Wait for the name rather than a fixed time: a slow start would otherwise
# fail every check, and a fast one wastes the wait.
for _ in $(seq 50); do
    busctl --address="$ADDR" status "$NAME" >/dev/null 2>&1 && break
    kill -0 "$DAEMON_PID" 2>/dev/null || break
    sleep 0.1
done

fail=0
calls=0
# check LABEL EXPECTED INTERFACE.METHOD [ARG…] — arguments in GVariant text.
check() {
    local label="$1" expect="$2" method="$3"; shift 3
    local out
    out="$(gdbus call --address "$ADDR" --dest "$NAME" --object-path "$OBJECT" \
            --method "$method" "$@" 2>&1)"
    if [[ "$out" == *"$expect"* ]]; then
        echo "  ok    $label"
    else
        echo "  FAIL  $label"
        echo "        expected to contain: $expect"
        echo "        got: $out"
        fail=1
    fi
}
# A privileged call must be answered by the helper itself, with its own
# denial: the bus's "Rejected send message" would mean the bus policy no
# longer lets the method through, and the helper was never asked.
HELPER_DENIED="org.freedesktop.DBus.Error.AccessDenied: authorization"
denied() {
    local label="$1"; shift
    check "$label" "$HELPER_DENIED" "$@"
    calls=$((calls + 1))
}

echo "Bus policy (the shipped file):"
check "Ping is allowed (unauthenticated)" "('pong',)" "$NAME.Ping"
check "Peer is allowed" "()" org.freedesktop.DBus.Peer.Ping
check "Introspectable is allowed" "<node" org.freedesktop.DBus.Introspectable.Introspect
check "an unlisted interface is refused by the bus" \
      "AccessDenied: Rejected send message" org.example.NotListed.Anything
check "an unlisted method reaches the helper and is unknown to it" \
      "UnknownMethod" "$NAME.NotAMethod"

echo "Privileged methods (must all be refused by the helper):"
denied "SaveProfile"        "$NAME.SaveProfile" "'Cyberpunk2077.exe'" "'name = \"Cyberpunk2077.exe\"'"
denied "SetCpuGovernor"     "$NAME.SetCpuGovernor" "'performance'"
denied "SetCpuEpp"          "$NAME.SetCpuEpp" "'performance'"
denied "SetGpuDpmLevel"     "$NAME.SetGpuDpmLevel" "'card1'" "'auto'"
denied "SetVCacheMode"      "$NAME.SetVCacheMode" "'cache'"
denied "ApplyFalcondConfig" "$NAME.ApplyFalcondConfig" "'scx_sched = none'"
denied "DeleteProfile"      "$NAME.DeleteProfile" "'Cyberpunk2077.exe'"
denied "SetGameBackend"     "$NAME.SetGameBackend" true
denied "ReleaseGameBackend" "$NAME.ReleaseGameBackend"

echo "Authorization comes before validation (invalid arguments still get Access denied):"
denied "SaveProfile ../etc/cron.d"  "$NAME.SaveProfile" "'../../../../../etc/cron.d/pwn'" "'evil'"
denied "DeleteProfile ../etc/passwd" "$NAME.DeleteProfile" "'../../../etc/passwd'"
denied "SetGpuDpmLevel high"        "$NAME.SetGpuDpmLevel" "'../../card1'" "'high'"
denied "ApplyFalcondConfig script"  "$NAME.ApplyFalcondConfig" "'start_script = \"/tmp/x\"'"

# The helper's log says what it did with each call: one failed Polkit check
# per privileged call, no argument ever examined, nothing written.
sleep 0.2
checked="$(grep -c 'polkit check failed' "$WORK/daemon.log")"
if (( checked == calls )); then
    echo "  ok    each of the ${calls} privileged calls went to Polkit"
else
    echo "  FAIL  ${checked} Polkit checks logged for ${calls} privileged calls"
    fail=1
fi
if grep -qE 'rejected an invalid argument|saved|deleted|written|set$| set |switched|released' "$WORK/daemon.log"; then
    echo "  FAIL  the helper examined or acted on a call it had not authorized:"
    grep -E 'rejected an invalid argument|saved|deleted|written|set$| set |switched|released' "$WORK/daemon.log" | sed 's/^/        /'
    fail=1
else
    echo "  ok    no argument examined, nothing written"
fi

echo
if (( fail )); then
    echo "FAILED"
    echo "--- daemon log ---"
    cat "$WORK/daemon.log"
    exit 1
fi
echo "PASSED — every privileged request refused with Polkit unreachable"
