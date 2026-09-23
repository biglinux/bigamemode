//! The standard result layout, and the report written from it.
//!
//! Every benchmark session lands in one directory with a fixed shape, so that a
//! result from six months ago can be read by the same code as one from today:
//!
//! ```text
//! benchmarks/2026-09-23-supertuxkart/
//!   system.json        the machine, as it was
//!   benchmark.json     what was run, and how
//!   baseline/run-01/   raw artifacts, one directory per run
//!   booster/run-01/
//!   comparison.json    the verdicts
//!   comparison.csv     the same, for a spreadsheet
//!   report.md          the same, for a person
//! ```
//!
//! The report is written to be read by someone deciding whether to trust it, so
//! it leads with the verdict and the evidence rather than a headline number,
//! and it states a regression in the same plain words it would use for a gain.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::result::{ArmSummary, Comparison, Verdict};

/// A complete benchmark session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    /// Schema version, so an old result stays readable.
    pub schema: String,
    /// Which workload was run.
    pub workload: String,
    /// What the numbers are — `avg_fps`, `score`.
    pub metric: String,
    /// When, as an ISO date.
    pub date: String,
    /// Machine fingerprint, matching `system.json`.
    pub fingerprint: String,
    /// Measured runs per arm, in the order taken. Warm-up already excluded.
    pub arms: BTreeMap<String, Vec<f64>>,
    /// Runs discarded before measuring.
    pub warmup_runs: usize,
    /// Whether arms were interleaved rather than grouped.
    pub alternating: bool,
    /// Anything that qualifies the result.
    pub caveats: Vec<String>,
}

impl Session {
    /// Compare every arm against the named baseline.
    ///
    /// The baseline arm is not compared with itself.
    #[must_use]
    pub fn compare(&self, baseline_arm: &str) -> Vec<Comparison> {
        let Some(baseline) = self
            .arms
            .get(baseline_arm)
            .and_then(|r| ArmSummary::new(baseline_arm, r.clone()))
        else {
            return Vec::new();
        };
        self.arms
            .iter()
            .filter(|(name, _)| name.as_str() != baseline_arm)
            .filter_map(|(name, runs)| {
                let candidate = ArmSummary::new(name.clone(), runs.clone())?;
                Some(Comparison::new(&self.metric, baseline.clone(), candidate))
            })
            .collect()
    }

    /// Write the whole layout to `dir`, which must already hold the run
    /// artifacts.
    ///
    /// # Errors
    /// Returns an error if any file cannot be written.
    pub fn write_layout(
        &self,
        dir: &Path,
        system: &Value,
        baseline_arm: &str,
    ) -> Result<Vec<Comparison>> {
        std::fs::create_dir_all(dir).with_context(|| format!("create {}", dir.display()))?;
        let comparisons = self.compare(baseline_arm);

        write_json(&dir.join("system.json"), system)?;
        write_json(&dir.join("benchmark.json"), self)?;
        write_json(&dir.join("comparison.json"), &comparisons)?;
        std::fs::write(dir.join("comparison.csv"), self.to_csv(&comparisons))
            .context("write comparison.csv")?;
        std::fs::write(dir.join("report.md"), self.to_markdown(&comparisons))
            .context("write report.md")?;
        Ok(comparisons)
    }

    /// The comparison as CSV, one row per arm.
    #[must_use]
    pub fn to_csv(&self, comparisons: &[Comparison]) -> String {
        let mut out =
            String::from("arm,runs,mean,median,min,max,stddev,cov_pct,delta_pct,verdict\n");
        // The baseline first, with no delta of its own to report.
        if let Some(first) = comparisons.first() {
            let b = &first.baseline;
            let _ = writeln!(
                out,
                "{},{},{:.2},{:.2},{:.2},{:.2},{:.2},{:.2},,baseline",
                b.arm,
                b.runs.len(),
                b.mean,
                b.median,
                b.min,
                b.max,
                b.stddev,
                b.cov * 100.0
            );
        }
        for c in comparisons {
            let a = &c.candidate;
            let _ = writeln!(
                out,
                "{},{},{:.2},{:.2},{:.2},{:.2},{:.2},{:.2},{:+.2},{:?}",
                a.arm,
                a.runs.len(),
                a.mean,
                a.median,
                a.min,
                a.max,
                a.stddev,
                a.cov * 100.0,
                c.delta_pct,
                c.verdict
            );
        }
        out
    }

