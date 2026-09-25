# Shadow of the Tomb Raider — settings on the lab laptop (2026-09-25)

Intel Core i7-7700HQ, GeForce GTX 1050 Ti Mobile (renders the game), Intel HD
630 (drives the panel), NVIDIA 580.178.04, Proton Experimental 11.0, KDE
Plasma Wayland, 1920×1080, the game's lowest preset. Turbo on.

One launch per arm, the built-in benchmark, MangoHud logging 110 s from the
first scene (`SOTTR_*.csv`); `gpu-samples.csv` is nvidia-smi once a second;
`game-result.png` is the game's own result panel where it was captured.
Runs A–B were started by hand, C–G driven with a virtual keyboard.

| Arm | Settings | MangoHud avg | 1 % low | Game's avg |
|---|---|---|---|---|
| A | DX12, XeSS Performance (960×540 → 1080p) | 36.2 | 13.0 | — |
| B | DX11 (DXVK), no XeSS | 33.9 | 7.7 | — |
| C | DX12, XeSS off, resolution modifier 60 % | — | — | 40 |
| D | C with async compute and high-precision RT off | 33.8 | 2.9 | 33 |
| E | A + OptiScaler 0.9.4 FSR 3.1 + OptiFG (frame generation) | 60.7 presented | 18.7 | — |
| F | A + OptiScaler 0.9.4 FSR 3.1, no frame generation | 40.6 | 13.7 | 36 |
| G | XeSS Quality (1280×720 → 1080p) + OptiScaler FSR 3.1 + OptiFG | 51.8 presented | 24.1 | — |

E and G count presented frames, generated ones included. OptiScaler's frame
generation raised two NVIDIA Xid errors (69 and 31) on this GTX and was
removed from the game afterwards; see docs/TOMB_RAIDER_BENCHMARK.md.

The GPU samples of F and G were overwritten by the sampler before they were
kept; their MangoHud logs are complete. E's result screen was not captured.
