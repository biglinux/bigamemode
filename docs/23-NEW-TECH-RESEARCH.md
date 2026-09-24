# 23 — New technology research

The rule for every candidate: it earns a place only by doing something
concrete that BiGame-mode does not already do, without a conflict, with a
measured or clearly argued benefit. Facts below were checked against the
source, the package database or the project's own documentation during this
pass; anything not checked says so. The distributions' approaches were
researched earlier in [03-GAMING-DISTROS-RESEARCH.md](03-GAMING-DISTROS-RESEARCH.md).

| Project | What it does | Do we have it? | Would add | Conflict | Verdict |
|---|---|---|---|---|---|
| **falcond** (PikaOS) | Per-game profiles by process name: power profile, sched-ext, V-Cache, idle inhibit | Yes — the backend | 2.0.14 adds **`dmem_protect`** (VRAM protection via the DMEM cgroup) and, on `main`, **`disable_split_lock`**; status in `/var/lib/falcond/status` | — | **Update the package.** Installed 2.0.2 is twelve releases behind. This kernel supports DMEM (`/sys/fs/cgroup/dmem.capacity` lists the RX 9060 XT). BiGame-mode now reads the new status path and the `DMEM Cgroup` feature. |
| **falcond-profiles** | Upstream per-game profiles, in `none`/`handheld`/`htpc` sets | Yes (r23) | — | BiGame-mode never edits them | Keep; they are falcond's. |
| **sched-ext/scx** | BPF CPU schedulers | `scx-scheds` 1.1.3, 16 schedulers | — | — | Present; unusable without the loader. |
| **scx-loader** (`scx-tools` on Arch) | D-Bus service `org.scx.Loader` that starts/switches schedulers; modes Auto/Gaming/LowLatency/PowerSave/Server | **No** — not installed on the reference machine | falcond's per-game scheduler switching, which failed on every game today | — | **Install.** VERIFIED on the VM: falcond → scx_loader → kernel `sched_ext` enabled with `lavd` while the game runs, disabled on exit. Diagnostics now names the package. Whether any scheduler is *faster* here is NOT MEASURED. |
| **Feral GameMode** | Governor, screensaver, ioprio, split-lock, GPU clocks, core pinning | Detected, not used | ioprio, split-lock, pinning | **Yes** — same power profile as falcond | Not integrated ([16](16-PERFORMANCE-BACKENDS.md)). Its AMD feature is the setting measured 8 % slower. |
| **Gamescope** | Micro-compositor: scaling, limiting, HDR/VRR | Yes, capability-driven (flags from `--help`) | — | with other limiters/upscalers only | Keep as is. 3.16.28 installed. Native vs nested NOT MEASURED. |
| **MangoHud** | Overlay and per-frame CSV | Yes (0.8.4) | — | — | Used as instrumentation where a game has no frametime log; games' own logs preferred. |
| **lsfg-vk** | Frame generation as a Vulkan layer | Yes | — | with any other frame generator | Keep; its settings now live only in its own config. Generated frames are never compared with rendered ones. |
| **vkBasalt** | Post-processing layer (CAS here) | Detected | — | — | User's choice. Measured cost NOT MEASURED (a toggle whose state cannot be read back could produce a false "no difference"). |
| **Phoronix Test Suite** | Scripted benchmark runner | VM only | Batch runs, exports | — | Useful for synthetic regression on the VM; batch mode needs `PRESET_OPTIONS`. Not a substitute for games. |
| **taaderbe/linuxgamebench** | MangoHud capture of gameplay the user plays and marks (Shift+F2); AVG/1 %/0.1 %/stutter; HTML/JSON; community uploads | No | A shared result database | — | Not adopted. It measures free play the user starts and stops by hand, which cannot give the alternating, same-scene runs our comparisons need. GPL-3.0, active (checked). Its community database is interesting for cross-machine context, never for local decisions. |
| **power-profiles-daemon** | Platform power profiles; on amd-pstate, EPP | Yes | — | BigLinux's `…-biglinux-cpufreq` also maps profiles to governors | Owner of EPP on amd-pstate; BiGame-mode no longer writes the governor there. |
| **CachyOS, Bazzite, Nobara, SteamOS, PikaOS** | Gaming distributions | — | — | — | See [03](03-GAMING-DISTROS-RESEARCH.md). Not re-researched in this pass. PikaOS is falcond's origin. |

## What this changes

1. **Two packages would make the existing design work fully:** `scx-tools`
   (scheduler switching), and a current `falcond` (DMEM protection, split-lock
   handling, and — worth asking upstream — a restore snapshot that survives
   its own crash; see [21](21-VM-TESTS.md)). Both are BigLinux packaging
   decisions, not code in this repository.
2. **Nothing new becomes a dependency.** Every candidate that would add a
   capability either duplicates falcond, competes with it, or cannot produce
   the controlled measurements this project's decisions rest on.
