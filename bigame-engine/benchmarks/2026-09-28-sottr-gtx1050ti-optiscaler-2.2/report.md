# 2026-09-28-sottr-gtx1050ti-optiscaler-2.2 — 2026-09-28

## Verdict

- **optiscaler** — FASTER: 20.1% faster, above the 2.6% run-to-run spread and significant at 95% (Welch's t = 8.76 against a 3.18 threshold)

## Measurements

| Arm | Runs | Mean avg_fps | Median | Min | Max | Spread | vs baseline |
|---|---:|---:|---:|---:|---:|---:|---:|
| native_xess | 3 | 24.2 | 24.1 | 23.6 | 24.8 | 2.5% | — |
| optiscaler | 3 | 29.0 | 28.8 | 28.5 | 29.9 | 2.6% | +20.1% |

## Method

- 3 measured run(s) per arm, 1 warm-up run(s) discarded.
- Arms were alternated (A B A B …) rather than grouped, so that drift over the session — chassis temperature above all — falls on both arms equally instead of on whichever ran last.
- A difference is called real only when it exceeds the run-to-run spread of both arms *and* passes Welch's t-test at 95%. Anything smaller is reported as no change, not as a small gain.
- Machine fingerprint `26b7fcc7109aa9b0`.

## Raw runs

- `native_xess`: 24.1, 24.8, 23.6
- `optiscaler`: 28.5, 28.8, 29.9
