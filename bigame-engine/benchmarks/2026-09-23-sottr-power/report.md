# 2026-09-23-sottr-power — 2026-09-23

## Verdict

- **baseline** — NO CHANGE: the 0.1% difference is within the 0.6% spread of the runs themselves, so it cannot be attributed to the change
- **gpu_dpm_level** — SLOWER: 8.3% slower, above the 1.1% run-to-run spread and significant at 95% (Welch's t = 13.91 against a 2.57 threshold)

## Measurements

| Arm | Runs | Mean avg_fps | Median | Min | Max | Spread | vs baseline |
|---|---:|---:|---:|---:|---:|---:|---:|
| rest | 4 | 88.8 | 88.8 | 88.2 | 89.3 | 0.6% | — |
| baseline | 4 | 88.7 | 88.7 | 88.2 | 89.2 | 0.6% | within noise |
| gpu_dpm_level | 4 | 81.4 | 81.6 | 80.2 | 82.3 | 1.1% | -8.3% |

## Method

- 4 measured run(s) per arm, 1 warm-up run(s) discarded.
- Arms were alternated (A B A B …) rather than grouped, so that drift over the session — chassis temperature above all — falls on both arms equally instead of on whichever ran last.
- A difference is called real only when it exceeds the run-to-run spread of both arms *and* passes Welch's t-test at 95%. Anything smaller is reported as no change, not as a small gain.
- Machine fingerprint `02db452880e054ab`.

## Caveats

- Shadow of the Tomb Raider (Windows build, Proton Experimental, DX12 via VKD3D-Proton), 3440x1440, preset High (AA=TAA), VSync off, in-game benchmark rerun with [R] from its results screen; one launch for the whole session.
- Launch options were the user's own and unchanged throughout: WINE_FULLSCREEN_FSR=1 ENABLE_VKBASALT=1 RADV_PERFTEST=afmf. vkBasalt (CAS) was loaded; the other two are inert on Proton Experimental and Mesa 26.2.
- Baseline for the verdicts is 'rest': performance power profile, performance governor and EPP, GPU DPM auto -- the state this machine keeps.
- The game reports itself 99% GPU-bound at these settings (its own CPU/GPU breakdown: CPU game 117 fps, CPU render 217 fps, GPU 84 fps).
- CONTAMINATED LOWS: the lab VM runs on this same host, and package installs and a Phoronix build inside it overlapped baseline run-02 (freezes of 3.6 s and 2.0 s), baseline run-04 (1.1 s) and gpu_dpm_level run-02 (333 ms). Average frame rate is robust to these; 1% and 0.1% lows from this session are not, and are superseded by 2026-09-23-sottr-power-clean.
- big-screen-monitor-display.service (a root python3 system monitor) used about 38% of one core continuously, in every arm.
- falcond's generic 'Proton' profile matched the game at launch and stayed active in every arm (performance_mode, idle_inhibit); per-run state.txt confirms each arm's settings held.

## Raw runs

- `baseline`: 88.3, 88.2, 89.2, 89.2
- `gpu_dpm_level`: 81.3, 80.2, 82.3, 81.9
- `rest`: 88.4, 88.2, 89.3, 89.1
