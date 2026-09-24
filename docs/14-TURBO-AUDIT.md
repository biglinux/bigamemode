# 14 — Turbo audit: what does it actually do?

**Objective.** Answer, with evidence, whether Turbo (the Booster control on
Home) does anything, and who does the work when a game runs.

**Reference machine.** Ryzen 7 5700G (`amd-pstate-epp`, no 3D V-Cache),
Radeon RX 9060 XT, KDE Plasma Wayland, falcond 2.0.2-2, falcond-profiles
r23, power-profiles-daemon, no `scx_loader`, no GameMode. Resting state:
power profile `performance`, governor `performance`, EPP `performance`,
GPU DPM `auto`.

**Method.** State read from sysfs, power-profiles-daemon, `/tmp/falcond_status`
and falcond's own journal; falcond's behaviour read from its 2.0.2 source
(`src/daemon.zig`, `config.zig`, `matcher.zig`) and then **tested** on the lab
VM with the same binary and profiles, using a copy of `sleep` named `cs2` as a
stand-in game that the upstream `cs2` profile matches by name.

---

## Answer

**On this machine, before this work, neither Turbo nor falcond did anything
that affects performance.**

- **Turbo** runs the Booster planner, which only touches global knobs (power
  profile, governor, EPP, GPU DPM, V-Cache). With the machine resting at
  `performance` and GPU DPM refused by calibration (−8.0 % measured), the plan
  is empty. Pressing Turbo wrote nothing.
- **falcond** runs independently of Turbo — it is a separate system service,
  always on — and applied a profile to every Proton game. But it was running
  the **handheld** profile set on a desktop (`profile_mode = handheld`), whose
  generic `Proton` profile is `scx=lavd mode=power perf=false vcache=cache
  inhibit=true`. Of those five settings, one took effect.

## The flow, stage by stage

