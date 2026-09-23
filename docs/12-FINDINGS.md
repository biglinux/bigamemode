# Findings

What was measured on 2026-09-23, on one machine, with the method in
`docs/11-BENCHMARK-LAB.md`. Everything here is a measurement or an explicitly
labelled absence. Nothing is an estimate.

**Machine.** AMD Ryzen 7 5700G (8 cores, `amd-pstate-epp`), Radeon RX 9060 XT
(Navi 44, `amdgpu`, 170 W cap), 3440×1440 and 2560×1080 displays, kernel
7.2.6-x64v3-xanmod1. Fingerprint `02db452880e054ab`.

---

## 1. The Booster made this machine slower, and one setting did it

The headline result, and the one the whole exercise existed to be able to
state honestly.

| Configuration | Mean fps | Spread | vs baseline | Verdict |
|---|---:|---:|---:|---|
| baseline | 298.4 | 3.2 % | — | — |
| `cpu_governor` → performance | 304.5 | 2.4 % | +2.1 % | no change |
| `gpu_dpm_level` → high | 275.9 | 4.7 % | **−7.5 %** | **slower** |
| full Booster | 281.0 | 0.7 % | **−5.8 %** | **slower** |

Four measured runs per arm, one warm-up discarded, arms alternated. Both
regressions clear the run-to-run spread of their arms and Welch's t-test at
95 % (t = 2.81 against 2.45, and t = 3.57 against 2.78). The CPU governor
change does not clear either bar and is therefore reported as no change, not
as a 2 % gain.

Raw runs, in the order taken:

```
baseline       304.9  287.7  307.7  293.1
cpu_governor   294.0  305.4  307.9  310.7
gpu_dpm_level  280.4  256.7  281.6  284.8
booster        279.0  280.5  280.8  283.9
```

### Why: "high" is not "fastest"

Telemetry sampled twice a second through every run explains it completely.

| Configuration | sclk mean | sclk range | Power | Temp | GPU busy |
|---|---:|---:|---:|---:|---:|
| baseline (`auto`) | 3042 MHz | 1607–3331 | 141 W | 60.6 °C | 87 % |
| `cpu_governor` (`auto`) | 3147 MHz | 1671–3350 | 150 W | 62.3 °C | 87 % |
| `gpu_dpm_level` (`high`) | 2640 MHz | 2565–2668 | 101 W | 58.7 °C | 88 % |
| full Booster (`high`) | 2642 MHz | 2603–2664 | 101 W | 58.0 °C | 88 % |

`power_dpm_force_performance_level=high` does not mean "run as fast as
possible". It pins the card to its highest **fixed** DPM state — 2.7 GHz on
this card, per `pp_dpm_sclk` — and takes the firmware's opportunistic boost
algorithm out of the loop. That algorithm reaches 3.35 GHz. Forcing "high"
therefore costs roughly 700 MHz of peak clock and leaves 40 W of a 170 W
budget unused, while the GPU stays equally busy. The name says one thing; the
silicon does another.

This is the case for measuring rather than reasoning. The setting is named for
performance, is widely recommended for gaming, does exactly what its
documentation says, and is a 7.5 % regression on this hardware.

### What changed as a result

The planner now consults the calibration before any other consideration about
a knob — including "it already holds the value we would write", because a
setting measured to be slower is not optimal just because it happens to be
set. On this machine the Booster now reports:

```
MeasuredHarmful { knob: "GPU power level (card1)",
  detail: "measured on this machine against 2026-09-23-supertuxkart-gpu-bound:
           7.5% slower, above the 4.7% run-to-run spread and significant at
           95% (Welch's t = 2.81 against a 2.45 threshold)" }
```

Only a measured regression removes a knob. "No measurable difference" is not
evidence of harm, so the CPU governor change still stands on the planner's
other reasoning.

**Scope.** This is one card on one kernel against one workload. It is not a
claim about every Radeon, and the calibration is tied to a hardware
fingerprint precisely so it is never treated as one.

---

## 2. The workload's settings decide what it can measure

The first attempt at this comparison found nothing, and the reason is worth
recording because it is an easy mistake to repeat.

SuperTuxKart's benchmark runs at whatever the config file says, which here was
1024×768 with shadows, SSAO and MLAA off. At those settings telemetry showed
the GPU at **50 % utilisation and 1686 MHz of a possible 2700** — the workload
was limited by CPU and driver submission, not by the GPU. No GPU-side setting
could have shown an effect through it, whatever its true value.

Reconfigured to 3440×1440 with shadows at 2048, SSAO, MLAA and full geometry,
the GPU sits at 85 % and 2574 MHz, and the frame rate falls from ~720 to ~300.
That is the configuration the result above was measured in.

A benchmark that is not bounded by the thing you are changing will report "no
difference" with complete confidence, and be wrong.

Before that, the benchmark had to be uncapped at all: stock `max_fps=120` with
vsync on pins it near 160 fps. Lifting the cap took the same 38-second replay
from 6 101 frames to 27 871. The provider now refuses to run against a capped
configuration rather than producing a meaningless comparison.

