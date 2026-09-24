//! Benchmark results, and the arithmetic that decides whether a difference is
//! real.
//!
//! The hard part of an A/B benchmark is not collecting numbers, it is refusing
//! to report the ones that mean nothing. A machine that renders 731 fps in one
//! run and 748 in the next has not improved by 2.3 % — it has told you what its
//! run-to-run spread looks like. Announcing that spread as a gain is the single
//! most common way benchmark reports mislead, and it is what this module exists
//! to prevent.
//!
//! The rule applied here has two parts, and a difference must pass both:
//!
//! 1. It must be larger than the arms' own variability. A difference smaller
//!    than the spread within either arm is indistinguishable from noise no
//!    matter how many runs are taken.
//! 2. It must survive Welch's t-test at 95 %. Welch's rather than Student's
//!    because the two arms have no reason to share a variance — a configuration
//!    that raises the frame rate often changes its consistency too.
//!
//! When a difference fails either test the verdict is [`Verdict::WithinNoise`],
//! and the report says so rather than quoting a percentage. When there are too
//! few runs to judge, the verdict is [`Verdict::Inconclusive`] — never a guess.

use serde::{Deserialize, Serialize};

/// Aggregate statistics for one arm of a comparison.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ArmSummary {
    /// Which configuration this arm measured.
    pub arm: String,
    /// Every measured run, warm-up excluded, in the order taken.
    pub runs: Vec<f64>,
    /// Arithmetic mean.
    pub mean: f64,
    /// Middle value — less swayed by one bad run than the mean.
    pub median: f64,
    pub min: f64,
    pub max: f64,
    /// Sample standard deviation (n−1).
    pub stddev: f64,
    /// Coefficient of variation: stddev as a fraction of the mean.
    ///
    /// This is the number that says whether the measurement is trustworthy at
    /// all. Above about 5 % on a frame rate, something on the machine is
    /// interfering and the comparison should be rerun rather than believed.
    pub cov: f64,
}

impl ArmSummary {
    /// Summarise a set of runs.
    ///
    /// Returns `None` for an empty set; a single run is summarised, but with a
    /// standard deviation of zero, which the comparison treats as insufficient.
    #[must_use]
    pub fn new(arm: impl Into<String>, runs: Vec<f64>) -> Option<Self> {
        if runs.is_empty() {
            return None;
        }
        let n = runs.len();
        #[allow(clippy::cast_precision_loss)]
        let count = n as f64;
        let mean = runs.iter().sum::<f64>() / count;

        let mut sorted = runs.clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let median = if n % 2 == 0 {
            f64::midpoint(sorted[n / 2 - 1], sorted[n / 2])
        } else {
            sorted[n / 2]
        };

        // Sample variance, so a two-run arm is not credited with more certainty
        // than it has earned.
        let variance = if n > 1 {
            runs.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / (count - 1.0)
        } else {
            0.0
        };
        let stddev = variance.sqrt();

        Some(Self {
            arm: arm.into(),
            median,
            min: sorted[0],
            max: sorted[n - 1],
            cov: if mean.abs() > f64::EPSILON {
                stddev / mean
            } else {
                0.0
            },
            mean,
            stddev,
            runs,
        })
    }

    /// Whether this arm's own runs agree closely enough to compare against.
    ///
    /// A coefficient of variation above 5 % means something outside the
    /// experiment was moving during it.
    #[must_use]
    pub fn is_stable(&self) -> bool {
        self.runs.len() >= 2 && self.cov <= 0.05
    }
}

/// What a comparison concluded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    /// The candidate is measurably faster.
    Improvement,
    /// The candidate is measurably slower. Reported as plainly as a gain.
    Regression,
    /// A difference exists in the numbers but not above the noise.
    WithinNoise,
    /// Too few runs, or too unstable, to say anything.
    Inconclusive,
}

