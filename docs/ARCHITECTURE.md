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
| UI | `bigame-engine/bigame-ui` (GTK4, libadwaita, `ksni` tray) | Never runs as root. Pages: Home, Profiles, Tuning, Details, Logs, Settings, and the Optimization Report over Home. `--background` starts it hidden in the tray (login autostart); `--diagnostics [--network]` prints the support report and exits. |
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
| Governor and EPP, wherever power-profiles-daemon runs | power-profiles-daemon, through the profile (on BigLinux its companion `power-profiles-daemon-biglinux-cpufreq` maps the profile to a governor on passive drivers); falcond asks for `performance` per game |
| Governor without power-profiles-daemon | Booster |
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
- The library (`library.rs`) is the installed games, each with the profile
  that matches one of its process names, if any. Profiles are looked up for
  games, never turned into games: falcond ships profiles for titles that may
  not be installed. The user's profiles that match no installed game are
  reported apart and never deleted by a scan.
- Games come from Steam, Lutris, Heroic and the application menu
  (`games.rs`), and each launcher's record is checked against the disk
  before it counts: a Steam manifest needs its `StateFlags` *FullyInstalled*
  bit and a non-empty install directory; a Lutris configuration needs the
  file it would run (`exe` or `main_file`, resolved against `working_dir` or
  the prefix; `$GAMEDIR` cannot be resolved without Lutris's database, so
  such entries are skipped); Heroic's store caches and the backends'
  `installed.json` need the install directory; a `.desktop` entry in the
  `Game` category needs its program on `PATH` (past wrappers such as `env`,
  `prime-run`, `gamemoderun` or `gamescope … --`), or, for `flatpak run`,
  the application's metadata, whose `command` is the process. Entries that
  also list `PackageManager`, `Utility`, `Settings`, `System` or
  `Development` are stores and tools, not games. The Flatpak installations
  of Steam, Lutris and Heroic are read as well.
- One card per game: the same title from two launchers is folded by Steam
  or Flatpak id, install directory or launch file, in the order Steam,
  Heroic, Lutris, menu.
- Detection is a plain filesystem read (about 40 ms for twenty titles) and
  runs off the main thread; the grid on screen is replaced only when the
  library changed.
- The running game is identified from `/proc` without spawning processes: the
  Steam reaper tree (`AppId=`), Wine `.exe` processes, native menu games, and
  names falcond has a profile for. Launchers, stores, game streaming and
  BiGame-mode itself are never taken for a game.
- What a running game really got is read from it, never from settings:
  MangoHud, vkBasalt and lsfg-vk count only when mapped in the game process
  (frame generation only with lsfg-vk mapped *and* an entry for the game),
  Gamescope only in the game's process tree, a scheduler only when
  `/sys/kernel/sched_ext` reports one (`running::in_game`, Home's "In the
  game" line). When lsfg-vk's file changed after the game started, Home says
  that turning it on or off waits for the next start.
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
    (`DISABLE_LSFG=1`) when OptiScaler generates frames — chosen on the page,
    or switched on later from OptiScaler's overlay, which writes the
    `OptiScaler.ini` in the game;
  - global settings are never changed by this; only the launch is.
- Nested Gamescope gets `--expose-wayland` where the build has it: without
  it a game that prefers Wayland (vkcube, SDL3) inherits the desktop's
  `WAYLAND_DISPLAY` and Gamescope's WSI layer ends it at start. vkBasalt is
  switched off for the Gamescope process and back on for the game
  (`env -u DISABLE_VKBASALT ENABLE_VKBASALT=1 <game>`): Gamescope otherwise
  loads it itself, filters its own output after FSR, and removes
  `ENABLE_VKBASALT` from the game's environment.
- lsfg-vk re-reads its file for a game that started with an entry and then
  applies a new multiplier, but it neither starts nor stops generating for a
  running game; on and off take effect at the next start. The layout older
  BiGame-mode versions wrote, which makes lsfg-vk ignore the whole file, is
  converted when the application starts (a copy is kept as
  `conf.toml.bigame-legacy`).
- MangoHud per game is written where the game's launcher reads it. On uses
  MangoHud's Vulkan layer (`MANGOHUD=1`), Forced its wrapper, which also
  reaches OpenGL games.
  - Steam: the launch option. A game played with Steam's defaults has no
    block in `localconfig.vdf`; one is created, in every Steam account of the
    user, only while Steam is closed, and the choice is saved only after the
    options read back.
  - Heroic: `GamesConfig/<app>.json` (`showMangohud` for Forced, `MANGOHUD=1`
    in its environment options for On), only while Heroic is closed: it keeps
    game settings in memory and writes them back.
  - Lutris: `mangohud: true` in the game's `system:` block.
  - Other keys are kept, the original is backed up once, Off removes what was
    added. A Flatpak Heroic or Lutris cannot see the system's MangoHud (Heroic
    refuses to start the game with its switch on) until
    `org.freedesktop.Platform.VulkanLayer.MangoHud` for its runtime is
    installed; the toast and a health check name the command.
  - Any other game gets it when BiGame-mode starts it.
