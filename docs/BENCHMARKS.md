# Benchmarks

BiGame-mode claims a setting helps only when a measurement says so. This page
describes how measurements are made and what they found on the reference
machine. The raw data of every result is in `bigame-engine/benchmarks/`.

## Method

- **Alternate the arms** (A B A B …), or rotate their order between rounds
  (ABC, BCA, CAB). Never group them: session drift was about 10 % in 20
  minutes, which grouped runs would attribute to the setting.
- **Discard the warm-up:** the first run of each arm, or the first pass of a
  launch.
- **At least three runs per arm**; fewer than two measured runs is
  inconclusive.
- **Spread** is the coefficient of variation of an arm. An arm above 5 % is
  inconclusive — something interfered.
- **A difference is real only if** it exceeds the larger spread of the two arms
  **and** passes Welch's t-test at 95 %. Otherwise the verdict is *no change*
  and the percentage is withheld (the raw number stays in the JSON).
- **A capped workload measures nothing.** SuperTuxKart's defaults (vsync on,
  `max_fps = 120`) hold it at the cap whatever changes; switch
  `swap-interval-vsync` off and raise `max_fps` in its configuration before
  a session.
- **Frame times, not averages alone.** Games' own per-frame logs are used where
  they exist (Crystal Dynamics `*_frametimes_*.txt`, Cyberpunk 2077
  `frames.csv`), otherwise MangoHud's. Frames of one second or more are scene
  transitions and are set aside. Settings must be identical within a session,
  except the keys under test. Runs with frame generation are not throughput
  results.
- **Compare like with like.** A game's own benchmark average covers loading
  transitions a MangoHud window excludes; in Shadow of the Tomb Raider it
  reads 3–4 fps lower for the same run.
- **The product is what is measured:** machine state is changed through
  BiGame-mode's own helper, captured before the session and restored after it,
  including on interrupt.
- glxgears and vkcube only confirm a driver works; they are never evidence.

### MangoHud capture

- Settings reach MangoHud through `MANGOHUD_CONFIGFILE`; `MANGOHUD_CONFIG`
  produced no log with 0.8.4.
- `no_display` must not be set: it suppresses the CSV too.
- OpenGL games need the `mangohud` wrapper, whose `LD_PRELOAD` attaches the
  overlay; `MANGOHUD=1` alone only enables the Vulkan layer.

## Tools

