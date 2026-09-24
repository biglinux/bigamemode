//! Hardware calibration: deciding which settings help *this* machine.
//!
//! The premise of the rest of this crate is that a setting's effect has to be
//! measured rather than assumed. This module is where that premise becomes
//! actionable: it records what was measured per knob, on which machine, and
//! answers the only question the Booster actually needs answered — should this
//! knob be applied here?
//!
//! ## Why a knob that sounds faster can be slower
//!
//! The concrete case this was built around: forcing a Radeon's DPM level to
//! `high` pins it to its top fixed clock state and takes the firmware's
//! opportunistic boost algorithm out of the loop. On a card whose top *fixed*
//! state sits below the boost clock the automatic algorithm reaches, or whose
//! power limit is hit sooner at the forced state, "high" is slower than "auto".
//! Nothing about the name suggests that, and no amount of reasoning from first
//! principles settles it. Only a measurement does.
//!
//! ## What is stored, and why it expires
//!
//! A calibration is tied to a hardware fingerprint. A new kernel, a driver
//! update or a different GPU can all reverse a result, so a calibration whose
//! fingerprint no longer matches the machine is not used — it is discarded and
//! re-measured. Stale calibration is worse than none, because it carries the
//! authority of a measurement without the truth of one.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use super::result::{Comparison, Verdict};

/// What measurement concluded about one knob on this machine.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct KnobFinding {
    /// The knob, by the name the isolation matrix used for it.
    pub knob: String,
    /// The verdict, in the same terms the comparison produced.
    pub verdict: Verdict,
    /// The measured change, whatever the verdict.
    pub delta_pct: f64,
    /// The workload it was measured against.
    pub workload: String,
    /// Why the verdict came out as it did.
    pub rationale: String,
}

impl KnobFinding {
    /// Whether the Booster should apply this knob.
    ///
    /// Only a measured improvement earns application. A measured regression is
    /// refused outright; so is anything within noise, on the reasoning that a
    /// knob with no demonstrated benefit is not worth the risk of changing the
    /// machine's state, however small that risk is.
    #[must_use]
    pub fn should_apply(&self) -> bool {
        self.verdict == Verdict::Improvement
    }

    /// Whether this knob was shown to actively hurt.
    #[must_use]
    pub fn is_harmful(&self) -> bool {
        self.verdict == Verdict::Regression
    }
}

/// Everything measured on one machine.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Calibration {
    /// Schema version.
    pub schema: String,
    /// The machine this describes. A calibration is void when this changes.
    pub fingerprint: String,
    /// ISO date of the measurement.
    pub measured: String,
    /// One finding per knob.
    pub findings: BTreeMap<String, KnobFinding>,
}

impl Calibration {
    /// A new, empty calibration for a machine.
    #[must_use]
    pub fn new(fingerprint: impl Into<String>, measured: impl Into<String>) -> Self {
        Self {
            schema: "bigame.calibration/1".into(),
            fingerprint: fingerprint.into(),
            measured: measured.into(),
            findings: BTreeMap::new(),
        }
    }

    /// Record what an isolation-matrix comparison found.
    ///
    /// The candidate arm's name is the knob's name, which is what ties the
    /// matrix to the Booster's plan without a second mapping to keep in sync.
    pub fn record(&mut self, workload: &str, comparison: &Comparison) {
        let knob = comparison.candidate.arm.clone();
        self.findings.insert(
            knob.clone(),
            KnobFinding {
                knob,
                verdict: comparison.verdict,
                delta_pct: comparison.delta_pct,
                workload: workload.to_owned(),
                rationale: comparison.rationale.clone(),
            },
        );
    }

    /// Whether this calibration still describes the machine in front of us.
    #[must_use]
    pub fn applies_to(&self, fingerprint: &str) -> bool {
        self.fingerprint == fingerprint
    }

