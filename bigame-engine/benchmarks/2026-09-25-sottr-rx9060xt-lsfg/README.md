# lsfg-vk turned on while the game runs — reference desktop

Shadow of the Tomb Raider, 3440×1440, High preset, TAA, DX12 (VKD3D-Proton),
Proton Experimental, Radeon RX 9060 XT, Turbo on, vkBasalt (CAS) loaded from
the session environment, MangoHud through the game's launch options.

One launch. The game started while `~/.config/lsfg-vk/conf.toml` still had
the `[[profile]]` layout an earlier BiGame-mode wrote, which lsfg-vk 1.0.0
ignores entirely. The `fg_x2` arm then wrote a valid `SOTTR.exe` entry with
multiplier 2 through BiGame-mode's own code (the file was valid from the
first `fg_x2` run on), and `fg_off` removed it. Order: warm-up, then
off · x2 | x2 · off | off · x2.

**lsfg-vk never generated a frame.** MangoHud (its summaries are kept
beside each run) counted the same frames as the game, 0.99× in every run,
with the entry in place for over ten minutes: lsfg-vk re-reads its file only
for a game that started with an entry ("Reloaded configuration for …"). The
two arms are therefore the same configuration, and the session is an A/A
measurement of the noise. From the game's own frame log:

| Run | Average (game) | 1 % low | p99 frame time | Frames over 2× median |
|---|---|---|---|---|
| off 1 | 88.5 | 68.8 | 14.5 ms | 2 |
| x2 1 | 87.7 | 67.8 | 14.8 ms | 6 |
| x2 2 | 89.3 | 68.6 | 14.6 ms | 2 |
| off 2 | 89.3 | 69.4 | 14.4 ms | 3 |
| off 3 | 89.4 | 69.3 | 14.4 ms | 3 |
| x2 3 | 89.3 | 68.7 | 14.6 ms | 2 |

Rendered: 88.9 ± 0.7 fps over six runs (spread 0.8 %); the first measured
run of each arm is its lowest.

What changed in BiGame-mode because of it: the legacy file is converted when
the application starts, and the Tuning page says that frame generation
turned on for a game already running starts with its next launch. The
measurement with the entry present at launch is
`2026-09-25-sottr-rx9060xt-lsfg-launch`.

MangoHud's per-frame logs are not kept: in this system's layer order (see
the `-launch` session) they only repeat the game's own count.
