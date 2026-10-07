# 2026-09-26-cyberpunk-rx9060xt-native-vs-optiscaler: every metric

The arms differ by design in FSR3Enabled, XeSSEnabled, upscalingType, FSR3Quality, XeSSQuality (the settings being compared); every other setting is identical across all runs, and every setting is identical within each arm.

Graphics settings identical across all 6 runs: ? at ?x?, VSync ?.

## Per run

| arm | run | avg fps | 1% low | 0.1% low | p99 ms | stutters | transitions | sclk MHz | power W | temp °C | GPU busy |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| native_fsr31 | run-01 | 38.5 | 31.1 | 29.1 | 31.39 | 0 | 0 | — | — | — | — |
| native_fsr31 | run-02 | 38.3 | 28.2 | 19.0 | 31.46 | 1 | 0 | — | — | — | — |
| native_fsr4 | run-01 | 38.3 | 30.5 | 27.9 | 31.78 | 0 | 0 | — | — | — | — |
| native_fsr4 | run-02 | 38.2 | 26.1 | 12.8 | 31.92 | 2 | 0 | — | — | — | — |
| optiscaler_fsr | run-01 | 36.0 | 28.5 | 25.2 | 33.61 | 0 | 0 | — | — | — | — |
| optiscaler_fsr | run-02 | 36.0 | 29.3 | 27.8 | 33.31 | 0 | 0 | — | — | — | — |

## Per arm (telemetry means while the GPU was busy)

| arm | sclk MHz | power W | temp °C |
|---|---:|---:|---:|
| native_fsr31 | NaN | NaN | NaN |
| native_fsr4 | NaN | NaN | NaN |
| optiscaler_fsr | NaN | NaN | NaN |

## Verdicts against `native_fsr31`

| metric | arm | mean → mean | change | verdict | why |
|---|---|---|---:|---|---|
| avg_fps | native_fsr4 | 38.4 → 38.2 | -0.6% | no difference above normal variation | a 0.6% difference, but Welch's t = 1.90 falls short of the 12.71 needed for 95% confidence at 1.6 degrees of freedom |
| avg_fps | optiscaler_fsr | 38.4 → 36.0 | -6.4% | measurably slower | 6.4% slower, above the 0.4% run-to-run spread and significant at 95% (Welch's t = 24.69 against a 12.71 threshold) |
| low_1_fps | native_fsr4 | 29.6 → 28.3 | -4.5% | not enough evidence to say | the runs within an arm disagree too much to compare (variation 6.8% and 11.0%, above the 5% ceiling); something on the machine was interfering |
| low_1_fps | optiscaler_fsr | 29.6 → 28.9 | -2.6% | not enough evidence to say | the runs within an arm disagree too much to compare (variation 6.8% and 2.1%, above the 5% ceiling); something on the machine was interfering |
| low_0_1_fps | native_fsr4 | 24.1 → 20.3 | -15.5% | not enough evidence to say | the runs within an arm disagree too much to compare (variation 29.6% and 52.3%, above the 5% ceiling); something on the machine was interfering |
| low_0_1_fps | optiscaler_fsr | 24.1 → 26.5 | +10.0% | not enough evidence to say | the runs within an arm disagree too much to compare (variation 29.6% and 6.9%, above the 5% ceiling); something on the machine was interfering |
