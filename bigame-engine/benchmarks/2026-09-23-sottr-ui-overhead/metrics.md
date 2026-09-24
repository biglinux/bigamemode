# 2026-09-23-sottr-ui-overhead: every metric

Graphics settings identical across all 6 runs: AA 2 at 3440x1440, VSync false.

## Per run

| arm | run | avg fps | 1% low | 0.1% low | p99 ms | stutters | transitions | sclk MHz | power W | temp °C | GPU busy |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| ui_paused | run-01 | 116.4 | 68.8 | 46.3 | 13.29 | 9 | 0 | 2264 | 68 | 54.2 | 84% |
| ui_paused | run-02 | 116.6 | 69.1 | 51.4 | 13.32 | 12 | 0 | 2262 | 69 | 54.1 | 84% |
| ui_paused | run-03 | 115.7 | 68.7 | 53.8 | 13.42 | 14 | 0 | 2277 | 69 | 54.2 | 84% |
| ui_polling | run-01 | 114.8 | 68.6 | 55.2 | 13.55 | 9 | 0 | 2222 | 67 | 54.4 | 83% |
| ui_polling | run-02 | 115.8 | 67.8 | 54.1 | 13.71 | 13 | 0 | 2238 | 69 | 54.1 | 84% |
| ui_polling | run-03 | 114.4 | 67.6 | 52.7 | 13.64 | 16 | 0 | 2203 | 67 | 54.1 | 84% |

## Per arm (telemetry means while the GPU was busy)

| arm | sclk MHz | power W | temp °C |
|---|---:|---:|---:|
| ui_paused | 2268 | 69 | 54.2 |
| ui_polling | 2221 | 68 | 54.2 |

## Verdicts against `ui_polling`

| metric | arm | mean → mean | change | verdict | why |
|---|---|---|---:|---|---|
| avg_fps | ui_paused | 115.0 → 116.2 | +1.1% | no difference above normal variation | a 1.1% difference, but Welch's t = 2.38 falls short of the 2.78 needed for 95% confidence at 3.4 degrees of freedom |
| low_1_fps | ui_paused | 68.0 → 68.8 | +1.2% | no difference above normal variation | a 1.2% difference, but Welch's t = 2.47 falls short of the 3.18 needed for 95% confidence at 2.6 degrees of freedom |
| low_0_1_fps | ui_paused | 54.0 → 50.5 | -6.6% | not enough evidence to say | the runs within an arm disagree too much to compare (variation 2.3% and 7.6%, above the 5% ceiling); something on the machine was interfering |
