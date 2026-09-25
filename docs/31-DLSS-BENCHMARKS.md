# 31 — AI Graphics benchmarks

Shadow of the Tomb Raider's built-in benchmark, 3440×1440, High preset, on
the reference machine (Ryzen 7 5700G, Radeon RX 9060 XT — RDNA 4, Mesa
26.2.2, Proton Experimental, KDE Wayland, performance power profile), on
2026-09-24. Raw data: `bigame-engine/benchmarks/2026-09-24-sottr-ai-graphics/`.
Report: `bench_native_report … native_taa --vary=XESS,AA`. Development
documentation.

## Method

One launch per arm; the first pass is a warm-up and is discarded; three
measured runs per arm; the game's own per-frame log is the data; verdicts
from run-to-run spread and Welch's t at 95 %. Every graphics setting other
than the two that *are* the comparison (`XESS`, `AA`) was identical across
all nine runs, and every setting was identical within each arm — checked from
the result files, not from the menu (that check is how a first attempt with
`XESS=0` was caught: the menu showed XeSS Quality, but the game had not
applied it).

The upscaler was set in the game's own menu, as a player would, because that
is how OptiScaler works: it takes over the upscaler the game runs. Render
resolution with XeSS Quality: 2293×960, from OptiScaler's log.

| Arm | What the game ran |
|---|---|
| `native_taa` | the game's TAA at native resolution — the user's normal settings |
| `native_xess` | the game's own XeSS 1.1 (the DLL it ships), Quality |
| `optiscaler_fsr` | XeSS Quality selected in the game; OptiScaler 0.9.4 as `dxgi.dll` running AMD's FSR (`fsr31` backend, `Fsr4Update: true`) in its place |

## Results

| Arm | avg fps | 1 % low | 0.1 % low | GPU busy | GPU power |
|---|---|---|---|---|---|
| `native_taa` | 89.6 · 89.8 · 89.9 | 60.5 · 61.6 · 65.9 | 36.6 · 39.9 · 51.2 | 99 % | 164 W |
| `native_xess` | 93.9 · 94.2 · 94.4 | 62.3 · 66.3 · 64.8 | 42.2 · 51.2 · 42.3 | 98 % | 154 W |
| `optiscaler_fsr` | 99.0 · 98.7 · 98.7 | 61.0 · 60.5 · 60.9 | 40.2 · 38.0 · 33.4 | 96 % | 156 W |

| Comparison (vs TAA) | Average | Verdict |
|---|---|---|
| the game's XeSS Quality | **+4.9 %** (89.8 → 94.2) | measurably faster — spread 0.2 %, t = 28.8 |
| OptiScaler FSR from XeSS Quality | **+10.1 %** (89.8 → 98.8) | measurably faster — spread 0.2 %, t = 71.7 |
| 1 % low, both | ±2.9 % | no difference above the 4.6 % spread |
| 0.1 % low, both | — | not enough evidence: runs within an arm vary 9–18 % |

So on this machine, for this game, OptiScaler's FSR at the same render
resolution is **+5 % over the game's own XeSS**, and a tenth faster than
native TAA, with the frame-time floor unchanged. GPU power fell with either
upscaler (164 → 154–156 W), as expected from rendering fewer pixels.

## What was measured, and what was not

- **Rendered frames**, not presented: no frame generation was on in any
  run, and the report refuses runs where it is.
- **FSR 4 or FSR 3?** OptiScaler's log proves the `fsr31` backend was
  created with `Fsr4Update: true` on RDNA 4, and the watermark option was
  set for the first session; which model actually ran is shown only by
  OptiScaler's overlay, which was not read. The UI therefore says "FSR",
  never "FSR 4 confirmed". A Valve developer has reported that Proton's own
  `amdxcffx64.dll` path renders the FSR 3 model on RDNA 4 — that path is what
  OptiScaler loaded here (`amdxcffx64.dll loaded from system path`). The
  +5 % over XeSS stands either way; the FSR 4 claim does not, yet.
- **Visual quality: inconclusive.** Screenshots were taken 25 s and 60 s into
  a pass of each arm. Frame timing differs between arms, so the captures are
  a few frames apart and cannot be compared pixel by pixel; the 60 s
  OptiScaler capture landed on a black transition frame. At a third of the
  size nothing was visibly wrong in any of the three: no ghosting behind
  Lara, no shimmer on the string lights, HUD intact. A proper comparison needs
  the game's photo mode or a fixed frame, which the benchmark does not offer.
- **Latency** was not measured (no frame generation was involved; upscaling
  does not add presentation latency beyond its render time, which the
  frame-time figures include).
- **One title, one GPU, one mode.** Cyberpunk 2077 was analysed and planned
  but not installed or run in this pass. NVIDIA and Intel GPUs: NOT TESTED.

## Earlier sessions on the same day

The Turbo pass measured CPU/power knobs and sched-ext schedulers in this
game ([20](20-BENCHMARK-RESULTS.md)); none moved the frame rate. AI
Graphics is the first setting BiGame-mode can apply that measurably does.

## Lab laptop: GeForce GTX 1050 Ti Mobile