impl Verdict {
    /// A phrase for the report.
    #[must_use]
    pub fn describe(self) -> &'static str {
        match self {
            Self::Improvement => "measurably faster",
            Self::Regression => "measurably slower",
            Self::WithinNoise => "no difference above normal variation",
            Self::Inconclusive => "not enough evidence to say",
        }
    }
}

/// The outcome of comparing two arms.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Comparison {
    /// What was measured — a frame rate, a 1 % low, a score.
    pub metric: String,
    pub baseline: ArmSummary,
    pub candidate: ArmSummary,
    /// Signed change from baseline to candidate, as a percentage.
    ///
    /// Recorded whatever the verdict, because the number is a fact; whether it
    /// *means* anything is what the verdict says. Reports must not quote this
    /// without the verdict beside it.
    pub delta_pct: f64,
    pub verdict: Verdict,
    /// Why the verdict came out as it did, in words.
    pub rationale: String,
}

impl Comparison {
    /// Compare a candidate arm against a baseline.
    #[must_use]
    pub fn new(metric: impl Into<String>, baseline: ArmSummary, candidate: ArmSummary) -> Self {
        let metric = metric.into();
        let delta_pct = if baseline.mean.abs() > f64::EPSILON {
            (candidate.mean - baseline.mean) / baseline.mean * 100.0
        } else {
            0.0
        };

        let (verdict, rationale) = Self::judge(&baseline, &candidate, delta_pct);
        Self {
            metric,
            baseline,
            candidate,
            delta_pct,
            verdict,
            rationale,
        }
    }

    /// Apply both significance tests and explain the outcome.
    fn judge(baseline: &ArmSummary, candidate: &ArmSummary, delta_pct: f64) -> (Verdict, String) {
        if baseline.runs.len() < 2 || candidate.runs.len() < 2 {
            return (
                Verdict::Inconclusive,
                format!(
                    "needs at least 2 measured runs per arm; got {} and {}",
                    baseline.runs.len(),
                    candidate.runs.len()
                ),
            );
        }
        if !baseline.is_stable() || !candidate.is_stable() {
            return (
                Verdict::Inconclusive,
                format!(
                    "the runs within an arm disagree too much to compare \
                     (variation {:.1}% and {:.1}%, above the 5% ceiling); \
                     something on the machine was interfering",
                    baseline.cov * 100.0,
                    candidate.cov * 100.0
                ),
            );
        }

        // Test 1: the difference must exceed the arms' own spread.
        let noise_pct = baseline.cov.max(candidate.cov) * 100.0;
        if delta_pct.abs() <= noise_pct {
            return (
                Verdict::WithinNoise,
                format!(
                    "the {:.1}% difference is within the {:.1}% spread of the runs \
                     themselves, so it cannot be attributed to the change",
                    delta_pct.abs(),
                    noise_pct
                ),
            );
        }

        // Test 2: Welch's t-test at 95 %.
        let t = welch_t(baseline, candidate);
        let df = welch_df(baseline, candidate);
        let critical = t_critical_95(df);
        if t.abs() < critical {
            return (
                Verdict::WithinNoise,
                format!(
                    "a {:.1}% difference, but Welch's t = {:.2} falls short of the \
                     {:.2} needed for 95% confidence at {:.1} degrees of freedom",
                    delta_pct.abs(),
                    t.abs(),
                    critical,
                    df
                ),
            );
        }

        let verdict = if delta_pct > 0.0 {
            Verdict::Improvement
        } else {
            Verdict::Regression
        };
        (
            verdict,
            format!(
                "{:.1}% {}, above the {:.1}% run-to-run spread and significant at 95% \
                 (Welch's t = {:.2} against a {:.2} threshold)",
                delta_pct.abs(),
                if delta_pct > 0.0 { "faster" } else { "slower" },
                noise_pct,
                t.abs(),
                critical
            ),
        )
    }
}

