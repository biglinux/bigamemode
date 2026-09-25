# 2026-09-24-sottr-gtx1050ti-ai-graphics: every metric

The arms differ by design in StereoSeparation (the settings being compared); every other setting is identical across all runs, and every setting is identical within each arm.

Graphics settings identical across all 8 runs: AA 0 at 1920x1080, VSync false.

## Per run

| arm | run | avg fps | 1% low | 0.1% low | p99 ms | stutters | transitions | sclk MHz | power W | temp °C | GPU busy |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| native_xess | run-01 | 15.9 | 9.2 | 5.6 | 85.30 | 4 | 0 | — | — | — | — |
| native_xess | run-02 | 15.7 | 11.0 | 8.5 | 82.52 | 0 | 0 | — | — | — | — |
| native_xess | run-03 | 15.5 | 10.3 | 6.1 | 84.39 | 3 | 0 | — | — | — | — |
| native_xess_2 | run-01 | 16.6 | 11.9 | 8.8 | 77.34 | 1 | 0 | — | — | — | — |
| native_xess_2 | run-02 | 16.8 | 10.6 | 5.4 | 77.21 | 2 | 0 | — | — | — | — |
| optiscaler_fsr | run-01 | 18.2 | 10.5 | 6.0 | 77.42 | 2 | 0 | — | — | — | — |
| optiscaler_fsr | run-02 | 18.3 | 10.2 | 5.4 | 74.55 | 6 | 0 | — | — | — | — |
| optiscaler_fsr | run-03 | 18.3 | 8.7 | 4.4 | 81.57 | 8 | 0 | — | — | — | — |

## Per arm (telemetry means while the GPU was busy)

| arm | sclk MHz | power W | temp °C |
|---|---:|---:|---:|
| native_xess | NaN | NaN | NaN |
| native_xess_2 | NaN | NaN | NaN |
| optiscaler_fsr | NaN | NaN | NaN |

## Verdicts against `native_xess`

| metric | arm | mean → mean | change | verdict | why |
|---|---|---|---:|---|---|
| avg_fps | native_xess_2 | 15.7 → 16.7 | +6.4% | measurably faster | 6.4% faster, above the 1.1% run-to-run spread and significant at 95% (Welch's t = 6.82 against a 3.18 threshold) |
| avg_fps | optiscaler_fsr | 15.7 → 18.2 | +16.3% | measurably faster | 16.3% faster, above the 1.1% run-to-run spread and significant at 95% (Welch's t = 23.91 against a 3.18 threshold) |
| low_1_fps | native_xess_2 | 10.2 → 11.3 | +11.0% | not enough evidence to say | the runs within an arm disagree too much to compare (variation 9.1% and 7.8%, above the 5% ceiling); something on the machine was interfering |
| low_1_fps | optiscaler_fsr | 10.2 → 9.8 | -3.3% | not enough evidence to say | the runs within an arm disagree too much to compare (variation 9.1% and 9.7%, above the 5% ceiling); something on the machine was interfering |
| low_0_1_fps | native_xess_2 | 6.7 → 7.1 | +5.8% | not enough evidence to say | the runs within an arm disagree too much to compare (variation 23.0% and 34.6%, above the 5% ceiling); something on the machine was interfering |
| low_0_1_fps | optiscaler_fsr | 6.7 → 5.3 | -21.5% | not enough evidence to say | the runs within an arm disagree too much to compare (variation 23.0% and 15.4%, above the 5% ceiling); something on the machine was interfering |
