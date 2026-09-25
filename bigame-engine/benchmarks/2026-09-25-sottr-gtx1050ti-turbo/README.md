# Shadow of the Tomb Raider — Turbo off versus on (2026-09-25)

Lab laptop (GTX 1050 Ti Mobile renders), lowest preset, 1920×1080, DX12,
XeSS Performance → OptiScaler FSR 3.1. Turbo switched with the same path as
the Home button (`examples/turbo`, D-Bus → helper → Polkit). Order A B A B,
one launch per run, 45 s rest.

What each arm had in force during the game (logged every 20 s):

- A, Turbo off: falcond stopped; power profile balanced, governor schedutil.
- B, Turbo on: falcond running; power profile performance, governor
  performance (set by power-profiles-daemon-biglinux-cpufreq from the
  profile), idle inhibited. In run 02 falcond kept its general `Proton`
  profile instead of switching to `SOTTR.exe` (it had just been started);
  both ask for performance.

| Run | Arm | Rendered (game) | MangoHud avg | 1 % low | p99 | GPU clock | CPU package |
|---|---|---|---|---|---|---|---|
| 01 | A off | 40 | 43.0 | 14.3 | 51.8 ms | 1608 MHz | 87 °C |
| 02 | B on | 39 | 42.8 | 14.7 | 50.4 ms | 1615 MHz | 89 °C |
| 03 | A off | 36 | 39.2 | 12.5 | 58.8 ms | 1593 MHz | 88 °C |
| 04 | B on | 37 | 41.4 | 15.5 | 49.0 ms | 1563 MHz | 88 °C |

Means: rendered 38.0 vs 38.0; MangoHud 41.1 vs 42.1 (+2.4 %, the off arm's
coefficient of variation is 6.5 %). No measurable difference: the game is
GPU-bound with the GTX at its power limit 30–38 % of the time, and CPU
frequency policy does not move that.
