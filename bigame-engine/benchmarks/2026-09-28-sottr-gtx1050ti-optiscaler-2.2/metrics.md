# 2026-09-28-sottr-gtx1050ti-optiscaler-2.2: every metric

Graphics settings identical across all 6 runs: AA 0 at 1920x1080, VSync false.

## Per run

| arm | run | avg fps | 1% low | 0.1% low | p99 ms | stutters | transitions | sclk MHz | power W | temp °C | GPU busy |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| native_xess | run-01 | 24.1 | 16.8 | 12.3 | 53.55 | 1 | 0 | 1571 | — | 82.1 | 100% |
| native_xess | run-02 | 24.8 | 17.7 | 12.9 | 52.29 | 1 | 0 | 1612 | — | 81.5 | 100% |
| native_xess | run-03 | 23.6 | 16.4 | 12.5 | 56.73 | 1 | 0 | 1521 | — | 78.9 | 99% |
| optiscaler | run-01 | 28.5 | 16.2 | 9.8 | 49.67 | 8 | 0 | 1571 | — | 84.3 | 99% |
| optiscaler | run-02 | 28.8 | 19.0 | 13.4 | 46.02 | 3 | 0 | 1567 | — | 84.9 | 99% |
| optiscaler | run-03 | 29.9 | 17.6 | 10.9 | 47.43 | 8 | 0 | 1634 | — | 85.6 | 100% |

## Per arm (telemetry means while the GPU was busy)

| arm | sclk MHz | power W | temp °C |
|---|---:|---:|---:|
| native_xess | 1568 | — | 80.8 |
| optiscaler | 1591 | — | 84.9 |

## Verdicts against `native_xess`

| metric | arm | mean → mean | change | verdict | why |
|---|---|---|---:|---|---|
| avg_fps | optiscaler | 24.2 → 29.0 | +20.1% | measurably faster | 20.1% faster, above the 2.6% run-to-run spread and significant at 95% (Welch's t = 8.76 against a 3.18 threshold) |
| low_1_fps | optiscaler | 17.0 → 17.6 | +3.8% | not enough evidence to say | the runs within an arm disagree too much to compare (variation 4.0% and 8.1%, above the 5% ceiling); something on the machine was interfering |
| low_0_1_fps | optiscaler | 12.6 → 11.4 | -9.5% | not enough evidence to say | the runs within an arm disagree too much to compare (variation 2.5% and 16.1%, above the 5% ceiling); something on the machine was interfering |
