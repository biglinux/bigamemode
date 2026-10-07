# Issue #4: falcond dies with SIGILL on CPUs without BMI2

[biglinux/bigamemode#4](https://github.com/biglinux/bigamemode/issues/4):
on an Intel Core i3-2120 (Sandy Bridge), falcond 2.0.14-1 stops the moment
it starts, systemd gives up restarting it, and Turbo does not stay on.

This page records how the cause was found, what was changed, and how each
claim was checked. Nothing in the application reads it.

## Summary

| | |
|---|---|
| **Cause** | BigLinux's `falcond-2.0.14-1` is built for **x86-64-v3** (AVX2, BMI1/2, FMA, MOVBE, LZCNT). Zig's standard library uses `SHLX` (BMI2) while building the process environment, before any falcond code runs, and a Sandy Bridge has no BMI2. |
| **Fix, falcond** | Build it with `-Dcpu=baseline` (the x86-64 baseline), as a package marked `x86_64` should be: [docs/falcond/PKGBUILD-baseline.patch](falcond/PKGBUILD-baseline.patch), `pkgrel` 2. The build costs **0.5 % more CPU** in a stress test and nothing measurable at the default polling interval. |
| **Fix, Big Game Mode** | Two bugs in the helper made the crash worse. It took falcond as started before it crashed, so Turbo applied the Booster and left falcond enabled. It also could not turn Turbo off while falcond was failed, so the unit stayed enabled and crashed again at every boot. SIGILL is now detected and explained, and the package is pinned to the x86-64 baseline. |
| **Prevention** | CI runs every test on an emulated 2003 Opteron (SSE2 only), and the packaged binaries on emulated Opteron, Sandy Bridge, Haswell and EPYC-Milan. |

## The report

| | |
|---|---|
| CPU | Intel Core i3-2120, Sandy Bridge, x86_64: SSE4.2 and AVX, no AVX2, BMI1, BMI2, FMA, F16C, LZCNT, MOVBE |
| System | BigLinux, falcond 2.0.14-1 (biglinux-stable), bigame-mode 2.2.0-1 |
| journal | `falcond.service: Main process exited, code=dumped, status=4/ILL` … `Start request repeated too quickly` … `Failed with result 'start-limit-hit'` |
| GDB | `SIGILL, Illegal instruction` at `0x105de47: shlx %r14,(%rcx,%rax,8),%r15` |

`SHLX` is a BMI2 instruction. Intel added BMI2 with Haswell (2013) and AMD with
Excavator (2015). Sandy Bridge (2011) and Ivy Bridge (2012) have AVX but not
AVX2 or BMI2.

### AVX, AVX2, BMI2 and the x86-64 levels

| Level | Adds | First Intel / AMD |
|---|---|---|
| x86-64 (v1) | SSE2, the baseline of every x86-64 CPU | Pentium 4 / Athlon 64 |
| x86-64-v2 | SSE3, SSSE3, SSE4.1, SSE4.2, POPCNT, CMPXCHG16B | Nehalem / Bulldozer |
| x86-64-v3 | **AVX, AVX2, BMI1, BMI2, FMA, F16C, LZCNT, MOVBE** | Haswell / Excavator |
| x86-64-v4 | AVX-512 (F, BW, CD, DQ, VL) | Skylake-X / Zen 4 |

AVX (Sandy Bridge) is not AVX2. A level needs *every* extension it lists, so a
Sandy Bridge is x86-64-v2 with AVX, and anything built for v3 may use
instructions it does not have. `LZCNT` and `TZCNT` are worse than BMI2: an
older CPU runs them as `BSR`/`BSF` and returns a wrong result instead of
stopping.

## Finding the cause

**Where BigLinux's falcond comes from.** `pacman -Si falcond` shows
`biglinux-stable`, version 2.0.14-1, packager "BigLinux Package Build". Its
`.BUILDINFO` has `pkgbuild_sha256sum = 92ccea25…`. That matches neither the
AUR recipe (`3af5c3d3…` at 2.0.14, `4132b59c…` now) nor chaotic-aur's. So
BigLinux builds from its own recipe, which is not in a public repository we
could find (biglinux, BigLinuxAur, BigLinux-Package-Build). The AUR recipe
passes no `-Dcpu`. Upstream's `falcond/debian/rules` (PikaOS) passes
`-Dcpu=x86_64_v3`.

**What the published binary is.** `.github/scripts/x86-64-isa-report.py`
disassembles a binary and sorts every instruction above the baseline into the
level that introduced it. On the installed binary, and on the one
biglinux-stable serves today (rebuilt 2026-10-03, still 2.0.14-1):

```text
/usr/bin/falcond: 232244 instructions, highest level x86-64-v3
  v3  AVX          5267  (vmovups 2146, vmovaps 518, vzeroupper 489, …)
  v3  AVX2          758  (vpermpd 226, vpermq 156, …)
  v3  BMI2          343  (mulx 126, shlx 93, rorx 65, shrx 34, bzhi 24)
  v3  MOVBE         122
  v3  LZCNT          24
```

falcond 2.0.14 rebuilt from the same tarball (sha256 `7cef1d62…`, as the AUR
recipe checks) with Zig 0.16.0:

| `-Dcpu=` | Instructions | Highest level | BMI2 |
|---|---|---|---|
| `x86_64_v3` | 232,244, the same count in every class as BigLinux's | v3 | 343 |
| (none: native, a Zen 3 here) | 234,697 (adds `bextr`) | v3 | 368 |
| `x86_64_v2` | 229,188 | v2 | 0 |
| `x86_64` / `baseline` (byte-identical `.text`) | 230,930 | **v1** | 0 |

BigLinux's binary has the v3 build's instruction mix exactly, not the native
one's. It is not byte-identical, because embedded paths differ, and the two
BigLinux builds of 2.0.14-1 differ from each other the same way.

**Is anything in falcond's dependencies forcing an ISA?** No. `otter_conf`,
`otter_desktop` and `otter_utils` call `standardTargetOptions`, but falcond
passes its own `target` to each `b.dependency()`, which overrides it.
`otter_desktop`'s vendored PipeWire picks AVX2/FMA code paths at compile time,
from the target, and falcond builds it with `enable_pipewire = false`. `aro`
and `translate_c` are build-time tools that never ship. falcond's own SIMD is
`scanner.zig`'s `@Vector(std.simd.suggestVectorLength(u8))` in `isAllDigits`.
That is compile-time too: 32 bytes for v3, 16 for the baseline. Nothing does
runtime dispatch, so the target alone decides.

**Reproducing the crash.** `qemu-x86_64 -cpu SandyBridge` runs a program with
Sandy Bridge's instruction set and raises SIGILL on anything else:

```text
$ qemu-x86_64 -cpu SandyBridge out-v3/bin/falcond --oneshot
qemu: uncaught target signal 4 (Illegal instruction) - core dumped     (exit 132)

$ gdb (attached through qemu's gdbstub)
Program received signal SIGILL, Illegal instruction.
array_hash_map.IndexHeader.alloc (new_bit_index=62) at /usr/lib/zig/std/array_hash_map.zig:1682
=> 0x105e327 <start.main+1991>:	shlx   %r14,(%rcx,%rax,8),%r15
#1 array_hash_map.Custom([]const u8,[]const u8,process.Environ.Map.EnvNameHashContext,…).ensureTotalCapacityContext
```

This is the instruction and operands of the report. Only the address differs,
with the build. It sits in Zig's standard library, building the environment
map at start-up, which is why falcond dies at once and every restart dies the
same way.

**What systemd does with it** (systemd 261 in a container, falcond's own unit:
`Restart=on-failure`, the default 100 ms delay and start limit of 5 in 10 s):

| Moment | `ActiveState` | `Result` | `ExecMainCode` / `Status` | `NRestarts` |
|---|---|---|---|---|
| right after `StartUnit` | `active` | | | 0 |
| ~0.5 s later | `failed` | `start-limit-hit` | 3 (dumped) / 4 (SIGILL) | 5 |
| after `StopUnit` | still `failed` | `start-limit-hit` | 3 / 4 | 5 |
| after `ResetFailedUnit` | `inactive` | `success` | **still 3 / 4** | |

The unit file stays `enabled` throughout.

## What Big Game Mode did with it (before)

The helper (`bigame-daemon`) controls falcond for Turbo. Replaying the issue
with the helper from `main` (3273fd4), falcond 2.0.14-v3 on an emulated Sandy
Bridge, through the same calls the Home button makes:

```text
--- turbo on
  Verified   GameBackend   [falcond] running (systemd reports it active; falcond has not published its status yet)
  (the Booster's plan applied)
   systemd: ActiveState=failed UnitFileState=enabled Result=start-limit-hit NRestarts=5 ExecMainCode=3 ExecMainStatus=4
--- turbo off
  Failed     GameBackend   [falcond] falcond.service is still failed; not disabled yet
   systemd: ActiveState=failed UnitFileState=enabled …
```

1. **Turbo was reported on, and the Booster applied, with falcond dead.**
   falcond is `Type=simple`, so systemd calls it `active` as soon as its
   process exists, and the helper took the first `active` as success. This is
   the issue's "Turbo is applied at first, then falcond fails". Turbo's own
   rule ("a falcond that does not start leaves Turbo off, and nothing of
   Turbo is applied") was defeated by that race.
2. **Turbo could not be turned off.** Stopping a failed unit leaves it
   `failed`, and the helper stopped there instead of disabling it. falcond
   stayed **enabled**, and crashed again at every boot.
3. Everything the user saw said "falcond's service failed; turn Turbo off and
   on again". With SIGILL that repeats the crash.

## What changed

### falcond's package (the cause)

[docs/falcond/PKGBUILD-baseline.patch](falcond/PKGBUILD-baseline.patch): the
AUR recipe with `-Dcpu=baseline` and `pkgrel=2`. `ReleaseFast` stays: the
optimisation level was never the problem.

* `baseline` is Zig 0.16's name for the architecture's generic CPU. On x86_64
  it is the x86-64 baseline, the same code as `-Dcpu=x86_64`, and it keeps
  namcap from flagging a literal `x86_64`.
* BigLinux's own recipe was not found. The patch has to be applied there, with
  the same two changes. To check a published package:

  ```bash
  python3 .github/scripts/x86-64-isa-report.py /usr/bin/falcond   # must say x86-64-v1
  qemu-x86_64 -cpu SandyBridge /usr/bin/falcond --oneshot         # must not say "signal 4"
  ```

Built with makepkg in a clean `archlinux:base-devel` container (`--nodeps`,
as `falcond-profiles` is only in the AUR): `falcond-2.0.14-2-x86_64.pkg.tar.zst`,
x86-64-v1, linked only to glibc. namcap shows the AUR recipe's own warnings
and none new.

### Big Game Mode

| Where | Change |
|---|---|
| `bigame-core/src/systemd.rs` | Reads `Result`, `ExecMainCode`, `ExecMainStatus` and `NRestarts` with the unit state, over D-Bus as before. `Failure` classifies them: `IllegalInstruction` (signal 4, killed or dumped), another signal, an exit code, or another result. The fields count only while `Result` is not `success`, because systemd keeps them after a reset. Adds `ResetFailedUnit`. |
| `bigame-core/src/isa.rs` (new) | `CpuIsa`: SSE2 … AVX-512 through `std::arch::is_x86_feature_detected!`, which includes the OS check for AVX state, plus the x86-64 level they add up to. |
| `bigame-daemon/src/backend.rs` | **On:** a unit left `failed` is reset first, because this is an explicit retry and it clears the start limit. After `active`, falcond has to *stay* active, without restarts, for 2 s. If it does not, systemd finishes its restarts (the unit stays `failed` with the cause), and the unit is disabled. **Off:** a `failed` unit is reset and still disabled. Never in a loop, only on the user's request. |
| `bigame-core/src/turbo.rs` | `explain_backend_failure`: a title, what happened, what to do, and whether retrying helps. An illegal instruction is called a CPU incompatibility of the package only when the processor is below x86-64-v3, which is the evidence. Otherwise it says "a CPU instruction-set incompatibility, or a damaged binary", without naming BMI2. Turbo on checks once more that falcond still runs before applying the Booster, and puts the explanation in its report. |
| Home, Details (Overview, Problems, Performance), the tray | Show that explanation. "Turn Turbo off and on again" is offered only where it can help. |
| Support report | The CPU's ISA, and falcond's last run with its cause. |
| `pkgbuild/PKGBUILD` | `-C target-cpu=x86-64` is added after the builder's `RUSTFLAGS`, so a `target-cpu=native` in its makepkg.conf cannot make the package need the builder's CPU. The last `-C target-cpu` wins (`rustc --print cfg` shows no AVX2 with `native` before it, AVX2 with it after). |
| CI | See [CPU-COMPATIBILITY.md](CPU-COMPATIBILITY.md). |

Not changed: falcond's unit (`Restart=`, `StartLimitBurst=`). Allowing more
restarts would only crash more often. Booster, presets, power profile and
rollback order are unchanged. The only change to them is that the rollback
now really runs.

## Tests

### Big Game Mode, after the change (same replay)

```text
### falcond v3 build on an emulated Sandy Bridge (Turbo client on Sandy Bridge too)
Turbo ON
  Failed   GameBackend [falcond] falcond crashed on a CPU instruction this processor does not have
           (illegal instruction). The installed falcond package appears to be built for a newer
           x86-64 level: this processor is x86-64-v2, without AVX2, BMI1, BMI2, FMA, F16C, LZCNT,
           MOVBE. Update or reinstall falcond with a build for this processor. Turning Turbo off and
           on again does not help: the same package crashes the same way.
  Skipped  Booster     [Booster] not applied: Turbo did not come on
   systemd: ActiveState=failed UnitFileState=disabled Result=start-limit-hit ExecMainStatus=4
Turbo OFF
  Restored GameBackend [falcond] stopped and disabled; …
   systemd: ActiveState=inactive UnitFileState=disabled

Support report:
  falcond unit failed · disabled · 5 automatic restart(s)
  falcond run  result start-limit-hit · main process code=dumped status=4
  last failure SIGILL (illegal instruction) · systemd stopped restarting it (start-limit-hit) ·
               this CPU is x86-64-v2 (no AVX2/BMI1/BMI2/FMA/F16C/LZCNT/MOVBE): falcond appears built for a newer level
  ISA          x86-64-v2 · SSE2 yes · SSE4.1 yes · SSE4.2 yes · AVX yes · AVX2 no · BMI1 no · BMI2 no · FMA no

### falcond baseline build on an emulated Sandy Bridge
Turbo ON:  Verified, systemd active, NRestarts=0, still active 3 s later; 0 SIGILL/restart lines in its journal
### falcond baseline build, native
Turbo ON:  Verified "running, 12 profiles loaded (desktop set)"; Turbo OFF: Restored, inactive, disabled
```

With the Turbo client running natively on the host (a Zen 3, which has BMI2),
the same failure is described without naming BMI2, as designed. The power
profile was `balanced` before a failed Turbo on and `balanced` after it.

Under qemu-user, falcond does not receive SIGTERM through its `signalfd`, so
stopping it waits for systemd's timeout. That is an emulator limitation: run
natively, the same binary stops on SIGTERM with exit status 0.

### falcond on emulated processors

As root in a container with a system bus and power-profiles-daemon. Each run
loads the 12 profiles, sees a process named like one (`factorio`), stays up
for 18 s, and is then stopped:

| Build | Opteron G1 (v1) | Sandy Bridge | Ivy Bridge | Haswell | EPYC-Milan |
|---|---|---|---|---|---|
| v3 (as published) | | **SIGILL at start** | | runs, profile seen | |
| baseline | runs, profile seen | runs, profile seen | runs, profile seen | runs, profile seen | runs, profile seen |

falcond's 58 unit tests pass in both builds (`zig build test`).

### Performance

falcond's only SIMD is the `/proc` scan. Its own CPU time was measured while
scanning 2,000 processes every 50 ms, 180 times its default rate, for 20 s
per run. The two builds ran natively on a Ryzen 7 5700G and were alternated
(A B A B), with the first round discarded:

| Build | CPU time per 20 s (5 runs) | Spread |
|---|---|---|
| x86-64-v3 | 8638 ms | 0.03 % |
| baseline | 8684 ms | 0.30 % |

That is **+0.53 %** for the baseline. It is real (Welch t = 3.9) and small:
21.71 ms against 21.60 ms per full scan. At the default 9 s interval falcond
uses about 0.24 % of one core in either build. Runtime dispatch would add
complexity for nothing measurable.

### The rest

Covered by the CI and the commands in
[ci-workflows-audit.md](ci-workflows-audit.md): rustfmt, Clippy, 927 tests
(including the new ones for SIGILL, start-limit-hit, a reset failure, CPU
levels with and without AVX2/BMI2, the explanations, the Support report and
Home), the same tests on an emulated Opteron G1, rustdoc, translations (29
catalogues, the 12 new messages translated) and the package build.

## Limits

* **No real Sandy Bridge was available.** The processor was emulated by QEMU
  (user mode for the binaries, with systemd 261 and the real helper in a
  container). The emulation's fidelity was checked both ways: `SHLX` runs on
  the Haswell model and stops with SIGILL on the Sandy Bridge one. A run on
  the reporter's machine is the remaining confirmation.
* **BigLinux's falcond recipe** has to receive the patch. Until a fixed
  falcond is published, Big Game Mode explains the failure but cannot make
  Turbo's per-game part run on those processors. The rest of Turbo, without
  falcond, is what a system without falcond gets.
* A falcond that crashes later than the 2 s grace period is not caught when
  Turbo starts. It shows up as a failed service, with its cause, on the same
  pages.
