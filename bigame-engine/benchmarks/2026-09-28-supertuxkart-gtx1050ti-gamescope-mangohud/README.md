# SuperTuxKart — Gamescope and MangoHud on the hybrid laptop (2026-09-28)

The same machine and game settings as `../2026-09-28-supertuxkart-gtx1050ti-prime`
(Core i7-7700HQ, HD 630 driving the 60 Hz panel, GTX 1050 Ti rendering,
SuperTuxKart 1.5 OpenGL `--benchmark`, 1920×1080 fullscreen, vsync off,
`max_fps` 1000, shadows 2048, SSAO, MLAA, Turbo off), after the harness bug
that made that session's Gamescope and MangoHud arms run the game plain was
fixed. Each run logged the command it started (`bench-lab.sh`), all three
through BiGame-mode's launcher:

| Arm | Started as |
|---|---|
| `dgpu` | `supertuxkart --benchmark` with NVIDIA PRIME render offload |
| `dgpu_gamescope` | `gamescope -W 1920 -H 1080 -F fsr --fsr-sharpness 0 -f --expose-wayland -- env __NV_PRIME_RENDER_OFFLOAD=1 … supertuxkart --benchmark` |
| `dgpu_mangohud` | `mangohud supertuxkart --benchmark` with the offload variables |

4 measured runs per arm, alternated, 1 warm-up discarded. The game's log
names the GTX 1050 Ti as its renderer in every arm, 1920×1080 in every arm.

## Results

| Arm | Avg FPS | 1 % low | Median | p95 | p99 | Stutters / run | GPU busy |
|---|---:|---:|---:|---:|---:|---:|---:|
| `dgpu` | 62.9 | 47.9 | 16.6 ms | 18.5 ms | 19.8 ms | 0 | 64 % |
| `dgpu_gamescope` | 90.4 | 36.1 | 10.1 ms | 19.0 ms | 26.9 ms | 142 | 91 % |
| `dgpu_mangohud` | 62.6 | 47.8 | 16.6 ms | 18.7 ms | 19.9 ms | 0 | 63 % |

Verdicts from `bench_report` against `dgpu` (average FPS): Gamescope **43.9 %
faster** (above the 4.4 % spread, Welch t = 13.85); MangoHud **no change**
(0.4 % within 0.8 %). Frame times from the game's own per-frame profile
(`stdout.log.profile-black_forest-cpu-0.csv`, frames of a second or more set
aside), means over the four runs.

## What this shows

- Without Gamescope the GTX is idle a third of the time (64 % busy): each
  frame goes through Xwayland and KWin to the HD 630 that drives the panel.
  Inside Gamescope the GPU is kept 91 % busy and renders 44 % more frames.
- Those frames are unevenly paced: the 1 % low drops from 47.9 to 36.1 FPS,
  p99 rises from 19.8 to 26.9 ms and about 140 frames per run take over twice
  the median, against none without Gamescope. More frames, worse pacing.
- MangoHud's overlay costs nothing measurable. With its wrapper the game
  aborted at exit in two of the four runs, after the benchmark had written
  its results.
- Gamescope did not send the game back to the integrated GPU.

Caveats: Steam's web helper and an audio effects daemon ran throughout, in
every arm alike. The per-frame profiles, GPU samples and the game's logs are
not in the repository.
