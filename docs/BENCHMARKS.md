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
- **A capped workload is refused.** SuperTuxKart's default vsync and
  `max_fps = 120` make any comparison meaningless, and the Benchmark page says
  which keys to change.
- **Frame times, not averages alone.** Games' own per-frame logs are used where
  they exist (Crystal Dynamics `*_frametimes_*.txt`, Cyberpunk 2077
  `frames.csv`), otherwise MangoHud's. Frames of one second or more are scene
  transitions and are set aside. Settings must be identical within a session,
  except the keys under test. Runs with frame generation are not throughput
  results.
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
| `scripts/gpu-telemetry.sh` | GPU clock, power, temperature and utilisation 4×/s, without forking (a forking sampler raised the spread from 1.4 % to 6 %) |
| `scripts/scx-switch.sh` | the root side of scheduler sessions: one Polkit approval, the profile restored when the session ends |
| `cargo run -p bigame-core --example bench_native_report -- <session> [baseline] [--vary=KEY,…]` | verdicts for a `bench-game.sh` session |
| `cargo run -p bigame-core --example bench_report -- <session> <baseline>` | verdicts for a `bench-lab.sh` session |
| *Measure the difference* (a game card's menu) | the same A/B method for any game that starts directly, driven by the application |

Each session directory holds `system.json` (the machine, with no host name,
user, home or address), the runs of every arm, and the report.

## Reference machine

AMD Ryzen 7 5700G (8C/16T, `amd-pstate-epp`, no 3D V-Cache), Radeon RX 9060 XT
(RDNA 4, 16 GB) plus the idle integrated GPU, Mesa 26.2.2, kernel 7.2.6,
KDE Plasma Wayland, Proton Experimental, 3440×1440.

Results are evidence for the defaults on this machine, not claims about other
hardware. Calibration applies only while the machine's fingerprint matches (CPU,
GPU, driver, kernel and memory), so a kernel update makes it absent until
measured again.

## Results

| Setting | Workload | Result | Verdict | Data |
|---|---|---|---|---|
| GPU DPM `high` vs `auto` | Shadow of the Tomb Raider (SotTR), 3440×1440 High, GPU-bound | 89.4 → 82.2 fps, **−8.0 %** (spread 0.1 %); 1 % low −7.6 %; 3246 → 2640 MHz, 163 → 103 W | slower | `2026-09-23-sottr-power-clean` |
| GPU DPM `high`; balanced vs performance power profile | same, first session | −8.3 %; power profile −0.1 % | slower; no difference | `2026-09-23-sottr-power` |
| GPU DPM `high` | SuperTuxKart, 3440×1440 max, uncapped | 298.4 → 275.9 fps, −7.5 % | slower | `2026-09-23-supertuxkart-gpu-bound` |
| Power profile, governor, EPP → performance | SotTR, CPU-bound (minimum render scale) | all within 1.5 %, no consistent order | no difference | `2026-09-23-sottr-cpu-bound` |
| sched-ext `lavd`, `bpfland` vs none, through falcond | SotTR, CPU-bound | +1.1 %, +1.8 %, inside the spread | no difference | `2026-09-24-sottr-scheduler` |
| AI Graphics: native TAA → the game's XeSS Quality → OptiScaler FSR from XeSS Quality | SotTR, 3440×1440 High | 89.8 → 94.2 fps (+4.9 %) → **98.8 fps (+10.1 %)**, spread 0.2 %; 1 % low unchanged; GPU 164 → 156 W | faster | `2026-09-24-sottr-ai-graphics` |

What follows for the code:

- `high` pins the highest *fixed* DPM state and removes the firmware's boost,
  leaving power budget unused while the GPU is equally busy. The Booster never
  proposes it by default; only a calibration that measured it faster enables it
  on that machine.
- Power profile, governor and EPP made no difference here, GPU- or CPU-bound,
  so the Booster leaves them to power-profiles-daemon and falcond.
- No scheduler was faster, so recommended profiles use `scx_sched = none`.
- AI Graphics is the first setting BiGame-mode applies that measurably moves the
  frame rate. This holds for OptiScaler 0.9.4, the pinned release.

## Limits

- Rendered frames only. Latency was never measured.
- Whether FSR 4 or FSR 3 ran cannot be told from OptiScaler's log (only its
  overlay shows it), so the UI says "FSR".
- A visual-quality comparison was inconclusive: captures at fixed times land on
  different frames.
- One CPU, one GPU and a handful of titles. Hybrid and multi-CCD CPUs, 3D
  V-Cache, NVIDIA and Intel GPUs, and Gamescope native versus nested have not
  been measured.
- Keep other load off the machine while benchmarking; a virtual machine on the
  same host caused multi-second stalls in one session.
