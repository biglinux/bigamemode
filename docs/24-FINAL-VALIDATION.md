# 24 — Final validation against the definition of done

Branch `feature/turbo-dynamic-profiles`. Statuses are exact: **VERIFIED**
means observed on a real system, **TESTED** means automated tests, and
**IMPLEMENTED** means written and compiled but not yet observed working end to
end. The one thing that blocked more end-to-end verification on the
reference machine was a Polkit approval to install the package, which was
not available while this pass ran; everything that needed it is marked.

| Criterion | Status | Evidence |
|---|---|---|
| Turbo really controls the flow | VERIFIED on the VM; IMPLEMENTED on the reference machine | `SetGameBackend`/`turbo on/off` on falcond 2.0.2 with a stand-in game ([21](21-VM-TESTS.md)) |
| falcond intervenes only per Turbo's policy | VERIFIED on the VM | off = stopped and disabled; T2 shows the stop restores |
| No two components control the same state | TESTED; reference-machine plan VERIFIED | `Skipped::OwnedBy`; dry run: power profile → falcond, governor → power-profiles-daemon ([16](16-PERFORMANCE-BACKENDS.md)) |
| Game detected automatically | VERIFIED | Shadow of the Tomb Raider, live |
| Real executable identified | VERIFIED | `SOTTR.exe`, among 17 processes |
| Profile can be created on first run | IMPLEMENTED | offer, review, save, verify; not run live (Polkit) |
| Profile applied/reloaded and verified | VERIFIED on the VM (reload keeps the PID; profile re-applied); IMPLEMENTED in the UI | |
| Home shows the current game | VERIFIED | screenshot |
| Home shows what is really active | VERIFIED | profile read from falcond; "general Proton profile" shown as such |
| Optimization details understandable | IMPLEMENTED | not seen with a real Turbo report (needs the package) |
| Logs useful and colour-coded | VERIFIED | screenshot; falcond's info-level failures shown as errors |
| Settings simplified | VERIFIED | screenshot |
| Info for technical options | PARTIAL | report, Settings, offer; not yet Details/Profiles/Tuning/Video/Benchmark |
| Benchmark uses the right method | VERIFIED | alternating, warm-up, significance, settings check ([13](13-AAA-BENCHMARKS.md)) |
| Results reproducible | VERIFIED | clean SotTR session: spread 0.1 % |
| Regressions rejected | VERIFIED | GPU DPM `high` refused with the numbers |
| Rollback works | VERIFIED on the VM | off restores mid-game; release restores the prior unit state |
| Crashes do not leave state stuck | PARTIAL | helper killed mid-switch: consistent. **falcond killed mid-game: power profile left boosted** — falcond's defect ([21](21-VM-TESTS.md)) |
| Gamescope zombie | VERIFIED fixed (previous pass) | children reaped |
| UI has low overhead | MEASURED | 0.77 % CPU, 9.9 wake-ups/s on Home in-game (was 7.38 %, 313/s) |
| Tests pass | TESTED | 430 tests; `cargo fmt --check`; `cargo clippy --workspace --all-targets` 0 warnings |
| Package installs | TESTED as a build | `makepkg` from the branch succeeded (its own `check()` ran the tests); installation awaits Polkit |
| Application starts | VERIFIED | the branch build ran on the reference machine |
| Games still start normally | VERIFIED | SotTR launched through Steam throughout |
| Translations still work | TESTED | catalogue check passes; all 138 strings added in this pass translated for pt_BR; other languages fall back to English for them |
| Documentation reflects reality | this set, 14–24 | |

## `cargo clippy -- -D warnings --all-features`

The workspace defines no features, and clippy (with the workspace's pedantic
lints) reports zero warnings, which is what `-D warnings` would enforce.

## To finish on the reference machine

1. Install the branch package (one Polkit approval).
2. Turbo on: expect the profile set corrected handheld → desktop (one approval
   for the config write), falcond running, report as in [14](14-TURBO-AUDIT.md).
3. With Shadow of the Tomb Raider running: accept the profile offer; expect
   `ACTIVE_PROFILE: SOTTR.exe`.
4. Settings → *Fix* the two old profiles.
5. Install `scx-tools`, enable `scx_loader`, and measure a scheduler
   CPU-bound with `scripts/bench-game.sh`.
