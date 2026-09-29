# SuperTuxKart — render offload, Gamescope and MangoHud on the hybrid laptop (2026-09-28)

Lab laptop: Core i7-7700HQ, Intel HD 630 (drives the 1920×1080 60 Hz panel),
GeForce GTX 1050 Ti Mobile (NVIDIA 580.178.04), KDE Plasma Wayland, on AC.
SuperTuxKart 1.5 (the distribution package), OpenGL renderer, its own
`--benchmark` race, 1920×1080 fullscreen, vsync off, `max_fps` 1000,
shadows 2048, SSAO, MLAA. Turbo off (falcond has no profile for this game).
Driven by `scripts/bench-lab.sh` with the arms in `../_arms-hybrid/arms.sh`:
every NVIDIA arm is started with the command BiGame-mode's launcher builds
(`use_launch_plan`). 4 measured runs per arm, alternated, 1 warm-up discarded.

| Arm | Started as | GPU the game rendered on (its own log) |
|---|---|---|
| `igpu` | `supertuxkart --benchmark`, no offload | HD 630 |
| `dgpu` | BiGame-mode's launch: NVIDIA PRIME render offload | GTX 1050 Ti |
| `dgpu_gamescope` | the same inside nested Gamescope (offload given to the game, not to Gamescope) | GTX 1050 Ti |
| `dgpu_mangohud` | the same with MangoHud Forced (its wrapper, for OpenGL) | GTX 1050 Ti |

> **Correction.** The `dgpu_gamescope` and `dgpu_mangohud` arms of this
> session ran the game without Gamescope and without MangoHud: the harness
> passed each arm's configuration as a relative `XDG_CONFIG_HOME`, which is
> not one, so BiGame-mode read the user's own settings (neither on). Those
> two rows measured the `dgpu` arm again, and say nothing about Gamescope or
> MangoHud; see `2026-09-28-supertuxkart-gtx1050ti-gamescope-mangohud`. The
> `igpu` and `dgpu` arms are unaffected: offload comes from the machine.

## Results

Verdicts from `bench_report` against `dgpu` (report.md):

| Arm | Avg FPS | 1 % low | Median | p95 | p99 | Frames > 2× median / run | Verdict |
|---|---:|---:|---:|---:|---:|---:|---|
| `igpu` | 13.0 | 8.4 | 75.3 ms | 104.3 ms | 115.1 ms | 0.0 | 78.7 % slower (Welch t = 142) |
| `dgpu` | 61.2 | 46.4 | 16.6 ms | 19.2 ms | 20.5 ms | 0.5 | baseline |
| `dgpu_gamescope` | 61.2 | 46.1 | 16.6 ms | 19.2 ms | 20.6 ms | 0.8 | no change |
| `dgpu_mangohud` | 61.3 | 46.5 | 16.6 ms | 19.2 ms | 20.5 ms | 0.2 | no change |

Frame times are the game's own per-frame "Main loop" durations
(`stdout.log.profile-black_forest-cpu-0.csv`, one row per frame), frames of
one second or more (loading) set aside, averaged over the four runs of each
arm; 1 % low is 1000 / the mean of the slowest 1 % of frames.

## What this shows

- Without offload the game renders on the integrated GPU; the command
  BiGame-mode's launcher builds puts it on the NVIDIA card, 4.7 times faster
  here.
- The NVIDIA arms are not held at the panel's 60 Hz: the same effects at
  1280×720 ran at 80.3 FPS and 720p with effects off at 144.4 FPS (single
  runs), so about 61 FPS at 1080p is the GPU's pace. The GPU reported 58 %
  busy on average, never above 95 %.

Caveats: Steam's web helper (about 17 % of one CPU) and an audio effects
daemon (about 16 %) ran throughout, in every arm alike. The per-frame
profiles, GPU samples and the game's logs are not in the repository.
