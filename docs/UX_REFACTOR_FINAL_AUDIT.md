# UX restructure — final audit

A last pass over the working tree against the acceptance list, and what
was found.

## Acceptance list

| Item | State | Where |
|---|---|---|
| New install opens in Gamer + Dark | done, tested | `settings.rs` |
| Existing settings are not changed | done, tested (legacy file → Default/System) | `settings.rs` |
| Benchmark is not a category | done | `window.rs::PAGES` test |
| Benchmark infrastructure still works | untouched: `bigame-core/src/benchmark/`, `booster/measure`, `scripts/bench-*.sh`, the `measure` example; Measure the difference in the card menu | — |
| Video is not a category | done | `window.rs` |
| Tuning holds Tuning + Video | done | `views/tuning.rs` |
| Diagnostics is not a category | done | `window.rs` |
| Details holds diagnostics and troubleshooting | done | `views/details/problems.rs`, `extras.rs` |
| Details has no Detected games | done | `views/details/` |
| Profiles centralises the games | done | `views/profiles.rs` |
| Create with Wizard exists | header button and card menu | `views/profiles.rs` |
| Launch (Turbo) where it makes sense | card menu, for a Steam id or a launcher command | `views/profiles.rs` |
| GPU cards match the telemetry look | done | `views/details/gpus.rs` |
| falcond, SCX, V-Cache states are understandable | state + line + body with meaning, evidence, fix | `views/details/performance.rs` |
| Video runtime is not a block of raw flags | done: the dialog and its report are gone | — |
| Configuration and execution are different states | `overview::State`, `feature_state`, tested | `bigame-core/src/overview.rs` |
| Problems have a diagnosis, fixable ones an action or instruction | classed rows, copy buttons | `views/details/problems.rs` |
| Hardware limits are not errors | Unsupported / Hardware are neutral, never counted as attention; tested | `overview.rs`, `widgets/status.rs` |
| No unsafe root action introduced | none: every fix is a copied command; the helper and its Polkit policy are unchanged; `tests/daemon-authorization.sh` passes | — |
| D-Bus / Polkit unchanged | `bigame-daemon` untouched | `git diff --stat` |
| Small window works | 800×600 checked | test report |
| Default / Gamer × Light / Dark work | all four checked | test report |
| gettext updated | `.pot` regenerated, `pt_BR.po` merged and completed, `POTFILES.in` updated | `locale/` |
| fmt, build, test, clippy `-D warnings` pass | yes | test report |
| Authorization test passes | yes | test report |
| README and ARCHITECTURE reflect the new structure | yes | `README.md`, `docs/ARCHITECTURE.md` |

## Findings during the audit

- **Regressions looked for, none found.** Home, Logs, the profile editor,
  the wizard, AI Graphics, the report page, the tray and the error
  indicator are untouched. The Restore Defaults action still rebuilds
  Tuning. The Settings "ping target" text still names Details, which is
  still where the latency card is.
- **Dead code.** `views/benchmark.rs`, `views/dashboard.rs`,
  `views/video.rs`, `views/diagnostics.rs` (moved to `details/extras.rs`
  minus the two groups that moved elsewhere) are gone; the wizard's
  functions, unused for a moment when Details lost its rows, are used
  again by Profiles. Clippy's dead-code pass is clean.
- **Strings.** No page name of a removed category remains in the UI; no
  Portuguese literal in Rust; every removed page's strings left the `.pot`
  (370 obsolete entries dropped from `pt_BR.po` by `msgmerge`).
- **Duplications.** One set of state words (`widgets/status.rs`), one
  launch path (`profiles::launch_game`), one place that reads the machine
  for Details (`overview::Snapshot`), one health list.
- **Evidence.** Every *Active* on Details comes from a read of the system:
  falcond's unit and status file, `/sys/kernel/sched_ext`,
  power-profiles-daemon, the game's process tree, environment and maps,
  OptiScaler's log. Nothing is reported as active because it was
  configured.
- **Buttons without an action.** None: every button on the new pages
  copies, opens, refreshes or switches something.
- **Empty pages.** None.
- **Warnings.** One GTK markup warning was found and fixed during the
  visual pass (`<game>` in Tuning's example command line).
- **Polling.** Details reads only while mapped; its snapshot spawns no
  process (`SchedExtCaps::detect`, `which`); the slow readings run every
  30 s. A future improvement would be to pause the 1 s telemetry while a
  game runs, as Home does.

## Known limitations (proven)

- A Steam game started by Steam is outside the launch pipeline: Gamescope,
  Wine FSR (unless in the session environment) and vkBasalt reach it only
  through Steam's launch options. Details says so in the Gamescope row and
  the Problems list rather than reporting it as a failure of the machine.
- Whether a scheduler was applied *by falcond* is inferred: the kernel
  reports what is loaded, falcond's status what it asked. A scheduler
  loaded by another tool shows as Active with "loaded by someone else"
  semantics (state Active when nobody asked).
- The Wine FSR / Gamescope banner reads Gamescope's render size from the
  group above through a shared cell; a change there is reflected when the
  Upscaling group is next mapped, not the same instant.

## Suggestions for later

- Pause Details' telemetry to the in-game cadence Home uses (10 s) while a
  game runs, since Details is usually behind the game then.
- A "Check again" button in Problems that runs the health checks at once
  (they run on every visit and every 30 s today).
- When lsfg-vk or Gamescope is missing, an *Install* button through the
  distribution's installer (`pamac-installer`, already used by the error
  indicator) instead of only a copied command.
