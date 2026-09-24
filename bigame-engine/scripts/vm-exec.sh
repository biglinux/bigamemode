#!/usr/bin/env bash
# Run a command inside a libvirt guest through the QEMU guest agent.
#
# Written because the lab VM's sshd was not running and there was no way in.
# The guest agent is: libvirt exposes a virtio channel to the guest, and the
# agent on the other side will execute a command and hand back its output. No
# network, no credentials, no sshd -- which is exactly what makes it the right
# tool for bringing an unreachable guest back up, and for a first contact with
# one whose configuration is unknown.
#
# The point of testing here at all is Rule 66: not optimising for one computer.
# This guest has no cpufreq directory, a virtio GPU with no DPM control, and a
# DRM tree whose first card is card1 -- every one of which is an assumption the
# host machine would let pass unnoticed.
#
# Usage: VM='<domain>' vm-exec.sh '<shell command>'
set -uo pipefail

VM=${VM:-BigLinux Teste}
CMD=${1:?usage: vm-exec.sh '<shell command>'}
CONNECT=${LIBVIRT_DEFAULT_URI:-qemu:///system}
TIMEOUT=${TIMEOUT:-240}

agent() { virsh -c "$CONNECT" qemu-agent-command "$VM" "$1" 2>&1; }

# The command is passed as JSON, so it has to be encoded rather than quoted --
# a command containing a quote would otherwise produce invalid JSON, and the
# failure would look like a guest problem rather than a shell one.
request=$(python3 -c '
import json, sys
print(json.dumps({"execute": "guest-exec", "arguments": {
    "path": "/bin/bash", "arg": ["-lc", sys.argv[1]], "capture-output": True}}))' "$CMD")

pid=$(agent "$request" | grep -o '"pid":[0-9]*' | cut -d: -f2)
if [ -z "$pid" ]; then
    echo "vm-exec: the guest agent did not accept the command." >&2
    echo "  Is the domain running, and does it have org.qemu.guest_agent.0?" >&2
    exit 1
fi

status=''
for _ in $(seq 1 "$TIMEOUT"); do
    status=$(agent "{\"execute\":\"guest-exec-status\",\"arguments\":{\"pid\":$pid}}")
    case $status in *'"exited":true'*) break ;; esac
    sleep 1
done

# Both streams are base64 in the reply. Exit with the guest's status, so a
# failing command in the guest fails here too.
python3 -c '
import base64, json, sys
try:
    reply = json.load(sys.stdin)["return"]
except (ValueError, KeyError):
    sys.stderr.write("vm-exec: the command did not finish within the timeout\n")
    sys.exit(124)
for stream, out in (("out-data", sys.stdout), ("err-data", sys.stderr)):
    if reply.get(stream):
        out.write(base64.b64decode(reply[stream]).decode(errors="replace"))
sys.exit(reply.get("exitcode", 0))' <<<"$status"
