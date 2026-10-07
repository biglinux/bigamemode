# 2026-09-24-sottr-scheduler: every metric

Graphics settings identical across all 9 runs: AA 2 at 3440x1440, VSync false.

## Per run

| arm | run | avg fps | 1% low | 0.1% low | p99 ms | stutters | transitions | sclk MHz | power W | temp °C | GPU busy |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| scx_bpfland | run-01 | 123.0 | 68.8 | 32.9 | 12.22 | 8 | 0 | 2388 | 74 | 54.4 | 85% |
| scx_bpfland | run-02 | 124.0 | 58.1 | 17.7 | 12.18 | 19 | 0 | 2392 | 73 | 54.3 | 85% |
| scx_bpfland | run-03 | 121.1 | 50.2 | 12.9 | 12.38 | 28 | 0 | 2372 | 73 | 54.4 | 85% |
| scx_lavd | run-01 | 123.0 | 73.9 | 56.2 | 12.57 | 14 | 0 | 2378 | 73 | 54.1 | 85% |
| scx_lavd | run-02 | 122.7 | 73.5 | 55.8 | 12.54 | 18 | 0 | 2365 | 74 | 54.5 | 86% |
| scx_lavd | run-03 | 119.8 | 68.6 | 40.5 | 12.73 | 18 | 0 | 2333 | 71 | 54.1 | 85% |
| scx_none | run-01 | 120.6 | 71.3 | 53.5 | 13.07 | 11 | 0 | 2328 | 72 | 54.0 | 85% |
| scx_none | run-02 | 121.5 | 70.8 | 48.9 | 12.88 | 16 | 0 | 2323 | 72 | 54.2 | 86% |
| scx_none | run-03 | 119.5 | 69.0 | 54.2 | 13.50 | 24 | 0 | 2291 | 71 | 54.2 | 85% |

## Per arm (telemetry means while the GPU was busy)

| arm | sclk MHz | power W | temp °C |
|---|---:|---:|---:|
| scx_bpfland | 2384 | 73 | 54.4 |
| scx_lavd | 2358 | 73 | 54.2 |
| scx_none | 2314 | 72 | 54.1 |

## Verdicts against `scx_none`

| metric | arm | mean → mean | change | verdict | why |
|---|---|---|---:|---|---|
| avg_fps | scx_bpfland | 120.6 → 122.7 | +1.8% | no difference above normal variation | a 1.8% difference, but Welch's t = 2.12 falls short of the 2.78 needed for 95% confidence at 3.5 degrees of freedom |
| avg_fps | scx_lavd | 120.6 → 121.8 | +1.1% | no difference above normal variation | the 1.1% difference is within the 1.4% spread of the runs themselves, so it cannot be attributed to the change |
| low_1_fps | scx_bpfland | 70.4 → 59.0 | -16.1% | not enough evidence to say | the runs within an arm disagree too much to compare (variation 1.7% and 15.8%, above the 5% ceiling); something on the machine was interfering |
| low_1_fps | scx_lavd | 70.4 → 72.0 | +2.3% | no difference above normal variation | the 2.3% difference is within the 4.1% spread of the runs themselves, so it cannot be attributed to the change |
| low_0_1_fps | scx_bpfland | 52.2 → 21.2 | -59.5% | not enough evidence to say | the runs within an arm disagree too much to compare (variation 5.5% and 49.3%, above the 5% ceiling); something on the machine was interfering |
| low_0_1_fps | scx_lavd | 52.2 → 50.8 | -2.6% | not enough evidence to say | the runs within an arm disagree too much to compare (variation 5.5% and 17.6%, above the 5% ceiling); something on the machine was interfering |
