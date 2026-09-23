//! Benchmark history, and detecting when the machine got slower on its own.
//!
//! A single A/B answers "did this setting help?". It cannot answer the question
//! that actually bites people, which is "why is this slower than it was last
//! month?" — because a kernel, a driver, a compositor or a Proton release
//! changed underneath, and nothing in the machine's configuration did.
//!
//! That question needs a record over time. This module keeps one: each
//! completed session appends its baseline arm, tagged with the machine
//! fingerprint and the kernel, and a later session can be checked against it.
//!
//! ## Why only the baseline arm is kept
//!
//! Comparing a booster-arm result against a past baseline-arm result would
//! confound a software regression with the effect of the Booster. The baseline
//! is the machine doing nothing special, which is the only arm whose meaning is
//! stable across sessions.
//!
//! ## Why a fingerprint mismatch ends the comparison
//!
//! A different CPU or GPU is a different machine, and a frame rate from one
//! says nothing about the other. A *kernel* change is different: it is exactly
//! what we want to detect, so it is recorded alongside each entry and reported
//! as a possible cause rather than used to discard the history.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use super::result::{ArmSummary, Comparison, Verdict};

/// One past session's baseline.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Entry {
    /// ISO date of the session.
    pub date: String,
    /// Which workload, and in which configuration — a CPU-bound and a
    /// GPU-bound run of the same game are not comparable.
    pub workload: String,
    /// Machine fingerprint at the time.
    pub fingerprint: String,
    /// Kernel at the time. The most common cause of an unexplained change.
    pub kernel: String,
    /// The baseline arm's measured runs.
    pub runs: Vec<f64>,
}

/// Every session recorded on this machine.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct History {
    /// Schema version.
    #[serde(default = "default_schema")]
    pub schema: String,
    /// Oldest first.
    #[serde(default)]
    pub entries: Vec<Entry>,
}

fn default_schema() -> String {
    "bigame.history/1".into()
}

impl Default for History {
    /// An empty history that already carries its schema version.
    ///
    /// Not derived: a derived `Default` would leave the schema an empty string,
    /// and a history saved from one would be unversioned on disk.
    fn default() -> Self {
        Self {
            schema: default_schema(),
            entries: Vec::new(),
        }
    }
}

/// What checking a new result against history concluded.
#[derive(Debug, Clone)]
pub enum Regression {
    /// Nothing comparable to check against.
    NoBaseline(String),
    /// The machine performs as it did.
    Stable(Box<Comparison>),
    /// It got measurably slower with no configuration change to explain it.
    Slower {
        /// The comparison that found it.
        comparison: Box<Comparison>,
        /// What changed between the two sessions, as candidate causes.
        changed: Vec<String>,
    },
    /// It got measurably faster. Worth saying, for the same reason.
    Faster(Box<Comparison>),
}

impl Regression {
    /// A sentence for the report.
    #[must_use]
    pub fn describe(&self) -> String {
        match self {
            Self::NoBaseline(why) => format!("No regression check: {why}"),
            Self::Stable(c) => format!("No change against the previous session: {}", c.rationale),
            Self::Faster(c) => format!(
                "Faster than the previous session by {:.1}%. {}",
                c.delta_pct, c.rationale
            ),
            Self::Slower {
                comparison,
                changed,
            } => {
                let causes = if changed.is_empty() {
                    "Nothing about the machine's identity changed, so the cause is \
                     software that does not appear in the fingerprint — a driver, \
                     a compositor or a game update."
                        .to_owned()
                } else {
                    format!("Changed since then: {}.", changed.join("; "))
                };
                format!(
                    "SLOWER than the previous session by {:.1}%. {} {causes}",
                    comparison.delta_pct.abs(),
                    comparison.rationale
                )
            }
        }
    }
}

