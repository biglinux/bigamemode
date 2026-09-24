# 24 — Final validation against the definition of done

Branch `feature/turbo-dynamic-profiles`. Statuses are exact: **VERIFIED**
means observed on a real system, **TESTED** means automated tests, and
**IMPLEMENTED** means written and compiled but not yet observed working end to
end. The package was installed on the reference machine near the end of the
pass, and the Turbo switch, the profile-set correction and the first-run
profile were then verified there.

| Criterion | Status | Evidence |
|---|---|---|
| Turbo really controls the flow | VERIFIED | reference machine, 2026-09-24 03:12: off → falcond `inactive/disabled`, ownership recorded (`enabled`, running); on → `active/enabled`, profile set corrected handheld → desktop, 12 profiles loaded; also on the VM with a stand-in game ([21](21-VM-TESTS.md)) |
| falcond intervenes only per Turbo's policy | VERIFIED | off = stopped and disabled on the reference machine; T2 on the VM shows the stop restores a running game's profile |
| No two components control the same state | TESTED; reference-machine plan VERIFIED | `Skipped::OwnedBy`; dry run: power profile → falcond, governor → power-profiles-daemon ([16](16-PERFORMANCE-BACKENDS.md)) |
| Game detected automatically | VERIFIED | Shadow of the Tomb Raider, live |
| Real executable identified | VERIFIED | `SOTTR.exe`, among 17 processes |
| Profile can be created on first run | VERIFIED | with Shadow of the Tomb Raider running and no profile, the offer appeared and *Create profile* was clicked (02:55); `SOTTR.exe.conf` holds exactly the recommended falcond fields |
| Profile applied/reloaded and verified | VERIFIED | next launch: falcond `matched … profile='SOTTR.exe'`, status `ACTIVE_PROFILE: SOTTR.exe`, Home shows *Perfil SOTTR.exe*; reload keeping the PID verified on the VM |
| Home shows the current game | VERIFIED | screenshot |
| Home shows what is really active | VERIFIED | profile read from falcond; "general Proton profile" shown as such |
| Optimization details understandable | PARTIAL | Home's summary read from the real Turbo report (*2 aplicados · 2 por jogo · 1 ignorados*); the report page itself not screenshotted — it is not reachable without a click |
| Logs useful and colour-coded | VERIFIED | screenshot; falcond's info-level failures shown as errors |
| Settings simplified | VERIFIED | screenshot |
| Info for technical options | PARTIAL | report, Settings, offer; not yet Details/Profiles/Tuning/Video/Benchmark |
| Benchmark uses the right method | VERIFIED | alternating, warm-up, significance, settings check ([13](13-AAA-BENCHMARKS.md)) |
| Results reproducible | VERIFIED | clean SotTR session: spread 0.1 % |
| Regressions rejected | VERIFIED | GPU DPM `high` refused with the numbers |
| Rollback works | VERIFIED on the VM | off restores mid-game; release restores the prior unit state |
| Crashes do not leave state stuck | PARTIAL | helper killed mid-switch: consistent. **falcond killed mid-game: power profile left boosted** — falcond's defect ([21](21-VM-TESTS.md)); Diagnostics now warns when systemd has restarted falcond (VERIFIED on the VM). A reload mid-game keeps the right baseline (VERIFIED on the VM) |
| Gamescope zombie | VERIFIED fixed (previous pass) | children reaped |
| UI has low overhead | MEASURED | 0.77 % CPU, 9.9 wake-ups/s on Home in-game (was 7.38 %, 313/s) |
| Tests pass | TESTED | 434 tests; `cargo fmt --check`; `cargo clippy --workspace --all-targets` 0 warnings |
| Package installs | VERIFIED | branch package installed on the reference machine (03:10); helper restarted and exposes `SetGameBackend` |
| Application starts | VERIFIED | the branch build ran on the reference machine |
| Games still start normally | VERIFIED | SotTR launched through Steam throughout |
| Translations still work | VERIFIED | installed UI shows *Modo Turbo ligado*, *Perfil*, *2 aplicados · 2 por jogo*; all strings added in this pass translated for pt_BR; other languages fall back to English for them |
| Documentation reflects reality | this set, 14–24 | |

## `cargo clippy -- -D warnings --all-features`

The workspace defines no features, and clippy (with the workspace's pedantic
lints) reports zero warnings, which is what `-D warnings` would enforce.

## Still to do on the reference machine

1. Measure a scheduler CPU-bound: the one command in
   [20](20-BENCHMARK-RESULTS.md), with the desktop left idle for ~45 minutes.
   It asks for the password once.

Done on 2026-09-24:

| Step | Status |
|---|---|
| Package rebuilt at `9b406a8`, installed with `scx-tools`; `scx_loader` enabled; falcond listing the schedulers; Diagnostics reporting sched-ext OK | VERIFIED |
| Old profiles migrated — by the user, from Settings → *Fix* (05:48): `Arc Raiders` → `PioneerGame.exe`, `Dead by Daylight` → `DeadByDaylight-Win64-Shipping.exe`, falcond fields only, the user's values kept, originals in `~/.local/state/bigame-mode/profiles-1790239687/` | VERIFIED |
| `user/iscriptevaluator.exe.conf` removed through the helper's `DeleteProfile`; falcond reloaded, 13 profiles | VERIFIED |
| Package rebuilt at `d495215` (the restart warning) and installed; helper and UI restarted | VERIFIED |
| Scheduler chain on this machine: profile → falcond → `scx_loader` → kernel ops `lavd_1.1.3` | VERIFIED |
| Scheduler measurement | NOT DONE — one run of nine; the game was closed mid-session |
| SotTR's render scale restored after the session (`ResolutionModifier` 200 → 1000, the game had saved the minimum on exit); all 28 graphics values match the clean session | VERIFIED |
