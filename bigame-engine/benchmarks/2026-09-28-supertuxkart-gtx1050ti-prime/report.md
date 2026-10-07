# 2026-09-28-supertuxkart-gtx1050ti-prime — 2026-09-28

## Verdict

- **dgpu_gamescope** — NO CHANGE: the 0.1% difference is within the 1.2% spread of the runs themselves, so it cannot be attributed to the change
- **dgpu_mangohud** — NO CHANGE: the 0.2% difference is within the 1.1% spread of the runs themselves, so it cannot be attributed to the change
- **igpu** — SLOWER: 78.7% slower, above the 1.1% run-to-run spread and significant at 95% (Welch's t = 142.00 against a 3.18 threshold)

## Measurements

| Arm | Runs | Mean avg_fps | Median | Min | Max | Spread | vs baseline |
|---|---:|---:|---:|---:|---:|---:|---:|
| dgpu | 4 | 61.2 | 61.3 | 60.4 | 61.9 | 1.1% | — |
| dgpu_gamescope | 4 | 61.2 | 61.1 | 60.3 | 62.1 | 1.2% | within noise |
| dgpu_mangohud | 4 | 61.3 | 61.3 | 61.1 | 61.6 | 0.3% | within noise |
| igpu | 4 | 13.0 | 13.0 | 12.9 | 13.2 | 0.9% | -78.7% |

## Method

- 4 measured run(s) per arm, 1 warm-up run(s) discarded.
- Arms were alternated (A B A B …) rather than grouped, so that drift over the session — chassis temperature above all — falls on both arms equally instead of on whichever ran last.
- A difference is called real only when it exceeds the run-to-run spread of both arms *and* passes Welch's t-test at 95%. Anything smaller is reported as no change, not as a small gain.
- Machine fingerprint `26b7fcc7109aa9b0`.

## Raw runs

- `dgpu`: 61.6, 61.9, 60.4, 61.0
- `dgpu_gamescope`: 62.1, 61.3, 60.3, 60.9
- `dgpu_mangohud`: 61.6, 61.4, 61.1, 61.2
- `igpu`: 13.2, 13.1, 12.9, 12.9