    /// The report a person reads.
    #[must_use]
    #[allow(clippy::too_many_lines)]
    pub fn to_markdown(&self, comparisons: &[Comparison]) -> String {
        let mut out = String::new();
        let _ = writeln!(out, "# {} — {}\n", self.workload, self.date);

        if comparisons.is_empty() {
            out.push_str(
                "**NOT TESTED.** No arm produced enough runs to compare.\n\n\
                 This is recorded rather than omitted, so that a missing result \
                 is visibly missing rather than silently absent.\n",
            );
            return out;
        }

        // Lead with the verdict, because it is what the reader came for and
        // burying it under a table invites skipping to the biggest number.
        out.push_str("## Verdict\n\n");
        for c in comparisons {
            let mark = match c.verdict {
                Verdict::Improvement => "FASTER",
                Verdict::Regression => "SLOWER",
                Verdict::WithinNoise => "NO CHANGE",
                Verdict::Inconclusive => "INCONCLUSIVE",
            };
            let _ = writeln!(out, "- **{}** — {}: {}", c.candidate.arm, mark, c.rationale);
        }

        out.push_str("\n## Measurements\n\n");
        let _ = write!(
            out,
            "| Arm | Runs | Mean {m} | Median | Min | Max | Spread | vs baseline |\n\
             |---|---:|---:|---:|---:|---:|---:|---:|\n",
            m = self.metric
        );
        if let Some(first) = comparisons.first() {
            let b = &first.baseline;
            let _ = writeln!(
                out,
                "| {} | {} | {:.1} | {:.1} | {:.1} | {:.1} | {:.1}% | — |",
                b.arm,
                b.runs.len(),
                b.mean,
                b.median,
                b.min,
                b.max,
                b.cov * 100.0
            );
        }
        for c in comparisons {
            let a = &c.candidate;
            // A percentage is quoted only when the verdict supports it.
            let delta = match c.verdict {
                Verdict::Improvement | Verdict::Regression => format!("{:+.1}%", c.delta_pct),
                Verdict::WithinNoise => "within noise".into(),
                Verdict::Inconclusive => "inconclusive".into(),
            };
            let _ = writeln!(
                out,
                "| {} | {} | {:.1} | {:.1} | {:.1} | {:.1} | {:.1}% | {} |",
                a.arm,
                a.runs.len(),
                a.mean,
                a.median,
                a.min,
                a.max,
                a.cov * 100.0,
                delta
            );
        }

        out.push_str("\n## Method\n\n");
        let _ = writeln!(
            out,
            "- {} measured run(s) per arm, {} warm-up run(s) discarded.",
            comparisons.first().map_or(0, |c| c.baseline.runs.len()),
            self.warmup_runs
        );
        out.push_str(if self.alternating {
            "- Arms were alternated (A B A B …) rather than grouped, so that drift \
             over the session — chassis temperature above all — falls on both arms \
             equally instead of on whichever ran last.\n"
        } else {
            "- **Arms were run in groups, not alternated.** Any drift over the \
             session is confounded with the configuration; treat the result with \
             corresponding caution.\n"
        });
        out.push_str(
            "- A difference is called real only when it exceeds the run-to-run \
             spread of both arms *and* passes Welch's t-test at 95%. Anything \
             smaller is reported as no change, not as a small gain.\n",
        );
        let _ = writeln!(out, "- Machine fingerprint `{}`.", self.fingerprint);

        if !self.caveats.is_empty() {
            out.push_str("\n## Caveats\n\n");
            for caveat in &self.caveats {
                let _ = writeln!(out, "- {caveat}");
            }
        }

        out.push_str("\n## Raw runs\n\n");
        for (arm, runs) in &self.arms {
            let formatted: Vec<String> = runs.iter().map(|r| format!("{r:.1}")).collect();
            let _ = writeln!(out, "- `{arm}`: {}", formatted.join(", "));
        }
        out
    }
}

fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let text = serde_json::to_string_pretty(value)
        .with_context(|| format!("serialise {}", path.display()))?;
    std::fs::write(path, text).with_context(|| format!("write {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session() -> Session {
        let mut arms = BTreeMap::new();
        arms.insert("baseline".to_owned(), vec![400.0, 402.0, 398.0]);
        arms.insert("booster".to_owned(), vec![520.0, 524.0, 518.0]);
        Session {
            schema: "bigame.benchmark/1".into(),
            workload: "SuperTuxKart".into(),
            metric: "avg_fps".into(),
            date: "2026-09-23".into(),
            fingerprint: "0123456789abcdef".into(),
            arms,
            warmup_runs: 1,
            alternating: true,
            caveats: vec!["The frame cap was lifted for this test.".into()],
        }
    }

    #[test]
    fn the_baseline_is_not_compared_with_itself() {
        let comparisons = session().compare("baseline");
        assert_eq!(comparisons.len(), 1);
        assert_eq!(comparisons[0].candidate.arm, "booster");
    }

    #[test]
    fn an_unknown_baseline_yields_nothing_rather_than_a_wrong_answer() {
        assert!(session().compare("no-such-arm").is_empty());
    }

    #[test]
    fn a_gain_reaches_the_report_with_its_percentage() {
        let s = session();
        let c = s.compare("baseline");
        let md = s.to_markdown(&c);
        assert!(md.contains("FASTER"));
        // 520.667 against 400.0 is +30.2%.
        assert!(
            md.contains("+30.2%"),
            "the report must quote the measured delta"
        );
        assert!(md.contains("alternated"));
    }

    #[test]
    fn a_difference_within_noise_is_never_quoted_as_a_percentage() {
        let mut s = session();
        s.arms.insert("booster".into(), vec![405.0, 399.0, 403.0]);
        let c = s.compare("baseline");
        let md = s.to_markdown(&c);
        assert!(md.contains("NO CHANGE"));
        assert!(md.contains("within noise"));
        // The whole point: no "+0.5%" anywhere in the comparison column.
        assert!(
            !md.contains("| +"),
            "a noise-level delta must not be shown as a gain"
        );
    }

    #[test]
    fn a_regression_is_stated_as_plainly_as_a_gain() {
        let mut s = session();
        s.arms.insert("booster".into(), vec![300.0, 302.0, 298.0]);
        let c = s.compare("baseline");
        let md = s.to_markdown(&c);
        assert!(md.contains("SLOWER"));
        assert!(
            md.contains("-25"),
            "the size of the loss is reported, not hidden"
        );
    }

    #[test]
    fn an_empty_session_says_not_tested() {
        let mut s = session();
        s.arms.clear();
        let md = s.to_markdown(&[]);
        assert!(md.contains("NOT TESTED"));
    }

    #[test]
    fn grouped_runs_are_flagged_as_weaker_evidence() {
        let mut s = session();
        s.alternating = false;
        let c = s.compare("baseline");
        let md = s.to_markdown(&c);
        assert!(md.contains("not alternated"));
        assert!(md.contains("caution"));
    }

    #[test]
    fn csv_carries_every_arm_including_the_baseline() {
        let s = session();
        let csv = s.to_csv(&s.compare("baseline"));
        let lines: Vec<&str> = csv.lines().collect();
        assert_eq!(lines.len(), 3, "header plus both arms");
        assert!(lines[0].starts_with("arm,runs,mean"));
        assert!(lines[1].starts_with("baseline,3,"));
        assert!(lines[2].starts_with("booster,3,"));
        assert!(lines[2].contains("Improvement"));
    }

    #[test]
    fn the_layout_is_written_whole() {
        let dir = std::env::temp_dir().join(format!("bigame_lab_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);

        let s = session();
        let system = serde_json::json!({"schema": "bigame.system/1"});
        let comparisons = s.write_layout(&dir, &system, "baseline").unwrap();
        assert_eq!(comparisons.len(), 1);

        for name in [
            "system.json",
            "benchmark.json",
            "comparison.json",
            "comparison.csv",
            "report.md",
        ] {
            assert!(dir.join(name).is_file(), "{name} was not written");
        }

        // The session must survive a round trip, or an old result becomes
        // unreadable by a later build.
        let text = std::fs::read_to_string(dir.join("benchmark.json")).unwrap();
        let back: Session = serde_json::from_str(&text).unwrap();
        assert_eq!(back.arms, s.arms);
        assert_eq!(back.workload, "SuperTuxKart");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn caveats_reach_the_report() {
        let s = session();
        let md = s.to_markdown(&s.compare("baseline"));
        assert!(md.contains("frame cap was lifted"));
    }
}
