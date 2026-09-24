# 2026-09-24-sottr-scheduler — 2026-09-24

## Verdict

- **scx_bpfland** — NO CHANGE: a 1.8% difference, but Welch's t = 2.12 falls short of the 2.78 needed for 95% confidence at 3.5 degrees of freedom
- **scx_lavd** — NO CHANGE: the 1.1% difference is within the 1.4% spread of the runs themselves, so it cannot be attributed to the change

## Measurements

| Arm | Runs | Mean avg_fps | Median | Min | Max | Spread | vs baseline |
|---|---:|---:|---:|---:|---:|---:|---:|
| scx_none | 3 | 120.6 | 120.6 | 119.5 | 121.5 | 0.8% | — |
| scx_bpfland | 3 | 122.7 | 123.0 | 121.1 | 124.0 | 1.2% | within noise |
| scx_lavd | 3 | 121.8 | 122.7 | 119.8 | 123.0 | 1.4% | within noise |

## Method

- 3 measured run(s) per arm, 1 warm-up run(s) discarded.
- Arms were alternated (A B A B …) rather than grouped, so that drift over the session — chassis temperature above all — falls on both arms equally instead of on whichever ran last.
- A difference is called real only when it exceeds the run-to-run spread of both arms *and* passes Welch's t-test at 95%. Anything smaller is reported as no change, not as a small gain.
- Machine fingerprint `02db452880e054ab`.

## Raw runs

- `scx_bpfland`: 123.0, 124.0, 121.1
- `scx_lavd`: 123.0, 122.7, 119.8
- `scx_none`: 120.6, 121.5, 119.5
