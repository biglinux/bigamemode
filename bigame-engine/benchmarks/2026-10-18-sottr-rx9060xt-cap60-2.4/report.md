# 2026-10-18-sottr-d1 — 2026-10-18

## Verdict

- **pretty** — FASTER: 33.0% faster, above the 0.5% run-to-run spread and significant at 95% (Welch's t = 70.23 against a 12.71 threshold)

## Measurements

| Arm | Runs | Mean avg_fps | Median | Min | Max | Spread | vs baseline |
|---|---:|---:|---:|---:|---:|---:|---:|
| cap60 | 2 | 59.7 | 59.7 | 59.7 | 59.8 | 0.2% | — |
| pretty | 2 | 79.4 | 79.4 | 79.2 | 79.7 | 0.5% | +33.0% |

## Method

- 2 measured run(s) per arm, 1 warm-up run(s) discarded.
- Arms were alternated (A B A B …) rather than grouped, so that drift over the session — chassis temperature above all — falls on both arms equally instead of on whichever ran last.
- A difference is called real only when it exceeds the run-to-run spread of both arms *and* passes Welch's t-test at 95%. Anything smaller is reported as no change, not as a small gain.
- Machine fingerprint `d6b5047b171af143`.

## Raw runs

- `cap60`: 59.8, 59.7
- `pretty`: 79.2, 79.7
