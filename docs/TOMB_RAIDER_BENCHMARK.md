# Shadow of the Tomb Raider on the lab laptop

The game used as the laboratory for the hybrid-laptop audit. Every row below
is a real run; raw data (MangoHud frame logs, nvidia-smi samples, the game's
own result panel) is in `bigame-engine/benchmarks/2026-09-25-sottr-gtx1050ti-*`.

## The game and the method

| | |
|---|---|
| Game | Shadow of the Tomb Raider, Steam AppID 750920, `SOTTR.exe`, build 11668867 |
| Runtime | Proton Experimental 11.0, DX12 through VKD3D-Proton (DX11 through DXVK in one arm) |
| GPU | GeForce GTX 1050 Ti Mobile renders (checked: the game's pid holds the NVIDIA graphics context) |
| Settings | the game's lowest preset, 1920×1080, fullscreen (non-exclusive) |
| Workload | the built-in benchmark ("Testar performance"), three scenes, about 150 s |
| Rendered frames | the game's own result: frames drawn and their average over the whole benchmark |
| Presented frames | MangoHud's log, 110 s from the first scene; includes generated frames |
| Driving | a virtual keyboard (`/dev/uinput`) and screenshots: launch, menu, benchmark, result, quit — no human in the loop from run 3 of the settings session on |
| Order | A B A B within a session, one launch per run, 45 s rest between runs |

The game's average covers loading transitions the MangoHud window excludes,
so it reads 3–4 fps lower for the same run; compare like with like.

## Results

### Settings (the laptop's ceiling)

| Arm | Settings | Presented avg | 1 % low | Rendered (game) | Verdict |
|---|---|---|---|---|---|
| A | DX12, XeSS Performance (960×540 → 1080p) | 36.2 | 13.0 | — | reference |
| B | DX11 (DXVK) | 33.9 | 7.7 | — | slower: CPU 91–98 %, GPU 30–60 %, 464 stutters |
| C | DX12, XeSS off, resolution 60 % | — | — | 40 | faster than XeSS: XeSS costs more than its lower resolution saves on this GPU |
| D | C with async compute and high-precision RT off | 33.8 | 2.9 | 33 | slower |
| F | A + OptiScaler FSR 3.1 in place of XeSS | 40.6 | 13.7 | 36 | +12 % over A |

### Frame generation

| Arm | Presented | Rendered | Notes |
|---|---|---|---|
| E: A + OptiScaler FSR 3.1 + OptiFG | 60.7 | — | two NVIDIA Xid errors (69, 31) in this configuration; removed from the game |
| G: XeSS Quality + OptiScaler FSR 3.1 + OptiFG | 51.8 | — | 18 stutters, the best pacing of the frame-generation arms |
| lsfg-vk x2 (A B A B, on top of F) | 41.3 / 42.3 → **57.9 / 57.0** | 38 / 39 → **27 / 27** | see [LOSSLESS_SCALING_VALIDATION.md](LOSSLESS_SCALING_VALIDATION.md) |

### Turbo (A B A B, on top of F)

| Run | Turbo | Rendered | Presented | 1 % low | p99 |
|---|---|---|---|---|---|
| 01 | off | 40 | 43.0 | 14.3 | 51.8 ms |
| 02 | on | 39 | 42.8 | 14.7 | 50.4 ms |
| 03 | off | 36 | 39.2 | 12.5 | 58.8 ms |
| 04 | on | 37 | 41.4 | 15.5 | 49.0 ms |

Rendered 38.0 vs 38.0, presented 41.1 vs 42.1: **applied and verified (power
profile and governor performance during the game, restored after), no
measurable gain**. The game is GPU-bound with the GTX at its power limit.

## What is proven, what is not

**Proven on this machine**

- OptiScaler FSR 3.1 in place of the game's XeSS: +12 % (F vs A; the day
  before, +13.4 % at the High preset). The biggest gain any setting gave.
- lsfg-vk x2: +37 % presented frames at −30 % rendered frames, repeatable,
  with worse frame pacing.
- XeSS is not free on a GTX: rendering at 60 % without it beat XeSS
  Performance.

**No measurable difference**

- Turbo (power profile and governor performance).

**Slower**

- DX11 instead of DX12 (CPU-bound on the throttling i7-7700HQ).
- Async compute off.

**Unstable**

- OptiScaler's own frame generation on the GTX 1050 Ti: Xid 69 and Xid 31,
  one of them ending the game. Not left installed.

**Not measured**

- sched-ext schedulers: switching one needs administrator authentication
  (scx_loader's `auth_admin_keep`, or saving the falcond profile), which
  could not be given unattended. The earlier measurement on the reference
  desktop found none faster ([BENCHMARKS.md](BENCHMARKS.md)).
- Gamescope performance: the game ran inside Gamescope (720p → 1080p, FSR),
  rendering on the GTX with Gamescope compositing on the Intel GPU, but the
  automated benchmark could not navigate the scaled menu, so no valid run
  was recorded.
- Latency, for any configuration.

## Reaching 60

Nothing in the rendered path reaches 60 fps on this GPU at this resolution:
the ceiling at the lowest preset is 40–42 fps. 60 is reached only by
presenting generated frames — lsfg-vk (57–58, stable) or OptiScaler's
frame generation (60.7, unstable here) — at the cost of rendered frames,
pacing and latency. That is the user's trade-off; BiGame-mode's plan says
so and never turns it on by itself.
