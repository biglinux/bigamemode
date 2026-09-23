//! The runner: turning a provider and a set of configurations into a verdict.
//!
//! Everything the method requires is enforced here rather than left to whoever
//! writes the calling code, because a benchmark harness that *can* be used
//! incorrectly eventually will be.
//!
//! - Arms alternate. The runner interleaves them; a caller cannot ask for
//!   grouped runs, because grouped runs confound the configuration with
//!   whatever drifts over the session.
//! - The first run is discarded, and its number is never returned.
//! - A run that fails is recorded as a failure rather than dropped, so an arm
//!   with three failures and one success cannot masquerade as a one-run arm.
//!
//! The runner knows nothing about governors or GPUs. An arm is a name and a
//! closure that puts the machine into some state; what that means is the
//! caller's business, which is what lets the same runner drive a component
//! isolation matrix and a plain before-and-after.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Result;

use super::provider::{BenchmarkProvider, RunContext};

/// One configuration to measure.
pub struct Arm<'a> {
    /// Name, used for the directory and in the report.
    pub name: String,
    /// Puts the machine into this configuration.
    ///
    /// # Errors
    /// Should return an error when the configuration could not be applied, so
    /// the runner can refuse to attribute a number to a state the machine was
    /// never actually in.
    pub apply: Box<dyn Fn() -> Result<()> + 'a>,
}

impl<'a> Arm<'a> {
    /// An arm from a name and a closure.
    pub fn new(name: impl Into<String>, apply: impl Fn() -> Result<()> + 'a) -> Self {
        Self {
            name: name.into(),
            apply: Box::new(apply),
        }
    }
}

/// What the runner reports as it goes.
#[derive(Debug, Clone)]
pub enum Progress {
    /// The warm-up is running. Its result will be thrown away.
    WarmingUp,
    /// A measured run is starting.
    Starting {
        /// Which configuration.
        arm: String,
        /// 1-based, of `total`.
        run: usize,
        /// Measured runs per arm.
        total: usize,
    },
    /// A run finished and produced a number.
    Measured {
        /// Which configuration.
        arm: String,
        /// What it measured.
        value: f64,
    },
    /// A run failed. Recorded rather than passed over in silence.
    Failed {
        /// Which configuration.
        arm: String,
        /// Why.
        reason: String,
    },
}

/// How to run a session.
pub struct Plan {
    /// Measured runs per arm. Below two, no verdict is possible.
    pub runs_per_arm: usize,
    /// Warm-up runs, discarded.
    pub warmup_runs: usize,
    /// Where run directories are created.
    pub output_dir: PathBuf,
    /// Recording length, for providers whose duration is not fixed.
    pub duration: Duration,
    /// Pause after applying a configuration, before measuring.
    ///
    /// A governor or DPM change is not instantaneous, and a run started in the
    /// gap measures the transition rather than the destination.
    pub settle: Duration,
}

impl Plan {
    /// A plan with the defaults the method calls for.
    #[must_use]
    pub fn new(output_dir: impl Into<PathBuf>) -> Self {
        Self {
            runs_per_arm: 3,
            warmup_runs: 1,
            output_dir: output_dir.into(),
            duration: Duration::from_secs(30),
            settle: Duration::from_secs(3),
        }
    }
}

/// What a session produced.
#[derive(Debug, Clone, Default)]
pub struct Outcome {
    /// Measured values per arm, in the order taken.
    pub arms: BTreeMap<String, Vec<f64>>,
    /// Failures per arm, as reasons.
    pub failures: BTreeMap<String, Vec<String>>,
}

impl Outcome {
    /// Whether every arm has enough successful runs to support a verdict.
    #[must_use]
    pub fn is_complete(&self, expected_runs: usize) -> bool {
        !self.arms.is_empty() && self.arms.values().all(|r| r.len() >= expected_runs)
    }

    /// A sentence about what went wrong, when something did.
    #[must_use]
    pub fn describe_failures(&self) -> Option<String> {
        if self.failures.is_empty() {
            return None;
        }
        let mut parts: Vec<String> = self
            .failures
            .iter()
            .map(|(arm, reasons)| format!("{arm}: {} failed run(s)", reasons.len()))
            .collect();
        parts.sort();
        Some(parts.join("; "))
    }
}