---

## 3. The instrument was perturbing the measurement

The first telemetry sampler called `cat` once per sensor plus `date` and `awk`
per sample — eight processes, four times a second, about **1400 forks per
run**. Against a session measured without it, the run-to-run spread rose from
**1.4 % to nearly 6 %**, larger than most differences a benchmark exists to
detect.

It was the stability check that caught it: the matrix run came back
`INCONCLUSIVE — the runs within an arm disagree too much to compare (variation
4.6 % and 5.9 %, above the 5 % ceiling)`. A harness without that check would
have reported a confident number from the same data.

The sampler was rewritten using only shell builtins — `read` for sensors,
`EPOCHREALTIME` for the clock, shell arithmetic, and a timed read on an empty
pipe in place of `sleep`, which is not a builtin either. Zero forks in the
loop. That session's results are preserved at
`benchmarks/2026-09-23-supertuxkart-cpu-bound/` and marked unusable rather than
deleted.

---

## 4. What can and cannot be benchmarked here

| Workload | Status | Detail |
|---|---|---|
| SuperTuxKart 1.5 | **Automated** | Deterministic replay, exits by itself, per-frame CSV. The only fully unattended workload found. |
| Shadow of the Tomb Raider | **Needs manual start** | Installed. Windows build under Proton; its benchmark is behind Options → Display and the game exposes no flag for it. |
| Cyberpunk 2077 | **Needs manual start** | Installed. Benchmark behind Settings → Graphics. |
| Rise of the Tomb Raider | **Needs manual start** | Installed. The Feral port accepts `-benchmark`, but its launcher window opens first and `-nolauncher` does not suppress it. |
| Tomb Raider (2013) | **NOT TESTED** | Cannot start. The Feral port is a 64-bit binary whose bundled `lib/` holds only 32-bit objects; `libicui18n.so.51` is absent from the system too. |
| Unigine Superposition | **NOT TESTED** | `/opt/unigine-superposition/bin` is `drwxr-x--- root root`. A packaging defect, not a hardware limit. |
| vkmark, glmark2, Phoronix | **NOT TESTED** | Not installed; installing them needs a package-manager authentication this session could not complete. |
| sched-ext arm | **NOT TESTED** | `scx_loader` is not running and `/sys/kernel/sched_ext/state` is `disabled`. Enabling it needs root, which was unavailable here; the daemon exposes no method for it. |
| glxgears, vkcube | **Sanity check only** | Confirm the driver stack is alive. Never treated as evidence about game performance. |

"Needs manual start" is reported distinctly from "unavailable" on purpose. The
benchmarks are there and are good; what cannot be automated is the starting.

falcond was active throughout but inert: `ACTIVE_PROFILE: None`, and it has no
profile matching SuperTuxKart. It was therefore not an uncontrolled variable.
Separately, its config has `profile_mode = handheld` on a desktop machine,
which is worth a look but was not changed here.

---

## 5. Hardware detection, verified somewhere it could have been wrong

The lab VM (BigLinux on Manjaro, kernel 6.18.49, reached through the QEMU guest
agent after enabling its sshd) is a machine where this codebase's assumptions
could have failed silently:

- **no `cpufreq` directory at all** → governors and EPP come back as empty
  lists, not a crash and not an invented default
- **DRM tree starting at `card1`**, no `card0` → `card1` correctly chosen as
  the render GPU
- **virtio GPU, no DPM control, no VRAM figure** → recorded as `null`, never as
  zero, so "not available" cannot be read as a measurement
- **different fingerprint** (`8deb360402571748`) → neither machine would apply
  the other's calibration
- every benchmark provider correctly `NotInstalled`; the inventory carries no
  hostname or username there either

Its GPU is virtual, so no graphics result from it would mean anything, and none
was taken.

---

## 6. What this does not establish

- **One workload.** SuperTuxKart is a kart racer. Its CPU/GPU balance is not a
  AAA title's, and a setting that hurts here could help elsewhere. The four
  installed AAA benchmarks would settle it; none can be started unattended.
- **One machine.** Everything in section 1 is about a Radeon RX 9060 XT on one
  kernel. The fingerprint exists so the result is never generalised past it.
- **Frame generation, latency, 1 % lows from this workload.** SuperTuxKart
  publishes a frame count over a fixed replay rather than a frametime series,
  so percentile metrics are **NOT AVAILABLE** from it. The per-frame CSV it
  writes is kept as an artifact for anyone who wants to derive more.
- **Gamescope, native vs nested.** NOT TESTED in this session.

---

## Reproducing

```sh
cd bigame-engine
RUNS=4 LABEL=gpu-bound ./scripts/bench-lab.sh \
    baseline cpu_governor gpu_dpm_level booster
cargo run -p bigame-core --example bench_report \
    benchmarks/<date>-supertuxkart-gpu-bound baseline
```

The harness captures the machine's state before it starts and restores it on
exit, including on interrupt, and does the same for the workload's own
configuration file.