| Tool | Use |
|---|---|
| `scripts/bench-game.sh` | a game's built-in benchmark, alternating arms in one launch (Shadow of the Tomb Raider's `[R]` rerun), with GPU telemetry. `GAME`, `RUNS`, `LABEL`, `SCX_PROFILE` |
| `scripts/bench-lab.sh` | SuperTuxKart A/B sessions. `RUNS`, `LABEL` |
| `scripts/gpu-telemetry.sh` | GPU clock, power, temperature and utilisation 4×/s, without forking (a forking sampler raised the run-to-run spread from about 1.4 % to about 9 %) |
| `scripts/scx-switch.sh` | the root side of scheduler sessions: one Polkit approval, the profile restored when the session ends |
| `cargo run -p bigame-core --example bench_native_report -- <session> [baseline] [--vary=KEY,…]` | verdicts for a `bench-game.sh` session |
| `cargo run -p bigame-core --example bench_report -- <session> <baseline>` | verdicts for a `bench-lab.sh` session |
| *Measure the difference* (a game card's menu) | the same A/B method for any game that starts directly, driven by the application |

Each session directory holds `system.json` (the machine, with no host name,
user, home or address), the runs of every arm, and the report.

## Reference machine

AMD Ryzen 7 5700G (8C/16T, `amd-pstate-epp`, no 3D V-Cache), Radeon RX 9060 XT
(RDNA 4, 16 GB) plus the idle integrated GPU, Mesa 26.2.2, kernel 7.2.6
(7.2.7 from 2026-09-25), KDE Plasma Wayland, Proton Experimental, 3440×1440
(the 160 Hz DP-1 monitor). The machines and tool versions are in
[ARCHITECTURE.md](ARCHITECTURE.md#hardware-notes).

A second machine, the **lab laptop**, measured AI Graphics on NVIDIA: Intel
Core i7-7700HQ, Intel HD 630 plus a GeForce GTX 1050 Ti Mobile (4 GB, NVIDIA
580.178.04) — a hybrid laptop, the game rendering on the GTX — 1920×1080,
Proton Experimental, KDE Plasma Wayland.

Results are evidence for the defaults BiGame-mode ships, not claims about other
hardware. They reach the application as fixed rules (the Booster never forces
GPU DPM) and as entries in the AI Graphics game list; nothing is measured on a
user's machine.

## Results

| Setting | Workload | Result | Verdict | Data |
|---|---|---|---|---|
| GPU DPM `high` vs `auto` | Shadow of the Tomb Raider (SotTR), 3440×1440 High, GPU-bound | 89.4 → 82.2 fps, **−8.0 %** (spread 0.1 %); 1 % low −7.6 %; 3246 → 2640 MHz, 163 → 103 W | slower | `2026-09-23-sottr-power-clean` |
| GPU DPM `high`; balanced vs performance power profile | same, first session | −8.3 %; power profile −0.1 % | slower; no difference | `2026-09-23-sottr-power` |
| GPU DPM `high` | SuperTuxKart, 3440×1440 max, uncapped | 298.4 → 275.9 fps, −7.5 % | slower | `2026-09-23-supertuxkart-gpu-bound` |
| Power profile, governor, EPP → performance | SotTR, CPU-bound (minimum render scale) | all within 1.5 %, no consistent order | no difference | `2026-09-23-sottr-cpu-bound` |
| sched-ext `lavd`, `bpfland` vs none, through falcond | SotTR, CPU-bound | +1.1 %, +1.8 %, inside the spread | no difference | `2026-09-24-sottr-scheduler` |
| AI Graphics: native TAA → the game's XeSS Quality → OptiScaler FSR from XeSS Quality | SotTR, 3440×1440 High | 89.8 → 94.2 fps (+4.9 %) → **98.8 fps (+10.1 %)**, spread 0.2 %; 1 % low unchanged; GPU 164 → 156 W | faster | `2026-09-24-sottr-ai-graphics` |
| A/A: the same configuration six times (lsfg-vk entry written mid-game, never active) | SotTR, 3440×1440 High, RX 9060 XT, vkBasalt loaded | 88.9 ± 0.7 fps (87.7–89.4), spread 0.8 % | the noise floor for this game on this machine | `2026-09-25-sottr-rx9060xt-lsfg` |
| lsfg-vk x2 and x3 (the user's `Lossless.dll`), entry present when the game started, multiplier changed while it ran | same | rendered 88.9 → **51.8 fps (x2, −42 %)** → **39.6 fps (x3, −55 %)**, 1 % low 68.8 → 44.7 → 35.4 (means of the runs); about 7–8 ms of GPU per generated frame; presented frames not measured (below) | costs rendered frames; not a throughput result | `2026-09-25-sottr-rx9060xt-lsfg-launch` |
| AI Graphics on the lab laptop: the game's XeSS Quality → OptiScaler FSR 3.1 from XeSS Quality (1280×720 → 1080p), order A B A, one launch per arm | SotTR, 1920×1080 High, GTX 1050 Ti | 15.9 · 15.7 · 15.5 → **18.2 · 18.3 · 18.3** → 16.6 · 16.8 fps: **+13.4 %** against both XeSS launches pooled, +9 % to +16 % against either (the XeSS arm itself drifted +6.4 % between launches); graphics clock lower with OptiScaler (1627 vs 1678–1684 MHz); 1 % and 0.1 % lows vary 8–35 % run to run | faster on average; lows inconclusive | `2026-09-24-sottr-gtx1050ti-ai-graphics` |
| Lab laptop, the game at its lowest preset, 1920×1080, DX12, XeSS Performance (960×540 → 1080p), one 110 s benchmark window per arm | SotTR, GTX 1050 Ti | **36.2 fps**, 1 % low 13.0; GPU 99–100 % busy, at its power limit 37 % of the time (1442–1721 MHz), CPU package 83–91 °C | the reference for the rows below | `2026-09-25-sottr-gtx1050ti-settings`, arm A |
| the same in DX11 (DXVK) | same | 33.9 fps, 1 % low 7.7, 464 stutters: CPU 91–98 %, GPU 30–60 % — the DXVK path is CPU-bound on this throttling i7-7700HQ; scene 2 ran at 19–21 fps | slower, and worse paced | arm B |
| DX12, XeSS off, the game's resolution modifier at 60 % (1152×648, TAA) | same | 40 fps (the game's own average; GPU-limited 97 %) — more pixels than XeSS Performance, yet faster: XeSS's own cost on this GPU exceeds what its lower render resolution saves | faster than XeSS | arm C |
| as C with async compute and high-precision render targets off (registry) | same | 33 fps, the game's CPU thread 47 fps average against 65: async compute is worth keeping on Pascal under VKD3D-Proton | slower | arm D |
| DX12, XeSS Performance → OptiScaler 0.9.4 FSR 3.1 with OptiFG frame generation, installed by AI Graphics (Choose yourself, experimental) | same | **60.7 fps presented** (MangoHud, 6652 frames in 110 s), 1 % low 18.7; the game's menu went from 41 to 71 fps | presented frames, not rendered ones, and more latency; **unstable on this GTX**: NVIDIA Xid 69 in this arm and Xid 31 in the next play session, which ended the game — removed from the game | arm E |
| as E without frame generation (OptiScaler FSR 3.1 from XeSS Performance) | same | 40.6 fps rendered, 1 % low 13.7: **+12 %** over the game's XeSS (arm A), the same gain as the day before at High | faster | arm F |
| as E with the game's XeSS at Quality (1280×720 → 1080p) | same | 51.8 fps presented, 1 % low 24.1, p99 30.7 ms, 18 stutters against E's 131 — smoother, below 60 | the better-paced choice | arm G |
| lsfg-vk 1.0.0 x2 (the user's Lossless.dll) on top of F, entry written by BiGame-mode, A B A B | same | rendered (the game's count) 38 / 39 → **27 / 27**; presented (MangoHud) 41.3 / 42.3 → **57.9 / 57.0**; frames over twice the median 70 / 132 → 1329 / 908 | more frames shown, −30 % rendered, worse pacing | `2026-09-25-sottr-gtx1050ti-lsfg` |
| Turbo off versus on (falcond per-game profile: power profile and governor performance, idle inhibit), on top of F, A B A B | same | rendered 40 / 36 vs 39 / 37; presented 43.0 / 39.2 vs 42.8 / 41.4; CPU package 87–88 vs 88–89 °C | no difference: GPU-bound at the GTX's power limit | `2026-09-25-sottr-gtx1050ti-turbo` |
| sched-ext `lavd`, `bpfland` vs none, set in the game's falcond profile (falcond loads it at game start, unloads it after; checked in `/sys/kernel/sched_ext`), A B C C B A + A B C | SotTR, lab laptop, same settings | rendered frames 5863 ± 175 / 6097 ± 196 / 6136 ± 215 (+4.0 %, +4.7 %); Welch's t 1.5 and 1.7, under the 95 % critical value; every arm drifted up through the evening | no difference | `2026-09-25-sottr-gtx1050ti-scheduler` |
| AI Graphics on a game that ships the FidelityFX API: the game's FSR 3.1 → the same with `FSR4_UPGRADE=1` (Proton's FSR 4 provider, verified mapped, `Replaced FSR3 with FSR4!` logged) → OptiScaler 0.9.4 FSR from the game's XeSS (`Fsr4Update`), two passes per arm | Cyberpunk 2077 2.3, RT Ultra, 3440×1440, upscaling Auto, RX 9060 XT | 38.4 → 38.2 fps (−0.6 %, Welch's t 1.9: no difference) → **36.0 fps (−6.4 %, t 24.7)**; lows too scattered over two runs to compare | FSR 4 through Proton costs nothing measurable; OptiScaler slower where the game's own FSR already reaches FSR 4 | `2026-09-26-cyberpunk-rx9060xt-native-vs-optiscaler` |

