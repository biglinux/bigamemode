# 16 — Performance backends: falcond, GameMode, and who owns what

**Objective.** Decide, against the versions installed and upstream, whether
BiGame-mode should offer a choice of backend, and fix the rule that every piece
of system state has exactly one writer.

## Decision

**falcond is the only backend. There is no selector.** Feral GameMode is
detected and reported as a conflict avoided, with the reason. A choice between
two backends is only worth the complexity if one of them does something useful
the other cannot, and on the evidence below it does not.

## What each can do

GameMode's features are taken from its own `example/gamemode.ini`
(FeralInteractive/gamemode, master); falcond's from its source at the
installed 2.0.2 and at upstream `main` (2.0.14 + 13 commits).

| Capability | GameMode | falcond 2.0.2 (installed) | falcond 2.0.14 (upstream) |
|---|---|---|---|
| Performance power profile / governor while playing | `desiredgov`, `desiredprof` | `performance_mode` | same |
| Screensaver inhibit | `inhibit_screensaver=1` | `idle_inhibit` | same |
| 3D V-Cache mode | `amd_x3d_mode_desired` | `vcache_mode` | same |
| sched-ext scheduler per game | — | `scx_sched`, `scx_sched_props` (via scx_loader) | same |
| Per-game process matching incl. Proton | the game must opt in (`gamemoderun`) | automatic, by process name; generic Proton fallback | same |
| VRAM protection (DMEM cgroup) | — | — | `dmem_protect` |
| Split-lock mitigation off while playing | `disable_splitlock=1` | — | `disable_split_lock` (on `main`) |
| I/O priority | `ioprio=0` | — | — |
| Renice | `renice` (default 0 = off) | — | — |
| Core parking / pinning | `park_cores`, `pin_cores` | — | — |
| AMD GPU performance level | `amd_performance_level=high` | — | — |
| NVIDIA clock offsets | `nv_core_clock_mhz_offset`, … | — | — |
| Start/stop scripts | `start`, `end` | `start_script`, `stop_script` | same |

## Is there a case where GameMode is better here?

| GameMode-only feature | Assessment |
|---|---|
| `amd_performance_level=high` | This is the setting measured **8.0 % slower** in Shadow of the Tomb Raider and 7.5 % in SuperTuxKart on this card. A backend that offers it is a liability here, not an advantage. |
| NVIDIA clock offsets | Overclocking; excluded by this project's rules. |
| `disable_splitlock` | Real, for the few games that trip split-lock "misery mode". falcond has it on upstream `main`, per profile. The gap closes with a falcond update. |
| `ioprio` | Only matters under I/O contention. NOT MEASURED; no workload here produces contention while a benchmark runs. |
| Core parking/pinning | For multi-CCD and X3D CPUs. NOT TESTED — X3D hardware unavailable. |

None of these is a measured benefit on this machine, and one is a measured
regression. **No selector was added.**

## Why never both

Both snapshot and restore the same state (the power profile above all)
independently. Whichever restores second writes the other's *changed* value
back as a baseline, and neither reports an error, because each did what it was
told. That is the failure the single-owner rule exists to prevent, and it is
why BiGame-mode reports GameMode as `ConflictAvoided` rather than using it.

GameMode is **not installed** on the reference machine; the detection path is
covered by the Turbo report's unit tests. NOT TESTED — with GameMode running.

## Who writes what (after this work)

| State | Owner | How it is enforced |
|---|---|---|
| falcond service on/off | Turbo, through the helper and systemd | `SetGameBackend`; ownership record in `/var/lib/bigame-mode` |
| Per-game profile (performance mode, scheduler, V-Cache, idle inhibit) | falcond | Booster reports `OwnedBy falcond` |
| Power profile | falcond (per game) | Booster skips it whenever falcond is installed |
| EPP / governor on amd-pstate-epp | power-profiles-daemon (via the profile) | Booster skips the governor there |
| Governor on other drivers | Booster, evidence-gated | NOT TESTED — no such hardware |
| GPU DPM level | Booster, only where measured faster | calibration gate; measured slower here |
| falcond's profile set (`profile_mode`) | BiGame-mode, corrected only when clearly wrong | handheld → desktop on a desktop/laptop |
| Frame generation (lsfg-vk) settings | lsfg-vk's own config | no longer mirrored into falcond profiles |
| Telemetry | read-only | — |

A second, unexpected writer found on this distribution:
`power-profiles-daemon-biglinux-cpufreq.service` runs on every power-profile
change and maps profiles to governors. On `amd-pstate-epp` it does nothing
(it only acts where `schedutil` is offered); on an `acpi-cpufreq` machine it
would race Booster's governor write. NOT TESTED — no such hardware.

## falcond's own state

| Finding | Status |
|---|---|
| Installed 2.0.2 is twelve releases behind upstream 2.0.14 | VERIFIED — the BigLinux `community-extra` package lags |
| This kernel supports what 2.0.14's `dmem_protect` needs | VERIFIED — `/sys/fs/cgroup/dmem.capacity` lists the RX 9060 XT's 16 GB and the iGPU |
| Newer falcond publishes status in `/var/lib/falcond/status` | VERIFIED in upstream README; BiGame-mode now prefers it |
| `DMEM Cgroup` feature line is parsed; absent on 2.0.2 | TESTED |
