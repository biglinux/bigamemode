# Shadow of the Tomb Raider — lsfg-vk frame generation (2026-09-25)

Lab laptop (GTX 1050 Ti Mobile renders), lowest preset, 1920×1080, DX12,
XeSS Performance → OptiScaler FSR 3.1 (no OptiScaler frame generation),
Turbo on. lsfg-vk 1.0.0 (community-extra) with the user's own Lossless.dll
3.2.2.0, entry written by BiGame-mode (`[[game]] exe = "SOTTR.exe"`,
multiplier 2, flow_scale 1.0). Order A B A B, one launch per run, 45 s rest.

Two counters, never mixed: the game's own result (`game-result.png`,
rendered frames over the whole benchmark) and MangoHud (`SOTTR_*.csv`,
presented frames over 110 s, generated ones included).

| Run | Arm | Rendered (game) | Presented (MangoHud) | 1 % low | frames > 2× median |
|---|---|---|---|---|---|
| 01 | A no frame generation | 38 | 41.3 | 15.4 | 70 |
| 02 | B lsfg-vk x2 | 27 | 57.9 | 16.1 | 1329 |
| 03 | A no frame generation | 39 | 42.3 | 12.7 | 132 |
| 04 | B lsfg-vk x2 | 27 | 57.0 | 14.2 | 908 |
