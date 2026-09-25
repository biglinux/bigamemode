//! A/B measurement of an optimization plan.
//!
//! This is the piece that lets a report say something other than
//! [`Outcome::NotMeasured`]. It is deliberately **not** part of a normal
//! Booster activation: measuring takes minutes and requires running a
//! workload, and silently launching a game because someone pressed a button
//! would be worse than not measuring at all.
//!
//! The method has four rules, and each exists because skipping it produces a
//! confident wrong answer:
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
    /// Seconds to wait after launching before recording starts.
    ///
    /// Long enough to get past menus, loading screens and shader compilation.
    /// Too short and each run captures a different mix of menu and gameplay,
    /// which is noise no statistic can rescue.
    pub start_delay_s: u32,
}

impl Default for MeasurementPlan {
    fn default() -> Self {
        Self {
            command: Vec::new(),
            duration_s: 30,
            runs_per_arm: 3,
            start_delay_s: 12,
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

/// Record one run and return its statistics.
///
/// Returns `Ok(None)` when the workload produced too few frames to summarise —
/// a crashed or instantly-exiting command, which must not be mistaken for a
/// result.
fn record_run(
    cmd: &[String],
    duration_s: u32,
    start_delay_s: u32,
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
        benchmark::mangohud_config(log_dir, duration_s, start_delay_s),
    )
    .context("write MangoHud config")?;

    // Run through the `mangohud` wrapper rather than setting MANGOHUD=1.
    //
    // The environment variable only enables MangoHud's *Vulkan* implicit
    // layer. Plenty of games are OpenGL — SuperTuxKart among them — and for
    // those the wrapper's LD_PRELOAD is what attaches the overlay. The
    // variable alone yields no capture, and because the failure is an empty
    // directory rather than an error, it looks as if the game rendered
    // nothing.
    let mut wrapped: Vec<String> = Vec::new();
    if crate::capabilities::which("mangohud").is_some() {
        wrapped.push("mangohud".to_owned());
    }
    wrapped.extend(cmd.iter().cloned());

    let (program, argv) = wrapped.split_first().context("empty workload command")?;
    let mut command = std::process::Command::new(program);
    command
        .args(argv)
        .env("MANGOHUD_CONFIGFILE", &config_path)
        .env("MANGOHUD", "1")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    // Its own process group, so ending the run ends the game even when the
    // command is a wrapper script: a game left running would still be there
    // in the next arm and skew it.
    crate::launcher::in_own_process_group(&mut command);
    let mut child = command
        .spawn()
        .with_context(|| format!("spawn workload: {program}"))?;

    // The log starts after a one-second delay and runs for `duration_s`;
    // a few seconds of slack covers start-up and the final flush.
    let start = std::time::Instant::now();
    let budget = Duration::from_secs(u64::from(duration_s) + u64::from(start_delay_s) + 10);
    // MangoHud writes the CSV once the log duration elapses; wait for it to
    // appear before killing the workload, rather than guessing a fixed sleep.
    let settle = budget.saturating_sub(Duration::from_secs(4));
    while start.elapsed() < budget {
        if start.elapsed() > settle && benchmark::newest_capture_in(log_dir)?.is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(250));
    }

    let _ = crate::launcher::terminate(&mut child);
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

                let stats = record_run(
                    &measurement.command,
                    measurement.duration_s,
                    measurement.start_delay_s,
                    log_dir,
                )?;
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

    // Each metric is judged over every run of both arms (Welch's t-test and
    // a 5 % stability bar, `benchmark::compare_arms`). The single floor kept
    // here is the 1% low's, for reporting.
    let baseline_lows: Vec<f64> = baseline.iter().map(|s| s.low_1_fps).collect();
    let noise_floor = benchmark::noise_floor(&baseline_lows).unwrap_or(0.0);

    let outcomes = benchmark::compare_arms(&baseline, &optimized);

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
    fn every_run_of_both_arms_is_judged_not_one_median_run() {
        use crate::booster::report::Outcome;
        // Steady arms, clearly apart: an improvement on every metric.
        let base = [stats(40.0), stats(40.4), stats(39.8)];
        let fast = [stats(50.0), stats(50.3), stats(49.9)];
        let out = crate::benchmark::compare_arms(&base, &fast);
        assert!(
            out.iter().all(|o| matches!(o, Outcome::Improved { .. })),
            "{out:?}"
        );
        // The same means, but one arm all over the place: no claim either way.
        let wild = [stats(30.0), stats(70.0), stats(50.0)];
        let out = crate::benchmark::compare_arms(&base, &wild);
        assert!(
            out.iter()
                .all(|o| matches!(o, Outcome::Inconclusive { .. })),
            "{out:?}"
        );
        // Identical arms: no change, not an improvement.
        let out = crate::benchmark::compare_arms(&base, &base);
        assert!(
            out.iter().all(|o| matches!(o, Outcome::NoChange { .. })),
            "{out:?}"
        );
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
