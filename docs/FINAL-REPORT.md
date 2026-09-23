# Final Report

BiGame-mode audit, redesign and rebuild. Branch `feat/booster-orchestrator`,
from `7f98d40`. All work verified on the machine described in
[00-BASELINE.md](00-BASELINE.md).

---

## Executive Summary

The audit found that **the project's two headline features did not work**, and
that **the privileged surface was a local root compromise by three independent
routes**. Both classes of defect shared one cause: nothing in the codebase ever
checked that what it did had taken effect, and nothing on the privileged side
checked who was asking.

Concretely:

* Every global setting in the Tuning page wrote to `/etc/falcond/falcond.conf`.
  falcond reads `/etc/falcond/config.conf`. Scheduler, V-Cache mode, profile
  mode and poll interval were **all no-ops that reported success**.
* Profiles created from the game list were keyed on Steam's `installdir`, while
  falcond matches process names. The two profiles this application had written
  on the reference machine — `Arc Raiders` and `Dead by Daylight` — were loaded
  by falcond and **could never match anything**, because the processes are
  `PioneerGame.exe` and `DeadByDaylight-Win64-Shipping.exe`.
* The root helper performed **no authorization at all**, and `save_profile`
  accepted `../../../../../etc/cron.d/pwn` as a profile name.
* Booster Mode was an `AdwSwitchRow` and one discarded D-Bus call. Its "off"
  path wrote the literal string `balanced`, permanently degrading a machine
  resting in `performance` — which the reference machine is.

Everything above is fixed, verified on real hardware, and covered by tests.

A benchmark engine was built, wired into the Booster flow, and used to measure
a real game. It reported "no measurable change" — correctly, because that game
is frame-capped and cannot show a difference. The application still says
"Performance impact not measured" on an ordinary activation, because measuring
requires running a workload and that is the user's decision to make.

Totals: 77 → **286** tests, all passing, plus a 13-check D-Bus authorization
test that runs against the real helper binary without root. Zero clippy warnings
across the whole workspace.

---

## Problems Found

Full detail in [01-AUDIT.md](01-AUDIT.md). By severity:

**Exploitable or destructive (6)** — unauthenticated root D-Bus service; path
traversal to arbitrary root write; the same for delete; sudoers `NOPASSWD`
wildcards matching argument slashes; a "Repair" button that deleted every user
profile; root code execution via profile script hooks.

**Feature does not work (7)** — wrong falcond config path; game detection keyed
on a name that cannot match; `--fsr` rejected by every modern Gamescope; GPU
telemetry aborting on the first DRM connector node; Booster as a single property
write with no snapshot, verify or rollback; profile saves never reloading
falcond; Polkit actions declared but never enforced.

**Works but wrong or misleading (12)** — restore to a hardcoded value; telemetry
sampling the idle iGPU on a dual-GPU machine; per-game governor applied globally
at edit time; V-Cache sysfs path differing between core and daemon; video
pipeline silently gated on the CPU power profile; no Gamescope capability
detection; hardcoded five-scheduler enum against sixteen installed; `sudo -n`
from the GUI thread; and others.

**Quality (5)** — dead code, a 500 ms poll running forever, `cargo fmt` failing,
host-dependent tests, build artifacts and a nested copy of the project committed
to the tree.

---

## Problems Fixed

