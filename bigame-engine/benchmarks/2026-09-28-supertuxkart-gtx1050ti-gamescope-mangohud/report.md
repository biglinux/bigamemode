# 2026-09-28-supertuxkart-gtx1050ti-gamescope-mangohud — 2026-09-28

## Verdict

- **dgpu_gamescope** — FASTER: 43.9% faster, above the 4.4% run-to-run spread and significant at 95% (Welch's t = 13.85 against a 3.18 threshold)
- **dgpu_mangohud** — NO CHANGE: the 0.4% difference is within the 0.8% spread of the runs themselves, so it cannot be attributed to the change

## Measurements

| Arm | Runs | Mean avg_fps | Median | Min | Max | Spread | vs baseline |
|---|---:|---:|---:|---:|---:|---:|---:|
| dgpu | 4 | 62.9 | 63.0 | 62.1 | 63.3 | 0.8% | — |
| dgpu_gamescope | 4 | 90.4 | 89.8 | 87.1 | 95.1 | 4.4% | +43.9% |
| dgpu_mangohud | 4 | 62.6 | 62.6 | 62.2 | 63.0 | 0.7% | within noise |

## Method

- 4 measured run(s) per arm, 1 warm-up run(s) discarded.
- Arms were alternated (A B A B …) rather than grouped, so that drift over the session — chassis temperature above all — falls on both arms equally instead of on whichever ran last.
- A difference is called real only when it exceeds the run-to-run spread of both arms *and* passes Welch's t-test at 95%. Anything smaller is reported as no change, not as a small gain.
- Machine fingerprint `26b7fcc7109aa9b0`.

## Raw runs

- `dgpu`: 63.0, 63.3, 63.0, 62.1
- `dgpu_gamescope`: 95.1, 87.1, 92.4, 87.2
- `dgpu_mangohud`: 63.0, 62.9, 62.3, 62.2