    /// What measurement says about one knob, if it was measured.
    ///
    /// Returns `None` for a knob never tested. The caller must treat that as
    /// "unknown", never as "safe" — an untested knob has no evidence either
    /// way, and this module's whole purpose is to keep those two apart.
    #[must_use]
    pub fn finding(&self, knob: &str) -> Option<&KnobFinding> {
        self.findings.get(knob)
    }

    /// Knobs measurement showed to help, worst-first by nothing in particular.
    #[must_use]
    pub fn beneficial(&self) -> Vec<&KnobFinding> {
        self.findings
            .values()
            .filter(|f| f.should_apply())
            .collect()
    }

    /// Knobs measurement showed to hurt. These must not be applied.
    #[must_use]
    pub fn harmful(&self) -> Vec<&KnobFinding> {
        self.findings.values().filter(|f| f.is_harmful()).collect()
    }

    /// Where a calibration is kept for this user.
    #[must_use]
    pub fn default_path() -> Option<PathBuf> {
        let base = std::env::var_os("XDG_STATE_HOME")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var("HOME")
                    .ok()
                    .map(|h| PathBuf::from(h).join(".local/state"))
            })?;
        Some(base.join("bigame-mode").join("calibration.json"))
    }

    /// Load a calibration, but only if it describes this machine.
    ///
    /// A fingerprint mismatch returns `Ok(None)` rather than an error: an
    /// out-of-date calibration is an ordinary situation, not a fault, and the
    /// right response is to measure again.
    ///
    /// # Errors
    /// Returns an error only if the file exists but cannot be parsed.
    pub fn load(path: &Path, fingerprint: &str) -> Result<Option<Self>> {
        let Ok(text) = std::fs::read_to_string(path) else {
            return Ok(None);
        };
        let calibration: Self =
            serde_json::from_str(&text).with_context(|| format!("parse {}", path.display()))?;
        Ok(calibration.applies_to(fingerprint).then_some(calibration))
    }

    /// Write the calibration, creating the directory if needed.
    ///
    /// # Errors
    /// Returns an error if the file cannot be written.
    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("create {}", parent.display()))?;
        }
        let text = serde_json::to_string_pretty(self).context("serialise calibration")?;
        std::fs::write(path, text).with_context(|| format!("write {}", path.display()))
    }

    /// A summary for the report and the diagnostics page.
    #[must_use]
    pub fn describe(&self) -> String {
        if self.findings.is_empty() {
            return "No knob has been measured on this machine yet.".into();
        }
        let helped = self.beneficial().len();
        let hurt = self.harmful().len();
        let neutral = self.findings.len() - helped - hurt;
        format!(
            "Measured {} setting(s) on {}: {helped} helped, {hurt} hurt, \
             {neutral} made no measurable difference.",
            self.findings.len(),
            self.measured
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::benchmark::result::ArmSummary;

    fn comparison(arm: &str, base: &[f64], cand: &[f64]) -> Comparison {
        Comparison::new(
            "avg_fps",
            ArmSummary::new("baseline", base.to_vec()).unwrap(),
            ArmSummary::new(arm, cand.to_vec()).unwrap(),
        )
    }

    #[test]
    fn only_a_measured_improvement_earns_application() {
        let mut c = Calibration::new("abc", "2026-09-23");
        c.record(
            "stk",
            &comparison("helps", &[400.0, 402.0, 398.0], &[520.0, 524.0, 518.0]),
        );
        c.record(
            "stk",
            &comparison("hurts", &[520.0, 524.0, 518.0], &[400.0, 402.0, 398.0]),
        );
        c.record(
            "stk",
            &comparison("neutral", &[400.0, 402.0, 398.0], &[401.0, 399.0, 402.0]),
        );

        assert!(c.finding("helps").unwrap().should_apply());
        assert!(!c.finding("hurts").unwrap().should_apply());
        // The point worth being explicit about: no measured benefit means no
        // application, not "apply it, it probably doesn't matter".
        assert!(!c.finding("neutral").unwrap().should_apply());
        assert!(c.finding("hurts").unwrap().is_harmful());
        assert!(!c.finding("neutral").unwrap().is_harmful());
    }

    #[test]
    fn an_untested_knob_is_unknown_not_safe() {
        let c = Calibration::new("abc", "2026-09-23");
        assert!(c.finding("never-measured").is_none());
        assert!(c.beneficial().is_empty());
        assert!(c.harmful().is_empty());
    }

    #[test]
    fn a_calibration_from_another_machine_is_not_used() {
        let c = Calibration::new("machine-one", "2026-09-23");
        assert!(c.applies_to("machine-one"));
        assert!(!c.applies_to("machine-two"));
    }

    #[test]
    fn a_stale_calibration_is_discarded_rather_than_trusted() {
        let dir = std::env::temp_dir().join(format!("bigame_cal_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("calibration.json");

        let mut c = Calibration::new("machine-one", "2026-09-23");
        c.record(
            "stk",
            &comparison("gpu", &[400.0, 402.0, 398.0], &[520.0, 524.0, 518.0]),
        );
        c.save(&path).unwrap();

        // Same machine: the calibration comes back.
        let loaded = Calibration::load(&path, "machine-one").unwrap().unwrap();
        assert!(loaded.finding("gpu").unwrap().should_apply());

        // Different machine -- a new kernel, a new GPU: nothing is returned.
        assert!(Calibration::load(&path, "machine-two").unwrap().is_none());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_file_is_not_an_error() {
        let path = std::env::temp_dir().join("bigame-no-such-calibration.json");
        let _ = std::fs::remove_file(&path);
        assert!(Calibration::load(&path, "any").unwrap().is_none());
    }

    #[test]
    fn a_corrupt_file_is_an_error_rather_than_silently_empty() {
        let dir = std::env::temp_dir().join(format!("bigame_cal_bad_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("calibration.json");
        std::fs::write(&path, "{ not json").unwrap();
        assert!(Calibration::load(&path, "any").is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn re_measuring_a_knob_replaces_the_old_finding() {
        let mut c = Calibration::new("abc", "2026-09-23");
        c.record(
            "stk",
            &comparison("gpu", &[400.0, 402.0, 398.0], &[520.0, 524.0, 518.0]),
        );
        assert!(c.finding("gpu").unwrap().should_apply());
        // A driver update reverses the result; the new measurement wins.
        c.record(
            "stk",
            &comparison("gpu", &[520.0, 524.0, 518.0], &[400.0, 402.0, 398.0]),
        );
        assert!(c.finding("gpu").unwrap().is_harmful());
        assert_eq!(c.findings.len(), 1, "the knob is replaced, not duplicated");
    }

    #[test]
    fn the_summary_counts_all_three_outcomes() {
        let mut c = Calibration::new("abc", "2026-09-23");
        assert!(c.describe().contains("No knob has been measured"));

        c.record(
            "stk",
            &comparison("helps", &[400.0, 402.0, 398.0], &[520.0, 524.0, 518.0]),
        );
        c.record(
            "stk",
            &comparison("hurts", &[520.0, 524.0, 518.0], &[400.0, 402.0, 398.0]),
        );
        c.record(
            "stk",
            &comparison("neutral", &[400.0, 402.0, 398.0], &[401.0, 399.0, 402.0]),
        );
        let text = c.describe();
        assert!(text.contains("1 helped"), "{text}");
        assert!(text.contains("1 hurt"), "{text}");
        assert!(text.contains("1 made no measurable difference"), "{text}");
    }

    #[test]
    fn a_calibration_round_trips() {
        let mut c = Calibration::new("abc", "2026-09-23");
        c.record(
            "stk",
            &comparison("gpu", &[400.0, 402.0, 398.0], &[520.0, 524.0, 518.0]),
        );
        let text = serde_json::to_string(&c).unwrap();
        let back: Calibration = serde_json::from_str(&text).unwrap();
        assert_eq!(back.fingerprint, "abc");
        assert_eq!(back.finding("gpu"), c.finding("gpu"));
    }
}
