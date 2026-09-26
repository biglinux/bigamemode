# Lossless Scaling frame generation (lsfg-vk) — validation

How BiGame-mode drives lsfg-vk, what it got wrong, and what frame generation
actually does to Shadow of the Tomb Raider on the lab laptop (GTX 1050 Ti
Mobile, see [HYBRID_NVIDIA_INTEL_AUDIT.md](HYBRID_NVIDIA_INTEL_AUDIT.md)).

`Lossless.dll` is the user's own file from their copy of Lossless Scaling. It
is read where the user keeps it and never copied, committed or downloaded;
the repository ignores `*.dll`.

## What was installed

| | |
|---|---|
| lsfg-vk | 1.0.0-1 from BigCommunity's community-extra (the package BigLinux users get), checksum-verified against the repository database |
| Installed as | for this user only, for the test: the manifest in `~/.local/share/vulkan/implicit_layer.d/` pointing at the package's `liblsfg-vk.so` (a system install with `pacman -S lsfg-vk` is equivalent) |
| Layer | `VK_LAYER_LS_frame_generation`, implicit, off switch `DISABLE_LSFG=1`, 64-bit library only (no 32-bit games) |
| `Lossless.dll` | version 3.2.2.0, PE32+ x86-64 |

## The format lsfg-vk 1.0.0 reads

Read from the strings of its library, including the default file it embeds:

```toml
version = 1
[global]
dll = "/path/to/Lossless.dll"
[[game]]
exe = "Game.exe"
multiplier = 3            # "Global Multiplier cannot be less than 2"
flow_scale = 0.7          # "Flow scale must be between 0.25 and 1.0"
performance_mode = true
hdr_mode = false
experimental_present_mode = "fifo"   # mailbox, immediate
```

One invalid value makes lsfg-vk ignore the whole file ("An error occured
while trying to parse the configuration, IGNORING"). It also honours
`LSFG_MULTIPLIER`, `LSFG_FLOW_SCALE`, `LSFG_PROCESS`, `LSFG_DLL_PATH` and
others from the environment, and reloads the file while a game runs.

## What BiGame-mode did before

| Requested in BiGame-mode | Written | What lsfg-vk 1.0.0 did |
|---|---|---|
| Frame generation x2/x3 for a game | `[[profile]]` with `name` / `active_in` / `gpu` / `pacing` / numeric `present_mode` — the layout of another lsfg-vk line | nothing: its library has no `active_in`; per-game settings never took effect |
| Frame generation off for a game | `multiplier = 1` | rejected the entire file, every game's entries with it |
| The Video page's switch off | every multiplier forced to 1, permanently | same as above; turning it back on restored nothing |
| x3 saved from a profile with no lsfg-vk installed | a warning in the log only ("failed to sync lsfg-vk profile; profile save will continue") | — the UI kept showing x3 |
| "Active (Generating Frames)" on Details | shown when `liblsfg-vk.so` was mapped in the game | the implicit layer is mapped into every Vulkan process |

## What it does now

- It writes the 1.0 format. Off is **no entry**. The old layout is converted.
- It keeps what it did not write: other keys and entries stay, and only the
  entries it wrote — recorded in its own state directory — are ever touched.
  The file is replaced atomically, so lsfg-vk's live reload never reads half
  of it.
- The Video switch is reversible: off sets BiGame-mode's entries aside, on
  puts them back.
- An installed lsfg-vk with another format is detected, and BiGame-mode
  refuses to write rather than produce a file that is ignored.
- Details says "On for this game" only when the layer is loaded in the game
  **and** the game has an entry.

## Checked on the machine

With the local `Lossless.dll` and an entry written by BiGame-mode:

```
lsfg-vk: Loaded configuration for vkcube:
  Using DLL from: /home/…/Lossless Scaling/Lossless.dll
lsfg-vk: Shaders extracted successfully.
lsfg-vk: Vulkan instance layer initialized successfully.
lsfg-vk: Vulkan device layer initialized successfully.
lsfg-vk: Swapchain context created (using 4 images).
```

In Shadow of the Tomb Raider under Proton, the layer is mapped into
`SOTTR.exe` inside the Steam container, next to MangoHud.

The GPU used by lsfg-vk is the game's: the layer runs inside the game's
Vulkan device (the GTX here). Nothing in lsfg-vk 1.0.0's configuration picks
another GPU (`LSFG_DEVICE_UUID` exists in its environment variables, unused
here). A dual-GPU split — render on one GPU, generate on the other — was not
attempted: nothing measured suggests it helps on this machine, and it would
be experimental.

## Rendered versus presented frames

Two independent counters, never mixed:

- **Rendered** — the game's own benchmark result ("Média de FPS", frames the
  game drew over the whole benchmark). Generated frames are invisible to it.