- lsfg-vk 1.0 reads `version = 1`, `[global] dll` and `[[game]]` entries
  (`exe`, `multiplier` ≥ 2, `flow_scale` 0.25–1.0, `performance_mode`,
  `hdr_mode`, `experimental_present_mode`); one invalid value makes it ignore
  the whole file, so off is *no entry*, never `multiplier = 1`. BiGame-mode
  touches only the entries it wrote (recorded in its state directory) and
  replaces the file atomically for lsfg-vk's live reload. The layer is
  64-bit only, runs on the game's GPU, and `DISABLE_LSFG=1` switches it off.

## AI Graphics

`bigame-core/src/graphics/` runs when the user opens a game's AI Graphics page,
and at launch only to answer "did BiGame-mode install something here?". No part
of it needs root.

- **Flow:** scan → report → plan → Apply (fetch the OptiScaler release the
  game's version choice names, verify it, build the payload, apply it as a
  transaction) / Update (to a newer release, keeping the previous one to Go
  back to) / Repair (put back missing files of the installed release) /
  Restore (remove what BiGame-mode placed and put every original back). Runtime status comes from the game's mapped libraries and an
  `OptiScaler.log` written since the process started.
- **Modules:** `pe` (import tables and file versions by positioned reads),
  `scan` (upscalers and their versions, proxy-DLL owners by content,
  anti-cheat markers), `report` (each value with its confidence: fact,
  detected, likely, assumed), `rules` (the compatibility matrix as code),
  `plan`, `optiscaler`, `versions` (which release a game gets: the tested
  one, the latest stable, or one kept), `outcomes` (benchmark results measured
  on this machine), `gamedb` (a short list of per-game facts detection cannot
  read, carried in the program, extended by the user's own), `manifest`,
  `transaction`, `ingame` (the game's own upscaler switch, in its Wine
  registry), `runtime`, `support` (a redacted report archive), `config`,
  `text` (translatable templates), `backend` (the three backends — the
  game's own, OptiScaler, the external AMD neural component — with their
  capabilities as data and what each is missing on this machine),
  `fsr4_upgrade` (FSR 4 through Proton: the one launch option, written with
  Steam closed and verified in the running game), `external` (the AMD
  neural component: detected, explained, linked, never placed), `diagnose`
  ("why is AI Graphics not working?" as findings with a level, what was
  found and what to do); the facade is `graphics/mod.rs`, per-game
  choices are in `game_settings.rs`.
- **Backends** (`backend.rs`). Capabilities are data; `check(backend,
  &Report)` returns *available* or the list of what is missing, so a page
  never greys a control out without saying why.

  | | The game's own | OptiScaler | AMD neural, external |
  |---|---|---|---|
  | GPUs | any | AMD, NVIDIA, Intel | AMD RDNA 3 / 4 |
  | Game | any | 64-bit Windows game (Proton) | 64-bit Windows game, DirectX 12, ships the FidelityFX API |
  | Jobs | upscaling, frame generation | upscaling, frame generation | neural rendering, on top of the game's own FSR, never with OptiScaler |
  | Files placed | none (one Steam launch option at most) | `dxgi.dll`, `OptiScaler.ini`, FidelityFX / XeSS runtimes, backed up | none: `managed: false`, no manifest |

  A manifest records the backend that placed its files (older manifests
  read as OptiScaler's); nothing removes files BiGame-mode does not manage.
- **Evidence** for every claim the page makes:

  | Claim | Evidence |
  |---|---|
  | OptiScaler loaded, active or failed | `/proc/<pid>/maps` and an `OptiScaler.log` written since the process started |
  | FSR 3.1 through OptiScaler | its log (`Fsr4Update: false`, or the provider missing) |
  | FSR 4 through Proton, the game's own FSR | `FSR4_UPGRADE=1` in the running game's environment **and** `amdxcffx64.dll` mapped |
  | DLSS-NR-on-AMD active | its proxy mapped and its log's "loaded into" banner since the process started |
  | The GPU the game renders on | DRM fdinfo of the render node it submitted work to |

- **FSR 4 through Proton** needs RDNA 4 for the FP8 model (RDNA 3 runs the
  slower INT8 build), a game that exposes FSR 3.1 through the FidelityFX API
  (`amd_fidelityfx_dx12.dll`), Proton's provider (`contrib/amdxcffx64.dll`,
  copied into each prefix's `system32`) and `FSR4_UPGRADE=1` (Valve) or
  `PROTON_FSR4_UPGRADE=1` (GE-Proton), which Wine's `amdxc64.dll` reads with
  `getenv`. Success shows as `Replaced FSR3 with FSR4!` in Wine's `amdxc`
  channel. `WINEDLLOVERRIDES=amdxcffx64=n`, `DXIL_SPIRV_CONFIG` and a copied
  DLL, seen in older guides, are not needed and not written. DirectX 12 is
  verified; Vulkan games are untested.
- **The AMD neural component** needs AMD's Windows HIP runtime
  (`amdhip64_7.dll`, from the Adrenalin driver) in the prefix. Proton ships
  none and no Linux package provides one, so under Proton it is reported as
  not currently compatible.
- **Decisions:**
  - OptiScaler goes in only as `dxgi.dll`, which Proton loads natively from
    the game folder with no `WINEDLLOVERRIDES`; another tool's `dxgi.dll`
    stops the plan instead of being overwritten.
  - Anti-cheat blocks injection in every mode, with no override; the game's
    own upscaler is still suggested as an in-game setting.
  - The GPU a game renders on is observed, not assumed: every NVIDIA card is
    discrete (the proprietary driver publishes no VRAM in sysfs), an Intel one
    when it is off the root bus; a running game's GPU is the one behind its
    open `/dev/nvidiaN`, then the card it has submitted work to (DRM fdinfo:
    `drm-engine-*`, `drm-cycles-*`). Any Vulkan program opens every render
    node just to enumerate the GPUs — on the reference desktop Shadow of the
    Tomb Raider holds the Vega's node with 12 KiB and no work next to the
    RX 9060 XT's with gigabytes and seconds of GPU time — so an open node
    proves nothing. Before any work there is no answer and the expected GPU
    is shown; only a driver without fdinfo falls back to "the non-boot card".
    With two GPUs the plan says which one it is for until the game runs.
  - DLSS is offered only on a card known to be RTX (from its PCI database
    name; frame generation from Ada on); an unknown model is unknown, not
    "yes". On an NVIDIA card without DLSS the ini sets `[DLSS] Enabled=false`:
    OptiScaler otherwise enables its DLSS path on any NVIDIA GPU, and on a GTX
    the game exited at start.
  - Frame generation is never automatic; OptiScaler's is Experimental.
  - FSR 4 is never claimed. The UI says "FSR 3.1" when OptiScaler's log proves
    it (FSR 4 off, or AMD's runtime missing — a warning, not a failure), and
    "FSR" otherwise: which model runs is only shown by OptiScaler's overlay.
  - A game that ships AMD's FidelityFX API keeps its own FSR: Proton upgrades
    it to FSR 4 when the game runs with `FSR4_UPGRADE=1`, so the plan writes
    that one launch option (Steam closed, backed up, read back) and installs
    nothing. The page says "FSR 4 expected" until the running game shows the
    provider mapped and the variable in its environment; measured on the
    reference desktop, OptiScaler on such a game was 6 % slower.
  - Three jobs, one owner each: upscaling (`plan.backend`), frame generation
    (`plan.frame_generation`: none, the game's own, OptiScaler's, lsfg-vk)
    and neural rendering (the external component's status). Two owners of a
    job never run in series.
  - The AMD neural component (DLSS-NR-on-AMD) is `managed: false`: its
    license forbids redistribution and modification, so BiGame-mode detects
    it by content, lists what it is missing (on Linux today: AMD's Windows
    HIP runtime and the user's own model), links the official page and never
    downloads, places or removes it. Under Proton it is "not currently
    compatible", and the page says Experimental.
  - A newer OptiScaler release is offered on the game's page — Update, Skip,
    Keep this version — and never applied by itself or at launch. An update
    has both releases verified in the cache before the game changes, and puts
    the previous one back if the new one fails.
  - What this machine measured beats what is known in general, only when the
    benchmark tests settle it: OptiScaler becomes Recommended when it was
    measured faster *and* its 1 % low shown no worse; a gain with a floor too
    scattered to compare is reported, not chosen. Never against native DLSS
    on RTX.
  - The game list can name a game's default API, prefer the game's own
    upscaler, OptiScaler or nothing, record a tested version, or block
    injection — never unblock a game with anti-cheat.
  - It can also say where a game keeps the switch for the upscaler
    OptiScaler takes over (Shadow of the Tomb Raider: `XESS` in its
    registry). Apply then switches it on when it is off — a preset the
    player chose is left alone — and records the old value in the manifest
    for Restore (module `ingame`). Only DWORD values under
    `HKEY_CURRENT_USER` in the game's own Proton prefix, and only while no
    process runs in that prefix: Wine's server writes its in-memory copy of
    the registry back when it exits.
  - Files BiGame-mode added are not counted as the game's own upscalers.
  - Only the Apply button changes a game's files, and not while the game runs.
  - An apply interrupted by a crash or power loss is rolled back when the
    application next starts.
- **Compatibility rules:** never two upscalers or two frame generators in
  series; each verdict says what it rests on (tested here, upstream, or the
  principle); a pair nobody has established is *unknown*, not "supported".
- **Scope:** the "DLSS 5" of some community tools is a leaked NVIDIA build;
  nothing from that path is bundled, fetched or suggested. A game that ships
  DLSS itself is simply "native DLSS".

## The pages, and where each state comes from

- **Home** is the one control: Turbo, and the running game.
- **Profiles** is the games. Every action on a game is in its card's menu —
  Launch (Turbo), Create with Wizard, Edit, AI Graphics, Measure the
  difference, Restore the game's graphics, Delete — and only the ones that
  apply are shown (Launch for a Steam id or a launcher command, Measure only
  for a direct command, Restore only with files installed).
- **Tuning** is what is applied: falcond's settings (through the helper) and
  the launch settings (`video.toml`, the session environment), as
  collapsible groups. Software that is not installed is a *missing* row with
  the command; hardware that cannot do something is a *not supported* row.
  Wine FSR and Gamescope's render size both on is named in a banner with a
  one-click way out; the other pairs (OptiScaler against Wine FSR and
  Gamescope scaling, OptiScaler's frame generation against lsfg-vk) are
  reconciled at launch and said on the page.
- **Details** is what is really happening. One reading
  (`bigame_core::overview::Snapshot`) taken off the main thread every 3 s
  while the page is on screen (6 s unfocused), and again when falcond's
  status file or the running game changes, feeds the overview chips, the
  Performance rows (Turbo, falcond and the profile it applied, the power
  profile, the scheduler, 3D V-Cache) and the Video pipeline rows
  (Gamescope, Wine FSR, vkBasalt, lsfg-vk, MangoHud, AI Graphics). Each row
  is one of eight states — active, waiting for a game, configured but not
  detected, configured (not readable), off, missing dependency, not
  supported, error — decided by pure functions in `overview.rs`
  (`feature_state`: configured × installed × game running × detected), so
  *configured* is never shown as *active*. The row's body holds what it
  means, the evidence (which file, which `/proc` entry) and the fix. The
  Problems group classes the health checks and the runtime findings as
  fixable (a command to copy), needs you, hardware or information; hardware
  limits are never errors. Telemetry (1 s), the GPU cards, the network, the
  background load, Steam's broken launch options and the support report
  complete the page. No fix is ever run from it: a fix is a command to copy.
- **falcond's "Proton" profile** is explained wherever it appears: it is
  falcond's general profile for a Proton game without one of its own, not
  the game's.

## Interface themes

The UI has two designs and one colour scheme choice (Settings → Appearance,
kept in `settings.toml` as `theme` and `color_scheme`; `theme.rs`). A new
installation opens in Gamer + Dark: a missing `settings.toml` means a first
run, and a file without the keys keeps its old meaning (Default, the
desktop's scheme), so an existing choice never changes (`settings.rs`,
`Settings::from_file`). The first save writes the values out.

- **Default** is libadwaita plus `style/style.css`, unchanged.
- **Gamer** is `style/gamer.css`, a second stylesheet added at
  `STYLE_PROVIDER_PRIORITY_APPLICATION + 1` and removed to go back, so
  Default never inherits from it. It maps libadwaita's named colours onto
  its own tokens (`--gm-*`: surfaces, text, borders, accents, radii,
  shadows, durations), so stock widgets and `style.css` follow without a
  widget being built differently, then reshapes a few components
  (navigation, cards, primary buttons, switches, progress, the Turbo
  control, the game library).
- Light and dark are media queries in the one Gamer stylesheet. GTK answers
  an application stylesheet's `prefers-color-scheme` from the provider's
  own property, which libadwaita sets only on its stylesheet; `theme.rs`
  sets it from the scheme libadwaita resolved and follows its changes.
- Charts read `bgm_chart_color` (Gamer defines it) before `accent_color`,
  so Default draws them as before.
- High contrast from the desktop shows Default whatever the choice.
- Effects are static: no blur, no looping animation, transitions only on
  hover and state changes, so the theme costs nothing while a game runs.

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
| `$XDG_CONFIG_HOME/bigame-mode/` | `settings.toml` (window, last page, theme; absent on a first run), `video.toml`, `gamescope.toml`, `games/<process>.toml` |
| `$XDG_STATE_HOME/bigame-mode/` | Booster journal, last Turbo report, calibration, benchmark history, profile-migration backups |
| `$XDG_CONFIG_HOME/bigame-mode/graphics-games.toml` | the user's own AI Graphics game list (optional) |
| `$XDG_STATE_HOME/bigame-mode/graphics/<game>/` | AI Graphics manifests and backups |
| `$XDG_STATE_HOME/bigame-mode/graphics-outcomes.json` | AI Graphics benchmark results measured on this machine (local only) |
| `$XDG_CACHE_HOME/bigame-mode/graphics/optiscaler/<version>/` | the downloaded, verified OptiScaler releases |
| `$XDG_CACHE_HOME/bigame-mode/graphics/optiscaler/releases.json` | the stable releases GitHub lists with a checksum, refreshed at most daily |
| `$XDG_CACHE_HOME/bigame-mode/benchmark/` | MangoHud captures of *Measure the difference* |

## Network, logs and the application's own cost

- Network is measured, never tuned: a DNS benchmark with hand-built UDP
  queries (median, p95, jitter, loss), the default-route interface and its
  queue discipline. fq_codel/CAKE, disabling IPv6, jumbo MTU, BBR and NIC
  offload changes are deliberately not applied.
- falcond's status file is watched with inotify, never polled. The game
  watcher reads `/proc` every 5 s without spawning a process. Details reads
  only while it is on screen; its snapshot spawns no process (no
  `gamescope --help`, no `systemctl`).
- The Logs page reads the journal with one `journalctl -o json` call,
  incrementally, only while it is on screen: falcond, the helper, the UI,
  `scx_loader`, power-profiles-daemon, Gamescope, BiGame-mode's Polkit records
  and the kernel's GPU and sched-ext messages.

## falcond behaviours worth knowing (2.0.2)

- Activation applies the profile and deactivation puts the previous state
  back; SIGTERM deactivates before exit; SIGHUP reloads and re-reads
  `user/`. The **power profile it puts back is the one in use when the
  service started**, not the one before the game: on the reference desktop,
  falcond started in balanced, the profile was switched to power-saver, a
  profiled process ran and balanced came back; started in performance, every
  game ended in performance. Turbo off and on again makes falcond take the
  current profile as its baseline. The helper reloads falcond with SIGHUP through systemd,
  never with a restart, which would tear down a running game's profile.
- If falcond is killed with SIGKILL while a profile is active, systemd restarts
  it and the new instance snapshots the boosted state. BiGame-mode does not
  paper over that with a second writer; Details → Problems warns when
  systemd has restarted falcond.
- A user profile with the same name as a shipped profile is not applied.
- New processes are checked at once only when they are `.exe`, a Wine loader or
  already known; others wait for falcond's rescan (9 s).
- Scheduler switching needs `scx-tools` (`scx_loader`); without it every
  switch fails.

## Hardware notes

- **Hybrid graphics.** `hardware::offload_for` decides whether games need
  render offload (the games' GPU drives no output while another does) and by
  which switch: `__NV_PRIME_RENDER_OFFLOAD=1 __GLX_VENDOR_LIBRARY_NAME=nvidia`
  for NVIDIA's driver (`DRI_PRIME=1` there gives zink, OpenGL on NVIDIA's
  Vulkan), `DRI_PRIME=pci-…` for Mesa. DXVK and VKD3D-Proton pick the
  discrete GPU themselves. A native OpenGL game started by the Steam client
  needs `prime-run %command%` in its launch options; the hybrid health check
  says so.
- **Gamescope composites where the display is.** Told to composite on a
  discrete GPU that drives no output (`--prefer-vk-device`), nested Gamescope
  shows no window; without it, it composites on the display's GPU while the
  game inside still renders on the discrete one. BiGame-mode does not pass it.
- **NVIDIA.** Any Vulkan program opens every GPU just to enumerate them, so a
  card a process only enumerated is filtered out with NVML's list of
  processes holding a graphics context. A runtime-suspended discrete GPU is
  reported asleep and never queried, so the panel cannot keep it awake. The
  GTX 1050 Ti Mobile reports no board power. On it, OptiScaler's frame
  generation in Shadow of the Tomb Raider raised Xid 69 and 31; FSR 3.1
  upscaling alone did not.
- **Power.** On BigLinux, `power-profiles-daemon-biglinux-cpufreq` maps
  performance → `performance`, balanced → `schedutil`, power-saver →
  `conservative` on every profile change, which is why the Booster leaves
  the governor to power-profiles-daemon where it runs. The Booster plans
  once, at Turbo on; there is no UPower watch, and a profile's
  `performance_mode` is decided by the power source when it is created.
- **Tested on:** Ryzen 7 5700G with Radeon RX 9060 XT and Radeon Vega
  (kernel 7.2, Mesa 26.2, KDE Plasma Wayland), and a laptop with a Core
  i7-7700HQ, Intel HD 630 and GeForce GTX 1050 Ti (NVIDIA 580), with
  falcond 2.0.2, power-profiles-daemon 0.30, scx 1.1.3, Gamescope 3.16.28,
  MangoHud 0.8.4, vkBasalt 0.3.2.10 and lsfg-vk 1.0.0. Not validated on real
  hardware: RDNA 3, RTX, Intel Arc, battery power, X11, 3D V-Cache, VRR and
  HDR in games, a Vulkan or anti-cheat game with AI Graphics, the presented
  frames of lsfg-vk.
