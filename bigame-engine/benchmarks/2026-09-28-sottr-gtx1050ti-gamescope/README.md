# Shadow of the Tomb Raider in Gamescope on the hybrid laptop (2026-09-28)

Lab laptop (HD 630 drives the panel, GTX 1050 Ti renders, NVIDIA 580.178.04),
KDE Plasma Wayland, Gamescope 3.16.28, DX12, OptiScaler FSR 3.1 from XeSS
Quality, preset Low, Turbo off. Gamescope reached the game the way it does
for a Steam game: the wrapper BiGame-mode writes into its launch options for
a profile with Gamescope "Always" (the `steam_gamescope` example), Steam
closed while writing. Not a comparison of arms: what happened.

| Run | Launch options | Outcome |
|---|---|---|
| `plain/run-01` | `MANGOHUD=1 %command%` | 28.3 FPS, 1920×1080 |
| two launches | `MANGOHUD=1 gamescope -f --expose-wayland -- %command%` | Gamescope ended (SIGABRT) about 8 s after the launcher handed over to the game, both times: "xdg_backend: Failed to dispatch input thread queue: protocol error 3 on xdg_surface@58", KWin "error in client communication". No Xid, no GPU error. |
| `gamescope_sdl/run-01` | the same with `--backend sdl`, written by hand | ran; 39.1 FPS, but at **1280×720**: the game saw a 1280×720 display inside Gamescope (OptiScaler rendered 853×480), stretched to the panel. Not comparable with the plain run. |

What follows from it:

- Nested Gamescope's Wayland backend (its default) did not survive this
  game's hand-over from its launcher to its window under KWin; the SDL
  backend did, once. Two failures and one success are too few to change
  which backend BiGame-mode asks for.
- With no output size, nested Gamescope offers the game 1280×720. BiGame-mode
  now passes the main screen's size (`-W 1920 -H 1080` here) when nothing
  else sets one.
- The game rendered on the GTX 1050 Ti inside Gamescope (`nvidia-smi pmon`:
  SOTTR.exe on the NVIDIA GPU, 99 %); Gamescope composited on the HD 630.

The game's logs are not in the repository.