/// Run one session, alternating the arms.
///
/// `report` is called as the session proceeds, so a caller can show progress
/// over what is necessarily a run of several minutes.
///
/// # Errors
/// Returns an error only when the session cannot start at all — an unusable
/// provider, or no arms. Individual run failures are collected into the
/// [`Outcome`] rather than aborting, because one bad run should not discard the
/// twenty minutes of good ones around it.
pub fn run_session(
    provider: &dyn BenchmarkProvider,
    arms: &[Arm<'_>],
    plan: &Plan,
    mut report: impl FnMut(Progress),
) -> Result<Outcome> {
    anyhow::ensure!(!arms.is_empty(), "a session needs at least one configuration");
    let availability = provider.availability();
    anyhow::ensure!(
        availability.is_ready(),
        "{} cannot be run here: {}",
        provider.name(),
        availability.reason().unwrap_or("unknown reason")
    );

    let mut outcome = Outcome::default();

    // Warm-up, under the first arm's configuration. Discarded: a cold shader
    // cache and a cold GPU make the first run unlike every run after it.
    for _ in 0..plan.warmup_runs {
        report(Progress::WarmingUp);
        (arms[0].apply)()?;
        std::thread::sleep(plan.settle);
        let ctx = RunContext {
            output_dir: plan.output_dir.join(".warmup"),
            duration: plan.duration,
        };
        let _ = provider.run(&ctx);
    }
    let _ = std::fs::remove_dir_all(plan.output_dir.join(".warmup"));

    // Alternate. The outer loop is the run index and the inner one the arm,
    // which is what makes the order A B A B rather than A A B B.
    for run in 1..=plan.runs_per_arm {
        for arm in arms {
            report(Progress::Starting {
                arm: arm.name.clone(),
                run,
                total: plan.runs_per_arm,
            });

            if let Err(error) = (arm.apply)() {
                let reason = format!("could not apply the configuration: {error}");
                report(Progress::Failed {
                    arm: arm.name.clone(),
                    reason: reason.clone(),
                });
                outcome.failures.entry(arm.name.clone()).or_default().push(reason);
                continue;
            }
            std::thread::sleep(plan.settle);

            let ctx = RunContext {
                output_dir: run_dir(&plan.output_dir, &arm.name, run),
                duration: plan.duration,
            };
            match provider.run(&ctx).map(|o| o.score) {
                Ok(Some(value)) => {
                    write_value(&ctx.output_dir, value);
                    report(Progress::Measured {
                        arm: arm.name.clone(),
                        value,
                    });
                    outcome.arms.entry(arm.name.clone()).or_default().push(value);
                }
                Ok(None) => {
                    let reason = "the run produced no comparable value".to_owned();
                    report(Progress::Failed {
                        arm: arm.name.clone(),
                        reason: reason.clone(),
                    });
                    outcome.failures.entry(arm.name.clone()).or_default().push(reason);
                }
                Err(error) => {
                    let reason = error.to_string();
                    report(Progress::Failed {
                        arm: arm.name.clone(),
                        reason: reason.clone(),
                    });
                    outcome.failures.entry(arm.name.clone()).or_default().push(reason);
                }
            }
        }
    }
    Ok(outcome)
}

/// `<output>/<arm>/run-NN`, zero-padded so a listing sorts correctly.
fn run_dir(root: &Path, arm: &str, run: usize) -> PathBuf {
    root.join(arm).join(format!("run-{run:02}"))
}

/// Record the run's value beside its artifacts, so the layout is readable
/// without re-deriving anything.
fn write_value(dir: &Path, value: f64) {
    let _ = std::fs::create_dir_all(dir);
    let _ = std::fs::write(dir.join("fps.txt"), format!("{value:.4}\n"));
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;
    use crate::benchmark::provider::{Availability, RunOutcome};

    /// A provider that returns a scripted sequence of results.
    struct Fake {
        availability: Availability,
        values: Vec<Option<f64>>,
        next: AtomicUsize,
    }

    impl Fake {
        fn ready(values: Vec<Option<f64>>) -> Self {
            Self {
                availability: Availability::Ready,
                values,
                next: AtomicUsize::new(0),
            }
        }
    }

    impl BenchmarkProvider for Fake {
        fn id(&self) -> &'static str {
            "fake"
        }
        fn name(&self) -> &'static str {
            "Fake"
        }
        fn source(&self) -> crate::benchmark::provider::Source {
            crate::benchmark::provider::Source::Score
        }
        fn availability(&self) -> Availability {
            self.availability.clone()
        }
        fn run(&self, _ctx: &RunContext) -> Result<RunOutcome> {
            let i = self.next.fetch_add(1, Ordering::SeqCst);
            let value = self.values.get(i).copied().flatten();
            value.map_or_else(
                || anyhow::bail!("scripted failure"),
                |v| {
                    Ok(RunOutcome {
                        stats: None,
                        score: Some(v),
                        source: crate::benchmark::provider::Source::Score,
                        artifacts: Vec::new(),
                        notes: Vec::new(),
                    })
                },
            )
        }
    }

    fn plan(dir: &Path, runs: usize) -> Plan {
        Plan {
            runs_per_arm: runs,
            warmup_runs: 1,
            output_dir: dir.to_path_buf(),
            duration: Duration::from_millis(1),
            settle: Duration::from_millis(0),
        }
    }

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("bigame_runner_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn arms_alternate_and_the_warmup_is_discarded() {
        let dir = temp("alternate");
        // One warm-up, then A B A B A B.
        let provider = Fake::ready(vec![
            Some(0.0), // warm-up, must not appear
            Some(10.0), Some(20.0),
            Some(11.0), Some(21.0),
            Some(12.0), Some(22.0),
        ]);
        let order = RefCell::new(Vec::new());
        let arms = vec![
            Arm::new("a", || Ok(())),
            Arm::new("b", || Ok(())),
        ];
        let outcome = run_session(&provider, &arms, &plan(&dir, 3), |p| {
            if let Progress::Starting { arm, .. } = p {
                order.borrow_mut().push(arm);
            }
        })
        .unwrap();

        assert_eq!(order.borrow().as_slice(), ["a", "b", "a", "b", "a", "b"]);
        assert_eq!(outcome.arms["a"], vec![10.0, 11.0, 12.0]);
        assert_eq!(outcome.arms["b"], vec![20.0, 21.0, 22.0]);
        // The warm-up's 0.0 must be nowhere.
        assert!(!outcome.arms.values().flatten().any(|v| *v == 0.0));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_failed_run_is_recorded_not_dropped() {
        let dir = temp("failure");
        let provider = Fake::ready(vec![
            Some(0.0),                 // warm-up
            Some(10.0), None,          // run 1: b fails
            Some(11.0), Some(21.0),    // run 2
        ]);
        let arms = vec![Arm::new("a", || Ok(())), Arm::new("b", || Ok(()))];
        let outcome = run_session(&provider, &arms, &plan(&dir, 2), |_| {}).unwrap();

        assert_eq!(outcome.arms["a"].len(), 2);
        assert_eq!(outcome.arms["b"].len(), 1);
        // The distinction that matters: b is not a healthy one-run arm.
        assert_eq!(outcome.failures["b"].len(), 1);
        assert!(outcome.describe_failures().unwrap().contains("b: 1 failed"));
        assert!(!outcome.is_complete(2));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_configuration_that_cannot_be_applied_is_not_credited_with_a_number() {
        let dir = temp("apply");
        let provider = Fake::ready(vec![Some(0.0), Some(10.0), Some(99.0), Some(11.0), Some(99.0)]);
        let arms = vec![
            Arm::new("a", || Ok(())),
            Arm::new("b", || anyhow::bail!("permission denied")),
        ];
        let outcome = run_session(&provider, &arms, &plan(&dir, 2), |_| {}).unwrap();

        assert!(!outcome.arms.contains_key("b"), "b never ran, so it has no runs");
        assert_eq!(outcome.failures["b"].len(), 2);
        assert!(outcome.failures["b"][0].contains("permission denied"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_unusable_provider_refuses_to_start() {
        let dir = temp("unusable");
        let provider = Fake {
            availability: Availability::MissingDependency("vsync is on".into()),
            values: vec![Some(1.0)],
            next: AtomicUsize::new(0),
        };
        let arms = vec![Arm::new("a", || Ok(()))];
        let error = run_session(&provider, &arms, &plan(&dir, 2), |_| {}).unwrap_err();
        assert!(error.to_string().contains("vsync is on"));
    }

    #[test]
    fn a_session_with_no_arms_is_refused() {
        let dir = temp("noarms");
        let provider = Fake::ready(vec![Some(1.0)]);
        let error = run_session(&provider, &[], &plan(&dir, 2), |_| {}).unwrap_err();
        assert!(error.to_string().contains("at least one configuration"));
    }

    #[test]
    fn each_run_writes_its_value_into_the_layout() {
        let dir = temp("layout");
        let provider = Fake::ready(vec![Some(0.0), Some(10.0), Some(11.0)]);
        let arms = vec![Arm::new("solo", || Ok(()))];
        run_session(&provider, &arms, &plan(&dir, 2), |_| {}).unwrap();

        assert_eq!(
            std::fs::read_to_string(dir.join("solo/run-01/fps.txt")).unwrap().trim(),
            "10.0000"
        );
        assert!(dir.join("solo/run-02/fps.txt").is_file());
        assert!(!dir.join(".warmup").exists(), "the warm-up directory is cleaned up");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn progress_reports_every_measurement_and_every_failure() {
        let dir = temp("progress");
        let provider = Fake::ready(vec![Some(0.0), Some(10.0), None]);
        let arms = vec![Arm::new("a", || Ok(()))];
        let events = RefCell::new(Vec::new());
        run_session(&provider, &arms, &plan(&dir, 2), |p| {
            events.borrow_mut().push(format!("{p:?}"));
        })
        .unwrap();

        let events = events.borrow();
        assert!(events.iter().any(|e| e.starts_with("WarmingUp")));
        assert!(events.iter().any(|e| e.contains("Measured")));
        assert!(events.iter().any(|e| e.contains("Failed")));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
