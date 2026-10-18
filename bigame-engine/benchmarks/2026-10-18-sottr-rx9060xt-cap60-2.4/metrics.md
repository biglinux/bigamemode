# 2026-10-18-sottr-d1: every metric

Graphics settings identical across all 4 runs: AA 0 at 1920x1080, VSync false.

## Per run

| arm | run | avg fps | 1% low | 0.1% low | p99 ms | stutters | transitions | sclk MHz | power W | temp °C | GPU busy |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| cap60 | run-01 | 59.8 | 34.0 | 20.4 | 23.96 | 11 | 0 | 1631 | 52 | 58.6 | 68% |
| cap60 | run-02 | 59.7 | 32.4 | 16.3 | 23.79 | 15 | 0 | 1642 | 52 | 55.8 | 68% |
| pretty | run-01 | 79.2 | 35.7 | 18.6 | 22.32 | 68 | 0 | 1662 | 55 | 56.7 | 76% |
| pretty | run-02 | 79.7 | 34.5 | 14.5 | 22.14 | 51 | 0 | 1656 | 55 | 55.9 | 75% |

## Per arm (telemetry means while the GPU was busy)

| arm | sclk MHz | power W | temp °C |
|---|---:|---:|---:|
| cap60 | 1636 | 52 | 57.2 |
| pretty | 1659 | 55 | 56.3 |

## Verdicts against `cap60`

| metric | arm | mean → mean | change | verdict | why |
|---|---|---|---:|---|---|
| avg_fps | pretty | 59.7 → 79.4 | +33.0% | measurably faster | 33.0% faster, above the 0.5% run-to-run spread and significant at 95% (Welch's t = 70.23 against a 12.71 threshold) |
| low_1_fps | pretty | 33.2 → 35.1 | +5.8% | no difference above normal variation | a 5.8% difference, but Welch's t = 1.96 falls short of the 12.71 needed for 95% confidence at 1.8 degrees of freedom |
| low_0_1_fps | pretty | 18.4 → 16.6 | -9.9% | not enough evidence to say | the runs within an arm disagree too much to compare (variation 15.7% and 17.7%, above the 5% ceiling); something on the machine was interfering |
