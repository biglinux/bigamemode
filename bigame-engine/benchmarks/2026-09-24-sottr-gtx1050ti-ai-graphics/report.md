# 2026-09-24-sottr-gtx1050ti-ai-graphics — 2026-09-24

## Verdict

- **native_xess_2** — FASTER: 6.4% faster, above the 1.1% run-to-run spread and significant at 95% (Welch's t = 6.82 against a 3.18 threshold)
- **optiscaler_fsr** — FASTER: 16.3% faster, above the 1.1% run-to-run spread and significant at 95% (Welch's t = 23.91 against a 3.18 threshold)

## Measurements

| Arm | Runs | Mean avg_fps | Median | Min | Max | Spread | vs baseline |
|---|---:|---:|---:|---:|---:|---:|---:|
| native_xess | 3 | 15.7 | 15.7 | 15.5 | 15.9 | 1.1% | — |
| native_xess_2 | 2 | 16.7 | 16.7 | 16.6 | 16.8 | 0.9% | +6.4% |
| optiscaler_fsr | 3 | 18.2 | 18.3 | 18.2 | 18.3 | 0.4% | +16.3% |

## Method

- 3 measured run(s) per arm, 1 warm-up run(s) discarded.
- Arms were alternated (A B A B …) rather than grouped, so that drift over the session — chassis temperature above all — falls on both arms equally instead of on whichever ran last.
- A difference is called real only when it exceeds the run-to-run spread of both arms *and* passes Welch's t-test at 95%. Anything smaller is reported as no change, not as a small gain.
- Machine fingerprint `95e20f0928d117b7`.

## Caveats

- Lab laptop: Core i7-7700HQ, Intel HD 630 + GeForce GTX 1050 Ti Mobile (hybrid, NVIDIA 580.178.04), 1920x1080 panel, Proton Experimental, KDE Wayland; the game rendered on the GTX (render card observed: card0 via /dev/nvidia0).
- Settings in every run: DX12, 1920x1080 fullscreen (borderless), preset High, Intel XeSS Quality selected in the game's menu, VSync off. The arms differ only in files: none (native_xess, native_xess_2) or OptiScaler 0.9.4 as dxgi.dll running FSR 3.1 in place of the game's XeSS (optiscaler_fsr).
- Order A, B, A: an arm needs its own launch (OptiScaler's files cannot change while the game runs); each launch had its own discarded warm-up pass, the first one including Vulkan shader compilation.
- GPU telemetry is in each run's nvidia.csv (nvidia-smi, 1 s): utilisation, graphics and memory clocks, temperature, VRAM. This laptop GPU reports no power draw. The report's GPU columns read amdgpu's gpu.csv and are empty here.
- The game itself reports "GPU limit 100%": GPU-bound throughout. VRAM use was ~3.77 GB of 4 GB at High in both arms.
- StereoSeparation read 0.0225 in the first launch and 0.0220 in the later two; stereoscopic 3D is off (Stereoscopic3DMode=0) in every run, so it does not affect rendering. Declared with --vary=StereoSeparation; every other setting is identical in all runs.

## Raw runs

- `native_xess`: 15.9, 15.7, 15.5
- `native_xess_2`: 16.6, 16.8
- `optiscaler_fsr`: 18.2, 18.3, 18.3
