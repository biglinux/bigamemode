# 2026-09-28-sottr-gtx1050ti-dx11-dx12 — 2026-09-28

## Verdict

- **dx11** — FASTER: 55.3% faster, above the 1.8% run-to-run spread and significant at 95% (Welch's t = 30.52 against a 4.30 threshold)

## Measurements

| Arm | Runs | Mean avg_fps | Median | Min | Max | Spread | vs baseline |
|---|---:|---:|---:|---:|---:|---:|---:|
| dx12 | 3 | 25.2 | 25.2 | 24.8 | 25.5 | 1.3% | — |
| dx11 | 3 | 39.1 | 38.7 | 38.6 | 39.9 | 1.8% | +55.3% |

## Method

- 3 measured run(s) per arm, 1 warm-up run(s) discarded.
- Arms were alternated (A B A B …) rather than grouped, so that drift over the session — chassis temperature above all — falls on both arms equally instead of on whichever ran last.
- A difference is called real only when it exceeds the run-to-run spread of both arms *and* passes Welch's t-test at 95%. Anything smaller is reported as no change, not as a small gain.
- Machine fingerprint `26b7fcc7109aa9b0`.

## Raw runs

- `dx11`: 39.9, 38.6, 38.7
- `dx12`: 25.5, 25.2, 24.8
