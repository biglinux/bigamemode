# lsfg-vk from the game's start — reference desktop

Same game, settings and machine as `2026-09-25-sottr-rx9060xt-lsfg`
(Shadow of the Tomb Raider, 3440×1440 High, TAA, DX12, RX 9060 XT on the
160 Hz DP-1 monitor, Turbo on, vkBasalt loaded). This launch started with a
`SOTTR.exe` entry at multiplier 2 already in `~/.config/lsfg-vk/conf.toml`,
written by BiGame-mode, with the user's own `Lossless.dll` (3.2.2.0, used in
place). The arms then changed the entry while the game ran. Order: warm-up
(x2), then x2 · off · x3 | off · x3 · x2.

| Run | File says | Rendered (game) | 1 % low | p99 frame time |
|---|---|---|---|---|
| 1 | x2 | 51.8 | 44.8 | 22.3 ms |
| 2 | off (entry removed) | 51.8 | 44.7 | 22.4 ms |
| 3 | x3 | 39.6 | 35.3 | 28.3 ms |
| 4 | off (entry removed) | 51.8 | 44.8 | 22.3 ms |
| 5 | x3 | 39.7 | 35.4 | 28.2 ms |
| 6 | x2 | 51.8 | 44.5 | 22.5 ms |

Without frame generation the same game renders 88.9 ± 0.7 fps (the A/A
session). What this shows:

- **Generation costs rendered frames:** x2 renders 51.8 fps (−42 %), x3
  39.6 fps (−55 %). Each generated frame takes about 7–8 ms of the GPU at
  3440×1440 with flow scale 1.0.
- **Presented frames were not measured.** Multiplied out, x2 would show
  about 104 fps and x3 about 119 on this 160 Hz monitor, but no counter
  confirmed it: MangoHud sits above lsfg-vk in this system's layer order
  and counts only the game's frames (0.99× here, against the lab laptop,
  where a per-user lsfg-vk manifest put it the other way round), and forcing
  the order with `VK_INSTANCE_LAYERS` hung the game at start (black
  screen, 0 % GPU).
- **What lsfg-vk applies while the game runs:** a new multiplier (x2 → x3
  and back took effect at once), not removal — with the entry gone the game
  kept paying x2's cost until it closed. Turning it on for a game already
  running does nothing (the A/A session). BiGame-mode now says so: on and
  off take effect at the game's next start, and Home warns when the file
  changed after the game started.

Verdict: working, and the trade is steep on this card at this resolution:
it gives up 42 % of the rendered frames, so the game responds at ~52 fps
instead of 88.9, for at most ~104 frames on screen instead of 88.9 (+17 %,
not confirmed by a counter). BiGame-mode never turns it on by itself.
