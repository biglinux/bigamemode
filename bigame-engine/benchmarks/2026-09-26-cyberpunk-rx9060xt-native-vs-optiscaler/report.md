# 2026-09-26-cyberpunk-rx9060xt-native-vs-optiscaler — 2026-09-26

## Verdict

- **native_fsr4** — NO CHANGE: a 0.6% difference, but Welch's t = 1.90 falls short of the 12.71 needed for 95% confidence at 1.6 degrees of freedom
- **optiscaler_fsr** — SLOWER: 6.4% slower, above the 0.4% run-to-run spread and significant at 95% (Welch's t = 24.69 against a 12.71 threshold)

## Measurements

| Arm | Runs | Mean avg_fps | Median | Min | Max | Spread | vs baseline |
|---|---:|---:|---:|---:|---:|---:|---:|
| native_fsr31 | 2 | 38.4 | 38.4 | 38.3 | 38.5 | 0.4% | — |
| native_fsr4 | 2 | 38.2 | 38.2 | 38.2 | 38.3 | 0.2% | within noise |
| optiscaler_fsr | 2 | 36.0 | 36.0 | 36.0 | 36.0 | 0.0% | -6.4% |

## Method

- 2 measured run(s) per arm, 1 warm-up run(s) discarded.
- Arms were alternated (A B A B …) rather than grouped, so that drift over the session — chassis temperature above all — falls on both arms equally instead of on whichever ran last.
- A difference is called real only when it exceeds the run-to-run spread of both arms *and* passes Welch's t-test at 95%. Anything smaller is reported as no change, not as a small gain.
- Machine fingerprint `4fe16c03c167c5cb`.

## Raw runs

- `native_fsr31`: 38.5, 38.3
- `native_fsr4`: 38.3, 38.2
- `optiscaler_fsr`: 36.0, 36.0