Second pass ([34](34-AI-GRAPHICS-AUDIT.md)), 2026-09-24. Core i7-7700HQ,
Intel HD 630 + **GeForce GTX 1050 Ti Mobile** (4 GB, NVIDIA 580.178.04),
1920×1080 laptop panel, Proton Experimental, KDE Wayland. The game rendered
on the GTX (observed: `/dev/nvidia0`, render card `card0`). Raw data:
`bigame-engine/benchmarks/2026-09-24-sottr-gtx1050ti-ai-graphics/`. Report:
`bench_native_report … native_xess --vary=StereoSeparation --record-graphics=…`.

### Method

Shadow of the Tomb Raider's built-in benchmark, DX12, 1920×1080, preset
**High**, **Intel XeSS Quality** selected in the game's menu in every run,
VSync off. The arms differ only in files:

| Arm | What the game ran |
|---|---|
| `native_xess` (first launch) and `native_xess_2` (third launch) | the game's own XeSS 1.1 — on a GTX through its DP4a path |
| `optiscaler_fsr` (second launch) | OptiScaler 0.9.4 as `dxgi.dll` running **FSR 3.1** in place of the game's XeSS, at the same render resolution: **1280×720 → 1920×1080** (OptiScaler's log) |

Order **A, B, A**: OptiScaler's files cannot change while the game runs, so
each arm had its own launch and its own discarded warm-up pass (the first
also compiled Vulkan shaders). Runs were started by the game's own `[R]`
key only while the game had focus, and counted only when its log said the
benchmark started and stopped. Settings were checked from the result files:
all identical except `StereoSeparation` (0.0225 in the first launch, 0.0220
after), a slider of stereoscopic 3D, which was **off** in every run — declared
with `--vary`. GPU telemetry from `nvidia-smi` each second (this laptop GPU
reports no power draw).

### Results

| Arm | avg fps | 1 % low | 0.1 % low | GPU busy | graphics clock | temp |
|---|---|---|---|---|---|---|
| `native_xess` | 15.9 · 15.7 · 15.5 | 9.2 · 11.0 · 10.3 | 5.6 · 8.5 · 6.1 | 99 % | 1684 MHz | 78 °C |
| `native_xess_2` | 16.6 · 16.8 | 11.9 · 10.6 | 8.8 · 5.4 | 99 % | 1678 MHz | 77 °C |
| `optiscaler_fsr` | **18.2 · 18.3 · 18.3** | 10.5 · 10.2 · 8.7 | 6.0 · 5.4 · 4.4 | 99 % | 1627 MHz | 77 °C |

| Comparison | Average | Verdict |
|---|---|---|
| second XeSS launch vs the first | +6.4 % (15.7 → 16.7) | measurably faster (t = 6.8) — **drift between launches**, with nothing changed |
| OptiScaler FSR 3.1 vs the first XeSS launch | +16.3 % (15.7 → 18.2) | measurably faster (t = 23.9) |
| OptiScaler FSR 3.1 vs both XeSS launches pooled | **+13.4 %** (16.1 → 18.3) | measurably faster (what the planner computes) |
| OptiScaler FSR 3.1 vs the last XeSS launch | about +9.4 % (16.7 → 18.3) | two runs against three: direction clear, size the smallest of the three |
| 1 % low and 0.1 % low | −3 % to −21 % against the first launch | **not enough evidence**: runs within an arm vary 8–35 % |

So on this laptop, FSR 3.1 through OptiScaler renders **9–16 % more frames
on average** than the game's own XeSS at the same render resolution — more
than on the RDNA 4 test machine (+5 %), as expected where XeSS runs on the
slower DP4a path — **while the GPU clock was lower** in the OptiScaler arm
(1627 vs 1678–1684 MHz), so the gain is not a clock effect. The run-to-run
spread of the frame-time floor is too wide at ~16 fps to say whether the 1 %
low is as good; its trend is not better.

### What the application does with it

The session is recorded in this machine's local measurements
(`--record-graphics`). The planner's verdict for this game on this GPU is
now: average frame rate **Improvement**, 1 % low **Inconclusive** — so it
keeps Recommended on **the game's own XeSS** and says why:

> measured on this computer: OptiScaler gave +13.4 % average frame rate over
> the game's own XeSS, but the 1 % low varied too much between runs to tell
> whether frame pacing is as good, so it is not chosen by itself — pick it
> under Choose yourself (runs, game's own / OptiScaler: 5 / 3)

That rule was written after this session showed it was needed: until then a
gain with an unmeasurable floor would have been promoted and described as
"1 % low no worse".

### Also found by this session

- With OptiScaler's default ini, the game **exited 4 s after start** on this
  GTX — OptiScaler enables its DLSS path on any NVIDIA card; fixed by
  `[DLSS] Enabled=false` where DLSS cannot run ([34](34-AI-GRAPHICS-AUDIT.md), defect 13).
- The game's own menu greys *NVIDIA RTX DLSS* out on the GTX — the same
  conclusion as the planner's capability check.
- OptiScaler's log proved **FSR 3.1** (`Fsr4Update: false` on NVIDIA); the
  status read *Active (fsr31), FSR 3.1* while the game ran.

Visual quality and latency were not measured in this session (no frame
generation was involved; the render resolution was the same in both arms).