What follows for the code:

- `high` pins the highest *fixed* DPM state and removes the firmware's boost,
  leaving power budget unused while the GPU is equally busy. The Booster never
  forces it.
- Power profile, governor and EPP made no difference here, GPU- or CPU-bound,
  so the Booster leaves them to power-profiles-daemon and falcond.
- No scheduler was faster, so recommended profiles use `scx_sched = none`.
- AI Graphics is the first setting BiGame-mode applies that measurably moves the
  frame rate. This holds for OptiScaler 0.9.4, the tested release. The gain
  is larger where the game's XeSS runs on the slower DP4a path (the GTX).
- On the lab laptop the planner, reading that session from the local
  measurements, reports the gain but keeps the game's own XeSS as
  Recommended: the 1 % low could not be shown to be no worse.
- The session also found that OptiScaler's default configuration made the
  game exit at start on a GTX (its DLSS path on a card without DLSS); the
  configuration BiGame-mode writes turns that path off there.
- At the lab laptop's lowest preset the GPU, not the settings, is the limit:
  36–40 fps rendered whatever the resolution, and DX11 only moves the limit to
  the CPU. Nothing an upscaler does reaches 60 there, so the plan says so when
  every measurement of a game is far below 60, and names frame generation as
  the one thing that presents more frames than are rendered. lsfg-vk x2
  presented 57–58 from 27 rendered, stable; OptiScaler's frame generation
  presented 60.7 (arm E) but raised NVIDIA Xid errors on the GTX. It stays
  the user's choice.
