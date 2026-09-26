# AMD desktop audit — Ryzen 7 5700G + Radeon RX 9060 XT

Every feature BiGame-mode offers, checked on the reference desktop: an APU
(Vega integrated graphics) plus a discrete RDNA 4 card, both on amdgpu. Each
row below was read back from the system or the game process; where a check
could not be made, the row says so. Branch `feature/gamer-theme-amd`.

## The machine, as the system reports it

| | |
|---|---|
| CPU | AMD Ryzen 7 5700G, 8 cores / 16 threads, `amd-pstate-epp` active, no 3D V-Cache |
| Discrete GPU | Radeon RX 9060 XT (Navi 44, `1002:7590`), `card1`, `renderD128`, PCI `0000:03:00.0`, 16 GB, boot display adapter, drives all three monitors |
| Integrated GPU | Radeon Vega (Cezanne, `1002:1638`), `card0`, `renderD129`, PCI `0000:0a:00.0`, 512 MiB, no output connected |
| Driver | amdgpu, kernel 7.2.7; Mesa 26.2.2: RADV for Vulkan (`RADV GFX1200` / `RADV RENOIR`), radeonsi for OpenGL |
| Default GPU | RX 9060 XT for Vulkan (listed first, discrete) and OpenGL; `DRI_PRIME=1` selects the Vega |
| Offload | none needed: the games' GPU drives the displays (BiGame-mode sets no `DRI_PRIME`) |
| Displays | DP-1 3440×1440 at 160 Hz, HDR on, VRR set to "Never" in KWin; HDMI-A-1 3440×1440 at 50 Hz; the kernel publishes no `vrr_capable` for these connectors |
| Resizable BAR | off: BAR 0 is 256 MiB of the card's 16 GB (firmware setting, reported, never changed) |
| Services | falcond 2.0.2, power-profiles-daemon 0.30 with BigLinux's cpufreq companion, scx-tools/scx-scheds 1.1.3 (16 schedulers), Gamescope 3.16.28, MangoHud 0.8.4, vkBasalt 0.3.2.10, lsfg-vk 1.0.0 |
| Kernel log | no ring timeout or GPU reset this boot; 1728 × "Unsupported screen format RA24" from the compositor, one display-core warning at probe and an HDMI infoframe error — driver and compositor messages, not BiGame-mode's |

## Feature matrix

| Feature | Detected | Active | Verified how | Benefit measured | Restored | Result |
|---|---|---|---|---|---|---|
| falcond | 2.0.2, 9 profiles | profile applied to `SOTTR.exe` (`Proton`, then its own) | journal, status file, power profile and idle inhibitor during the game | power profile: no difference here (2026-09-23) | yes — to the profile in use **when falcond started** (below) | works |
| Turbo | falcond unit | off → stopped and disabled; on → started and verified | systemd state through the helper, no password (`allow_active`) | — | yes | works |
| sched-ext / scx_loader | 16 schedulers | `lavd` in Gaming mode from the game's profile | `/sys/kernel/sched_ext`: `lavd_1.1.3…` during, disabled after | no difference (2026-09-24, CPU-bound) | yes | works, no gain |
| power-profiles-daemon | 3 profiles | falcond switches to performance per game | `powerprofilesctl` before / during / after | no difference | see falcond | works |
| Gamescope | 3.16.28, 97 options | a program started through the launch plan | process tree, Gamescope's device (RX 9060 XT), the program's fdinfo | not measured (Steam games are not wrapped) | ends with the game | **fixed**: Wayland games were ended at start |
| MangoHud | 0.8.4 | Shadow of the Tomb Raider through Steam launch options | `MANGOHUD=1` in the game's environment, `libMangoHud.so` mapped | a tool | Off removes `MANGOHUD=1` | **fixed**: games without a Steam block |
| vkBasalt | 0.3.2.10 | in the game (session environment) | `libvkbasalt.so` mapped | a look, not a speed-up | environment file | **fixed** under Gamescope |
| OptiScaler (AI Graphics) | 0.9.4, verified cache | installed as `dxgi.dll` | mapped in the game, `OptiScaler.log` of this run, status "Loaded 0.9.4" | +10.1 % (2026-09-24) | restore refused while the game ran; after it, the folder matched the snapshot (81 files, SHA-256) | works |
| lsfg-vk | 1.0.0 + the user's `Lossless.dll` 3.2.2.0 | x2 and x3 from the game's start | rendered frames fell by what generation costs; layer mapped | presented frames not measurable here | removal only at the next start | works; **UI fixed** |
| FSR | Gamescope FSR; OptiScaler | Gamescope FSR on a test program; OptiScaler's FidelityFX libraries mapped today, and its 2026-09-24 log shows the FSR 4 upgrade path (`RDNA4: true`, `Fsr4Update: true`, `amdxcffx64` loaded) | logs, mapped libraries | OptiScaler +10.1 % | — | FSR 4 not claimed as running without OptiScaler's overlay |
| XeSS | the game ships 1.1.0.21 | — | scan | +4.9 % over TAA (2026-09-24) | — | detected, not assumed away on AMD |
| DLSS | the game ships 2.3.2.0 | never offered | GPU is not RTX | — | — | correct |
| VKD3D-Proton | `d3d12.dll`, `d3d12core.dll` mapped | yes | maps → "VKD3D-Proton" on Home | — | — | detected correctly |
| DXVK | DXVK's `d3d11.dll`/`dxgi.dll` mapped under the DX12 game | — | maps; no DX11 title run | — | — | detection only |
| Proton | Proton Experimental, prefix in the library that holds the manifest | — | Home ("Proton - Experimental") | — | — | prefix never touched |
| Diagnostics | — | — | health report on this machine | — | — | adds Resizable BAR |
| Helper security | — | — | `tests/daemon-authorization.sh`: every method refused with Polkit unreachable, traversal refused | — | — | passed |

