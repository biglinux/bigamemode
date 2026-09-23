//! A/B measurement of an optimization plan.
//!
//! This is the piece that lets a report say something other than
//! [`Outcome::NotMeasured`]. It is deliberately **not** part of a normal
//! Booster activation: measuring takes minutes and requires running a
//! workload, and silently launching a game because someone pressed a button
//! would be worse than not measuring at all.
//!
//! The method follows what [`docs/09-BENCHMARKS.md`] specifies, and each rule
//! exists because skipping it produces a confident wrong answer:
//!
//! * **Alternate the arms** (A-B-A-B, not AA-BB) so a machine warming up over
//!   the session does not hand all its drift to whichever arm ran last.
//! * **Discard the first run of each arm**; it is dominated by shader
//!   compilation and cold caches.
//! * **Measure the noise floor** from the spread between same-arm runs, and
//!   report nothing smaller than it.
//! * **Restore the baseline afterwards**, whatever happened, including on
//!   failure — a measurement must not leave the machine somewhere it was not.

use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result};

use crate::benchmark::{self, FrameStats};
use crate::booster::plan::Plan;
use crate::booster::report::Outcome;
use crate::booster::snapshot::Snapshot;

/// What to run, how long, and how many times.
#[derive(Debug, Clone)]
pub struct MeasurementPlan {
    /// Workload and its arguments. Must render continuously for `duration_s`.
    pub command: Vec<String>,
    /// Seconds of frametime to record per run.
    pub duration_s: u32,
    /// Runs per arm, **including** the discarded warm-up.
    ///
    /// Three is the practical minimum: one warm-up plus two that count, which
    /// is the fewest that can produce a noise floor at all.
    pub runs_per_arm: usize,
}

impl Default for MeasurementPlan {
    fn default() -> Self {
        Self {
            command: Vec::new(),
            duration_s: 30,
            runs_per_arm: 3,
        }
    }
}

impl MeasurementPlan {
    /// Runs whose results are kept, after discarding the warm-up.
    #[must_use]
    pub fn counted_runs(&self) -> usize {
        self.runs_per_arm.saturating_sub(1)
    }

    /// Whether this plan can produce a comparison.
    ///
    /// Two counted runs per arm is the floor: with one there is no spread to
    /// measure, and without a noise floor every difference looks real.
    #[must_use]
    pub fn is_usable(&self) -> bool {
        !self.command.is_empty() && self.duration_s >= 5 && self.counted_runs() >= 2
    }
}

/// Which configuration a run was recorded under.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arm {
    /// The machine as it was before the plan was applied.
    Baseline,
    /// The machine with the plan applied.
    Optimized,
}

/// Progress of a measurement session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MeasureProgress {
    /// A run is starting.
    Running {
        /// Which arm.
        arm: Arm,
        /// 1-based run number within the arm.
        run: usize,
        /// Runs per arm.
        total: usize,
        /// True when this run will be discarded as a warm-up.
        warmup: bool,
    },
    /// Switching the machine between arms.
    Switching {
        /// The arm being switched to.
        to: Arm,
    },
    /// Computing statistics.
    Analysing,
}

/// A completed measurement.
#[derive(Debug, Clone)]
pub struct Measurement {
    /// Statistics for the counted baseline runs.
    pub baseline: Vec<FrameStats>,
    /// Statistics for the counted optimized runs.
    pub optimized: Vec<FrameStats>,
    /// Noise floor measured from the baseline spread, as a fraction.
    pub noise_floor: f64,
    /// What may honestly be said about the difference.
    pub outcomes: Vec<Outcome>,
}

impl Measurement {
    /// Best (median) statistics from each arm, by 1% low.
    #[must_use]
    fn representative(runs: &[FrameStats]) -> Option<&FrameStats> {
        let mut sorted: Vec<&FrameStats> = runs.iter().collect();
        sorted.sort_by(|a, b| a.low_1_fps.total_cmp(&b.low_1_fps));
        sorted.get(sorted.len() / 2).copied()
    }
}

