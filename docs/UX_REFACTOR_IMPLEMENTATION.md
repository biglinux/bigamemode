# UX restructure — what was implemented

The plan is in `UX_REFACTOR_PLAN.md`; this is what the code does now, and
where it departed from the plan.

## Navigation

```text
Home · Profiles · Tuning · Details · Logs · Settings
```

`window.rs` holds the page list (`PAGES`) and `migrate_tab`, which maps a
saved `last_tab` of `video` to Tuning and of `benchmark` or `diagnostics`
to Details. `views/benchmark.rs`, `views/diagnostics.rs` and
`views/video.rs` are gone; `views/dashboard.rs` was replaced by the
`views/details/` module.

## Theme: Gamer + Dark on a first run

`settings.rs`: `Settings::from_file(text)` is the one place a file's text
becomes settings. No file (or one that does not parse) is a first run —
`Settings::first_run()`, Gamer + Dark. A file that exists keeps its old
meaning for a missing key: Default, the desktop's scheme. Nothing else
changed: `theme.rs` still reads `theme` / `color_scheme`, Default is still
the absence of the key, high contrast still shows Default.

Tests (`settings.rs`): no file → Gamer + Dark; a legacy file without the
keys → Default + System, other keys kept; Default chosen, saved and read
back → Default; Light chosen → Light; a first run saved once → Gamer + Dark
again; a file that does not parse → first run, not a legacy file.

## Details (`views/details/`)

One reading, `bigame_core::overview::Snapshot::collect(game)`, taken off
the main thread every 3 s (6 s unfocused) while the page is mapped, and at
once when falcond's status file changes (`GFileMonitor`) or the game watch
reports a change. The snapshot spawns no process. The slower readings —
`health::collect` (package database) and the AI Graphics installs (file
hashes) — run on every visit and every 30 s.

| Module | What it shows |
|---|---|
| `overview.rs` | the headline (`Headline`: ready, Turbo off, optimizing, game without profile, falcond silent, falcond failed) and nine chips |
| `telemetry.rs` | CPU, GPU, GPU temperature, RAM, disk, latency (1 s / 5 s) |
| `gpus.rs` | one card per GPU: role, load, clock, VRAM, temperature, power, driver |
| `performance.rs` | Turbo, falcond (and the profile it applied, explained), power profile, scheduler (asked / loaded / by whom / installed / switchable), 3D V-Cache (not supported is a fact, not a fault); the calibration findings as evidence where they exist |
| `pipeline.rs` | the running game (process, PID, runtime, graphics API, GPU, uptime), the strip (Game → Proton/Wine → Gamescope → Upscaling → vkBasalt → Frame generation → Display), one row per stage with configured / detected / installed and the reasons and fix when configured but not detected; the AI Graphics row lists every game with files installed |
| `problems.rs` | runtime findings (states that need attention) and the health checks, classed Fixable / Needs you / Hardware / Information / OK; commands copied, never run; the passed checks collapsed last |
| `extras.rs` | Network, Background load, Steam launch options, Support report — from the old Diagnostics page, unchanged |

The states are one vocabulary, `overview::State` (Active, Waiting,
NotDetected, Configured, Off, Missing, Unsupported, Error), drawn by
`widgets/status.rs` (chip, status row, fact / note / command rows). The
decisions are pure functions with tests:

- `feature_state(configured, installed, game, detected)`: seen in the game
  → Active (even when BiGame-mode did not ask: Steam's launch options can
  add MangoHud); asked without the software → Missing; not asked → Off; no
  game → Waiting; game and not found → NotDetected; not readable →
  Configured.
- `Scheduler::state`: what was asked (the game's profile, or falcond's
  global setting) against `/sys/kernel/sched_ext/root/ops`, with the
  capabilities deciding Unsupported / Missing first.
- `VCache::state`: a CPU without one is Unsupported, never attention.
- `applied_profile`: falcond's `Proton` is `GenericProton`, explained as
  falcond's general profile, never as the game's.
- `headline` and `Snapshot::attention_count`.

## Tuning (`views/tuning.rs`)

Six groups: System performance (performance mode; the scheduler as an
expander when it can be switched, a *missing* row with the command or a
*not supported* row otherwise; 3D V-Cache likewise; falcond's settings
collapsed: scan interval, profile set, the governor for reference), Display
and Gamescope (an expander with its own enable switch; a *missing* row
without Gamescope), Upscaling and sharpening (Wine FSR and its quality;
vkBasalt as an expander with its switch, or a *missing* row; a note on AI
Graphics), Frame generation (the global lsfg-vk switch first, then the
`Lossless.dll` path and the per-game entries from `widgets/fg_controls.rs`;
a *missing* row without lsfg-vk), Overlay (MangoHud: installed or not, per
game in Profiles), Advanced (collapsed: sched-ext availability, the
environment file, Gamescope's accepted options, the example command line).

Conflict: Wine FSR on while Gamescope is on with a render size shows an
`AdwBanner` — "two upscalers in series, keep one" — with a *Turn Wine FSR
off* button. The other pairs are reconciled at launch (`launcher.rs`) and
said in the groups' notes.

## Profiles (`views/profiles.rs`, `widgets/game_card.rs`)

The card menu shows only what applies: Launch (Turbo) when the game has a
Steam id or a launcher command (`Entry::launch`); Create with Wizard and
Create profile without a profile, Edit profile with one; AI Graphics with
an install folder; Measure the difference for a direct command; Restore the
game's graphics when BiGame-mode placed files (`Entry::ai_installed`, from
the manifest); Delete for the user's own profile. The library header has a
*Create with Wizard* button. The launch code moved here from Details
(`launch_game`), with the honest toast for a Steam game: its profile
applies, the launch settings reach it only through Steam's launch options.

## Removed

- Details' *Detected games*, the setup assistant, *Launch (Turbo)* and
  *Create with Wizard (guided)* rows.
- The runtime diagnostics dialog and its raw report
  (`gamescope_enabled: true` …), *Save diagnostics log*, *Copy diagnostics
  report*.
- The three-item Troubleshooting group.
- The Benchmark page. The method text is in the Measure dialog; the
  calibration findings are evidence in Details' Performance rows; the
  workload availability has no UI (the engine, `scripts/bench-*.sh` and the
  `measure` example keep it).
- The Video page (merged into Tuning) and the Diagnostics page (merged
  into Details).
- Tutorial entries for the removed pages; `views/mod.rs`, `POTFILES.in`.

## Core additions

- `bigame-core/src/overview.rs` (the snapshot and the state functions).
- `running::loaded_scheduler` and `processes::env_has_key` made public;
  `SchedExtCaps::detect()` (sysfs and `/usr/bin` only, no process).
- `SchedExtCaps` derives `PartialEq`.

## Departures from the plan

- The AI Graphics list of installed games lives inside the AI Graphics
  pipeline row rather than in a group of its own: it is diagnostic state,
  and the row already says what the running game does.
- MangoHud has a row in Details' pipeline and an Overlay group in Tuning
  (installed or not; the choice stays per game in Profiles), rather than a
  Monitoring group with controls: there is no global MangoHud setting.
- No separate "real-time diagnosis" mode: the pipeline group *is* the
  live view while a game runs, at the page's own cadence.

## Strings

Every new string goes through `i18n` (UI) or `N_` (core). `POTFILES.in`
lists the new modules; `bigame-mode.pot` was regenerated (1 189 strings)
and `pt_BR.po` merged and completed (1 187 translated).