## Benchmark matrix

Shadow of the Tomb Raider's built-in benchmark, 3440×1440 High, TAA, DX12,
alternating arms, warm-up discarded. Data in `bigame-engine/benchmarks/`.

| Test | Average fps | 1 % low | Notes |
|---|---|---|---|
| Baseline (Turbo on, vkBasalt loaded) | 88.9 ± 0.7 | 68.8 | six identical runs, spread 0.8 % — the noise floor |
| Turbo / power profile | — | — | no difference, 2026-09-23 sessions |
| sched-ext lavd / bpfland | — | — | no difference, 2026-09-24 |
| GPU DPM `high` | −8.0 % | −7.6 % | slower, 2026-09-23; the Booster never proposes it |
| OptiScaler FSR from XeSS Q | +10.1 % | unchanged | 2026-09-24 |
| lsfg-vk x2 | 51.8 rendered | 44.7 | −42 % rendered; presented not counted |
| lsfg-vk x3 | 39.6 rendered | 35.4 | −55 % rendered |
| Gamescope | not conclusive | — | Steam games are not wrapped by BiGame-mode; functional check only |

## Problems found and fixed

| Problem | Cause | Fix | Test |
|---|---|---|---|
| Home and Details said Shadow of the Tomb Raider ran "on Radeon Vega" | enumerating Vulkan devices opens every render node; the rule "the non-boot card among several" picked the Vega | the card the game submitted work to, from DRM fdinfo; no answer before any work | same game, old build "na Radeon Vega…", new build "na Radeon RX 9060 XT"; unit tests from the real fdinfo |
| GPU names "Navi 44 [Radeon RX 9060 XT]", "Cezanne [Radeon Vega Series / Radeon Vega Mobile Series]", "GPU AMD" | the PCI database name shown raw | product names for display; measurements stay keyed on the database name | unit tests; Home and Details on the machine |
| MangoHud "on" for the game, never in it | Steam keeps no `localconfig.vdf` block for a game played with defaults; the writer refused, the choice was saved anyway | the block is created; the choice is saved after the options read back | environment and mapped library of the game |
| A Wayland-preferring game ended at start inside Gamescope | the game inherited the desktop's `WAYLAND_DISPLAY`; Gamescope's WSI layer failed ("Failed to get Wayland objects") | `--expose-wayland` where supported | vkcube through the launch plan: ended before, runs after, on the RX 9060 XT |
| vkBasalt with Gamescope filtered Gamescope, not the game | Gamescope loads the layer itself and removes `ENABLE_VKBASALT` from its child | off for Gamescope, on for the game | libraries mapped in both processes |
| Frame generation turned on or off mid-game did nothing | lsfg-vk applies only new values for a game that started with an entry | the UI says it takes effect at the next start; Home warns when the file changed after the game started | three benchmark sessions |
| The whole lsfg-vk file ignored | the `[[profile]]` layout an older BiGame-mode wrote | converted when the application starts, copy kept | installed package on the machine |
| Details keyed on "Proton", counted any Gamescope | falcond's active profile used as the game's name | the watched game's process; Gamescope only in its tree | code path; Home on the machine |
| "falcond restores the power profile when the game exits" | falcond 2.0.2 restores the profile it saw at service start | report and docs say so | dummy process: started balanced, switched to power-saver, balanced came back |
| V-Cache controls on a CPU without V-Cache | shown disabled (Tuning) or live (profile editor) | hidden | this CPU |
| "Configurações" hyphenated in a narrow window | 180 px sidebar | 200 px minimum | screenshot at 888 px |

## Not validated here

- **Presented frames with lsfg-vk**: MangoHud counts only the game's frames
  in this layer order, and forcing the other order hung the game.
- **Gamescope performance** with a real game: BiGame-mode does not wrap Steam
  games; only functional checks with a test program.
- **VRR and HDR in games**: the kernel does not report VRR capability for
  these connectors and KWin has VRR off; the game was run in SDR.
- **Latency**, for any configuration: no instrument.
- **HiDPI** rendering of the Gamer design: the test compositor offered only
  scale 1; the stylesheet uses logical units and vector icons.
- **Resizable BAR's effect**: reported, not changed or measured.

## Machine state

Everything changed for the tests was put back: power profile (performance,
with falcond restarted so it keeps that as its baseline), both Steam
accounts' launch options, the MangoHud configuration, the game's registry
and folder. One change stays by design: the lsfg-vk file is now in the 1.x
layout. The old file held a `SOTTR.exe` entry at x3 (flow 0.6, HDR mode on)
that lsfg-vk never applied; it is active now and can be switched off in
Profiles. The old file is kept as `conf.toml.bigame-legacy`.
