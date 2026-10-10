# CPU compatibility

## Baseline

Big Game Mode's package is `arch=('x86_64')`. It has to run on **any x86-64
processor**: the x86-64 baseline (v1, SSE2), the same level as Arch's and
Manjaro's own packages. falcond, which Turbo depends on, has to be built the
same way.

| Level | Adds | Examples without the next level |
|---|---|---|
| x86-64 (v1) | SSE2 | Athlon 64, Core 2 |
| x86-64-v2 | SSE3, SSSE3, SSE4.1/4.2, POPCNT, CMPXCHG16B | Sandy Bridge, Ivy Bridge (they have AVX, not AVX2) |
| x86-64-v3 | AVX, AVX2, BMI1/2, FMA, F16C, LZCNT, MOVBE | Haswell, Zen 1–3 |
| x86-64-v4 | AVX-512 | Zen 4, Skylake-X |

A program built for a level runs only where the CPU has *every* extension of
that level. Otherwise it stops with SIGILL on the first instruction it lacks
(falcond 2.0.14 built for v3,
[issue #4](https://github.com/biglinux/bigamemode/issues/4)), or,
for LZCNT/TZCNT, silently gets a wrong result.

## Why not `-march=native` or a newer level

* **native** builds for the build machine's CPU. A builder with AVX2 makes a
  package that needs AVX2, and nothing in its name or `arch` says so.
* **x86-64-v3** excludes every Sandy and Ivy Bridge, older AMD parts and many
  low-power chips, still common among the people BigLinux is for.
* Neither buys anything here. Big Game Mode and falcond are control software,
  not number crunching. The one SIMD loop in falcond, measured under 180 times
  its normal load, is 0.5 % slower at the baseline, which amounts to nothing
  at its normal rate. Hot paths that do gain from newer instructions (Rust's
  `memchr`, glibc's string functions) check the CPU at run time and choose.

If BigLinux ever sets a higher minimum (x86-64-v2, say), that is a
distribution decision, to be written here and in the packages' `arch`, and
then checked like the baseline. A Sandy Bridge is v2.

## How it is enforced

| Where | What |
|---|---|
| `pkgbuild/PKGBUILD` | `-C target-cpu=x86-64` after the builder's `RUSTFLAGS`, so a `target-cpu=native` in makepkg.conf is overridden (the last one wins). |
| `.github/workflows/backend-tests.yml` | *Run workspace tests on a baseline x86-64 CPU*: every test, run by `qemu-x86_64 -cpu Opteron_G1` (SSE2 only). |
| `.github/workflows/build-package.yml` | *Check CPU compatibility (legacy x86-64)*: `.github/scripts/legacy-cpu-test.sh` runs the **packaged** binaries on emulated Opteron G1, Sandy Bridge, Haswell and EPYC-Milan. `bigame-ui --diagnostics` must run, and must report the emulated CPU's level, which also tests Big Game Mode's own detection. The helper's authorization test must pass on the oldest two. A positive control (a `SHLX` that must die on Sandy Bridge) proves the emulation would catch it. `x86-64-isa-report.py` logs what the binaries could ask of a CPU. |

The static report does not decide anything. Rust's std and `memchr` contain
AVX2 code they run only after a CPUID check, so a newer instruction in the
disassembly is not a bug by itself. Running the code on an old CPU is the
test.

## A falcond built for a newer level

falcond comes from another recipe, so a falcond built for x86-64-v3 can reach
an older processor. Zig's standard library uses `SHLX` (BMI2) while building
the process environment, before any falcond code runs, so such a build dies
at once and at every restart. systemd restarts it five times in about half a
second and leaves the unit `failed` with `start-limit-hit`, main process
`code=dumped, status=4/ILL`.

What Big Game Mode does about it:

| Where | Behaviour |
|---|---|
| Helper (`bigame-daemon/src/backend.rs`) | Turbo on resets a `failed` unit first (an explicit retry), then requires falcond to *stay* active, without restarts, for 2 s. If it does not, the unit is disabled, so it does not crash again at every boot. Turbo off resets a failed unit and still disables it. |
| Crash record (`bigame-core/src/falcond_compat.rs`) | A death by illegal instruction is recorded in `/var/lib/bigame-mode/game-backend.incompatible.json`: the binary's size and modification time (set by pacman from the package, so they name the build) and the processor's x86-64 level. While both match, falcond is not started again. A package update changes the binary and falcond is tried again; reinstalling the same build does not. |
| Turbo (`bigame-core/src/turbo.rs`) | With a recorded build, Turbo goes on without falcond, as on a system without it: the Booster applies the general settings and the power profile, and Turbo's state is the Booster's journal. Every surface that reads Turbo from falcond's unit asks `turbo::backend_governs` first. |
| Explanations | Home, Details, the tray and the support report say that the falcond package is built for a newer level, name the extensions this processor lacks (`bigame-core/src/isa.rs`), and do not suggest turning Turbo off and on again. An illegal instruction on an x86-64-v3 processor is described as an incompatibility *or* a damaged binary. |

A falcond that crashes later than the 2 s grace period is not caught when
Turbo starts; it shows up as a failed service, with its cause, on the same
pages.

### Building falcond for the baseline

Zig builds for the build machine's CPU unless told otherwise, and upstream's
Debian rules ask for `x86_64_v3`. The AUR recipe needs one option:

```diff
   DESTDIR=build zig build \
     --summary all \
     --prefix /usr \
-    -Doptimize=ReleaseFast
+    -Doptimize=ReleaseFast \
+    -Dcpu=baseline
```

`baseline` is Zig's name for the architecture's generic CPU, the x86-64
baseline here (the same code as `-Dcpu=x86_64`). Nothing in falcond's
dependencies forces another level, and its only SIMD (`isAllDigits` in the
`/proc` scan) is sized at compile time. Measured on a Ryzen 7 5700G scanning
2,000 processes every 50 ms (180 times the default rate), the baseline build
costs 0.5 % more CPU time; at the default interval falcond uses about 0.24 %
of one core either way.

## Checking by hand

```bash
# What a binary could ask of a processor (needs objdump):
python3 .github/scripts/x86-64-isa-report.py /usr/bin/falcond
python3 .github/scripts/x86-64-isa-report.py /usr/bin/bigame-ui

# Run it as an older processor (qemu-user):
qemu-x86_64 -cpu SandyBridge /usr/bin/bigame-ui --diagnostics | grep ISA
qemu-x86_64 -cpu SandyBridge /usr/bin/falcond --oneshot      # "signal 4" = SIGILL

# This processor:
bigame-ui --diagnostics | grep ISA
```

For a program with no runtime dispatch, such as falcond, the static report is
a verdict: it must say `x86-64-v1`.

## Avoiding regressions

* No `-C target-cpu`, `-march`, `-mtune=native`, `target-feature` or Zig
  `-Dcpu` other than the baseline in this repository, its CI or its package.
  `grep -RniE 'target-cpu|march=|x86_64_v[34]|x86-64-v[34]' --exclude-dir=target`
  should find only documentation and the pin above.
* A dependency that uses newer instructions without checking for them fails
  the emulated test runs.
* falcond comes from another recipe. After a falcond update, check it with
  the two commands above. biglinux-stable's 2.0.14-1 (built 2026-10-03) was
  still x86-64-v3.