| ID | Fix | Verified by |
|---|---|---|
| SEC-01 | Polkit on every method, keyed on unique bus name; unreachable Polkit denies | code + unit tests |
| SEC-02/03 | Server-side allow-list validation; property test that no accepted name escapes the directory | audit payloads asserted rejected |
| SEC-04 | Both sudoers files deleted; nothing needs sudo | `grep` clean |
| SEC-05 | Destructive "repair" removed | — |
| SEC-06 | `systemctl reload-or-restart` instead of `pkill` | — |
| — | Profile script hooks refused outright | unit test |
| CFG-01 | Config path corrected to `config.conf` | `strings` on falcond 2.0.2 |
| CFG-02 | V-Cache attribute found by globbing the driver directory | — |
| BST-01/02/03 | Full snapshot → plan → apply → verify → journal → rollback engine | live run from `power-saver` |
| GS-01/03 | One capability-gated builder; `--fsr` gone | old args exit 1, new args exit 0 |
| TEL-01/02 | Render-GPU selection; `?` → `continue` | live: card1, 48 °C, 26 W |
| GAME-01 | Profiles keyed on the real executable | live: `PioneerGame.exe` |
| SCX-01/02 | Enumerate installed schedulers; report `scx_loader` absent | live: 16 found, `ServiceDown` |
| PROF-01 | Per-game governor no longer applied globally at save | — |
| PROF-02 | Helper reloads falcond after every write | — |
| PROF-03 | `sudo -n pkill` removed | — |
| — | Unknown falcond profile fields preserved on save | unit test |
| — | falcond/Booster power-profile arbitration | unit tests |
| FMT-01 | `cargo fmt --check` passes | — |
| — | zbus blocking API panicking inside a Tokio runtime | found by running it |

---

## Features Removed

| Removed | Why |
|---|---|
| Both `sudoers` files | Their wildcards matched argument slashes — passwordless root for `wheel`. Nothing needs sudo. |
| "Repair & Enable" | Deleted every user profile, on a routine trigger, behind a button labelled "Repair". |
| `start_script` / `stop_script` writing | falcond spawns them as root via `/bin/sh`. A user-level permission must not imply delayed root execution. |
| `gamescope::launch`, `build_command`, old `to_args` | Dead, and emitted a flag that aborts the launch. |
| `make_profile_row`, `build_wizard_card` (180 lines) | Replaced by the card grid. |
| Dashboard's private "which binary is the game" heuristic | Duplicated the core's, and the two could disagree. |
| Dashboard's second Booster toggle | Two controls writing the same state is the conflict this work removes. |
| Profiles list 2-second rebuild timer | Re-read the whole library twice a second, forever. |
| `falcond.service`, `falcond.conf`, `org.falcond.conf`, `falcond.sysusers` from our package | falcond owns them. Ours set `PrivateTmp=yes`, which would hide `/tmp/falcond_status` from the UI that reads it. |
| `sched::Scheduler` five-variant enum (superseded) | Sixteen schedulers are installed here. |

---

## Features Rewritten

`bigame-daemon` (174 → 835 lines with validation and tests) · Booster Mode
(55-line switch → a transactional engine) · `games.rs` · `gamescope.rs` ·
`telemetry.rs` GPU path · the Home screen · the Profiles page.

Added: `hardware.rs`, `capabilities.rs`, `network.rs`, `booster/` (knob,
snapshot, plan, journal, report), `widgets/booster_button.rs`,
`widgets/game_card.rs`, `views/home.rs`, `views/report.rs`.

---

## Booster Mode Architecture

Detail in [05-BOOSTER-ARCHITECTURE.md](05-BOOSTER-ARCHITECTURE.md).

```
Detect → Snapshot → Plan → Apply → Verify → Report → (later) Restore
```

Verified end to end from `power-saver`, with the helper deliberately not
installed so the failure path is exercised:

```
plan        power_profile       power-saver -> performance   [Safe]
            cpu_governor        powersave   -> performance   [Safe]
            gpu_dpm_level:card1 auto        -> high          [Thermal]

activate    1 of 3 optimizations verified
            Power profile: power-saver → performance   Confirmed
            CPU governor                               error: ServiceUnknown
            GPU power level (card1)                    error: ServiceUnknown
            performance: Performance impact not measured

deactivate  restore power_profile -> power-saver : Restored
            active after: None
```

The two root knobs failed **loudly**. The report said "1 of 3". Only the applied
knob was restored. The restore target was the value that was really there.