- **Presented** — MangoHud's log over a 110 s window. It counts the frames
  sent to the display, generated ones included.

Shadow of the Tomb Raider, lowest preset, 1920×1080, DX12, XeSS Performance →
OptiScaler FSR 3.1 (no OptiScaler frame generation), Turbo on. Order A B A B,
one launch per run, 45 s rest between runs.

| Run | Arm | Rendered (game) | Presented (MangoHud) | 1 % low presented | p99 frametime | frames > 2× median |
|---|---|---|---|---|---|---|
| 1 | A — no frame generation | 38 fps | 41.3 fps | 15.4 | 50.2 ms | 70 |
| 2 | B — lsfg-vk x2 | 27 fps | 57.9 fps | 16.1 | 52.2 ms | 1329 |
| 3 | A — no frame generation | 39 fps | 42.3 fps | 12.7 | 57.2 ms | 132 |
| 4 | B — lsfg-vk x2 | 27 fps | 57.0 fps | 14.2 | 56.3 ms | 908 |

- **Rendered frames: −30 %** (38.5 → 27): generating frames costs the GTX
  GPU time the game no longer gets.
- **Presented frames: +37 %** (41.8 → 57.5), repeatable within ±1 fps.
- **Frame pacing is worse**: an order of magnitude more frames take over
  twice the median, and the menu showed 0.9 ms frames between generated
  ones. The 1 % low of presented frames did not change beyond run-to-run
  spread.
- **Latency** was not measured (no instrument). The game responds at its
  rendered rate, 27 fps, plus the frame generation's own delay.

Verdict: **applied and working; more frames on screen, fewer rendered, and
uneven pacing**. It is the user's trade-off to make, and BiGame-mode never
turns it on by itself.

## On the reference desktop (Radeon RX 9060 XT)

Shadow of the Tomb Raider at 3440×1440 High on the 160 Hz monitor, lsfg-vk
1.0.0 from the system package, the same `Lossless.dll` (3.2.2.0, used in
place). Raw data: `bigame-engine/benchmarks/2026-09-25-sottr-rx9060xt-lsfg*`.

**What lsfg-vk applies while a game runs**, from three sessions:

| Change while the game runs | Effect |
|---|---|
| entry added (the game started while the file was in the legacy layout) | none: presented = rendered for ten minutes |
| multiplier x2 → x3 → x2 (the game started with an entry) | applied at once |
| entry removed | none: the game kept paying x2's cost until it closed |

lsfg-vk logs "Reloaded configuration for …" only for a game it loaded a
configuration for at start. BiGame-mode therefore says that turning frame
generation on or off takes effect at the game's next start, Home warns when
the file changed after the game started, and the legacy layout — which made
lsfg-vk 1.0 ignore the whole file on this machine, including a `SOTTR.exe`
entry at x3 the user had chosen long ago — is converted when the
application starts.

**Cost:** rendered frames 88.9 fps without generation, 51.8 with x2 (−42 %),
39.6 with x3 (−55 %), about 7–8 ms of GPU time per generated frame at this
resolution with flow scale 1.0. Presented frames could not be counted:
MangoHud sits above lsfg-vk in this system's layer order, and forcing the
other order hung the game at start. Multiplied out, x2 would show about 104
fps on this 160 Hz screen, x3 about 119, while the game responds at 52 and
40 fps.

## Pending

- x3 and higher multipliers, performance mode and flow scale below 1.0 were
  not measured.
- A system-wide `pacman -S lsfg-vk` install behaves the same way as the
  per-user one used here (same library and manifest) but was not the one
  exercised.
