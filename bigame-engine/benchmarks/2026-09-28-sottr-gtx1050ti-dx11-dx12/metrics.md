# 2026-09-28-sottr-gtx1050ti-dx11-dx12: every metric

The arms differ by design in EnableDX12 (the settings being compared); every other setting is identical across all runs, and every setting is identical within each arm.

Graphics settings identical across all 6 runs: AA 0 at 1920x1080, VSync false.

## Per run

| arm | run | avg fps | 1% low | 0.1% low | p99 ms | stutters | transitions | sclk MHz | power W | temp °C | GPU busy |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| dx11 | run-01 | 39.9 | 17.5 | 14.8 | 53.66 | 238 | 0 | 1493 | — | 80.9 | 89% |
| dx11 | run-02 | 38.6 | 16.4 | 11.2 | 53.43 | 180 | 0 | 1477 | — | 80.0 | 88% |
| dx11 | run-03 | 38.7 | 17.3 | 14.7 | 53.55 | 181 | 0 | 1486 | — | 80.3 | 89% |
| dx12 | run-01 | 25.5 | 17.2 | 12.1 | 52.55 | 2 | 0 | 1566 | — | 83.7 | 100% |
| dx12 | run-02 | 25.2 | 18.0 | 14.4 | 52.30 | 1 | 0 | 1574 | — | 82.6 | 100% |
| dx12 | run-03 | 24.8 | 15.1 | 8.2 | 54.78 | 5 | 0 | 1528 | — | 80.0 | 100% |

## Per arm (telemetry means while the GPU was busy)

| arm | sclk MHz | power W | temp °C |
|---|---:|---:|---:|
| dx11 | 1485 | — | 80.4 |
| dx12 | 1556 | — | 82.1 |

## Verdicts against `dx12`

| metric | arm | mean → mean | change | verdict | why |
|---|---|---|---:|---|---|
| avg_fps | dx11 | 25.2 → 39.1 | +55.3% | measurably faster | 55.3% faster, above the 1.8% run-to-run spread and significant at 95% (Welch's t = 30.52 against a 4.30 threshold) |
| low_1_fps | dx11 | 16.8 → 17.1 | +1.9% | not enough evidence to say | the runs within an arm disagree too much to compare (variation 8.9% and 3.2%, above the 5% ceiling); something on the machine was interfering |
| low_0_1_fps | dx11 | 11.5 → 13.6 | +17.4% | not enough evidence to say | the runs within an arm disagree too much to compare (variation 26.9% and 15.3%, above the 5% ceiling); something on the machine was interfering |
