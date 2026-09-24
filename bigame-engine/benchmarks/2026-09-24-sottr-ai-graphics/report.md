# 2026-09-24-sottr-ai-graphics — 2026-09-24

## Verdict

- **native_xess** — FASTER: 4.9% faster, above the 0.2% run-to-run spread and significant at 95% (Welch's t = 28.80 against a 2.78 threshold)
- **optiscaler_fsr** — FASTER: 10.1% faster, above the 0.2% run-to-run spread and significant at 95% (Welch's t = 71.68 against a 2.78 threshold)

## Measurements

| Arm | Runs | Mean avg_fps | Median | Min | Max | Spread | vs baseline |
|---|---:|---:|---:|---:|---:|---:|---:|
| native_taa | 3 | 89.8 | 89.8 | 89.6 | 89.9 | 0.2% | — |
| native_xess | 3 | 94.2 | 94.2 | 93.9 | 94.4 | 0.2% | +4.9% |
| optiscaler_fsr | 3 | 98.8 | 98.7 | 98.7 | 99.0 | 0.2% | +10.1% |

## Method

- 3 measured run(s) per arm, 1 warm-up run(s) discarded.
- Arms were alternated (A B A B …) rather than grouped, so that drift over the session — chassis temperature above all — falls on both arms equally instead of on whichever ran last.
- A difference is called real only when it exceeds the run-to-run spread of both arms *and* passes Welch's t-test at 95%. Anything smaller is reported as no change, not as a small gain.
- Machine fingerprint `02db452880e054ab`.

## Raw runs

- `native_taa`: 89.6, 89.8, 89.9
- `native_xess`: 93.9, 94.2, 94.4
- `optiscaler_fsr`: 99.0, 98.7, 98.7
