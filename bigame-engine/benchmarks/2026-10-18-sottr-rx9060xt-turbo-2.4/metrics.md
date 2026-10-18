# 2026-10-18-sottr-d2-turbo: every metric

Graphics settings identical across all 4 runs: AA 0 at 1920x1080, VSync false.

## Per run

| arm | run | avg fps | 1% low | 0.1% low | p99 ms | stutters | transitions | sclk MHz | power W | temp °C | GPU busy |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| turbo_off | run-01 | 79.5 | 34.5 | 15.6 | 22.45 | 72 | 0 | 1657 | 54 | 55.7 | 75% |
| turbo_off | run-02 | 81.9 | 37.5 | 19.8 | 21.66 | 72 | 0 | 1662 | 54 | 55.6 | 75% |
| turbo_on | run-01 | 80.7 | 35.7 | 17.8 | 22.26 | 69 | 0 | 1663 | 54 | 55.6 | 74% |
| turbo_on | run-02 | 80.0 | 37.1 | 20.2 | 22.15 | 60 | 0 | 1653 | 54 | 55.7 | 74% |

## Per arm (telemetry means while the GPU was busy)

| arm | sclk MHz | power W | temp °C |
|---|---:|---:|---:|
| turbo_off | 1660 | 54 | 55.7 |
| turbo_on | 1658 | 54 | 55.6 |

## Verdicts against `turbo_off`

| metric | arm | mean → mean | change | verdict | why |
|---|---|---|---:|---|---|
| avg_fps | turbo_on | 80.7 → 80.4 | -0.4% | no difference above normal variation | the 0.4% difference is within the 2.0% spread of the runs themselves, so it cannot be attributed to the change |
| low_1_fps | turbo_on | 36.0 → 36.4 | +1.2% | not enough evidence to say | the runs within an arm disagree too much to compare (variation 5.8% and 2.8%, above the 5% ceiling); something on the machine was interfering |
| low_0_1_fps | turbo_on | 17.7 → 19.0 | +7.4% | not enough evidence to say | the runs within an arm disagree too much to compare (variation 16.8% and 9.0%, above the 5% ceiling); something on the machine was interfering |
