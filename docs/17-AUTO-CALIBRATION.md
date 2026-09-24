# 17 — Auto-calibration: what exists, and what does not yet

**Objective.** Let the machine teach BiGame-mode which settings are faster
for it, one knob at a time, and never apply an old result to changed
software.

## What exists

| Piece | Where | Status |
|---|---|---|
| Alternating sessions, warm-up discarded, rotation between rounds | `scripts/bench-game.sh`, `scripts/bench-lab.sh` | VERIFIED — five AAA sessions ([13](13-AAA-BENCHMARKS.md)) |
| Games' own frametimes as the measurement | `benchmark/native.rs` | VERIFIED — averages match the games' own figures |
| Significance: spread of the runs, then Welch's t at 95 % | `benchmark/result.rs` | TESTED |
| Settings identical across a session, or it is refused | `bench_native_report` | TESTED |
| Frame generation refused as a throughput result | `native.rs`, `bench_native_report` | TESTED |
| Single-knob verdicts feed the planner | `benchmark/calibration.rs`, `booster/plan.rs` | VERIFIED — GPU DPM `high` refused after −8.0 % |
| Stack recorded with the calibration; "Needs revalidation" after a change | `calibration.rs` (`stack`, `stack_changes`) | TESTED |

## The fingerprint (§13)

A calibration applies to a machine by **hardware fingerprint**: CPU model,
thread count, GPU PCI id, driver, kernel, RAM. A different machine never
inherits one. A kernel update changes the fingerprint, so the calibration
is then treated as absent.

On top of that, each calibration records the **software stack** it was
measured under (`mesa`, `vulkan-radeon`/`nvidia-utils`, `falcond`,
`scx-scheds`, kernel). When any of these changes:

- a setting measured **slower** keeps being avoided — the safe side;
- a setting measured **faster** is not applied until measured again;
- the report shows *Needs revalidation* and what changed.

Calibrations written before the stack was recorded (the reference machine's
current one) are treated as unknown-stack, not stale.

## What does not exist yet

| Wanted | Status | Why |
|---|---|---|
| "Calibrate this game" from the UI, driving a built-in benchmark | NOT IMPLEMENTED | Only Shadow of the Tomb Raider can be driven unattended (its `[R]` rerun key), and driving it means sending keys to a fullscreen game — acceptable from a script a developer watches, not yet as a button. The harness is the prototype. |
| Per-game calibration (a finding scoped to one title) | NOT IMPLEMENTED | Findings are per machine. The data model needs a game key before a UI can offer it. |
| Scheduler calibration | MEASURED by the harness, not by the product | SotTR CPU-bound on the reference machine: lavd and bpfland no faster than the default ([13](13-AAA-BENCHMARKS.md)); the recommendation's `none` stands. The product has no scheduler calibration run of its own yet. |
| Combinations of winning knobs | NOT DONE | No knob has yet measured as a winner here. |

The honest state is that the machinery is complete for one game and one
machine, and the evidence it has produced so far is negative: nothing Turbo
could set was measured faster on the reference machine.
