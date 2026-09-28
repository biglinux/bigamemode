# Shadow of the Tomb Raider — OptiScaler (FSR 3.1) versus the game's own XeSS, BiGame-mode 2.2 (2026-09-28)

Lab laptop: Core i7-7700HQ, Intel HD 630 (drives the panel), GeForce GTX 1050
Ti Mobile (renders; NVIDIA 580.178.04), KDE Plasma Wayland, on AC, Turbo off.
The game's built-in benchmark, DX12 (VKD3D-Proton, Proton Experimental),
1920×1080 borderless fullscreen, preset Low, XeSS Quality selected (render
1280×720), VSync off, MangoHud loaded, no frame generation, no Wine FSR.

`scripts/bench-relaunch.sh native_xess optiscaler` with the arms in
`../_arms-hybrid/sottr-upscaling.sh`: one launch per run (OptiScaler's files
cannot change while the game runs), a discarded warm-up pass in every launch,
3 runs per arm, the order rotated.

| Arm | What ran | Evidence |
|---|---|---|
| `native_xess` | the game's XeSS 1.1 Quality | OptiScaler restored away by AI Graphics (no `dxgi.dll`, its log removed; the next arm's log starts later) |
| `optiscaler` | AI Graphics' Recommended plan: OptiScaler 0.9.4 as `dxgi.dll`, FSR 3.1 in place of the game's XeSS, frame generation off | `OptiScaler.log`: "Render Resolution: 1280x720, Display Resolution 1920x1080", "init successful for fsr31" |

## Results (bench_native_report, the game's own frame times)

| Arm | Avg FPS | 1 % low | 0.1 % low | p99 | Stutters / run | GPU clock | GPU temp |
|---|---:|---:|---:|---:|---:|---:|---:|
| `native_xess` | 24.2 | 17.0 | 12.6 | 54.2 ms | 1, 1, 1 | 1568 MHz | 80.8 °C |
| `optiscaler` | 29.0 | 17.6 | 11.4 | 47.7 ms | 8, 3, 8 | 1591 MHz | 84.9 °C |

- Average: OptiScaler **20.1 % faster**, above the 2.6 % run-to-run spread and
  significant at 95 % (Welch t = 8.76).
- 1 % and 0.1 % low: not enough evidence; OptiScaler's runs varied 8.1 % and
  16.1 %, above the 5 % ceiling.
- Stutters: more with OptiScaler in every round (frames over twice the
  median).

So on this GTX, FSR 3.1 through OptiScaler renders more frames per second
than the game's own XeSS (which runs XeSS's DP4a path on a card without
Intel's XMX units) at the same render size, with more uneven pacing. The GPU
was busy 99–100 % in every run.

Caveats: CPU package 91–97 °C with the kernel counting thermal throttling;
Steam's web helper and an audio effects daemon ran throughout. Frame times,
GPU samples and the game's logs are not in the repository.
