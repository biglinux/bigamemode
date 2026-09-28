# 2026-09-28-sottr-gtx1050ti-turbo-2.2 — 2026-09-28

## Verdict

- **turbo_on** — NO CHANGE: the 0.0% difference is within the 0.5% spread of the runs themselves, so it cannot be attributed to the change

## Measurements

| Arm | Runs | Mean avg_fps | Median | Min | Max | Spread | vs baseline |
|---|---:|---:|---:|---:|---:|---:|---:|
| turbo_off | 3 | 24.1 | 24.1 | 24.1 | 24.1 | 0.2% | — |
| turbo_on | 3 | 24.1 | 24.1 | 24.0 | 24.3 | 0.5% | within noise |

## Method

- 3 measured run(s) per arm, 1 warm-up run(s) discarded.
- Arms were alternated (A B A B …) rather than grouped, so that drift over the session — chassis temperature above all — falls on both arms equally instead of on whichever ran last.
- A difference is called real only when it exceeds the run-to-run spread of both arms *and* passes Welch's t-test at 95%. Anything smaller is reported as no change, not as a small gain.
- Machine fingerprint `26b7fcc7109aa9b0`.

## Raw runs

- `turbo_off`: 24.1, 24.1, 24.1
- `turbo_on`: 24.1, 24.0, 24.3
