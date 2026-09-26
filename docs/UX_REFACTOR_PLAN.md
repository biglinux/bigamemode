# UX restructure — audit and plan

Written before the change, from a reading of every page and of where each
value it shows comes from. What was decided here is what the implementation
does; where the implementation departed from it, `UX_REFACTOR_IMPLEMENTATION.md`
says why.

## Navigation as found

| Page | Module | What it held |
|---|---|---|
| Home | `views/home.rs` | Turbo, the running game, the last report's summary, three live tiles |
| Details | `views/dashboard.rs` (1 559 lines) | six telemetry cards; the GPU list; Power profile, Turbo, lsfg-vk rows; four "video runtime" rows (Gamescope, Wine FSR, vkBasalt, frame generation) with a "Run diagnostics" dialog that printed `gamescope_enabled: true` and friends; falcond's active profile, SCX and V-Cache; a three-item Troubleshooting group (SCX, lsfg-vk, V-Cache); **Detected games** with a setup assistant, *Create with Wizard* and *Launch (Turbo)* per game |
| Profiles | `views/profiles.rs` | the poster grid, the profile editor, the card menu (Measure, AI Graphics, Edit, Delete) |
| Tuning | `views/tuning.rs` | falcond's daemon settings, scheduler and mode, the read-only governor, V-Cache, the per-profile lsfg-vk controls, device mode, an Advanced expander (sched-ext availability, Gamescope's accepted options) |
| Video | `views/video.rs` | one expander with Gamescope upscaling, filter, sharpness, render and output size, Wine FSR and its quality, vkBasalt and its config; the global lsfg-vk switch; a "restart Steam" notice |
| Benchmark | `views/benchmark.rs` | which workloads exist (read-only), the calibration findings (read-only), the method (static text). No control ran anything |
| Diagnostics | `views/diagnostics.rs` | System health (from `health::collect`, with copyable fixes), the AI Graphics installs, Background load, broken Steam launch options (one action), Network and the DNS comparison, the support report |
| Logs | `views/logs.rs` | the journal view |
| Settings | `views/settings.rs` | appearance, login start, hand falcond back, profile offer, migration fix, notifications, ping target |

## Duplications and contradictions

- **Games in two places.** Details' *Detected games* and Profiles' library
  list the same titles; Details offered *Create with Wizard* and *Launch
  (Turbo)*, Profiles offered *Optimize* / *Edit* and its own menu. Two lists,
  two refresh buttons, two ideas of what a game's primary action is.
- **Two homes for falcond's global settings.** Tuning held falcond's daemon
  and scheduler; Video held the presentation settings; the per-profile
  lsfg-vk controls sat in Tuning while the global lsfg-vk switch sat in
  Video. Turning frame generation on for a game in Tuning silently flipped
  the Video backend.
- **Two health views.** Details' Troubleshooting (three hard-coded checks:
  SCX, lsfg-vk, V-Cache) next to Diagnostics' System health (`health::collect`,
  fourteen checks with fixes). The three-item list said "System fully
  configured" while Diagnostics could show an error.
- **Configuration shown as execution.** `Video runtime status` badges read
  *Active* from environment variables; the diagnostics dialog printed raw
  flags (`framegen_enabled: true`, `lsfg_active: false`). "V-Cache not
  available" was listed as a *problem* on every CPU without it.
- **falcond's active profile "Proton".** Shown as the profile name with no
  explanation; it is falcond's generic Proton profile applied to a game
  that has none of its own (Home explained it, Details did not).
- **A page with nothing to do.** Benchmark showed what could be measured
  and what was measured, and no control on it measured anything; the one
  measuring action lives in a game's card menu.
- **Labels vs. values.** Details said *Frame Generation (lsfg-vk) · Ready*
  when the layer was merely installed; *Pending* when a feature was enabled
  and no game ran under it.

## Where each state really comes from

| Fact | Source of truth |
|---|---|
| Turbo on/off | `turbo::state_blocking()` — falcond's systemd unit |
| falcond running, active profile, current SCX, current V-Cache, inhibit | `status::read()` — falcond's own status file (root-owned) |
| the profile falcond matched for the running process | `running::matching_profile(process, profile_mode)` |
| power profile | `dbus::power_profile_get()` — power-profiles-daemon |
| scheduler loaded now | `/sys/kernel/sched_ext/root/ops` (`running::in_game`) |
| schedulers switchable | `capabilities::SchedExtCaps::switchable()` |
| 3D V-Cache present / mode | `hardware::detect_vcache()`; falcond's `current_vcache` |
| running game, runtime, graphics API, GPU | `game_watch::current()` → `running::GameIdentity` |
| Gamescope in the game | the game's process tree (`InGame::gamescope`) |
| Wine FSR in the game | `WINE_FULLSCREEN_FSR` in the game's environment |
| vkBasalt in the game | `libvkbasalt` mapped in the game (`InGame::vkbasalt`) |
| lsfg-vk generating | layer mapped **and** an entry with multiplier > 1 (`InGame::frame_generation`) |
| MangoHud in the game | `libMangoHud` mapped (`InGame::mangohud`) |
| AI Graphics in the game | `graphics::status_running()` — OptiScaler's log since the process started |
| what is configured | `video_config::load()`, `fg::*`, the game's profile |
| health checks and fixes | `health::collect()` |