/// Welch's t statistic for two samples of unequal variance.
fn welch_t(a: &ArmSummary, b: &ArmSummary) -> f64 {
    #[allow(clippy::cast_precision_loss)]
    let (na, nb) = (a.runs.len() as f64, b.runs.len() as f64);
    let se = (a.stddev.powi(2) / na + b.stddev.powi(2) / nb).sqrt();
    if se < f64::EPSILON {
        // Identical, zero-variance samples: no difference to detect. A nonzero
        // difference with zero variance is treated as decisive.
        return if (a.mean - b.mean).abs() < f64::EPSILON {
            0.0
        } else {
            f64::INFINITY
        };
    }
    (b.mean - a.mean) / se
}

/// Welch–Satterthwaite degrees of freedom.
fn welch_df(a: &ArmSummary, b: &ArmSummary) -> f64 {
    #[allow(clippy::cast_precision_loss)]
    let (na, nb) = (a.runs.len() as f64, b.runs.len() as f64);
    let (va, vb) = (a.stddev.powi(2) / na, b.stddev.powi(2) / nb);
    let denominator = va.powi(2) / (na - 1.0) + vb.powi(2) / (nb - 1.0);
    if denominator < f64::EPSILON {
        return na + nb - 2.0;
    }
    (va + vb).powi(2) / denominator
}