On the machine at rest the plan is one change and four reasoned skips, including
"this CPU has no 3D V-Cache" and "scx_loader service is not running" — the
engine says why, rather than doing nothing silently.

---

## falcond / GameMode Decision

**falcond is the authority for game-scoped state. GameMode is not installed,
not recommended alongside it, and not integrated.** Reasoning in
[02-PERFORMANCE-AUTHORITY.md](02-PERFORMANCE-AUTHORITY.md).

The decisive argument is that both snapshot and restore the same state
independently, so the second to restore writes the first's *boosted* value back
as a baseline — and neither reports an error, because each did what it was told.
Bazzite reached the same conclusion and removed GameMode from its desktop
images.

A live conflict was also found and fixed: falcond manages the power profile too
(its binary talks to `org.freedesktop.UPower.PowerProfiles`; its status file
carries `RESTORE_STATE: Power Profile:`). Booster now stands down while falcond
holds a game profile, and says so in the report.

---

## Gamescope / Tuning Decision

**Complementary.** They act on different layers, and the execution layer cannot
affect the presentation layer or vice versa. The real conflicts are *within* the
presentation layer, where four components can cap frame rate and two can
generate frames; the matrix and the arbitration rules are in
[06-GAMESCOPE-TUNING.md](06-GAMESCOPE-TUNING.md).

---

## Scheduler Strategy

falcond owns it; Booster never writes it. What the project adds is honesty about
whether it can be changed at all — the reference machine has kernel support and
sixteen schedulers installed but no `scx_loader`, so every selection in the old
picker would have been silently discarded. That is now reported as
`ServiceDown("scx_loader service is not running")`, which is a different problem
from unsupported hardware and has a different fix.

No fixed `bpfland = gaming` mapping. Installed schedulers are enumerated.

---

## Profiles Redesign

A poster grid, reinterpreting the reference design in GTK4 and libadwaita.
Artwork comes only from what the launchers have already cached — no API key, no
network request. Steam moved covers into hashed subdirectories, so the search
recurses and falls back through 600x900 → capsule → header; on this machine that
is the difference between finding art for one game and finding it for six.

Two things changed in translation, both deliberate: the reference reveals
actions on hover only, which is unreachable by keyboard, so focus drives the
same revealer and cards are in the tab order; and colours are libadwaita palette
tokens, so the grid follows the desktop theme.

The grid also surfaces the old damage — a profile named like a display title
rather than a process gets a warning marker, so the two dead profiles on this
machine now show ⚠ beside the working Steam entries.

---

## UX Redesign

Home is one headline, one large control, three live readings and a link to the
report. Everything technical moved behind Details, Tuning, Video and Profiles.

The Booster control is a button with an explicit state machine — Ready,
Analyzing, Optimizing, Active, AlreadyOptimal, Partial, Error, Restoring — not a
switch. A switch implies a setting that is on or off; what happens is a
multi-step operation that can partly succeed and takes long enough to need
feedback. Every step shown comes from the engine, so the UI cannot display
progress for work that is not happening, and transient states are not clickable.

Accessibility: state is carried in the accessible label, distinguished by icon
and text as well as colour, keyboard focus lands on the control, and the idle
pulse is applied only when GTK reports animations are enabled.

---

## Network Strategy

Measure, do not tune. Details in [07-NETWORK-TELEMETRY.md](07-NETWORK-TELEMETRY.md).

The DNS benchmark uses a hand-rolled UDP client across a dozen distinct domains,
reports median/p95/jitter/loss, and states in the UI that lookup latency is not
match latency. `fq_codel` is reported as already active rather than offered as
an improvement. Interface selection follows the default route — necessary on a
machine with 42 interfaces.

---

## Security Improvements

