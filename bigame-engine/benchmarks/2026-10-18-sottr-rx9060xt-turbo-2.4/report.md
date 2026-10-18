# 2026-10-18-sottr-d2-turbo — 2026-10-18

## Verdict

- **turbo_on** — NO CHANGE: the 0.4% difference is within the 2.0% spread of the runs themselves, so it cannot be attributed to the change

## Measurements

| Arm | Runs | Mean avg_fps | Median | Min | Max | Spread | vs baseline |
|---|---:|---:|---:|---:|---:|---:|---:|
| turbo_off | 2 | 80.7 | 80.7 | 79.5 | 81.9 | 2.0% | — |
| turbo_on | 2 | 80.4 | 80.4 | 80.0 | 80.7 | 0.6% | within noise |

## Method

- 2 measured run(s) per arm, 1 warm-up run(s) discarded.
- Arms were alternated (A B A B …) rather than grouped, so that drift over the session — chassis temperature above all — falls on both arms equally instead of on whichever ran last.
- A difference is called real only when it exceeds the run-to-run spread of both arms *and* passes Welch's t-test at 95%. Anything smaller is reported as no change, not as a small gain.
- Machine fingerprint `d6b5047b171af143`.

## Raw runs

- `turbo_off`: 79.5, 81.9
- `turbo_on`: 80.7, 80.0
