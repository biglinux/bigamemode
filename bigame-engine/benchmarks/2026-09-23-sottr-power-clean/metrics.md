# 2026-09-23-sottr-power-clean: every metric

Graphics settings identical across all 6 runs: AA 2 at 3440x1440, VSync false.

## Per run

| arm | run | avg fps | 1% low | 0.1% low | p99 ms | stutters | transitions | sclk MHz | power W | temp °C | GPU busy |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| gpu_dpm_level | run-01 | 82.2 | 59.2 | 39.3 | 15.61 | 6 | 0 | 2641 | 103 | 59.0 | 99% |
| gpu_dpm_level | run-02 | 82.2 | 59.5 | 42.3 | 15.55 | 4 | 0 | 2641 | 102 | 58.2 | 99% |
| gpu_dpm_level | run-03 | 82.2 | 61.5 | 48.8 | 15.49 | 2 | 0 | 2637 | 103 | 58.9 | 99% |
| rest | run-01 | 89.4 | 65.5 | 51.6 | 14.38 | 2 | 0 | 3248 | 163 | 64.6 | 98% |
| rest | run-02 | 89.4 | 65.7 | 52.5 | 14.32 | 2 | 0 | 3248 | 163 | 64.4 | 99% |
| rest | run-03 | 89.3 | 63.8 | 46.6 | 14.43 | 3 | 0 | 3241 | 162 | 64.7 | 99% |

## Per arm (telemetry means while the GPU was busy)

| arm | sclk MHz | power W | temp °C |
|---|---:|---:|---:|
| gpu_dpm_level | 2640 | 103 | 58.7 |
| rest | 3246 | 163 | 64.6 |

## Verdicts against `rest`

| metric | arm | mean → mean | change | verdict | why |
|---|---|---|---:|---|---|
| avg_fps | gpu_dpm_level | 89.4 → 82.2 | -8.0% | measurably slower | 8.0% slower, above the 0.1% run-to-run spread and significant at 95% (Welch's t = 199.75 against a 2.78 threshold) |
| low_1_fps | gpu_dpm_level | 65.0 → 60.1 | -7.6% | measurably slower | 7.6% slower, above the 2.1% run-to-run spread and significant at 95% (Welch's t = 5.28 against a 2.78 threshold) |
| low_0_1_fps | gpu_dpm_level | 50.2 → 43.4 | -13.5% | not enough evidence to say | the runs within an arm disagree too much to compare (variation 6.4% and 11.2%, above the 5% ceiling); something on the machine was interfering |

## Calibration

Recorded for this machine: gpu_dpm_level.

Measured 3 setting(s) on 2026-09-23: 0 helped, 2 hurt, 1 made no measurable difference.
