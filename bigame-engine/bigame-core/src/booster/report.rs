//! Reporting — and the distinction the whole project turns on.
//!
//! **"Applied" and "improved" are not the same claim**, and this module refuses
//! to let them be conflated. Writing `performance` to a knob and reading it back
//! proves the system changed. It does not prove a single frame got faster. The
//! only honest thing to say about performance without a measurement is that it
//! was not measured — so [`Outcome::NotMeasured`] is a first-class value here,
//! not an error state.
//!
//! A switch must not turn green because a D-Bus call was sent; everything
//! below makes success mean "written and read back".

use serde::{Deserialize, Serialize};

use super::knob::{Knob, Verification};
use super::plan::{Change, Skipped};

/// What happened to one planned change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppliedChange {
    /// The knob.
    pub knob: Knob,
    /// Value before.
    pub from: String,
    /// Value requested.
    pub to: String,
    /// Why it was attempted.
    pub rationale: String,
    /// Whether the system was observed to actually change.
    pub verification: Verification,
    /// Present when the write itself failed.
    pub error: Option<String>,
}

impl AppliedChange {
    /// True when the write succeeded **and** the read-back agreed.
    #[must_use]
    pub fn succeeded(&self) -> bool {
        self.error.is_none() && self.verification.is_confirmed()
    }

    /// One line describing the state transition, for the report UI.
    #[must_use]
    pub fn summary(&self) -> String {
        format!("{}: {} → {}", self.knob.title(), self.from, self.to)
    }
}

/// A performance claim. Deliberately tri-state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum Outcome {
    /// No benchmark was run, so nothing may be claimed. This is the correct,
    /// expected answer for an ordinary Booster activation — not a failure.
    NotMeasured,
    /// A measurement ran and the metric moved in the desired direction.
    Improved {
        /// Metric name, e.g. `P99 frametime`.
        metric: String,
        /// Baseline value.
        before: f64,
        /// Post-change value.
        after: f64,
        /// Unit, e.g. `ms`.
        unit: String,
    },
    /// A measurement ran and found no difference beyond its own noise floor.
    NoChange {
        /// Metric name.
        metric: String,
    },
    /// A measurement ran and the metric got worse.
    Regressed {
        /// Metric name.
        metric: String,
        /// Baseline value.
        before: f64,
        /// Post-change value.
        after: f64,
        /// Unit.
        unit: String,
    },
}

impl Outcome {
    /// Text safe to show a user. Never invents a number.
    #[must_use]
    pub fn describe(&self) -> String {
        match self {
            Self::NotMeasured => "Performance impact not measured".into(),
            Self::Improved {
                metric,
                before,
                after,
                unit,
            } => {
                format!("{metric}: {before:.1} {unit} → {after:.1} {unit}")
            }
            Self::NoChange { metric } => format!("{metric}: no measurable change"),
            Self::Regressed {
                metric,
                before,
                after,
                unit,
            } => {
                format!("{metric}: {before:.1} {unit} → {after:.1} {unit} (worse)")
            }
        }
    }
}

/// The result of one Booster activation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Report {
    /// One-line description of the machine.
    pub machine: String,
    /// Every change that was attempted.
    pub applied: Vec<AppliedChange>,
    /// Candidates that were considered and rejected, with reasons.
    pub skipped: Vec<Skipped>,
    /// Measured results. Empty means nothing was benchmarked.
    pub measurements: Vec<Outcome>,
}

impl Report {
    /// Count of changes written **and** verified.
    #[must_use]
    pub fn verified_count(&self) -> usize {
        self.applied.iter().filter(|a| a.succeeded()).count()
    }

    /// Count of changes that failed to write or failed verification.
    #[must_use]
    pub fn failed_count(&self) -> usize {
        self.applied.iter().filter(|a| !a.succeeded()).count()
    }

    /// Overall state to show on the Home screen.
    #[must_use]
    pub fn state(&self) -> ReportState {
        if self.applied.is_empty() {
            ReportState::AlreadyOptimal
        } else if self.failed_count() == 0 {
            ReportState::Active
        } else if self.verified_count() > 0 {
            ReportState::Partial
        } else {
            ReportState::Failed
        }
    }

    /// Headline summary, built only from things that were actually observed.
    #[must_use]
    pub fn headline(&self) -> String {
        match self.state() {
            ReportState::AlreadyOptimal => {
                "No changes needed — this system is already configured for gaming".into()
            }
            ReportState::Active => format!(
                "{} optimization{} applied and verified",
                self.verified_count(),
                if self.verified_count() == 1 { "" } else { "s" }
            ),
            ReportState::Partial => format!(
                "{} of {} optimizations verified",
                self.verified_count(),
                self.applied.len()
            ),
            ReportState::Failed => "No optimization could be applied".into(),
        }
    }