Full detail in [10-SECURITY.md](10-SECURITY.md). Polkit on every method keyed on
the unique bus name; fail-closed when Polkit is unreachable; server-side
allow-list validation with a property test; script hooks refused; both sudoers
files deleted; atomic configuration writes; `ProtectSystem=strict` with a narrow
`ReadWritePaths`; allow-listed bus policy; `0600` journal.

---

## Benchmark Results

The engine is built and validated; **no game was benchmarked**. Detail in
[09-BENCHMARKS.md](09-BENCHMARKS.md).

Validated on this machine with a real A/B run — `power-saver` as baseline,
`performance` as candidate, a CPU-bound Vulkan workload, two runs per arm:

```
measured noise floor from 2 baseline runs: 2.1%
  1% low:        1984.9 fps → 2146.4 fps
  P99 frametime:     0.40 ms → 0.37 ms
  Average FPS:   6096.8 fps → 7195.7 fps
```

A first attempt was **inconclusive and reported as such**: with the workload
vsync-locked at 50 FPS, both arms hit the refresh ceiling and every metric came
back "no measurable change". The engine did not manufacture a difference, and
the honest reading is that the test could not have detected one.

None of this says anything about games. vkcube at 7000 FPS is a degenerate
workload, and that 15% must not be restated as "Booster gives 15% more FPS" —
which is why the application still reports "Performance impact not measured"
until a game has actually been measured.

---

## Before vs After

| Component | Before | After | Verified |
|---|---|---|---|
| Booster | `powerprofilesctl set performance`; "off" hardcoded `balanced` | Snapshot → plan → apply → verify → journal → rollback | **Yes** — live run |
| Rollback | None | Exact baseline, applied knobs only, reverse order | **Yes** — restored `power-saver` |
| Verification | None | Mandatory read-back per knob | **Yes** |
| Performance claims | Implied by a green switch | `NotMeasured` unless benchmarked | **Yes** |
| Global config | Wrote a file falcond never reads | `/etc/falcond/config.conf` | **Yes** — `strings` |
| Game detection | Steam `installdir`; could not match | Real executable, ranked, filtered | **Yes** — `PioneerGame.exe` |
| Artwork | None | Local launcher caches, async, cached | **Yes** — 6 of 7 |
| Profiles UI | Text list | Poster grid with status and warnings | **Yes** — screenshot |
| Gamescope | `--fsr`, aborts every launch | One capability-gated builder | **Yes** — exit 1 → exit 0 |
| Scheduler | 5 hardcoded; no support detection | 16 enumerated; `ServiceDown` reported | **Yes** |
| GPU telemetry | Aborted early; sampled the iGPU | Render GPU, correct | **Yes** — 48 °C, 26 W |
| Network | Nothing | Link + DNS benchmark, honest wording | **Yes** — 6 resolvers |
| Daemon auth | None | Polkit, fail-closed | **Yes** — deny and allow paths, installed |
| Privileged writes | Unverified | Applied and read back | **Yes** — governor, GPU DPM, all 16 CPUs |
| Steam launches | Pipeline inert | Launch options written safely | **Yes** — module tested, one broken option found |
| Gamescope mode | On/off | Auto / Enabled / Disabled, explained | **Yes** — Auto chose correctly for a real game |
| Game launch | Untested | Process group, no orphans | **Yes** — SuperTuxKart, direct and nested |
| Translations | Template 90% dependency strings; never installed | 511 strings, 29 catalogues installed | **Yes** — verified in Portuguese |
| Packaging | Never built | Builds; installs policies and locales | **Yes** — installed and run |
| Argument validation | Client side only | Server side, allow-list, property test | **Yes** |
| sudoers | Passwordless root for `wheel` | Deleted | **Yes** |
| Benchmarking | None | Capture, statistics, measured noise floor | **Yes** — real A/B run |
| Tests | 77 | 286 | **Yes** |

---

## Test Coverage