## What moves, what goes, what stays

| Feature | Decision |
|---|---|
| Telemetry cards | stay in Details |
| GPU list | becomes GPU cards with load, clock, VRAM, temperature, power, driver |
| Video runtime rows + raw diagnostics dialog | replaced by the pipeline rows (configured / detected / status / why / fix); the dialog and its report are removed |
| Details → Detected games, setup assistant | **removed**; *Create with Wizard* and *Launch (Turbo)* move to the Profiles card menu and header |
| Details → Troubleshooting (3 checks) | replaced by `health::collect` plus the runtime findings, classified |
| Diagnostics → System health, Network, Background load, Steam launch options, Support report, AI Graphics installs | move into Details |
| Benchmark page | **removed**; the calibration findings show in Details' Performance rows as evidence; the method text moves to the Measure dialog; workload availability is dropped from the UI (the engine keeps it) |
| Video page | merged into Tuning as collapsible groups |
| Tuning → per-profile lsfg-vk controls | stay, inside the Frame generation group next to the global switch |
| Tuning → Advanced | stays, last, collapsed |
| Settings | unchanged in content; the Details ping target explanation updated |
| `views/benchmark.rs`, `views/diagnostics.rs`, `views/video.rs` | deleted; their live parts re-homed |
| `bigame-core/src/benchmark/`, `scripts/bench-*.sh`, `measure` | kept: the Measure dialog, tests and the planner use them |

## New navigation

```text
Home
Profiles
Tuning
Details
Logs
Settings
```

A saved `last_tab` of `video`, `benchmark` or `diagnostics` opens Tuning,
Details and Details respectively.

## Details, rebuilt

One rule for every optimization: *what is it, is it available, is it
configured, is it active, how do we know, what is it doing, why not, how to
fix*. The page is the answer to that, top to bottom:

1. **Overview** — one line ("Ready to play", "Turbo is off", "Waiting for a
   game") and a grid of state chips: Turbo, falcond, profile, power,
   scheduler, GPU, Gamescope, upscaling, frame generation.
2. **Telemetry** — the six cards.
3. **Graphics cards** — one card per GPU: role, load, clock, VRAM,
   temperature, power, driver.
4. **Performance** — Turbo, falcond, power profile, scheduler, 3D V-Cache
   as expander rows: state and one line visible; inside, what it means,
   the evidence, and what to do.
5. **Video pipeline** — the running game's identity (process, PID,
   runtime, graphics API, GPU), a pipeline strip (Game → Proton → Gamescope →
   Upscaling → vkBasalt → Frame generation → Display) drawn from evidence,
   then one expander row per stage with configured / detected / status and,
   when configured but not detected, the reasons and the fix.
6. **Problems** — the health checks and runtime findings, each classed
   *fixable*, *needs you*, *hardware*, *information*, with a copy button
   for commands. Hardware limits are never errors.
7. **Network**, **Background load**, **Steam launch options**, **Support
   report** — from Diagnostics, as they were.

States are one vocabulary (`widgets/status.rs`): Active, Configured (not
verified yet), Available, Waiting, Off, Not detected, Not supported, Missing
dependency, Error. Each has an icon, a label and a colour; none depends on
colour alone.

The data behind the rows is collected in one place off the main thread
(`bigame_core::overview`), so the page renders a snapshot and the decisions
"configured but not detected → attention" are pure functions with tests.

## Theme

New installs open in Gamer + Dark. `settings.toml` absent means first run;
a file without `theme` / `color_scheme` keeps its old meaning (Default,
System), so nobody's choice changes. The first save writes the values
explicitly. Tests cover: no file, a legacy file, Default chosen, Light chosen.

## Risks

- **Details polls.** The state snapshot reads `/proc`, sysfs, falcond's
  status file and one D-Bus property. It runs only while the page is mapped,
  every 3 s focused and 6 s not, and falcond's file is watched, not polled.
- **Long page.** Expanders keep the page short; the groups below the
  pipeline are what Diagnostics already was.
- **Strings.** Every removed page's strings go; the `.pot` is regenerated and
  `pt_BR.po` merged.
- **Profiles' card menu grows** — only entries that apply to the game are
  shown (Launch for games with a launcher entry or a Steam id, Measure only
  for direct commands, AI Graphics only with an install folder, Restore only
  when files are installed).

## Files affected

`bigame-ui/src/window.rs`, `settings.rs`, `theme.rs`, `views/mod.rs`,
`views/dashboard.rs` (replaced by `views/details/`), `views/tuning.rs`,
`views/video.rs` (removed), `views/benchmark.rs` (removed),
`views/diagnostics.rs` (removed), `views/profiles.rs`,
`views/measure_dialog.rs`, `views/settings.rs`, `widgets/tutorial.rs`,
`widgets/status.rs` (new), `widgets/mod.rs`, `style/style.css`,
`style/gamer.css`, `bigame-core/src/overview.rs` (new), `lib.rs`,
`locale/POTFILES.in`, `locale/bigame-mode.pot`, `locale/pt_BR.po`,
`README.md`, `docs/ARCHITECTURE.md`.