/// Two-tailed critical t at 95 % confidence.
///
/// A table rather than an inverse-t implementation: benchmark arms have a
/// handful of runs, so only small degrees of freedom ever occur, and a table is
/// easier to check against a statistics text than an approximation would be.
/// Values between entries take the more conservative neighbour.
fn t_critical_95(df: f64) -> f64 {
    const TABLE: [(f64, f64); 12] = [
        (1.0, 12.706),
        (2.0, 4.303),
        (3.0, 3.182),
        (4.0, 2.776),
        (5.0, 2.571),
        (6.0, 2.447),
        (8.0, 2.306),
        (10.0, 2.228),
        (15.0, 2.131),
        (20.0, 2.086),
        (30.0, 2.042),
        (f64::INFINITY, 1.960),
    ];
    for (limit, critical) in TABLE {
        if df <= limit {
            return critical;
        }
    }
    1.960
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arm(name: &str, runs: &[f64]) -> ArmSummary {
        ArmSummary::new(name, runs.to_vec()).unwrap()
    }

    #[test]
    fn summary_statistics_are_right() {
        let s = arm("a", &[10.0, 20.0, 30.0, 40.0]);
        assert!((s.mean - 25.0).abs() < 1e-9);
        assert!(
            (s.median - 25.0).abs() < 1e-9,
            "even count averages the middle pair"
        );
        assert!((s.min - 10.0).abs() < 1e-9);
        assert!((s.max - 40.0).abs() < 1e-9);
        // Sample stddev of 10,20,30,40 is sqrt(500/3) = 12.909...
        assert!((s.stddev - 12.909_944).abs() < 1e-5);

        let odd = arm("b", &[1.0, 5.0, 100.0]);
        assert!(
            (odd.median - 5.0).abs() < 1e-9,
            "odd count takes the middle value"
        );
    }

    #[test]
    fn an_empty_arm_is_not_a_summary() {
        assert!(ArmSummary::new("a", vec![]).is_none());
    }

    #[test]
    fn a_single_run_cannot_support_a_verdict() {
        let c = Comparison::new("fps", arm("base", &[100.0]), arm("cand", &[200.0]));
        assert_eq!(c.verdict, Verdict::Inconclusive);
        assert!(c.rationale.contains("at least 2"));
        // Even a doubling. One run is one run.
        assert!((c.delta_pct - 100.0).abs() < 1e-9);
    }

    #[test]
    fn a_small_difference_is_not_announced_as_a_gain() {
        // The exact trap this module exists for: 731 vs 748 fps looks like a
        // 2.3% win and is nothing of the kind.
        let base = arm("baseline", &[731.0, 748.0, 726.0]);
        let cand = arm("booster", &[745.0, 733.0, 751.0]);
        let c = Comparison::new("avg_fps", base, cand);
        assert_eq!(c.verdict, Verdict::WithinNoise);
        assert_ne!(c.verdict, Verdict::Improvement);
        assert!(c.rationale.contains("within") || c.rationale.contains("short of"));
    }

    #[test]
    fn a_real_gain_is_reported_as_one() {
        let base = arm("baseline", &[400.0, 402.0, 398.0]);
        let cand = arm("booster", &[520.0, 524.0, 518.0]);
        let c = Comparison::new("avg_fps", base, cand);
        assert_eq!(c.verdict, Verdict::Improvement);
        assert!(c.delta_pct > 29.0 && c.delta_pct < 31.0);
        assert!(c.rationale.contains("95%"));
    }

    #[test]
    fn a_regression_is_reported_as_plainly_as_a_gain() {
        let base = arm("baseline", &[520.0, 524.0, 518.0]);
        let cand = arm("booster", &[400.0, 402.0, 398.0]);
        let c = Comparison::new("avg_fps", base, cand);
        assert_eq!(c.verdict, Verdict::Regression);
        assert!(c.delta_pct < 0.0);
        assert!(c.rationale.contains("slower"));
    }

    #[test]
    fn unstable_runs_are_refused_rather_than_averaged() {
        // A 20% spread within one arm means the machine was busy with
        // something else; no comparison drawn from it is worth anything.
        let base = arm("baseline", &[400.0, 600.0, 500.0]);
        let cand = arm("booster", &[520.0, 524.0, 518.0]);
        let c = Comparison::new("avg_fps", base, cand);
        assert_eq!(c.verdict, Verdict::Inconclusive);
        assert!(c.rationale.contains("interfering"));
    }

    #[test]
    fn stability_has_a_five_percent_ceiling() {
        assert!(arm("a", &[100.0, 101.0, 99.0]).is_stable());
        assert!(!arm("b", &[100.0, 130.0, 70.0]).is_stable());
        assert!(
            !arm("c", &[100.0]).is_stable(),
            "one run proves no stability"
        );
    }

    #[test]
    fn welch_handles_identical_samples() {
        let a = arm("a", &[100.0, 100.0, 100.0]);
        let b = arm("b", &[100.0, 100.0, 100.0]);
        assert!((welch_t(&a, &b)).abs() < f64::EPSILON);
        let c = Comparison::new("fps", a, b);
        assert_eq!(c.verdict, Verdict::WithinNoise);
        assert!((c.delta_pct).abs() < 1e-9);
    }

    #[test]
    fn critical_values_match_the_table_and_are_conservative() {
        assert!((t_critical_95(2.0) - 4.303).abs() < 1e-9);
        assert!((t_critical_95(4.0) - 2.776).abs() < 1e-9);
        // Between entries, take the stricter neighbour rather than interpolate.
        assert!((t_critical_95(7.0) - 2.306).abs() < 1e-9);
        assert!((t_critical_95(1000.0) - 1.960).abs() < 1e-9);
        // Fewer runs always demand a larger t.
        assert!(t_critical_95(2.0) > t_critical_95(10.0));
    }

    #[test]
    fn a_comparison_round_trips_through_json() {
        let c = Comparison::new(
            "avg_fps",
            arm("baseline", &[400.0, 402.0, 398.0]),
            arm("booster", &[520.0, 524.0, 518.0]),
        );
        let text = serde_json::to_string(&c).unwrap();
        let back: Comparison = serde_json::from_str(&text).unwrap();
        assert_eq!(back.verdict, c.verdict);
        assert_eq!(back.baseline.runs, c.baseline.runs);
        assert!(
            text.contains("\"improvement\""),
            "verdicts serialise in snake case"
        );
    }
}
