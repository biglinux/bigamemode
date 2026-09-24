# 2026-09-23-sottr-power-clean — 2026-09-23

## Verdict

- **gpu_dpm_level** — SLOWER: 8.0% slower, above the 0.1% run-to-run spread and significant at 95% (Welch's t = 199.75 against a 2.78 threshold)

## Measurements

| Arm | Runs | Mean avg_fps | Median | Min | Max | Spread | vs baseline |
|---|---:|---:|---:|---:|---:|---:|---:|
| rest | 3 | 89.4 | 89.4 | 89.3 | 89.4 | 0.1% | — |
| gpu_dpm_level | 3 | 82.2 | 82.2 | 82.2 | 82.2 | 0.0% | -8.0% |

## Method

- 3 measured run(s) per arm, 1 warm-up run(s) discarded.
- Arms were alternated (A B A B …) rather than grouped, so that drift over the session — chassis temperature above all — falls on both arms equally instead of on whichever ran last.
- A difference is called real only when it exceeds the run-to-run spread of both arms *and* passes Welch's t-test at 95%. Anything smaller is reported as no change, not as a small gain.
- Machine fingerprint `02db452880e054ab`.

## Caveats

- Confirmation session for 2026-09-23-sottr-power, run with the lab VM idle and no other work on the machine; same launch, settings and method.
- Shadow of the Tomb Raider (Windows build, Proton Experimental, DX12 via VKD3D-Proton), 3440x1440, preset High (AA=TAA), VSync off, in-game benchmark rerun with [R]; the game reports itself 99% GPU-bound.
- User launch options unchanged: WINE_FULLSCREEN_FSR=1 ENABLE_VKBASALT=1 RADV_PERFTEST=afmf (vkBasalt CAS loaded; the other two inert here).
- falcond's generic 'Proton' profile was active in both arms. big-screen-monitor-display.service used ~38% of one core throughout.
- The 0.1% low rests on about 14 frames per run and varies 6-11% between runs of the same arm; it is reported, not judged.

## Raw runs

- `gpu_dpm_level`: 82.2, 82.2, 82.2
- `rest`: 89.4, 89.4, 89.3
