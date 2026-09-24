# 15 — Dynamic profiles: created when the game is found

**Objective.** Stop shipping arbitrary profiles; instead notice the game the
user actually plays, identify its real process, and offer a profile built for
this machine — verified once created.

## The flow

```text
Turbo on
  → the application-wide game watcher notices a game
      (file monitor on falcond's status + a 5 s fork-free /proc scan)
  → running::detect identifies it: GameIdentity
  → matching_profile asks what falcond would apply (name field, mode dir + user/)
  → no specific profile, offer not declined, not offered this session
  → desktop notification: "Create profile" · "Don't ask again" · click to review
  → create: recommend::recommend → SaveProfile (falcond fields only)
  → verify: falcond's status reports the new profile active for the running game
```

## GameIdentity — `bigame-core/src/running.rs`

| Field | Source |
|---|---|
| `process_name` | basename of `argv[0]`, split on `/` **and** `\` — falcond's own rule (`scanner.zig: getProcessName`) |
| `pid`, `tree` | the Steam reaper's descendants (`reaper SteamLaunch AppId=N`) |
| `steam_app_id` | the reaper's `AppId=` |
| `display_name`, `install_path` | `appmanifest_N.acf` in the library that holds it |
| `compatdata_path` | beside the manifest — not the first `compatdata/N` found (a stale one exists in the home library) |
| `runtime` | the `proton` script's directory (`Proton - Experimental`), native, or Wine |
| `graphics` | mapped DLLs: `d3d12.dll`/`d3d12core.dll` → VKD3D-Proton, `d3d11.dll` etc. → DXVK, `wined3d.dll`; native Vulkan/OpenGL only without Windows DLLs |
| `render_card` | the open `/dev/dri/renderD*` node, mapped to its `cardN` |

The game is the busiest process in the tree that is not machinery — the
container (`srt-bwrap`, `pv-adverb`), the `proton` script, `wineserver`,
Wine's services (`services.exe`, `explorer.exe`, …), crash handlers,
launchers, anti-cheat helpers. In a Proton tree a Windows binary is preferred,
so a Linux helper cannot outrank a game that is still loading.

| Check | Status |
|---|---|
| Shadow of the Tomb Raider, live: `SOTTR.exe`, `Proton - Experimental`, VKD3D-Proton, `card1`, live prefix | VERIFIED |
| Detection cost | MEASURED — 9–10 ms, no forks |
| Tree from the real SotTR session, machinery out-burning a loading game, launcher-only tree, native Steam game, Wine outside Steam | TESTED (unit, trees copied from the real machine) |
| `CrashBandicoot.exe` is not taken for a crash handler | TESTED |
| Cyberpunk 2077, Rise of the Tomb Raider live | NOT TESTED |
| Lutris / Heroic live | NOT TESTED |

**A bug found on the way:** the first graphics check looked for
`vkd3d-proton/` in the mapped paths and reported *OpenGL* for a DX12 game.
Under Proton the layers are mapped from the prefix's `system32` under their
Windows names, and `libGL` is always mapped by Wine's display driver. Fixed
and tested against the real mapping.

## Does it already have a profile?

falcond matches a profile's **`name` field**, and loads the directory for its
`profile_mode` plus `user/`. The old `profiles::list_names()` compared **file
names** and ignored the mode directory, so it could not answer this question.
`running::matching_profile` asks it the way falcond does: exact, then
case-insensitive, generic `Proton` excluded. TESTED.

## The recommendation — `bigame-core/src/recommend.rs`

| Key | Value on the reference machine | Evidence |
|---|---|---|
| `name` | the process | Fact |
| `performance_mode` | `true` on AC, `false` on battery | falcond default — explicitly *not* a speed claim: measured no faster than balanced here |
| `scx_sched` | `none` | Unsupported — `scx_loader` not running |
| `vcache_mode` | `none` (`cache` only with a V-Cache CPU) | Unsupported |
| `idle_inhibit` | `true` | Capability only — keeps the screen on with a controller |

Only falcond's six fields are written. falcond's default for an omitted
`vcache_mode` is `cache`, so it is always stated.

## The offer — `bigame-ui/src/profile_offer.rs`

A notification, not a dialog: the game is usually fullscreen, often under
Gamescope, and a window that takes focus mid-play is worse than no offer. The
notification waits in the desktop's queue on Wayland and X11 alike; its body
opens a review listing every value, its evidence and its reason, with
*Create profile*, *Use general optimization*, *Not now* and *Don't ask again
for this game*.

| Check | Status |
|---|---|
| Offer, review, create and verify | IMPLEMENTED — compiles, tested units; NOT TESTED live (needs the new package installed and a Polkit approval) |
| Wayland (KDE) notification path | NOT TESTED live |
| X11 | NOT TESTED — X11 session unavailable |
| Gamescope session | NOT TESTED |

## Old profiles — `bigame-core/src/migration.rs`

Inventory on the reference machine:

| Location | Owner | Profiles | Action |
|---|---|---|---|
| `profiles/*.conf` | `falcond-profiles` r23 | 9 | never touched |
| `profiles/handheld/` | `falcond-profiles` | 6 | never touched |
| `profiles/htpc/` | `falcond-profiles` | 7 | never touched |
| `profiles/user/Arc Raiders.conf` | an older BiGame-mode | 1 | re-key → `PioneerGame.exe` |
| `profiles/user/Dead by Daylight.conf` | an older BiGame-mode | 1 | re-key → `DeadByDaylight-Win64-Shipping.exe` |

A user profile is recognised as ours by fields falcond does not define. The
plan re-keys what resolves to an installed game's executable, cleans what is
already process-keyed, and reports — never deletes — what it cannot resolve.
Every file is copied to `~/.local/state/bigame-mode/profiles-<time>/` first.

| Check | Status |
|---|---|
| Plan on the real directory | VERIFIED (dry run, shown above) |
| Backup, re-key, clean, unresolved | TESTED (unit) |
| Applied on the real machine | NOT DONE — needs a Polkit approval; available in Settings → Game profiles |

## Placebo controls found and removed

falcond 2.0.2 reads exactly `name`, `performance_mode`, `scx_sched`,
`scx_sched_props`, `vcache_mode`, `start_script`, `stop_script`,
`idle_inhibit` (`profiles.zig: ActivationData`).

| Control | Why it did nothing | Now |
|---|---|---|
| Per-game CPU governor | not a falcond field | removed |
| Custom scheduler flags | not a falcond field | removed |
| "Enabled" flag | not a falcond field — a "disabled" profile stayed active | dropped on migration |
| Start/stop scripts | the helper refuses them (they would run as root) | removed |
| Frame-generation sliders saving into the falcond profile | fields falcond ignores; each move reloaded falcond mid-game | writes lsfg-vk's config only |
