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
