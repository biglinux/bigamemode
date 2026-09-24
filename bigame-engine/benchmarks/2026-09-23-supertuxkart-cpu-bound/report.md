# 2026-09-23-supertuxkart-cpu-bound — 2026-09-23

## Verdict

- **booster** — INCONCLUSIVE: the runs within an arm disagree too much to compare (variation 4.6% and 5.9%, above the 5% ceiling); something on the machine was interfering
- **governor_only** — NO CHANGE: the 0.7% difference is within the 4.6% spread of the runs themselves, so it cannot be attributed to the change
- **gpu_only** — NO CHANGE: the 0.5% difference is within the 4.6% spread of the runs themselves, so it cannot be attributed to the change

## Measurements

| Arm | Runs | Mean avg_fps | Median | Min | Max | Spread | vs baseline |
|---|---:|---:|---:|---:|---:|---:|---:|
| baseline | 4 | 672.3 | 666.6 | 646.9 | 709.0 | 4.6% | — |
| booster | 4 | 695.0 | 711.0 | 635.3 | 722.7 | 5.9% | inconclusive |
| governor_only | 4 | 677.1 | 676.4 | 666.9 | 688.8 | 1.4% | within noise |
| gpu_only | 4 | 675.7 | 684.9 | 632.6 | 700.6 | 4.5% | within noise |

## Method

- 4 measured run(s) per arm, 1 warm-up run(s) discarded.
- Arms were alternated (A B A B …) rather than grouped, so that drift over the session — chassis temperature above all — falls on both arms equally instead of on whichever ran last.
- A difference is called real only when it exceeds the run-to-run spread of both arms *and* passes Welch's t-test at 95%. Anything smaller is reported as no change, not as a small gain.
- Machine fingerprint `02db452880e054ab`.

## Caveats

- SuperTuxKart's frame limiter was lifted for this test (max_fps 120 -> 1000, vsync off). At its default settings the workload pins at about 160 fps and cannot show a difference between two configurations, however large that difference is. The original configuration is restored after the benchmark session.
- This workload is a kart racer, not a AAA title. A configuration that helps here will not necessarily help a game with a different CPU/GPU balance; it is evidence about this machine's response to the settings, not a universal result.

## Raw runs

- `baseline`: 646.9, 709.0, 686.2, 647.1
- `booster`: 701.8, 635.3, 720.2, 722.7
- `governor_only`: 672.4, 666.9, 680.4, 688.8
- `gpu_only`: 632.6, 691.1, 678.6, 700.6
