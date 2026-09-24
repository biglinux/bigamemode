# 2026-09-23-sottr-cpu-bound: every metric

Graphics settings identical across all 9 runs: AA 2 at 3440x1440, VSync false.

## Per run

| arm | run | avg fps | 1% low | 0.1% low | p99 ms | stutters | transitions | sclk MHz | power W | temp °C | GPU busy |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| baseline | run-01 | 116.6 | 67.6 | 53.8 | 13.69 | 21 | 0 | 2264 | 69 | 54.1 | 84% |
| baseline | run-02 | 117.2 | 69.5 | 56.0 | 13.42 | 11 | 0 | 2273 | 70 | 54.2 | 85% |
| baseline | run-03 | 109.1 | 38.6 | 15.7 | 19.49 | 305 | 1 | 2222 | 67 | 54.5 | 81% |
| cpu_governor | run-01 | 116.0 | 68.0 | 53.4 | 13.63 | 19 | 0 | 2274 | 70 | 54.2 | 85% |
| cpu_governor | run-02 | 115.1 | 66.9 | 53.9 | 13.90 | 17 | 0 | 2247 | 69 | 54.2 | 84% |
| cpu_governor | run-03 | 107.8 | 41.4 | 13.1 | 16.47 | 108 | 0 | 2086 | 61 | 55.9 | 82% |
| rest | run-01 | 117.7 | 71.5 | 59.5 | 13.13 | 4 | 0 | 2310 | 70 | 54.3 | 84% |
| rest | run-02 | 116.4 | 66.5 | 44.8 | 13.57 | 18 | 0 | 2254 | 69 | 54.3 | 84% |
| rest | run-03 | 116.0 | 67.6 | 51.7 | 13.65 | 14 | 0 | 2253 | 69 | 54.3 | 84% |

## Per arm (telemetry means while the GPU was busy)

| arm | sclk MHz | power W | temp °C |
|---|---:|---:|---:|
| baseline | 2253 | 69 | 54.3 |
| cpu_governor | 2202 | 67 | 54.8 |
| rest | 2273 | 69 | 54.3 |

## Verdicts against `baseline`

| metric | arm | mean → mean | change | verdict | why |
|---|---|---|---:|---|---|
| avg_fps | cpu_governor | 114.3 → 113.0 | -1.1% | no difference above normal variation | the 1.1% difference is within the 4.0% spread of the runs themselves, so it cannot be attributed to the change |
| avg_fps | rest | 114.3 → 116.7 | +2.1% | no difference above normal variation | the 2.1% difference is within the 3.9% spread of the runs themselves, so it cannot be attributed to the change |
| low_1_fps | cpu_governor | 58.6 → 58.8 | +0.4% | not enough evidence to say | the runs within an arm disagree too much to compare (variation 29.6% and 25.6%, above the 5% ceiling); something on the machine was interfering |
| low_1_fps | rest | 58.6 → 68.5 | +17.0% | not enough evidence to say | the runs within an arm disagree too much to compare (variation 29.6% and 3.9%, above the 5% ceiling); something on the machine was interfering |
| low_0_1_fps | cpu_governor | 41.8 → 40.1 | -4.0% | not enough evidence to say | the runs within an arm disagree too much to compare (variation 54.2% and 58.2%, above the 5% ceiling); something on the machine was interfering |
| low_0_1_fps | rest | 41.8 → 52.0 | +24.3% | not enough evidence to say | the runs within an arm disagree too much to compare (variation 54.2% and 14.2%, above the 5% ceiling); something on the machine was interfering |

## Calibration

Recorded for this machine: cpu_governor.

Measured 3 setting(s) on 2026-09-23: 0 helped, 2 hurt, 1 made no measurable difference.