/// Record one run and return its statistics.
///
/// Returns `Ok(None)` when the workload produced too few frames to summarise —
/// a crashed or instantly-exiting command, which must not be mistaken for a
/// result.
fn record_run(
    cmd: &[String],
    duration_s: u32,
    log_dir: &std::path::Path,
) -> Result<Option<FrameStats>> {
    std::fs::create_dir_all(log_dir)
        .with_context(|| format!("create log dir: {}", log_dir.display()))?;
    for entry in std::fs::read_dir(log_dir)?.flatten() {
        let _ = std::fs::remove_file(entry.path());
    }

    let config_path = log_dir.join("mangohud.conf");
    std::fs::write(
        &config_path,
        benchmark::mangohud_config(log_dir, duration_s),
    )
    .context("write MangoHud config")?;

    let (program, args) = cmd.split_first().context("empty workload command")?;
    let mut child = std::process::Command::new(program)
        .args(args)
        .env("MANGOHUD_CONFIGFILE", &config_path)
        // MangoHud is loaded as a Vulkan layer; `mangohud` the wrapper sets
        // this itself, but the workload may be invoked directly.
        .env("MANGOHUD", "1")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .with_context(|| format!("spawn workload: {program}"))?;

    // The log starts after a one-second delay and runs for `duration_s`;
    // a few seconds of slack covers start-up and the final flush.
    let start = std::time::Instant::now();
    let budget = Duration::from_secs(u64::from(duration_s) + 8);
    // MangoHud writes the CSV once the log duration elapses; wait for it to
    // appear before killing the workload, rather than guessing a fixed sleep.
    let settle = budget.saturating_sub(Duration::from_secs(4));
    while start.elapsed() < budget {
        if start.elapsed() > settle && benchmark::newest_capture_in(log_dir)?.is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(250));
    }

    let _ = child.kill();
    let _ = child.wait();
    std::thread::sleep(Duration::from_millis(500));

    let Some(capture_path) = benchmark::newest_capture_in(log_dir)? else {
        return Ok(None);
    };
    Ok(benchmark::read_capture(&capture_path)?.stats())
}

/// Run an A/B measurement of `plan` against the live machine.
///
/// Applies and reverts the plan between arms, so each run is recorded under the
/// configuration it is attributed to. The baseline is restored before
/// returning, on every path including failure.
///
/// # Errors
/// Returns an error if the measurement plan is unusable, if the workload
/// cannot be run, or if too few runs succeeded to compare.
pub async fn run<F: FnMut(MeasureProgress)>(
    measurement: &MeasurementPlan,
    plan: &Plan,
    snapshot: &Snapshot,
    log_dir: &Path,
    mut progress: F,
) -> Result<Measurement> {
    anyhow::ensure!(
        measurement.is_usable(),
        "measurement plan needs a command, at least 5 seconds, and at least \
         2 counted runs per arm (runs_per_arm includes one discarded warm-up)"
    );
    anyhow::ensure!(
        !plan.is_empty(),
        "nothing to measure: the plan changes nothing on this machine"
    );

    let mut baseline: Vec<FrameStats> = Vec::new();
    let mut optimized: Vec<FrameStats> = Vec::new();

    // Alternate the arms so drift over the session is shared between them.
    let outcome = async {
        for run in 1..=measurement.runs_per_arm {
            for arm in [Arm::Baseline, Arm::Optimized] {
                progress(MeasureProgress::Switching { to: arm });
                match arm {
                    Arm::Baseline => restore(plan, snapshot).await,
                    Arm::Optimized => apply(plan).await,
                }

                let warmup = run == 1;
                progress(MeasureProgress::Running {
                    arm,
                    run,
                    total: measurement.runs_per_arm,
                    warmup,
                });

                let stats = record_run(&measurement.command, measurement.duration_s, log_dir)?;
                let Some(stats) = stats else {
                    anyhow::bail!(
                        "the workload produced too few frames to measure; check that \
                         it renders continuously and that MangoHud is installed"
                    );
                };
                // The first run of each arm is discarded: shader compilation
                // and cold caches make it unrepresentative.
                if warmup {
                    tracing::debug!(target: "booster", ?arm, "warm-up run discarded");
                    continue;
                }
                match arm {
                    Arm::Baseline => baseline.push(stats),
                    Arm::Optimized => optimized.push(stats),
                }
            }
        }
        Ok::<(), anyhow::Error>(())
    }
    .await;

    // Whatever happened, put the machine back.
    progress(MeasureProgress::Switching { to: Arm::Baseline });
    restore(plan, snapshot).await;
    outcome?;

    progress(MeasureProgress::Analysing);
    anyhow::ensure!(
        baseline.len() >= 2 && !optimized.is_empty(),
        "too few usable runs to compare"
    );

    // The floor is the spread between runs of the *same* configuration.
    // Without it, every comparison finds an improvement.
    let baseline_lows: Vec<f64> = baseline.iter().map(|s| s.low_1_fps).collect();
    let noise_floor = benchmark::noise_floor(&baseline_lows).unwrap_or(0.0);

    let a = Measurement::representative(&baseline).context("no baseline runs")?;
    let b = Measurement::representative(&optimized).context("no optimized runs")?;
    let outcomes = benchmark::compare_all(a, b, noise_floor);

    tracing::info!(
        target: "booster",
        baseline_runs = baseline.len(),
        optimized_runs = optimized.len(),
        noise_floor_pct = noise_floor * 100.0,
        "measurement complete"
    );

    Ok(Measurement {
        baseline,
        optimized,
        noise_floor,
        outcomes,
    })
}

