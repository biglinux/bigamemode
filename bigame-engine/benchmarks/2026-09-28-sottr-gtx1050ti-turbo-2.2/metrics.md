# 2026-09-28-sottr-gtx1050ti-turbo-2.2: every metric

Graphics settings identical across all 6 runs: AA 0 at 1920x1080, VSync false.

## Per run

| arm | run | avg fps | 1% low | 0.1% low | p99 ms | stutters | transitions | sclk MHz | power W | temp °C | GPU busy |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| turbo_off | run-01 | 24.1 | 16.8 | 10.4 | 51.63 | 4 | 0 | 1668 | — | 90.5 | 100% |
| turbo_off | run-02 | 24.1 | 17.0 | 9.7 | 51.08 | 3 | 0 | 1657 | — | 92.0 | 100% |
| turbo_off | run-03 | 24.1 | 17.2 | 10.3 | 51.44 | 3 | 0 | 1657 | — | 92.1 | 100% |
| turbo_on | run-01 | 24.1 | 17.4 | 10.3 | 50.16 | 2 | 0 | 1662 | — | 91.6 | 100% |
| turbo_on | run-02 | 24.0 | 17.0 | 10.4 | 51.36 | 3 | 0 | 1658 | — | 91.8 | 100% |
| turbo_on | run-03 | 24.3 | 17.6 | 10.6 | 50.46 | 3 | 0 | 1657 | — | 92.3 | 100% |

## Per arm (telemetry means while the GPU was busy)

| arm | sclk MHz | power W | temp °C |
|---|---:|---:|---:|
| turbo_off | 1661 | — | 91.5 |
| turbo_on | 1659 | — | 91.9 |

## Verdicts against `turbo_off`

| metric | arm | mean → mean | change | verdict | why |
|---|---|---|---:|---|---|
| avg_fps | turbo_on | 24.1 → 24.1 | +0.0% | no difference above normal variation | the 0.0% difference is within the 0.5% spread of the runs themselves, so it cannot be attributed to the change |
| low_1_fps | turbo_on | 17.0 → 17.3 | +1.8% | no difference above normal variation | the 1.8% difference is within the 2.0% spread of the runs themselves, so it cannot be attributed to the change |
| low_0_1_fps | turbo_on | 10.1 → 10.4 | +3.1% | no difference above normal variation | the 3.1% difference is within the 3.6% spread of the runs themselves, so it cannot be attributed to the change |
