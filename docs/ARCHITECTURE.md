# Architecture

BiGame-mode is a GTK4/libadwaita application for BigLinux that decides *who
tunes what* while a game runs, sets it up, shows what is really in effect and
measures whether it helped. System performance during a game belongs to
[falcond](https://git.pika-os.com/general-packages/falcond); BiGame-mode
switches falcond on and off (Turbo), writes the per-game profiles it reads,
and owns what happens *inside* the game: upscaling, frame generation and the
files a game needs for them.

One rule runs through all of it: **detect → measure → optimize → verify →
report → restore**. A change is reported only after it has been read back
from the system, "applied" is never rounded up to "improved", and without a
measurement the answer is *performance impact not measured* — a value, not an
error.

## Components

| Component | Where | Responsibility |
|---|---|---|
| UI | `bigame-engine/bigame-ui` (GTK4, libadwaita, `ksni` tray) | Never runs as root. Pages: Home, Details, Profiles, Tuning, Video, Benchmark, Diagnostics, Logs, Settings, and the Optimization Report. `--background` starts it hidden in the tray (login autostart); `--diagnostics [--network]` prints the support report and exits. |
| Core | `bigame-engine/bigame-core` (library, no UI) | Hardware and capability detection, Turbo (`turbo.rs`), the Booster engine (`booster/`), profiles and their migration, running-game detection (`running.rs`), recommendations (`recommend.rs`), the launch pipeline (`launcher.rs`, `gamescope.rs`), AI Graphics (`graphics/`), benchmarking (`benchmark/`), network measurement, logs, health and diagnostics. |
| Helper | `bigame-engine/bigame-daemon` (root, system bus `com.biglinux.BiGameMode`, object `/com/biglinux/BiGameMode`) | The handful of writes the UI cannot do: `SaveProfile`, `DeleteProfile`, `ApplyFalcondConfig`, `SetVCacheMode`, `SetCpuGovernor`, `SetCpuEpp`, `SetGpuDpmLevel`, `SetGameBackend`, `ReleaseGameBackend`, plus the unauthenticated `Ping`. Started on demand by D-Bus through `bigame-daemon.service`. See [SECURITY.md](SECURITY.md). |
| falcond | external system service | Matches game processes by name and applies their profile — performance power profile, sched-ext scheduler (through `scx_loader`), 3D V-Cache mode, idle inhibit — and restores everything when the game exits. |

## Turbo

Turbo off means BiGame-mode does not intervene in games; on means falcond runs
and applies per-game profiles. **Turbo's state is falcond's unit state as
systemd reports it**, never a flag of BiGame-mode's own, so it cannot say
"off" while falcond runs.

- **On:** conflicts are noted → falcond's profile set is corrected when clearly
  wrong (`handheld` on a desktop or laptop) → falcond is enabled and started
  through systemd's D-Bus API and verified → the Booster applies its
  evidence-gated global plan → a report is written.
- **Off**, in reverse on purpose: falcond is stopped (its shutdown restores the
  profile of a running game) and disabled → the Booster journal is restored →
  report. falcond's snapshot of a running game was taken after the Booster's
  changes, so it has to be put back first.
- The service is the switch because it is the only one that works with
  falcond 2.0.2: stopping it restores an active profile, while
  `enable_performance_mode = false` is honoured only at start-up.
- The first time BiGame-mode changes falcond, the state it found (enabled or
  not, running or not) is recorded in `/var/lib/bigame-mode/game-backend.json`.
  *Settings → Hand back* and package removal restore exactly that.

## Who writes what

Every setting has one owner. Two writers of the same state undo each other:
the second to restore writes the first one's value back as the "baseline".

| State | Owner |
|---|---|
| falcond on/off | Turbo, through the helper and systemd |
| Per-game performance mode, scheduler, V-Cache, idle inhibit | falcond, from the profile BiGame-mode writes |
| Power profile | falcond per game; the Booster never writes it while falcond is installed |
| Governor and EPP on `amd-pstate` (active) | power-profiles-daemon, through the profile |
| Governor on other cpufreq drivers | Booster, only with evidence |
| GPU DPM level | Booster, **only where a benchmark on this machine measured it faster**; `high` pins the highest fixed state and loses the firmware's boost (−7.5 to −8.3 % on an RX 9060 XT, see [BENCHMARKS.md](BENCHMARKS.md)) |
| sched-ext scheduler | falcond, through `scx_loader`; the Booster never loads a scheduler |
| Files inside a game folder | AI Graphics |
| Gamescope, environment and frame generation at launch | the launch pipeline |
| Telemetry and network | read and measured, never tuned |

Feral GameMode is not integrated and there is no backend selector: GameMode and
falcond snapshot and restore the same state independently. When GameMode is
installed it is reported as a conflict, never removed.

Presentation settings (Gamescope, FSR/NIS, frame generation, FPS cap) and
execution settings (scheduler, V-Cache, governor, EPP, GPU DPM, power profile)
are separate layers: the launch pipeline applies what the user configured
whatever Turbo is doing.

## Booster engine

`booster/` applies the global part of Turbo: **detect → snapshot → plan →
apply → verify → report → restore**.

- Knobs: power profile, CPU governor, CPU EPP, GPU DPM level per card, V-Cache
  mode. Allowed values come from the machine; reads are never cached; write and
  verify are separate, and a mismatch means another writer.
- A knob whose baseline cannot be read is never written. Restoring puts back
  only the knobs that were applied, in reverse order.
- The planner may plan nothing. Every skip is typed: unsupported, already
  optimal, not beneficial, not restorable, measured harmful, owned by another
  component. Calibration from this machine is consulted first. On battery the
  plan raises nothing. Only the GPU games render on is considered.
- The journal (`$XDG_STATE_HOME/bigame-mode/booster-journal.json`, 0600) is
  written before the first change and carries the boot id, so a journal from a
  previous boot is discarded.
- *Measure the difference* (a game card's menu) alternates baseline and
  optimized runs of the game, discards the first run of each arm and restores
  the baseline on every path. It checks the plan first and says when there is
  nothing to compare.

## Profiles and game detection

- falcond 2.0.2 reads exactly `name`, `performance_mode`, `scx_sched`,
  `scx_sched_props`, `vcache_mode`, `start_script`, `stop_script` and
  `idle_inhibit`. Profiles are keyed on the **process name**
  (`/proc/<pid>/comm`; falcond splits `argv[0]` on `/` and `\`), never on the
  install folder or the title.
- The global configuration is `/etc/falcond/config.conf`; system profiles are
  in `/usr/share/falcond/profiles/` (and `handheld/`, `htpc/`), user profiles
  in `…/user/`.
- The library comes from Steam, Lutris and Heroic, and from the application
  menu: `.desktop` entries in the `Game` category are native games, launched
  with their `Exec` line (past wrappers such as `env`, `prime-run`,
  `gamemoderun` or `gamescope … --`).
- The running game is identified from `/proc` without spawning processes: the
  Steam reaper tree (`AppId=`), Wine `.exe` processes, native menu games, and
  names falcond has a profile for. Launchers, stores, game streaming and
  BiGame-mode itself are never taken for a game.
- When an unknown game starts with Turbo on, a notification offers a profile.
  The recommendation writes only falcond's fields and says where each value
  comes from: `performance_mode` on AC power only, `scx_sched = none` unless a
  scheduler was measured faster, `vcache_mode` explicit, `idle_inhibit = true`.
- Profiles written by older BiGame-mode versions are migrated: re-keyed to the
  game's executable, cleaned of fields falcond ignores, backed up first and
  never deleted when unresolved.

## Launch pipeline

- There is one Gamescope argument builder, and it cannot be called without the
  capabilities parsed from `gamescope --help` of the installed binary; an
  unsupported request is reported, not silently dropped. Gamescope per game:
  Auto, Enabled or Disabled.
- `LaunchPlan` takes the machine as a `Host`: real launches detect it, tests
  describe it, so tests pass the same in a desktop session, over ssh or in a
  build chroot.
- Games started from BiGame-mode run in their own process group, so ending one
  ends everything it started, including a wrapper script's game.
- `steam -applaunch` is not wrapped: it starts the client, not the game. A game
  started through the Steam client gets falcond's profile (falcond matches its
  process) and AI Graphics (the files are in its folder), but not the launch
  pipeline's Gamescope and environment settings.
- **Harmony policy** — two technologies doing the same job never run in series:
  - lsfg-vk selected without its `Lossless.dll` is switched off for the launch;
  - a game with OptiScaler installed launches without Wine FSR and without a
    Gamescope render size (second upscalers), and without lsfg-vk
    (`DISABLE_LSFG=1`) when OptiScaler generates frames;
  - global settings are never changed by this; only the launch is.

## AI Graphics

`bigame-core/src/graphics/` runs when the user opens a game's AI Graphics page,
and at launch only to answer "did BiGame-mode install something here?". No part
of it needs root.

- **Flow:** scan → report → plan → Apply (fetch the pinned OptiScaler release,
  verify it, build the payload, apply it as a transaction) / Repair (put back
  missing files) / Restore (remove what BiGame-mode placed and put every
  original back). Runtime status comes from the game's mapped libraries and an
  `OptiScaler.log` written since the process started.
- **Modules:** `pe` (import tables and file versions by positioned reads),
  `scan` (upscalers and their versions, proxy-DLL owners by content,
  anti-cheat markers), `report` (each value with its confidence: fact,
  detected, likely, assumed), `rules` (the compatibility matrix as code),
  `plan`, `optiscaler`, `manifest`, `transaction`, `runtime`, `support` (a
  redacted report archive), `config`, `text` (translatable templates); the
  facade is `graphics/mod.rs`, per-game choices are in `game_settings.rs`.
- **Decisions:**
  - OptiScaler goes in only as `dxgi.dll`, which Proton loads natively from
    the game folder with no `WINEDLLOVERRIDES`; another tool's `dxgi.dll`
    stops the plan instead of being overwritten.
  - Anti-cheat blocks injection in every mode, with no override; the game's
    own upscaler is still suggested as an in-game setting.
  - Frame generation is never automatic; OptiScaler's is Experimental.
  - FSR 4 is never claimed from configuration; the UI says "FSR".
  - Only the Apply button changes a game's files, and not while the game runs.
  - An apply interrupted by a crash or power loss is rolled back when the
    application next starts.
- **Compatibility rules:** never two upscalers or two frame generators in
  series; each verdict says what it rests on (tested here, upstream, or the
  principle); a pair nobody has established is *unknown*, not "supported".
- **Scope:** the "DLSS 5" of some community tools is a leaked NVIDIA build;
  nothing from that path is bundled, fetched or suggested. A game that ships
  DLSS itself is simply "native DLSS".

## Where data lives

User paths follow the XDG base directories (`paths.rs`); with `HOME` unset the
home directory comes from the password database, never from a shared
directory.

| Path | Content |
|---|---|
| `/etc/falcond/config.conf` | falcond's global configuration (written by the helper) |
| `/usr/share/falcond/profiles/user/<process>.conf` | per-game falcond profiles (written by the helper) |
| `/var/lib/bigame-mode/game-backend.json` | how falcond was before BiGame-mode took charge |
| `/var/lib/falcond/status`, `/tmp/falcond_status` | falcond's status, read only when it is a root-owned regular file |
| `$XDG_CONFIG_HOME/bigame-mode/` | `settings.toml`, `video.toml`, `gamescope.toml`, `games/<process>.toml` |
| `$XDG_STATE_HOME/bigame-mode/` | Booster journal, last Turbo report, calibration, benchmark history, profile-migration backups |
| `$XDG_STATE_HOME/bigame-mode/graphics/<game>/` | AI Graphics manifests and backups |
| `$XDG_CACHE_HOME/bigame-mode/graphics/optiscaler/<version>/` | the downloaded, verified OptiScaler release |
| `$XDG_CACHE_HOME/bigame-mode/benchmark/` | MangoHud captures of *Measure the difference* |

## Network, logs and the application's own cost

- Network is measured, never tuned: a DNS benchmark with hand-built UDP
  queries (median, p95, jitter, loss), the default-route interface and its
  queue discipline. fq_codel/CAKE, disabling IPv6, jumbo MTU, BBR and NIC
  offload changes are deliberately not applied.
- falcond's status file is watched with inotify, never polled. The game
  watcher reads `/proc` every 5 s without spawning a process.
- The Logs page reads the journal with one `journalctl -o json` call,
  incrementally, only while it is on screen: falcond, the helper, the UI,
  `scx_loader`, power-profiles-daemon, Gamescope, BiGame-mode's Polkit records
  and the kernel's GPU and sched-ext messages.

## falcond behaviours worth knowing (2.0.2)

- Activation snapshots the power profile, scheduler and V-Cache mode, then
  applies the profile; SIGTERM deactivates before exit; SIGHUP reloads and
  re-reads `user/`. The helper reloads falcond with SIGHUP through systemd,
  never with a restart, which would tear down a running game's profile.
- If falcond is killed with SIGKILL while a profile is active, systemd restarts
  it and the new instance snapshots the boosted state. BiGame-mode does not
  paper over that with a second writer; Diagnostics warns when systemd has
  restarted falcond.
- A user profile with the same name as a shipped profile is not applied.
- New processes are checked at once only when they are `.exe`, a Wine loader or
  already known; others wait for falcond's rescan (9 s).
- Scheduler switching needs `scx-tools` (`scx_loader`); without it every
  switch fails.
