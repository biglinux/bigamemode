#!/usr/bin/env bash
#
# Run Big Game Mode's binaries on emulated processors, oldest first: the
# package is x86_64, so it must run on any x86-64 processor, not only on the
# builder's. falcond 2.0.14 built for x86-64-v3 died with SIGILL on a Sandy
# Bridge (issue #4); nothing here may do the same.
#
#   .github/scripts/legacy-cpu-test.sh <bigame-ui> <bigame-daemon>
#
# qemu-x86_64 (qemu-user) executes the program with the chosen CPU's
# instruction set and CPUID, so an instruction the model lacks raises
# SIGILL, and the program's own CPU detection sees the emulated CPU. A first
# check proves that: a program built around one BMI2 instruction must die
# on the Sandy Bridge model. Run as an ordinary user (the helper's test needs
# it); needs qemu-user, gcc, dbus and busctl.
set -Eeuo pipefail
# The SIGILLs below are expected: no core dumps of them in the working tree.
ulimit -c 0

ui="$(realpath -- "${1:?usage: $0 <bigame-ui> <bigame-daemon>}")"
daemon="$(realpath -- "${2:?usage: $0 <bigame-ui> <bigame-daemon>}")"
repo="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)"
# Beside the repository rather than in /tmp, which may be mounted noexec:
# the helper's wrapper below is executed.
work="$(mktemp -d -p "$repo" .legacy-cpu.XXXXXX)"
trap 'rm -rf -- "$work"' EXIT

failures=0
ok()   { echo "  ok    $*"; }
fail() { echo "  FAIL  $*"; failures=$((failures + 1)); }

# model:level the program's own detection must report.
models=(
    "Opteron_G1:1"   # AMD K8, 2003: SSE2, the x86-64 baseline
    "SandyBridge:2"  # Core i3-2120 of issue #4: AVX, no AVX2/BMI2
    "Haswell:3"      # first with AVX2, BMI1/2, FMA
    "EPYC-Milan:3"   # a current AMD part
)

echo "QEMU enforces the emulated instruction set:"
cat > "$work/bmi2.c" <<'EOF'
int main(void) {
    unsigned long x = 1, n = 3;
    __asm__ volatile("shlx %1, %0, %0" : "+r"(x) : "r"(n));
    return x == 8 ? 0 : 1;
}
EOF
gcc -O0 -o "$work/bmi2" "$work/bmi2.c"
set +e
qemu-x86_64 -cpu Haswell "$work/bmi2" 2>/dev/null; on_haswell=$?
qemu-x86_64 -cpu SandyBridge "$work/bmi2" 2>/dev/null; on_sandy=$?
set -e
if (( on_haswell == 0 && on_sandy == 128 + 4 )); then
    ok "SHLX runs on Haswell and dies with SIGILL on Sandy Bridge"
else
    fail "SHLX: exit $on_haswell on Haswell, $on_sandy on Sandy Bridge (expected 0 and 132)"
fi

echo "bigame-ui --diagnostics (bigame-core's detection, the report):"
for entry in "${models[@]}"; do
    model="${entry%%:*}" level="${entry##*:}"
    set +e
    qemu-x86_64 -cpu "$model" "$ui" --diagnostics >"$work/report.txt" 2>"$work/stderr.txt"
    rc=$?
    set -e
    if (( rc != 0 )); then
        fail "$model: exit $rc$( (( rc > 128 )) && echo " (signal $((rc - 128)))")"
        tail -n 5 "$work/stderr.txt" | sed 's/^/        /'
        continue
    fi
    isa="$(grep -m1 '^  ISA ' "$work/report.txt" || true)"
    if [[ "$isa" == *"x86-64-v$level "* ]]; then
        ok "$model: runs; reports ${isa#  ISA          }"
    else
        fail "$model: runs, but reports '${isa:-no ISA line}', expected x86-64-v$level"
    fi
done

echo "bigame-daemon refuses what it cannot authorize (tests/daemon-authorization.sh):"
for model in Opteron_G1 SandyBridge; do
    printf '#!/bin/sh\nexec qemu-x86_64 -cpu %s %q "$@"\n' "$model" "$daemon" >"$work/daemon-$model"
    chmod +x "$work/daemon-$model"
    if bash "$repo/tests/daemon-authorization.sh" "$work/daemon-$model" >"$work/authz.txt" 2>&1; then
        ok "$model: every privileged method refused"
    else
        fail "$model: the authorization test failed"
        tail -n 20 "$work/authz.txt" | sed 's/^/        /'
    fi
done

echo
if (( failures )); then
    echo "${failures} legacy CPU check(s) failed"
    exit 1
fi
echo "Runs on every x86-64 level, from the baseline up"
