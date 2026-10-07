# Shadow of the Tomb Raider — DX11 (DXVK) versus DX12 (VKD3D-Proton) on the GTX 1050 Ti (2026-09-28)

Lab laptop: Core i7-7700HQ, Intel HD 630 (drives the panel), GeForce GTX 1050
Ti Mobile (renders; NVIDIA 580.178.04), KDE Plasma Wayland, on AC, Turbo off.
The game's built-in benchmark, Proton Experimental, 1920×1080 borderless
fullscreen, preset Low, **no upscaler** (XeSS and DLSS off, OptiScaler
restored away: 1920×1080 rendered), VSync off, MangoHud loaded.

`scripts/bench-relaunch.sh dx12 dx11` with the arms in
`../_arms-hybrid/sottr-upscaling.sh`: one launch per run (the API is chosen at
start), a discarded warm-up pass in every launch, 3 runs per arm, rotated.
The only setting that differs between the arms is `EnableDX12` (the game's
own record of each run; `bench_native_report --vary=EnableDX12`).

## Results (bench_native_report, the game's own frame times)

| Arm | Avg FPS | 1 % low | 0.1 % low | p99 | Stutters / run | GPU busy | GPU clock | GPU temp |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| `dx12` | 25.2 | 16.8 | 11.5 | 53.2 ms | 2, 1, 5 | 100 % | 1556 MHz | 82.1 °C |
| `dx11` | 39.1 | 17.1 | 13.6 | 53.5 ms | 238, 180, 181 | 88–89 % | 1485 MHz | 80.4 °C |

- Average: DX11 **55.3 % faster**, above the 1.8 % spread and significant at
  95 % (Welch t = 30.52).
- 1 % and 0.1 % low: not enough evidence (the runs varied more than 5 %).
- Stutters (frames over twice the median): about 200 per pass in DX11, under
  5 in DX12.

At native 1080p DX11 renders many more frames, and paces them far worse:
the GPU was not kept busy (89 %), the frames the CPU held back show as
stutters. DX12 keeps the GPU at 100 % and runs smoothly, at a lower rate.
Neither is better on every count; on this GPU the choice is between the
higher average and the even pacing. At the game's lowest preset with XeSS
Performance (540p render, 2026-09-25) DX11 was the slower of the two: which
API renders more depends on how much of the frame is the GPU's.

Caveats: CPU package 91–97 °C with thermal throttling counted by the kernel;
Steam's web helper and an audio effects daemon ran throughout. Frame times,
GPU samples and the game's logs are not in the repository.