- Turbo (falcond's per-game profile) was applied and verified — power
  profile and governor performance in the game, restored after — and made
  no measurable difference in this GPU-bound game.
- On that laptop the CPU hit its temperature limit all day: on cpu0, core
  throttling events went from 373 to 14 219 and the package spent 289 s
  slowed. The CPU temperature check in Details → Problems reports the
  kernel's throttle counters: a CPU capped by its cooling is not helped by a performance
  governor, and the check says so.
- A game that ships AMD's FidelityFX API (FSR 3.1) reaches FSR 4 through
  Proton with one launch option and no files, at the same frame rate as its
  FSR 3.1, while OptiScaler on top of the same game measured 6 % slower. The
  planner therefore keeps the game's own FSR as Recommended there, and
  OptiScaler only where the game has no FSR path (Shadow of the Tomb Raider)
  or the game list records it as faster.
- The game's `XESS` registry value 1 is *Performance* in its menu and 3 is
  *Quality*; the results above record the upscaler, not its preset.

## Limits

- Rendered frames only. Latency was never measured.
- FSR 4 cannot be proven from OptiScaler's log (only its overlay shows which
  model runs); FSR 3.1 can, and on NVIDIA it was. For the game's own FSR
  through Proton the evidence is the provider mapped in the running game and
  Wine's `amdxc` channel (off by default), not an image.
- A visual-quality comparison was inconclusive: captures at fixed times land on
  different frames.
- Two machines and a handful of titles. Hybrid and multi-CCD CPUs, 3D
  V-Cache, RTX and Intel GPUs, and Gamescope native versus nested have not
  been measured; NVIDIA only as the GTX above, for AI Graphics.
- At the lab laptop's ~16 fps the frame-time floor varies too much between
  runs to compare; averages are what those runs establish.
- Presented frames on the reference desktop: with the system's lsfg-vk and
  MangoHud packages, MangoHud sits above lsfg-vk in the layer order and
  counts only the game's own frames (0.99× with x2 on); forcing the order
  with `VK_INSTANCE_LAYERS` hung Shadow of the Tomb Raider at start. On the
  lab laptop, with a per-user lsfg-vk manifest, the order was the other way
  round and MangoHud counted generated frames.
- lsfg-vk does not start or stop generating for a game already running, so
  an lsfg-vk arm needs its own launch; only the multiplier can alternate
  within one.
- Keep other load off the machine while benchmarking; a virtual machine on the
  same host caused multi-second stalls in one session.
