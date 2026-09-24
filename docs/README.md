# BiGame-mode engineering documentation

Written during the audit and rebuild of 2026-09-23, on the branch
`feat/booster-orchestrator`.

This is engineering documentation. The application does not read it and does not
depend on it.

## Read in this order

| Document | What it covers |
|---|---|
| [00-BASELINE.md](00-BASELINE.md) | The reference machine and the project's state before any change |
| [01-AUDIT.md](01-AUDIT.md) | Every defect found, with evidence and severity |
| [02-PERFORMANCE-AUTHORITY.md](02-PERFORMANCE-AUTHORITY.md) | Who owns which resource; the falcond / GameMode decision |
| [03-GAMING-DISTROS-RESEARCH.md](03-GAMING-DISTROS-RESEARCH.md) | PikaOS, CachyOS, Bazzite, Nobara — what was taken and what was not |
| [04-TEST-MATRIX.md](04-TEST-MATRIX.md) | What was tested, and what could not be |
| [05-BOOSTER-ARCHITECTURE.md](05-BOOSTER-ARCHITECTURE.md) | The optimization engine |
| [06-GAMESCOPE-TUNING.md](06-GAMESCOPE-TUNING.md) | Gamescope and Tuning; the conflict matrix |
| [07-NETWORK-TELEMETRY.md](07-NETWORK-TELEMETRY.md) | Measurement, and what is deliberately not tuned |
| [09-BENCHMARKS.md](09-BENCHMARKS.md) | Why nothing was measured, and what building it needs |
| [10-SECURITY.md](10-SECURITY.md) | Threat model, findings, and what was done |
| [11-BENCHMARK-LAB.md](11-BENCHMARK-LAB.md) | The method: why runs alternate, what counts as a real difference |
| [12-FINDINGS.md](12-FINDINGS.md) | What the method found, including where the Booster was wrong |
| [13-AAA-BENCHMARKS.md](13-AAA-BENCHMARKS.md) | Shadow of the Tomb Raider driven unattended; GPU- and CPU-bound results; the UI's own overhead |
| [14-TURBO-AUDIT.md](14-TURBO-AUDIT.md) | What Turbo and falcond actually did, stage by stage, with evidence |
| [15-DYNAMIC-PROFILES.md](15-DYNAMIC-PROFILES.md) | Finding the running game, offering a profile, migrating old ones |
| [16-PERFORMANCE-BACKENDS.md](16-PERFORMANCE-BACKENDS.md) | falcond vs GameMode; who owns each piece of state |
| [17-AUTO-CALIBRATION.md](17-AUTO-CALIBRATION.md) | What calibration does now, and what it does not yet |
| [18-UX-REDESIGN.md](18-UX-REDESIGN.md) | Home, report, Settings, Diagnostics, Logs; controls removed because they did nothing |
| [19-LOGGING-OBSERVABILITY.md](19-LOGGING-OBSERVABILITY.md) | The journal-based Logs page, polling removed, the UI's measured cost |
| [20-BENCHMARK-RESULTS.md](20-BENCHMARK-RESULTS.md) | Every measurement in one table, including Turbo off vs on |
| [21-VM-TESTS.md](21-VM-TESTS.md) | Behaviour tests on the lab VM, including crash recovery |
| [22-AAA-VALIDATION.md](22-AAA-VALIDATION.md) | Checks against the real installed games |
| [23-NEW-TECH-RESEARCH.md](23-NEW-TECH-RESEARCH.md) | falcond 2.0.14, scx-loader, GameMode, linuxgamebench and others |
| [24-FINAL-VALIDATION.md](24-FINAL-VALIDATION.md) | The definition of done, criterion by criterion |
| [25-DLSS-RESEARCH.md](25-DLSS-RESEARCH.md) | AI Graphics: the two reference projects, upstream, and what was taken or rejected |
| [26-DLSS-LICENSE-AUDIT.md](26-DLSS-LICENSE-AUDIT.md) | What may be fetched, what the user must obtain, what is never touched |
| [27-GRAPHICS-COMPATIBILITY-MATRIX.md](27-GRAPHICS-COMPATIBILITY-MATRIX.md) | Which graphics technologies may run together, and why |
| [28-DLSS-ARCHITECTURE.md](28-DLSS-ARCHITECTURE.md) | The `graphics` subsystem and where it plugs in |
| [29-DLSS-IMPLEMENTATION.md](29-DLSS-IMPLEMENTATION.md) | User flow, integration points, what was removed, defects found |
| [30-DLSS-TESTS.md](30-DLSS-TESTS.md) | Automated and real-machine tests, and what was not tested |
| [31-DLSS-BENCHMARKS.md](31-DLSS-BENCHMARKS.md) | TAA vs the game's XeSS vs OptiScaler FSR, measured |
| [32-DLSS-FINAL-REPORT.md](32-DLSS-FINAL-REPORT.md) | AI Graphics: summary, decisions, limitations, next steps |
| [FINAL-REPORT.md](FINAL-REPORT.md) | Summary, before/after, and known limitations |

There is no `08-UX-REDESIGN.md`. The UX rationale is in FINAL-REPORT and in the
module documentation for `views/home.rs` and `widgets/booster_button.rs`; the
implementation narrative is the commit history. An empty document would not have
earned its place.

`09-BENCHMARKS.md` describes the state before there was a benchmark lab and is
kept for that history; `11` and `12` supersede it.

## Historical

`ARQUITETURA.md`, `IMPLEMENTATION_REVIEW.md`, `IMPLEMENTATION_STATUS.md`,
`CHAT_KNOWLEDGE_BiGameMode.md` and `PROMPT_NOVAS_FUNCIONALIDADES.md` predate the
rebuild and describe components that no longer exist. Each carries a banner
saying so.

## The rule

> Detect → Measure → Optimize → Verify → Report → Restore

Two consequences run through everything here. A change is not reported until it
has been read back from the system. And "applied" is never rounded up to
"improved" — without a benchmark, the report says **Performance impact not
measured**, and that is a correct answer rather than a missing one.
