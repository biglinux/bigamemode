# 2026-09-23-sottr-ui-overhead — 2026-09-23

## Verdict

- **ui_paused** — NO CHANGE: a 1.1% difference, but Welch's t = 2.38 falls short of the 2.78 needed for 95% confidence at 3.4 degrees of freedom

## Measurements

| Arm | Runs | Mean avg_fps | Median | Min | Max | Spread | vs baseline |
|---|---:|---:|---:|---:|---:|---:|---:|
| ui_polling | 3 | 115.0 | 114.8 | 114.4 | 115.8 | 0.7% | — |
| ui_paused | 3 | 116.2 | 116.4 | 115.7 | 116.6 | 0.4% | within noise |

## Method

- 3 measured run(s) per arm, 1 warm-up run(s) discarded.
- Arms were alternated (A B A B …) rather than grouped, so that drift over the session — chassis temperature above all — falls on both arms equally instead of on whichever ran last.
- A difference is called real only when it exceeds the run-to-run spread of both arms *and* passes Welch's t-test at 95%. Anything smaller is reported as no change, not as a small gain.
- Machine fingerprint `02db452880e054ab`.

## Raw runs

- `ui_paused`: 116.4, 116.6, 115.7
- `ui_polling`: 114.8, 115.8, 114.4