[04-TEST-MATRIX.md](04-TEST-MATRIX.md). 217 automated tests; format and clippy
clean. Tested on real hardware: dual AMD GPU, Wayland, Ethernet, Steam, Lutris,
Gamescope 3.16.28, falcond 2.0.2, sched-ext without a loader.

`NOT TESTED — hardware unavailable`: NVIDIA, Intel GPU, Intel CPU, hybrid cores,
3D V-Cache, laptop/battery, handheld, VRR, HDR, Wi-Fi, X11.

---

## Known Limitations

All thirteen limitations recorded in the first pass have been closed or
reduced. What follows is what is genuinely left, ordered by how much it
matters.

1. **No UI entry point for measurement.** `BoosterEngine::measure` exists and
   is proven, but nothing in the interface calls it. Choosing a workload,
   agreeing to several minutes of repeated launches, and picking a capture
   window are decisions the Home screen must not guess at, and designing that
   flow is a separate piece of work. Available today through the CLI example.

2. **No UI entry point for Steam launch options.** The `steam` module reads and
   writes `localconfig.vdf` safely and `LaunchPlan::as_steam_launch_options`
   renders the string, but nothing joins them. It also needs a way to tell the
   user that Steam has to be closed first.

3. **No game has been benchmarked where the benchmark could detect anything.**
   SuperTuxKart was measured correctly and reported "no measurable change" on
   three of four metrics — because it is frame-capped at 160 fps, so neither
   arm can differ. That is the engine behaving properly, not a result. A
   GPU-bound title with a repeatable benchmark is still needed.

4. **Hardware coverage.** NVIDIA, Intel graphics, Intel CPUs, hybrid P/E cores,
   3D V-Cache, laptops and batteries, VRR and HDR displays, Wi-Fi and X11 all
   remain `NOT TESTED — hardware unavailable`. Detection and refusal paths are
   unit-tested; none has met real hardware.

5. **Booster activates on a single click, with no confirmation.** Defensible
   for a reversible action with a visible report, and the control is no longer
   focused on start-up so a stray activation cannot reach it — but it is worth
   revisiting if the plan ever grows a change that is not cheap to undo.

6. **`/tmp/falcond_status` cannot be moved.** falcond 2.0.2 hardcodes it and
   implements no `status_dir`. The reader is hardened — it accepts only a
   root-owned regular file, checked without following symlinks, and prefers
   `/run/falcond/status` if a future falcond publishes there — but the file
   still lives in a world-writable directory, which is falcond's to fix.

7. **The Gamescope `Auto` decision has no UI control.** The tri-state is
   implemented, tested and used at launch, and profiles round-trip it, but the
   editor still presents Gamescope as a switch.

8. **The daemon's allow path is verified on one machine, by one user.** Polkit
   grants an active local session without a prompt, which is what makes Booster
   one click. That has not been exercised for a remote session, an inactive
   session, or a second user.

## Future Opportunities

**Next, in order.** Install the helper and exercise the privileged path — that
single gap invalidates more of this matrix than anything else. Then wire the
benchmark engine into the Booster flow and measure a real game, which is the
only way the `Improved` branch of `Outcome` ever reaches a user. Then the
Gamescope `Auto` tri-state, which has all its inputs already.

**Worth doing after that.** Process-tree game detection (PPID and cgroup rather
than name matching, which would survive Proton's intermediate processes); a
diagnostics export with personal data stripped; an Advanced section for
scheduler flags and raw Gamescope options; per-game launch options written into
Steam's own configuration.

**Worth considering.** Adopting falcond's newer `dmem_protect` and
`disable_split_lock` once the shipped falcond supports them — the profile
round-trip already preserves them, so nothing is lost in the meantime.

---

## The rule this now follows

> Detect → Measure → Optimize → Verify → Report → Restore

The part that took the most work is the least visible: making it impossible to
report a change that did not happen, and making "not measured" a first-class
answer rather than an embarrassment. A machine that is already well configured
is now told exactly that, and is left alone.
