# 33 — Installing from GitHub on the lab VM, and SuperTuxKart

The lab VM (`192.168.132.87`, BigLinux on Manjaro, kernel 6.18, KDE Wayland,
virtio GPU without virgl — software rendering, no cpufreq) on 2026-09-24,
right after PR #4. It is now installed to disk; the stock repositories
(`biglinux-stable`, `core`, `extra`, `multilib`) are the only ones enabled.
The install followed the README as a new user would. Behaviour only: nothing
measured here is a performance result. No host benchmark ran during this pass.

Earlier VM sessions had left a hand-installed falcond and helper, owned by
no package. Both were backed up (`~/bigame-test/*-manual-backup.tar`) and
replaced by the packages. A pre-1.0 helper that the bus had started outside
its unit was still running.

## What failed, and the fix

| Found | Cause | Fix |
|---|---|---|
| `makepkg -si` stops: *falcond: target not found* | falcond and lsfg-vk come from the BigCommunity `community-extra` repository, which a stock install does not enable | README: the repository, its key (on keyserver.ubuntu.com) and `base-devel` come before `makepkg` |
| `check()` aborts the build: 3 launcher tests fail | the tests asked the real machine for Gamescope and a graphical session; over ssh there is no session. Any clean build (server, chroot) would fail the same way | `LaunchPlan` takes a `Host`: real launches detect it, tests describe it. Two tests cover "Gamescope absent" and "no session" |
| SuperTuxKart running, Turbo on: Home says *waiting for games*, no profile offered | detection knew Steam trees and Wine `.exe` only | menu entries in the `Game` category, and names falcond has profiles for, are native games. Launchers, streaming, tools and BiGame-mode itself are excluded |
| The SuperTuxKart profile card has no *Measure the difference* | the library held Steam, Lutris and Heroic only | menu games are in the library (source *Native*), launched with their `Exec` |
| A profile made from Home is missing in Profiles until ↻ | the list refreshed on navigation within the page only | it refreshes whenever it comes on screen |
| Details: *Modo Turbo: Inativo — Ative o Modo Booster* with Turbo on | read "power profile is performance" as Turbo | reads `turbo::state`, as Home does; presentation rows no longer say they need Turbo (the launcher applies them regardless) |
| *Measure* promises 6 launches, then *Measurement failed: nothing to measure* | the Booster plan is empty on a VM, and that was found only after starting | the dialog checks the plan first and says there is nothing to compare |
| SuperTuxKart READY, then BLOCKED after its first launch, with no remedy | STK writes vsync on and `max_fps=120` the first time it runs; the limit is in no menu. The page read availability once | the reason names both keys and the file; availability is re-read whenever the page is shown |
| Logs: `INFO turbo: … failed=0` listed as an error; lines read `INFO helper INFO bigame_daemon:` | level guessed from wording | a tracing line's own level decides, and is stripped from the text |
| `journalctl -u bigame-daemon` shows `[2m…[32m INFO` and two timestamps | colour and time written into the journal | plain, untimed output when not on a terminal |
| Old helper still running beside the new one after an upgrade | pre-1.0 activation had no `SystemdService`; `try-restart` does not reach a bus-spawned instance | upgrade and removal stop `dbus-*com.biglinux.BiGameMode@*.service` |
| GTK: `app.profile-review … parameter type mismatch` ×6 per update; Tuning's example command line blank | button had no target until a game appeared; `<game>` parsed as markup | empty target, ignored by the action; placeholder escaped |
| First start opens on Details | default last tab | Home |
| Notifications signed *bigame-ui* | no application name set | `BiGame-mode` |
| In pt_BR, whole pages in English | 303 strings never translated | 810/810 |

## What worked

- **Install:** `pacman -Qkk` reports 126 files, 0 altered. The helper starts on demand through `bigame-daemon.service`, with `ProtectSystem=strict` and `NoNewPrivileges`.
- **Upgrade:** the helper is restarted (new PID).
- **Authorisation:**
  - From the ssh session (not the active one), `SetGameBackend` and `SaveProfile` are refused by Polkit, and falcond is untouched.
  - As root, every malicious argument is refused with a reason: `../../etc/evil`, a name mismatch, `performance; rm -rf /` as a governor, `../../../etc` as a DRM card, V-Cache on a CPU without it.
- **Turbo:**
  - From the UI in the KDE session it needs no password (`allow_active`). falcond is enabled and started, and the prior state is recorded (disabled, inactive).
  - Turbo off stops and disables falcond.
- **SuperTuxKart, end to end:**
  1. The game is detected; Home shows *Nativo · OpenGL · supertuxkart*.
  2. *Create profile* shows the review, then the KDE Polkit prompt in pt_BR (*É necessário autenticar-se para alterar os perfis de jogos*).
  3. The profile is written and falcond activates it: `perf=true`, screensaver inhibited. The notification says *Perfil criado e ativo*.
  4. When the game quits, falcond's grace period runs, then it deactivates the profile and uninhibits the screensaver.
- **Package removal with Turbo on:** `pre_remove` hands falcond back (disabled, inactive, as recorded). The record, helper and binaries are gone. The user's profile stays.
- **Fresh reinstall from GitHub:** 520 tests pass inside `makepkg`. The first start lands on Home, fully in pt_BR.

## Left as is

- Reasons produced in `bigame-core` (for example *is not installed (Steam app 750920)*) are English inside translated sentences. They are built there without `N_()`.
- The benchmark lab lists and explains workloads. Runs start from a game card (*Measure the difference*) or the scripts, not from the page.
- `/var/lib/bigame-mode/` stays after removal. It is systemd's `StateDirectory`, and is empty then.
