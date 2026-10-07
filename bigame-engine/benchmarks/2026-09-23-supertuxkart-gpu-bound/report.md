# 2026-09-23-supertuxkart-gpu-bound — 2026-09-23

## Verdict

- **booster** — SLOWER: 5.8% slower, above the 3.2% run-to-run spread and significant at 95% (Welch's t = 3.57 against a 2.78 threshold)
- **cpu_governor** — NO CHANGE: the 2.1% difference is within the 3.2% spread of the runs themselves, so it cannot be attributed to the change
- **gpu_dpm_level** — SLOWER: 7.5% slower, above the 4.7% run-to-run spread and significant at 95% (Welch's t = 2.81 against a 2.45 threshold)

## Measurements

| Arm | Runs | Mean avg_fps | Median | Min | Max | Spread | vs baseline |
|---|---:|---:|---:|---:|---:|---:|---:|
| baseline | 4 | 298.4 | 299.0 | 287.7 | 307.7 | 3.2% | — |
| booster | 4 | 281.0 | 280.6 | 279.0 | 283.9 | 0.7% | -5.8% |
| cpu_governor | 4 | 304.5 | 306.7 | 294.0 | 310.7 | 2.4% | within noise |
| gpu_dpm_level | 4 | 275.9 | 281.0 | 256.7 | 284.8 | 4.7% | -7.5% |

## Method

- 4 measured run(s) per arm, 1 warm-up run(s) discarded.
- Arms were alternated (A B A B …) rather than grouped, so that drift over the session — chassis temperature above all — falls on both arms equally instead of on whichever ran last.
- A difference is called real only when it exceeds the run-to-run spread of both arms *and* passes Welch's t-test at 95%. Anything smaller is reported as no change, not as a small gain.
- Machine fingerprint `02db452880e054ab`.

## Caveats

- SuperTuxKart's frame limiter was lifted for this test (max_fps 120 -> 1000, vsync off), and the workload was reconfigured to be GPU-bound: 3440x1440 instead of 1024x768, with shadows at 2048, SSAO, MLAA and full geometry detail. The original configuration is restored when the benchmark session ends.
- The reconfiguration was necessary. At the stock 1024x768 with effects off, telemetry showed the discrete GPU at about 50% utilisation and 1686 MHz of a possible 2700 -- the workload was limited by CPU and driver submission, not by the GPU, so no GPU-side setting could have shown an effect through it whatever its true value. After the change the GPU sits at about 85% and 2574 MHz.
- An earlier session of this same workload (benchmarks/2026-09-23-supertuxkart-cpu-bound) was measured with a telemetry sampler that spawned about 1400 processes per run. It raised the run-to-run spread from 1.4% to nearly 6% and its results are not usable; the sampler was rewritten to use only shell builtins.
- This workload is a kart racer, not a AAA title. A setting that helps here will not necessarily help a game with a different CPU/GPU balance.

## Raw runs

- `baseline`: 304.9, 287.7, 307.7, 293.1
- `booster`: 279.0, 280.5, 280.8, 283.9
- `cpu_governor`: 294.0, 305.4, 307.9, 310.7
- `gpu_dpm_level`: 280.4, 256.7, 281.6, 284.8