impl History {
    /// Where history is kept for this user.
    #[must_use]
    pub fn default_path() -> Option<PathBuf> {
        let base = std::env::var_os("XDG_STATE_HOME")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var("HOME")
                    .ok()
                    .map(|h| PathBuf::from(h).join(".local/state"))
            })?;
        Some(base.join("bigame-mode").join("benchmark-history.json"))
    }

    /// Read the history, treating a missing file as an empty one.
    ///
    /// # Errors
    /// Returns an error only if the file exists and cannot be parsed.
    pub fn load(path: &Path) -> Result<Self> {
        let Ok(text) = std::fs::read_to_string(path) else {
            return Ok(Self::default());
        };
        serde_json::from_str(&text).with_context(|| format!("parse {}", path.display()))
    }

    /// Write the history back.
    ///
    /// # Errors
    /// Returns an error if the file cannot be written.
    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("create {}", parent.display()))?;
        }
        std::fs::write(path, serde_json::to_string_pretty(self)?)
            .with_context(|| format!("write {}", path.display()))
    }

    /// The most recent comparable entry, if there is one.
    ///
    /// Comparable means the same workload on the same hardware. The current
    /// session is excluded by date, so re-running the report on a session
    /// already recorded does not compare it with itself.
    #[must_use]
    pub fn latest_comparable(&self, entry: &Entry) -> Option<&Entry> {
        self.entries.iter().rev().find(|e| {
            e.workload == entry.workload
                && e.fingerprint == entry.fingerprint
                && e.date != entry.date
        })
    }

    /// Check a new baseline against the most recent comparable one.
    #[must_use]
    pub fn check(&self, entry: &Entry) -> Regression {
        let Some(previous) = self.latest_comparable(entry) else {
            return Regression::NoBaseline(format!(
                "no earlier session of {} on this hardware",
                entry.workload
            ));
        };
        let Some(before) = ArmSummary::new(
            format!("{} ({})", previous.date, "baseline"),
            previous.runs.clone(),
        ) else {
            return Regression::NoBaseline("the earlier session recorded no runs".into());
        };
        let Some(now) = ArmSummary::new("this session", entry.runs.clone()) else {
            return Regression::NoBaseline("this session recorded no runs".into());
        };

        let comparison = Comparison::new("avg_fps", before, now);
        match comparison.verdict {
            Verdict::Regression => {
                let mut changed = Vec::new();
                if previous.kernel != entry.kernel {
                    changed.push(format!(
                        "the kernel, from {} to {}",
                        previous.kernel, entry.kernel
                    ));
                }
                Regression::Slower {
                    comparison: Box::new(comparison),
                    changed,
                }
            }
            Verdict::Improvement => Regression::Faster(Box::new(comparison)),
            Verdict::WithinNoise | Verdict::Inconclusive => {
                Regression::Stable(Box::new(comparison))
            }
        }
    }

    /// Append an entry, replacing any for the same date, workload and machine.
    ///
    /// Re-running the report on a session must update its record, not add a
    /// second one that would then be compared against the first.
    pub fn record(&mut self, entry: Entry) {
        self.entries.retain(|e| {
            !(e.date == entry.date
                && e.workload == entry.workload
                && e.fingerprint == entry.fingerprint)
        });
        self.entries.push(entry);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(date: &str, kernel: &str, runs: &[f64]) -> Entry {
        Entry {
            date: date.into(),
            workload: "stk-gpu-bound".into(),
            fingerprint: "machine-one".into(),
            kernel: kernel.into(),
            runs: runs.to_vec(),
        }
    }

    #[test]
    fn with_no_history_there_is_nothing_to_check_against() {
        let history = History::default();
        let result = history.check(&entry("2026-09-23", "7.2.6", &[300.0, 301.0, 299.0]));
        assert!(matches!(result, Regression::NoBaseline(_)));
        assert!(result.describe().contains("no earlier session"));
    }

    #[test]
    fn a_real_slowdown_is_caught_and_the_kernel_is_named() {
        let mut history = History::default();
        history.record(entry("2026-08-01", "7.1.0", &[400.0, 402.0, 398.0]));

        let result = history.check(&entry("2026-09-23", "7.2.6", &[300.0, 302.0, 298.0]));
        let Regression::Slower {
            comparison,
            changed,
        } = &result
        else {
            panic!("expected a regression, got {result:?}");
        };
        assert!(comparison.delta_pct < -24.0);
        assert!(
            changed
                .iter()
                .any(|c| c.contains("7.1.0") && c.contains("7.2.6"))
        );
        assert!(result.describe().contains("SLOWER"));
    }

    #[test]
    fn a_slowdown_with_no_identity_change_says_so() {
        let mut history = History::default();
        history.record(entry("2026-08-01", "7.2.6", &[400.0, 402.0, 398.0]));
        let result = history.check(&entry("2026-09-23", "7.2.6", &[300.0, 302.0, 298.0]));
        // Same kernel, same hardware -- the cause is something the fingerprint
        // does not capture, and saying that is more useful than saying nothing.
        assert!(
            result
                .describe()
                .contains("does not appear in the fingerprint")
        );
    }

    #[test]
    fn noise_between_sessions_is_not_called_a_regression() {
        let mut history = History::default();
        history.record(entry("2026-08-01", "7.2.6", &[400.0, 402.0, 398.0]));
        let result = history.check(&entry("2026-09-23", "7.2.6", &[401.0, 399.0, 402.0]));
        assert!(matches!(result, Regression::Stable(_)));
    }

    #[test]
    fn a_different_machine_is_not_compared() {
        let mut history = History::default();
        history.record(entry("2026-08-01", "7.2.6", &[400.0, 402.0, 398.0]));
        let mut other = entry("2026-09-23", "7.2.6", &[300.0, 302.0, 298.0]);
        other.fingerprint = "machine-two".into();
        assert!(matches!(history.check(&other), Regression::NoBaseline(_)));
    }

    #[test]
    fn a_different_workload_configuration_is_not_compared() {
        let mut history = History::default();
        history.record(entry("2026-08-01", "7.2.6", &[400.0, 402.0, 398.0]));
        let mut other = entry("2026-09-23", "7.2.6", &[300.0, 302.0, 298.0]);
        // The same game at a different resolution is a different experiment.
        other.workload = "stk-cpu-bound".into();
        assert!(matches!(history.check(&other), Regression::NoBaseline(_)));
    }

    #[test]
    fn re_recording_a_session_replaces_it_rather_than_duplicating() {
        let mut history = History::default();
        history.record(entry("2026-09-23", "7.2.6", &[400.0]));
        history.record(entry("2026-09-23", "7.2.6", &[500.0]));
        assert_eq!(history.entries.len(), 1);
        assert_eq!(history.entries[0].runs, vec![500.0]);
    }

    #[test]
    fn a_session_is_never_compared_with_itself() {
        let mut history = History::default();
        let today = entry("2026-09-23", "7.2.6", &[400.0, 402.0, 398.0]);
        history.record(today.clone());
        assert!(matches!(history.check(&today), Regression::NoBaseline(_)));
    }

    #[test]
    fn an_improvement_over_time_is_reported_too() {
        let mut history = History::default();
        history.record(entry("2026-08-01", "7.1.0", &[300.0, 302.0, 298.0]));
        let result = history.check(&entry("2026-09-23", "7.2.6", &[400.0, 402.0, 398.0]));
        assert!(matches!(result, Regression::Faster(_)));
        assert!(result.describe().contains("Faster"));
    }

    #[test]
    fn history_round_trips_and_a_missing_file_is_empty() {
        let dir = std::env::temp_dir().join(format!("bigame_hist_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("benchmark-history.json");

        assert!(History::load(&path).unwrap().entries.is_empty());

        let mut history = History::default();
        history.record(entry("2026-08-01", "7.1.0", &[400.0, 402.0]));
        history.save(&path).unwrap();

        let back = History::load(&path).unwrap();
        assert_eq!(back.entries, history.entries);
        assert_eq!(back.schema, "bigame.history/1");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
