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
