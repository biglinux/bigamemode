# 2026-09-23-sottr-power: every metric

Graphics settings identical across all 12 runs: AA 2 at 3440x1440, VSync false.

## Per run

| arm | run | avg fps | 1% low | 0.1% low | p99 ms | stutters | transitions | sclk MHz | power W | temp °C | GPU busy |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| baseline | run-01 | 88.3 | 61.6 | 39.4 | 14.56 | 6 | 0 | 3231 | 161 | 64.8 | 99% |
| baseline | run-02 | 88.2 | 58.4 | 39.3 | 15.13 | 6 | 3 | 3239 | 161 | 64.5 | 99% |
| baseline | run-03 | 89.2 | 61.2 | 39.3 | 14.56 | 6 | 0 | 3218 | 161 | 65.2 | 98% |
| baseline | run-04 | 89.2 | 62.3 | 47.1 | 14.65 | 3 | 1 | 3229 | 161 | 64.6 | 98% |
| gpu_dpm_level | run-01 | 81.3 | 57.7 | 38.7 | 15.82 | 4 | 0 | 2639 | 102 | 59.5 | 98% |
| gpu_dpm_level | run-02 | 80.2 | 44.6 | 15.4 | 16.29 | 14 | 0 | 2632 | 101 | 59.2 | 97% |
| gpu_dpm_level | run-03 | 82.3 | 61.3 | 48.8 | 15.53 | 2 | 0 | 2641 | 103 | 59.4 | 99% |
| gpu_dpm_level | run-04 | 81.9 | 60.0 | 42.9 | 15.56 | 3 | 0 | 2635 | 102 | 59.1 | 98% |
| rest | run-01 | 88.4 | 64.5 | 49.9 | 14.49 | 2 | 0 | 3242 | 162 | 64.7 | 99% |
| rest | run-02 | 88.2 | 59.7 | 35.7 | 14.77 | 7 | 0 | 3233 | 162 | 65.2 | 99% |
| rest | run-03 | 89.3 | 63.0 | 43.1 | 14.51 | 6 | 0 | 3230 | 163 | 65.1 | 99% |
| rest | run-04 | 89.1 | 63.0 | 47.5 | 14.53 | 2 | 0 | 3227 | 161 | 64.6 | 99% |

## Per arm (telemetry means while the GPU was busy)

| arm | sclk MHz | power W | temp °C |
|---|---:|---:|---:|
| baseline | 3229 | 161 | 64.8 |
| gpu_dpm_level | 2637 | 102 | 59.3 |
| rest | 3233 | 162 | 64.9 |

## Verdicts against `rest`

| metric | arm | mean → mean | change | verdict | why |
|---|---|---|---:|---|---|
| avg_fps | baseline | 88.8 → 88.7 | -0.1% | no difference above normal variation | the 0.1% difference is within the 0.6% spread of the runs themselves, so it cannot be attributed to the change |
| avg_fps | gpu_dpm_level | 88.8 → 81.4 | -8.3% | measurably slower | 8.3% slower, above the 1.1% run-to-run spread and significant at 95% (Welch's t = 13.91 against a 2.57 threshold) |
| low_1_fps | baseline | 62.6 → 60.9 | -2.7% | no difference above normal variation | the 2.7% difference is within the 3.2% spread of the runs themselves, so it cannot be attributed to the change |
| low_1_fps | gpu_dpm_level | 62.6 → 55.9 | -10.7% | not enough evidence to say | the runs within an arm disagree too much to compare (variation 3.2% and 13.7%, above the 5% ceiling); something on the machine was interfering |
| low_0_1_fps | baseline | 44.0 → 41.3 | -6.3% | not enough evidence to say | the runs within an arm disagree too much to compare (variation 14.2% and 9.4%, above the 5% ceiling); something on the machine was interfering |
| low_0_1_fps | gpu_dpm_level | 44.0 → 36.4 | -17.3% | not enough evidence to say | the runs within an arm disagree too much to compare (variation 14.2% and 40.2%, above the 5% ceiling); something on the machine was interfering |

## Calibration

Recorded for this machine: gpu_dpm_level.

Measured 3 setting(s) on 2026-09-23: 0 helped, 2 hurt, 1 made no measurable difference.