    /// The honest one-liner about performance.
    #[must_use]
    pub fn performance_claim(&self) -> String {
        if self.measurements.is_empty() {
            return Outcome::NotMeasured.describe();
        }
        self.measurements
            .iter()
            .map(Outcome::describe)
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// Coarse state for the Home screen's Booster control.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReportState {
    /// Nothing needed changing.
    AlreadyOptimal,
    /// Everything planned was applied and verified.
    Active,
    /// Some changes took, some did not.
    Partial,
    /// Nothing took.
    Failed,
}

/// Build an [`AppliedChange`] describing a write that was never attempted
/// because it failed validation or the privileged call errored.
#[must_use]
pub fn failed(change: &Change, error: String) -> AppliedChange {
    AppliedChange {
        knob: change.knob.clone(),
        from: change.from.clone(),
        to: change.to.clone(),
        rationale: change.rationale.clone(),
        verification: Verification::Unreadable,
        error: Some(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn change(knob: Knob, from: &str, to: &str) -> Change {
        Change {
            knob,
            from: from.into(),
            to: to.into(),
            rationale: "because".into(),
            risk: super::super::plan::Risk::Safe,
        }
    }

    fn ok_change(knob: Knob) -> AppliedChange {
        AppliedChange {
            knob,
            from: "balanced".into(),
            to: "performance".into(),
            rationale: "because".into(),
            verification: Verification::Confirmed,
            error: None,
        }
    }

    #[test]
    fn a_write_that_did_not_take_is_not_a_success() {
        let mismatched = AppliedChange {
            verification: Verification::Mismatch {
                actual: "balanced".into(),
            },
            ..ok_change(Knob::PowerProfile)
        };
        // An accepted write whose read-back disagrees is not a success.
        assert!(!mismatched.succeeded());
        assert_eq!(mismatched.error, None, "the write itself did succeed");
    }

    #[test]
    fn unreadable_verification_is_not_a_success_either() {
        let unreadable = AppliedChange {
            verification: Verification::Unreadable,
            ..ok_change(Knob::CpuGovernor)
        };
        assert!(!unreadable.succeeded());
    }

    #[test]
    fn performance_is_never_claimed_without_a_measurement() {
        let report = Report {
            machine: "bench".into(),
            applied: vec![ok_change(Knob::PowerProfile)],
            skipped: Vec::new(),
            measurements: Vec::new(),
        };
        assert_eq!(report.verified_count(), 1);
        // One verified change, and still no performance claim.
        assert_eq!(
            report.performance_claim(),
            "Performance impact not measured"
        );
        assert!(!report.performance_claim().contains('%'));
    }

    #[test]
    fn measured_results_are_reported_with_their_units() {
        let report = Report {
            measurements: vec![Outcome::Improved {
                metric: "P99 frametime".into(),
                before: 18.4,
                after: 15.6,
                unit: "ms".into(),
            }],
            ..Report::default()
        };
        assert_eq!(
            report.performance_claim(),
            "P99 frametime: 18.4 ms → 15.6 ms"
        );
    }

    #[test]
    fn a_regression_is_reported_as_a_regression() {
        let o = Outcome::Regressed {
            metric: "1% low".into(),
            before: 84.0,
            after: 72.0,
            unit: "fps".into(),
        };
        assert!(o.describe().contains("worse"));
    }

    #[test]
    fn no_change_is_said_plainly() {
        assert_eq!(
            Outcome::NoChange {
                metric: "Average FPS".into()
            }
            .describe(),
            "Average FPS: no measurable change"
        );
    }

    #[test]
    fn an_already_optimal_machine_says_so() {
        let report = Report::default();
        assert_eq!(report.state(), ReportState::AlreadyOptimal);
        assert!(report.headline().contains("already configured"));
        assert_eq!(report.verified_count(), 0);
    }

    #[test]
    fn full_success_is_counted_exactly() {
        let report = Report {
            applied: vec![ok_change(Knob::PowerProfile), ok_change(Knob::CpuGovernor)],
            ..Report::default()
        };
        assert_eq!(report.state(), ReportState::Active);
        assert_eq!(report.verified_count(), 2);
        assert_eq!(report.failed_count(), 0);
        assert_eq!(report.headline(), "2 optimizations applied and verified");
    }

    #[test]
    fn singular_headline_for_one_change() {
        let report = Report {
            applied: vec![ok_change(Knob::PowerProfile)],
            ..Report::default()
        };
        assert_eq!(report.headline(), "1 optimization applied and verified");
    }

    #[test]
    fn partial_application_is_reported_partial() {
        let report = Report {
            applied: vec![
                ok_change(Knob::PowerProfile),
                failed(
                    &change(Knob::CpuGovernor, "powersave", "performance"),
                    "nope".into(),
                ),
            ],
            ..Report::default()
        };
        assert_eq!(report.state(), ReportState::Partial);
        assert_eq!(report.verified_count(), 1);
        assert_eq!(report.failed_count(), 1);
        assert_eq!(report.headline(), "1 of 2 optimizations verified");
    }

    #[test]
    fn total_failure_is_reported_as_failure() {
        let report = Report {
            applied: vec![failed(
                &change(Knob::CpuGovernor, "powersave", "performance"),
                "daemon unreachable".into(),
            )],
            ..Report::default()
        };
        assert_eq!(report.state(), ReportState::Failed);
        assert!(report.headline().contains("No optimization"));
    }

    #[test]
    fn change_summary_reads_as_a_transition() {
        assert_eq!(
            ok_change(Knob::PowerProfile).summary(),
            "Power profile: balanced → performance"
        );
    }
}
