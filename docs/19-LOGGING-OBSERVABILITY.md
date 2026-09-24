# 19 — Logging, observability, and the application's own cost

## Logs page

| Before | After |
|---|---|
| Three `journalctl` processes (one `--grep` over the whole journal) and `dmesg`, every 5 s, whether or not the page was on screen | One `journalctl -o json` call, incremental by cursor, only while the page is visible |
| `dmesg` — usually refused to a normal user (`kernel.dmesg_restrict`) | kernel records from the journal (`_TRANSPORT=kernel`), filtered to DRM/GPU/sched-ext |
| Plain text | Severity from the journal priority, then the message; only the level label coloured |
| — | Filters (severity, falcond, BiGame-mode, helper, kernel/GPU, Gamescope, sched-ext, power profiles), search, copy, export with home/user/host masked |

Sources, all through the journal: falcond, the helper, the UI, `scx_loader`,
power-profiles-daemon, Gamescope, Polkit (only records about BiGame-mode),
and the kernel.

| Check | Status |
|---|---|
| 400 entries in 34 ms, one process; an immediate incremental read returns 0 | MEASURED |
| falcond's scx failures, logged at info priority, shown as ERROR | VERIFIED |
| A cancelled Polkit prompt shown as a Polkit ERROR | VERIFIED |
| Parsing, attribution, filtering, ANSI stripping, redaction | TESTED |
| NVIDIA, Intel kernel lines | NOT TESTED — hardware unavailable |
| DXVK/VKD3D logs | NOT COLLECTED — they go to the game's own files only when enabled |

## Observations the logs surfaced

- falcond, to inhibit the screensaver, runs `sudo busctl --user call
  org.freedesktop.ScreenSaver … Inhibit` as the desktop user — so every game
  start opens a `sudo` PAM session in the journal. falcond's design, not
  BiGame-mode's; noted.
- The old UI had written 7 653 lines of `1585184` (a zombie's PID) in one
  day. They are still in the journal and dominate a 600-line window of the
  last twelve hours until they age out.

## Polling, found and removed

| Loop | Before | After |
|---|---|---|
| Dashboard runtime detection | `pgrep` ×4 + `timeout grep` ×2 per game process, every 1 s (≈ 32 forks/s during a Proton game) | `/proc` read directly, game processes found once per tick |
| Dashboard telemetry (incl. `ping`) | every 1 s, forever | nothing while the page is off screen |
| Home tiles | full hardware probe every 2 s | probed once; paused while hidden; 10 s during a game |
| Tray actions | main loop polled a channel every 250 ms, forever | the tray thread wakes the main loop when an action arrives |
| Tray/indicator status | every 2 s, D-Bus + status file | every 10 s, one cached systemd connection |
| Logs | 4 processes every 5 s, forever | 1 process every 5 s, only while visible |
| Game detection | — (new) | file monitor on falcond's status, plus a 5 s fork-free `/proc` scan (9–10 ms) |
| Default log level | `debug` for BiGame-mode's crates | `info` |

## The application's own cost, measured

Shadow of the Tomb Raider running (menu), 60 s per row, measured from
`/proc/<pid>` of the UI process:

| Build | Window | CPU | Context switches | Own child processes | RSS |
|---|---|---:|---:|---:|---:|
| `main` (as installed) | Home | 7.38 % | 313/s | 3 | 129 MB |
| this branch | Home | **0.77 %** | **9.9/s** | 2 | 130 MB |
| this branch | hidden (tray, `--background`) | **0.52 %** | **3.8/s** | 2 | 84 MB |

`main` cannot start hidden: `--background` is new, and GApplication rejects
the unknown option. The two child processes are GTK's sandboxed image
loaders, long-lived. System-wide process creation was also sampled, but the
sampler itself forked ten times a second per thread, so that figure measures
the instrument and is not reported. The earlier SIGSTOP experiment
([13](13-AAA-BENCHMARKS.md)) found +1.1 % fps with the old UI frozen, not
significant at three runs.
