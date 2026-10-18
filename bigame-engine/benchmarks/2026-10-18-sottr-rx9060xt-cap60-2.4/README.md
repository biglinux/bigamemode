# Shadow of the Tomb Raider — the old presets' 60 FPS cap versus none, Big Game Mode 2.4.0 (2026-10-18)

Reference desktop: Ryzen 7 5700G, Radeon RX 9060 XT, KDE Plasma Wayland,
3440×1440 160 Hz main screen (adaptive sync on). The game's built-in
benchmark, DX12 (VKD3D-Proton, Proton Experimental), windowed, XeSS Quality
taken over by OptiScaler (FSR), VSync off, vkBasalt CAS and MangoHud loaded in
both arms, and the 2.4.0 preset Pretty in the session (no cap). Big Game
Mode's nested Gamescope wrapper was left out of the launch options in both
arms: the benchmark's menu keys did not reach the game inside it.

Two launches, one per arm, each with a discarded warm-up pass and 2 measured
passes. `cap60` puts `VKD3D_FRAME_RATE=60 DXVK_CONFIG=dxvk.maxFrameRate=60` in
the game's launch options, which is what *Locked 60 FPS* and *Enhanced
graphics* put in the whole session before 2.4.0; `pretty` has no cap.

## Results (bench_native_report, the game's own frame times)

| Arm | Avg FPS | 1 % low | 0.1 % low | p99 |
|---|---:|---:|---:|---:|
| `cap60` | 59.8 · 59.7 | 34.0 · 32.4 | 20.4 · 16.3 | 24.0 · 23.8 ms |
| `pretty` | 79.2 · 79.7 | 35.7 · 34.5 | 18.6 · 14.5 | 22.3 · 22.1 ms |

- Average: **+33.0 %** (Welch's t 70.2 against 12.71, above the 0.5 % spread):
  measurably faster.
- 1 % low: +5.8 %, no difference above normal variation; 0.1 % low: the runs
  disagree too much to compare.

On a 160 Hz screen the old presets held every Proton game at 60 FPS, a
third below what the card gave here; without adaptive sync, 60 does not
divide 160 and frames alternate between two and three refreshes.
