# 20 — Benchmark results: everything measured, in one place

Reference machine: Ryzen 7 5700G (`amd-pstate-epp`), Radeon RX 9060 XT
(RDNA 4, `amdgpu`), kernel 7.2.6-x64v3-xanmod1, Mesa 26.2.2, Proton
Experimental, KDE Plasma Wayland. Alternating runs, warm-up discarded,
verdicts from run-to-run spread and Welch's t at 95 %. Raw data under
`bigame-engine/benchmarks/`.

## What the settings Turbo can touch are worth here

| Setting | Workload | Result | Verdict |
|---|---|---|---|
| GPU DPM `high` (vs `auto`) | SotTR 3440×1440 High, GPU-bound | **−8.0 % avg, −7.6 % 1 % low** (spread 0.1 %) | slower — MEASURED |
| GPU DPM `high` | SuperTuxKart, GPU-bound | −7.5 % | slower — MEASURED |
| Performance power profile vs balanced (+ governor, EPP) | SotTR GPU-bound | −0.1 % | no difference — MEASURED |
| Same | SotTR CPU-bound (render scale minimum) | within 1.5 % | no difference — MEASURED (round 3 interference noted) |
| CPU governor + EPP alone | SotTR CPU-bound | −1.1 % | no difference — MEASURED |
| CPU governor | SuperTuxKart, GPU-bound | +2.1 % | within noise — MEASURED |
| BiGame-mode's own UI (old build) running vs frozen | SotTR CPU-bound | +1.1 % frozen | not significant — MEASURED |
| sched-ext scheduler (any) | — | — | NOT MEASURED — `scx-tools` absent on the reference machine |
| Gamescope native vs nested | — | — | NOT MEASURED |

## Turbo off vs Turbo on

Turbo on means falcond runs and applies a profile per game; its only
performance-relevant action on this machine is switching to the
**performance** power profile while a game runs (the scheduler it asks for
cannot be set without `scx-tools`; V-Cache does not exist on this CPU). So
Turbo off vs on here *is* balanced-or-resting-state vs performance power
profile, and that was measured: **no difference**, GPU-bound or CPU-bound.

A direct Turbo off/on session through the new switch was NOT RUN: it needs
the new package installed on the reference machine, which needs a Polkit
approval that was not available while this pass ran. The expected result,
from the measurements above, is no difference; the session would confirm it.

## The application's own cost

On Home with a game running: **0.77 % CPU, 9.9 context switches/s** (branch)
against 7.38 % and 313/s (the build on `main`); hidden in the tray: 0.52 %,
3.8/s. See [19](19-LOGGING-OBSERVABILITY.md).

## What would change these conclusions

- **A scheduler.** The one lever not yet measured here, and the one falcond
  exists to pull. `scx-tools` is the prerequisite.
- **Other hardware.** Every result above is one CPU and one GPU. The planner's
  GPU DPM gate is local evidence, not a rule for every Radeon.
- **Other titles.** Cyberpunk 2077 and Rise of the Tomb Raider are read
  automatically but need a person per run.
