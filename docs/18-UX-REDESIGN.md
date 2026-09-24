# 18 — UX redesign

**Objective.** A beginner presses Turbo and plays; someone curious can see
what was applied and why; an expert can investigate. Nothing on screen may
claim what did not happen.

## Home

| Before | After |
|---|---|
| "Booster Mode", driven by the Booster journal: on a well-configured machine it said "Already optimal" and could not be turned off, while falcond — not shown — intervened in every game | **Turbo Mode**, the master switch. Its state is falcond's service state as systemd reports it, so it cannot read "off" while falcond runs |
| Three readings, with a full hardware probe every 2 s | The same readings, probed once, refreshed on show, paused while hidden, every 10 s during a game |
| Nothing about the game | A card while a game runs: cover from Steam's local cache (no network), name, time running, runtime · graphics · process, and the profile actually in force — with *Create profile* when there is none |
| — | One summary line from the last transition: "N applied · N per game · N skipped · N conflict avoided" |

Verified on screen (2026-09-24): *Turbo Mode On · Optimizing Shadow of the
Tomb Raider*; card with cover, *00:49:35*, *Proton - Experimental ·
VKD3D-Proton · DX12 · SOTTR.exe*, *No profile of its own yet · using
falcond's general Proton profile*, readings *4.1 GHz · 46 °C · 1000 Mb/s*.

Two defects were found only by looking, and fixed: the cover as a
`GtkPicture` stretched the card to the art's 600×900; the readings showed
"—" for ten seconds into a game.

## Optimization Report

Rebuilt around the Turbo report: *Right now* (read live: profile, process,
graphics, the actual power profile from power-profiles-daemon — not
falcond's ambiguous "Performance Mode: Active" — screen blanking,
scheduler), then *Applied and verified*, *Managed per game*, *Put back*,
*Conflicts avoided*, *Did not take effect*, *Skipped*, *Not available*,
and *Measured on this machine* — the only place speed is claimed. Every
row names its owner and has an ⓘ.

Status: IMPLEMENTED. NOT seen on screen with a real Turbo report: the
reference machine's installed helper predates `SetGameBackend`, so Turbo
could not be switched from the new UI without installing the package.

## Info buttons

`widgets/info.rs`: a symbolic button whose popover explains a row — what
it is, what it changes, who owns it, what was measured. Keyboard-reachable,
labelled for screen readers. Used in the report, Settings, and the offer's
review. NOT yet added to Details, Profiles, Tuning, Video and Benchmark.

## Settings

Removed: Dark Mode and About, which the application menu already has.
Added: start in the background at login; hand falcond back; offer profiles
for new games; fix profiles an older BiGame-mode wrote. VERIFIED on screen.

## Diagnostics

A *System health* group at the top: each check with its status, what was
found, and a fix — a copyable command, or advice. VERIFIED on screen; on the
reference machine it reports the handheld profile set and missing
`scx-tools`, the two problems this audit found. A defect found only by
looking: rows parsed their text as Pango markup, so a fix containing `&&`
rendered as an empty line. Fixed for every row that shows dynamic text.

## Logs

One list, colour-coded by severity label only (palette chosen for light or
dark), filters by severity and source, search, copy, export with personal
data masked. VERIFIED on screen. It opened on the oldest entry until a
scroll was deferred until after layout.

## Navigation

NOT CHANGED. The sidebar still has Home, Details, Profiles, Tuning, Video,
Benchmark, Diagnostics, Logs, Settings. Collapsing Details/Tuning/Video into
one *Performance* page (§25) is a larger rework than was safe to do without
the live validation this pass could not complete, and is the first UX item
for the next one.

## Controls removed because they did nothing

| Control | Why |
|---|---|
| Per-game CPU governor | not a falcond field |
| Custom scheduler flags | not a falcond field |
| Start/stop scripts | refused by the helper (would run as root) |
| "Repair & Enable" | deleted every user profile through `sh -c` |

## Honesty fixes

Profile save always said "saved" and froze the window behind a password
prompt; *Restore Defaults* announced success whatever happened; frame-gen
sliders reloaded falcond mid-game on every move. All fixed.

## Accessibility

State carried in accessible labels for the Turbo control, the info buttons
and the Logs toolbar. NOT TESTED with a screen reader.