/// Apply every change in `plan`, ignoring individual failures.
///
/// A knob that will not move is reported by the normal activation path; here
/// it simply means both arms share that knob's value, which weakens the
/// comparison rather than invalidating it.
async fn apply(plan: &Plan) {
    for change in &plan.changes {
        if let Err(e) = change.knob.write(&change.to).await {
            tracing::warn!(
                target: "booster",
                knob = %change.knob.id(),
                error = %format!("{e:#}"),
                "could not apply for measurement"
            );
        }
    }
}

/// Put every knob the plan touches back to its captured value.
async fn restore(plan: &Plan, snapshot: &Snapshot) {
    let ids: Vec<String> = plan.changes.iter().map(|c| c.knob.id()).collect();
    for outcome in snapshot.restore_applied(&ids).await {
        if !outcome.status.is_ok() {
            tracing::warn!(
                target: "booster",
                knob = %outcome.knob.id(),
                ?outcome.status,
                "could not restore between measurement arms"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stats(low: f64) -> FrameStats {
        FrameStats {
            frames: 1000,
            duration_s: 10.0,
            avg_fps: low * 1.5,
            mean_ms: 1000.0 / low,
            median_ms: 1000.0 / low,
            p95_ms: 1000.0 / low,
            p99_ms: 1000.0 / low,
            low_1_fps: low,
            low_0_1_fps: Some(low * 0.9),
            stutters: 0,
        }
    }

    #[test]
    fn a_plan_needs_a_command_a_duration_and_repeats() {
        assert!(!MeasurementPlan::default().is_usable(), "no command");

        let base = MeasurementPlan {
            command: vec!["vkcube".into()],
            ..MeasurementPlan::default()
        };
        assert!(base.is_usable());

        // One counted run cannot produce a noise floor.
        assert!(
            !MeasurementPlan {
                runs_per_arm: 2,
                ..base.clone()
            }
            .is_usable()
        );
        assert!(
            MeasurementPlan {
                runs_per_arm: 3,
                ..base.clone()
            }
            .is_usable()
        );

        // A two-second run measures start-up, not the workload.
        assert!(
            !MeasurementPlan {
                duration_s: 2,
                ..base.clone()
            }
            .is_usable()
        );
    }

    #[test]
    fn the_warm_up_run_is_not_counted() {
        let plan = MeasurementPlan {
            command: vec!["x".into()],
            runs_per_arm: 5,
            ..MeasurementPlan::default()
        };
        assert_eq!(plan.counted_runs(), 4);
        assert_eq!(
            MeasurementPlan {
                runs_per_arm: 1,
                ..plan
            }
            .counted_runs(),
            0
        );
    }

    #[test]
    fn the_representative_run_is_the_median_not_the_best() {
        // Reporting the best run of each arm would flatter whichever arm got
        // luckier, which is how a measurement turns into an advertisement.
        let runs = vec![stats(100.0), stats(140.0), stats(120.0)];
        let chosen = Measurement::representative(&runs).unwrap();
        assert!((chosen.low_1_fps - 120.0).abs() < f64::EPSILON);

        assert!(Measurement::representative(&[]).is_none());
    }

    #[tokio::test]
    async fn measuring_an_empty_plan_is_refused() {
        let measurement = MeasurementPlan {
            command: vec!["true".into()],
            ..MeasurementPlan::default()
        };
        let err = run(
            &measurement,
            &Plan::default(),
            &Snapshot::default(),
            &std::env::temp_dir(),
            |_| {},
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("nothing to measure"));
    }

    #[tokio::test]
    async fn an_unusable_measurement_plan_is_refused_before_anything_runs() {
        let err = run(
            &MeasurementPlan::default(),
            &Plan::default(),
            &Snapshot::default(),
            &std::env::temp_dir(),
            |_| {},
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("measurement plan needs"));
    }
}
