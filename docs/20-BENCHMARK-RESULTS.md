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
| sched-ext scheduler (none × lavd × bpfland) | SotTR CPU-bound | one run of nine (`none`: 123.0 fps) | NOT MEASURED — the session ran on 2026-09-24 06:04 with the chain verified (falcond → `scx_loader` → kernel reported `lavd_1.1.3`), and stopped when the game was closed during the first lavd run (below) |
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
  exists to pull. The prerequisite is now in place; the session is one
  command, with the game on its results screen at minimum render scale:
  `GAME=sottr RUNS=3 LABEL=scheduler SCX_PROFILE=SOTTR.exe
  scripts/bench-game.sh scx_none scx_lavd scx_bpfland`. It asks for the
  password once, up front — the scheduler is set the way the product sets it,
  by rewriting the game's falcond profile and reloading falcond, which takes
  a root helper for the length of the session. Two attempts on 2026-09-24
  found three defects in the harness (a lost key press, a helper that could
  outlive it, a hang when the prompt expired) and one in the helper
  (signalling falcond's inhibitor along with falcond); all fixed
  ([13](13-AAA-BENCHMARKS.md)). The prompt then went unanswered for 20
  minutes, so the game's settings were restored and it was closed.
  A third attempt, with the user present (06:04), proved the chain on this
  machine — the switcher rewrote `SOTTR.exe`'s profile, falcond re-activated
  it with `scx=lavd, mode=gaming`, and the kernel reported sched_ext
  `enabled`, ops `lavd_1.1.3` — and measured `none` once (123.0 fps). The
  user then needed the machine and closed the game, confirmed by the user, so
  lavd is not implicated. The session needs ~45 minutes of an idle desktop:
  every focus change holds the next run, and anything busy is noise in a
  CPU-bound measurement.
- **Other hardware.** Every result above is one CPU and one GPU. The planner's
  GPU DPM gate is local evidence, not a rule for every Radeon.
- **Other titles.** Cyberpunk 2077 and Rise of the Tomb Raider are read
  automatically but need a person per run.
