# 21 — Lab VM tests

The lab VM (`192.168.132.87`, BigLinux on Manjaro, kernel 6.18.49, a libvirt
guest **on the benchmark host itself**, booted from a live ISO) is for
behaviour, not performance: its GPU is virtio and llvmpipe. Nothing measured
here is a performance result. No VM work ran during a host benchmark in this
pass.

falcond is not in the VM's repositories; the host's exact 2.0.2 binary,
unit and `falcond-profiles` were copied in. A copy of `sleep` named `cs2`
serves as a stand-in game that the upstream `cs2` profile matches by name.

| Test | Result | Status |
|---|---|---|
| T1 — profile applied on launch, restored on exit | `balanced` → `performance` → `balanced` | VERIFIED |
| T2 — `systemctl stop falcond` with a profile active | restored to `balanced` at once; restart re-applies to the running game | VERIFIED |
| T3 — `enable_performance_mode=false` + reload, no restart | still switched to `performance`; honoured only after restart | VERIFIED |
| Helper would not start: `ReadWritePaths` listed the V-Cache driver path, absent here | `226/NAMESPACE`; fixed with `-` prefixes | FAILED → fixed → VERIFIED |
| `SetGameBackend false` mid-game | `inactive/disabled`, `balanced` restored, ownership record written | VERIFIED |
| `SetGameBackend true` | `active/enabled`, running game re-detected, profile re-applied | VERIFIED |
| Profile save mid-game | same falcond PID (reload, not restart); inotify and SIGHUP both reload | VERIFIED |
| `ReleaseGameBackend` after an external `disable` | back to `inactive/disabled` exactly | VERIFIED |
| `turbo off` / `turbo on` through the core orchestrator | falcond stopped/started, report correct, power profile attributed to falcond | VERIFIED |
| Chassis `Unknown` → no profile-set correction | not corrected (only certain mismatches are) | VERIFIED |
| `scx-tools` + `scx_loader` + a profile asking `lavd/gaming` | kernel `sched_ext` `disabled` → `enabled`, ops `lavd_1.1.3`, → `disabled` on exit | VERIFIED |
| vkmark on llvmpipe | score 459 — smoke test only | TESTED |
| glmark2 off-screen | could not create a canvas | FAILED |
| Phoronix Test Suite 10.8.4 batch mode | runs; `pts/vkmark` needed meson/ninja, then still prompted for options until `RunAllTestCombinations` — `PRESET_OPTIONS` is the proper fix | INCONCLUSIVE |
| Reboot persistence of Turbo off | — | NOT TESTED — a live ISO loses the install on reboot |
| `kill -9` of falcond with a game profile active | power profile **stays `performance`**: systemd restarts falcond, the new instance snapshots the boosted value as its baseline, and "restores" it when the game exits. `balanced` is lost | FAILED — falcond's; see below |
| `kill -9` of the helper during `SetGameBackend false` | systemd completes the job it was given: `inactive/disabled`, consistent; a retry works | VERIFIED |
| `kill -9` of the UI | holds no system state; nothing to recover | NOT TESTED as a run; by construction |

### falcond does not survive its own SIGKILL

Reproduced: `balanced` → game starts → `performance` → `kill -9 falcond` →
systemd restarts it (`Restart=on-failure`) → the new instance finds the game
running and records `performance` as the state to restore → game exits →
`performance`. The machine is left boosted until something else changes it.

BiGame-mode does not fix this by writing the power profile itself: that would
make it a second writer of falcond's state, which is the failure the
single-owner rule exists to prevent. The fix belongs in falcond — persisting
its restore snapshot (it already publishes it as `RESTORE_STATE` in the
status file) and reading it back at start-up. On the BiGame-mode side the
useful step is detection: systemd counts the unit's restarts (`NRestarts`),
and a restart while a profile was active is worth a health warning. Both are
P1 in the final report.
