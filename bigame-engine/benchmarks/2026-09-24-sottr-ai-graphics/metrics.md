# 2026-09-24-sottr-ai-graphics: every metric

The arms differ by design in XESS, AA (the settings being compared); every other setting is identical across all runs, and every setting is identical within each arm.

Graphics settings identical across all 9 runs: AA 2 at 3440x1440, VSync false.

## Per run

| arm | run | avg fps | 1% low | 0.1% low | p99 ms | stutters | transitions | sclk MHz | power W | temp °C | GPU busy |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| native_taa | run-01 | 89.6 | 60.5 | 36.6 | 14.44 | 6 | 0 | 3263 | 164 | 65.4 | 99% |
| native_taa | run-02 | 89.8 | 61.6 | 39.9 | 14.42 | 6 | 0 | 3258 | 164 | 65.4 | 99% |
| native_taa | run-03 | 89.9 | 65.9 | 51.2 | 14.29 | 2 | 0 | 3248 | 164 | 65.6 | 99% |
| native_xess | run-01 | 93.9 | 62.3 | 42.2 | 14.35 | 4 | 0 | 3276 | 155 | 64.8 | 98% |
| native_xess | run-02 | 94.2 | 66.3 | 51.2 | 13.78 | 2 | 0 | 3242 | 153 | 64.8 | 98% |
| native_xess | run-03 | 94.4 | 64.8 | 42.3 | 13.75 | 6 | 0 | 3246 | 153 | 65.0 | 98% |
| optiscaler_fsr | run-01 | 99.0 | 61.0 | 40.2 | 14.55 | 7 | 0 | 3152 | 156 | 65.0 | 96% |
| optiscaler_fsr | run-02 | 98.7 | 60.5 | 38.0 | 14.51 | 10 | 0 | 3154 | 155 | 65.1 | 96% |
| optiscaler_fsr | run-03 | 98.7 | 60.9 | 33.4 | 14.06 | 8 | 0 | 3157 | 156 | 65.1 | 97% |

## Per arm (telemetry means while the GPU was busy)

| arm | sclk MHz | power W | temp °C |
|---|---:|---:|---:|
| native_taa | 3256 | 164 | 65.5 |
| native_xess | 3254 | 154 | 64.9 |
| optiscaler_fsr | 3154 | 156 | 65.1 |

## Verdicts against `native_taa`

| metric | arm | mean → mean | change | verdict | why |
|---|---|---|---:|---|---|
| avg_fps | native_xess | 89.8 → 94.2 | +4.9% | measurably faster | 4.9% faster, above the 0.2% run-to-run spread and significant at 95% (Welch's t = 28.80 against a 2.78 threshold) |
| avg_fps | optiscaler_fsr | 89.8 → 98.8 | +10.1% | measurably faster | 10.1% faster, above the 0.2% run-to-run spread and significant at 95% (Welch's t = 71.68 against a 2.78 threshold) |
| low_1_fps | native_xess | 62.7 → 64.5 | +2.9% | no difference above normal variation | the 2.9% difference is within the 4.6% spread of the runs themselves, so it cannot be attributed to the change |
| low_1_fps | optiscaler_fsr | 62.7 → 60.8 | -2.9% | no difference above normal variation | the 2.9% difference is within the 4.6% spread of the runs themselves, so it cannot be attributed to the change |
| low_0_1_fps | native_xess | 42.6 → 45.2 | +6.3% | not enough evidence to say | the runs within an arm disagree too much to compare (variation 17.9% and 11.4%, above the 5% ceiling); something on the machine was interfering |
| low_0_1_fps | optiscaler_fsr | 42.6 → 37.2 | -12.6% | not enough evidence to say | the runs within an arm disagree too much to compare (variation 17.9% and 9.3%, above the 5% ceiling); something on the machine was interfering |
