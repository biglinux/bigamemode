# 2026-09-23-sottr-cpu-bound — 2026-09-23

## Verdict

- **cpu_governor** — NO CHANGE: the 1.1% difference is within the 4.0% spread of the runs themselves, so it cannot be attributed to the change
- **rest** — NO CHANGE: the 2.1% difference is within the 3.9% spread of the runs themselves, so it cannot be attributed to the change

## Measurements

| Arm | Runs | Mean avg_fps | Median | Min | Max | Spread | vs baseline |
|---|---:|---:|---:|---:|---:|---:|---:|
| baseline | 3 | 114.3 | 116.6 | 109.1 | 117.2 | 3.9% | — |
| cpu_governor | 3 | 113.0 | 115.1 | 107.8 | 116.0 | 4.0% | within noise |
| rest | 3 | 116.7 | 116.4 | 116.0 | 117.7 | 0.8% | within noise |

## Method

- 3 measured run(s) per arm, 1 warm-up run(s) discarded.
- Arms were alternated (A B A B …) rather than grouped, so that drift over the session — chassis temperature above all — falls on both arms equally instead of on whichever ran last.
- A difference is called real only when it exceeds the run-to-run spread of both arms *and* passes Welch's t-test at 95%. Anything smaller is reported as no change, not as a small gain.
- Machine fingerprint `02db452880e054ab`.

## Raw runs

- `baseline`: 116.6, 117.2, 109.1
- `cpu_governor`: 116.0, 115.1, 107.8
- `rest`: 117.7, 116.4, 116.0