| Stage | Requested | Owner | Expected | Observed | Status |
|---|---|---|---|---|---|
| Turbo OFF, idle | nothing | — | resting state | resting state | VERIFIED |
| Turbo ON, no game | Booster activate | Booster | plan applied and verified | **empty plan**: power profile and governor already `performance`; GPU DPM `MeasuredHarmful`; V-Cache unsupported; scx `ServiceDown` | VERIFIED (does nothing) |
| Game starts (any state of Turbo) | — | falcond | profile activated | journal: `activating profile 'Proton' (scx=lavd, mode=power, perf=false, vcache=cache, inhibit=true)` | VERIFIED |
| └ scheduler | `lavd` / `power` | falcond | scheduler switched | `D-Bus error: ServiceUnknown` → `failed to switch scx scheduler` — no scx_loader | **FAILED** (silently, in falcond's log only) |
| └ power profile | none (`perf=false`) | falcond | untouched | untouched | VERIFIED (does nothing by design) |
| └ V-Cache | `cache` | falcond | mode written | no `amd_x3d_vcache` on this CPU | UNSUPPORTED |
| └ idle inhibit | on | falcond | screensaver inhibited | `Screensaver Inhibit: Active` in status | VERIFIED — **the only effective action** |
| Game exits | — | falcond | profile deactivated, snapshot restored | `last pid … exited, grace period started` → `deactivating profile 'Proton'` 3 s later | VERIFIED |
| Turbo OFF again | Booster deactivate | Booster | journal restored | nothing to restore (nothing was applied) | VERIFIED |

**Turbo has no effect on falcond.** Turning Turbo off does not stop falcond
from applying profiles; turning it on does not change what falcond applies.
They are two unrelated controls, and the one labelled as the product's main
feature was the inert one.

Every Proton game launched on 2026-09-23 got exactly that handheld profile
(activations at 19:20, 19:50, 19:52, 20:13, 23:46, 00:34, each with the same
scx failure).

### An observation I first got wrong

While writing this, a check reported SotTR as running during a period when the
journal shows falcond deactivating and reactivating its profile, which looked
like falcond losing a live game. It was not: the check was `pgrep -f`
matching my own shell, whose command line contained the pattern — the same
self-match the UI was fixed for. The journal shows a new SotTR PID at each
activation; each deactivation followed a real exit. falcond was correct.

---

## What falcond 2.0.2 actually does (from source, then tested)

| Fact | Source | Test on the VM |
|---|---|---|
| Profiles load from `profiles/` for mode `none`, `profiles/handheld/` or `profiles/htpc/` otherwise, plus `profiles/user/` | `config.zig: profilesDirForMode`, `daemon.zig: loadUserProfiles` | `LOADED_PROFILES: 11` with mode `none` (vs 8 = 6 handheld + 2 user here) |
| A profile activation snapshots power profile, scx scheduler and V-Cache, then applies `performance_mode`, `scx_sched`, `vcache_mode`, `idle_inhibit`, `start_script` | `daemon.zig: activateProfile` | **T1**: `balanced` → game starts → `performance` → game exits → `balanced`. VERIFIED |
| SIGTERM deactivates the active profile before exit | `daemon.zig: deinit` | **T2**: game running, `systemctl stop falcond` → `balanced` immediately; starting falcond again with the game still running re-applies it. VERIFIED |
| `enable_performance_mode` decides whether falcond connects to power-profiles-daemon **at start-up only**; `reload()` does not reconnect | `daemon.zig: init` vs `reload` | **T3**: set to `false` and reloaded without restart → game still switched to `performance`; only after a restart did it stop. VERIFIED |
| Config changes are picked up by inotify; SIGHUP also reloads, re-reading `user/` profiles | `event_loop.zig`, `daemon.zig` | reload logged: `reloaded 11 profiles` |
| The generic `Proton` profile matches any `.exe` not in `system.conf`'s `system_processes`, under a proton/wine/reaper parent; a specific profile supersedes it | `matcher.zig`, `daemon.zig: activateProfile` | journal: `matched pid=… name='SOTTR.exe' profile='Proton'` |
| Process discovery is event-driven (netlink proc connector) with a `/proc` rescan | `event_loop.zig`, `scanner.zig` | — |

Two consequences for the design:

1. **`enable_performance_mode` cannot be Turbo's switch.** Flipping it does
   nothing until falcond restarts (T3), so it would be a switch whose effect
   depends on something else happening later.
2. **Stopping the service is a clean restore** (T2), and systemd holds the
   state: it survives a BiGame-mode crash, and enablement survives a reboot.

## Other defects found on the way

| Defect | Evidence |
|---|---|
| `profile_mode = handheld` on a desktop, so every game got power-saving handheld profiles | `/etc/falcond/config.conf`; status `Profile Mode: handheld`; journal `mode=power, perf=false` |
| Two user profiles written by an old BiGame-mode, keyed on display names falcond can never match (`Arc Raiders`, `Dead by Daylight`), carrying fields falcond does not define (`fg_multiplier`, `fg_flow_scale`, `fg_perf_mode`) | `/usr/share/falcond/profiles/user/*.conf` |
| Saving a profile **restarts** falcond (`systemctl reload-or-restart` on a unit with no `ExecReload`), tearing down a running game's profile; falcond supports SIGHUP reload | `bigame-daemon/src/main.rs: reload_falcond`, `systemctl cat falcond` |
| Booster and falcond both write the power profile, and on `amd-pstate-epp` the power profile is what sets EPP, which Booster also writes: two writers of the same state | `plan.rs: consider_power_profile`, `consider_cpu_epp`; falcond `activateProfile` |
| A skip message contains a run of spaces — a string continuation missing its backslash | `plan.rs`, visible in `booster_dryrun`: `will                      restore` |
| Running `falcond --help` starts a second daemon in the foreground rather than printing help | observed; killed at once, the real daemon unaffected |

## Cost

falcond: event-driven, no polling loop beyond a 9 s rescan. Booster: runs only
when pressed. The UI's own polling was measured separately and fixed
([13-AAA-BENCHMARKS.md](13-AAA-BENCHMARKS.md), "The product's own overhead").

## Measured effect

Nothing to measure for Turbo on this machine: it changed nothing. For
falcond's contribution, the relevant measurements already exist — the
performance power profile against balanced made no measurable difference in
*Shadow of the Tomb Raider*, GPU-bound or CPU-bound
([13-AAA-BENCHMARKS.md](13-AAA-BENCHMARKS.md)) — and the scheduler it tried to
set could not be set here.
