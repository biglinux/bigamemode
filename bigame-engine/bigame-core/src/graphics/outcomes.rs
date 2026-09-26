//! What was measured on this computer, and what it says about a plan.
//!
//! A plan's verdicts come from what is known in general: the game's own
//! upscaler, upstream documentation, the reference machine. A measurement on
//! *this* machine is better evidence than any of them, so it is kept: each
//! benchmark session that compares the game's own upscaler with `OptiScaler`
//! records its runs here, and the plan reads them back — with the same
//! significance tests as the benchmark reports ([`Comparison`]), never a
//! single number.
//!
//! The record is local: a file in the user's state directory, read by
//! nothing but this module, never sent anywhere. It holds a game key, a GPU
//! model, the setup and frame rates — no paths, no names.

use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use serde::{Deserialize, Serialize};

use crate::benchmark::result::{ArmSummary, Comparison, Verdict};

/// Current format.
pub const SCHEMA: u32 = 1;

/// What the game ran in one arm of a session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum Setup {
    /// No upscaler: the game's anti-aliasing at native resolution.
    NoUpscaler,
    /// The game's own upscaler (`dlss`, `fsr`, `xess`).
    Native {
        /// Which.
        upscaler: String,
    },
    /// `OptiScaler` running `output` in place of the game's `input`.
    #[serde(rename = "optiscaler")]
    OptiScaler {
        /// The game's upscaler it took over.
        input: String,
        /// What it ran.
        output: String,
    },
}

impl Setup {
    /// Parse `none`, `native:xess`, `optiscaler:xess:fsr`.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        let parts: Vec<&str> = s.split(':').map(str::trim).collect();
        let ok = |p: &str| !p.is_empty() && p.chars().all(|c| c.is_ascii_alphanumeric());
        match parts.as_slice() {
            ["none"] => Some(Self::NoUpscaler),
            ["native", u] if ok(u) => Some(Self::Native {
                upscaler: u.to_ascii_lowercase(),
            }),
            ["optiscaler", i, o] if ok(i) && ok(o) => Some(Self::OptiScaler {
                input: i.to_ascii_lowercase(),
                output: o.to_ascii_lowercase(),
            }),
            _ => None,
        }
    }
}

/// Which frames a measurement counted. Rendered and presented are never
/// compared with each other: a frame-generation arm presents frames that
/// were not rendered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Frames {
    /// Frames the game rendered (its own benchmark log).
    #[default]
    Rendered,
    /// Frames sent to the display (an overlay's log), generated ones
    /// included.
    Presented,
}

/// One arm of one session.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Measurement {
    /// ISO date.
    pub date: String,
    /// The game ([`super::manifest::game_key`]).
    pub game: String,
    /// GPU model, as the report names it.
    pub gpu: String,
    /// What ran.
    pub setup: Setup,
    /// Output resolution (`1920x1080`), when known.
    #[serde(default)]
    pub resolution: Option<String>,
    /// `OptiScaler` version, for its arms.
    #[serde(default)]
    pub optiscaler_version: Option<String>,
    /// Which frames were counted; absent in older records, which counted
    /// rendered frames.
    #[serde(default)]
    pub frames: Frames,
    /// Average frame rate of each measured run (warm-up excluded).
    pub avg_fps: Vec<f64>,
    /// 1 % low of each measured run.
    #[serde(default)]
    pub low_1pct: Vec<f64>,
}

/// Everything recorded on this machine.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Outcomes {
    /// Format.
    pub schema: u32,
    /// Oldest first.
    #[serde(default)]
    pub entries: Vec<Measurement>,
}

impl Default for Outcomes {
    fn default() -> Self {
        Self {
            schema: SCHEMA,
            entries: Vec::new(),
        }
    }
}

/// `$XDG_STATE_HOME/bigame-mode/graphics-outcomes.json`.
#[must_use]
pub fn path() -> PathBuf {
    crate::paths::state_home().join("bigame-mode/graphics-outcomes.json")
}

/// The record at `path` (empty when there is none or it cannot be read — a
/// damaged record costs evidence, never a plan).
#[must_use]
pub fn load(path: &Path) -> Outcomes {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

/// Append a session's arms to the record at `path`.
///
/// # Errors
/// Returns an error if the record cannot be written.
pub fn record(path: &Path, arms: &[Measurement]) -> Result<()> {
    let mut o = load(path);
    o.entries.extend_from_slice(arms);
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(&o)?)?;
    std::fs::rename(&tmp, path).with_context(|| format!("write {}", path.display()))?;
    tracing::info!(target: "graphics", arms = arms.len(), "graphics measurements recorded");
    Ok(())
}

/// The measurements for one game on one GPU.
#[must_use]
pub fn for_game<'a>(o: &'a Outcomes, game: &str, gpu: &str) -> Vec<&'a Measurement> {
    o.entries
        .iter()
        .filter(|m| m.game == game && m.gpu == gpu)
        .collect()
}

/// What this machine's measurements say about running `OptiScaler` in place
/// of the game's own upscaler.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Learned {
    /// The game's upscaler (the baseline, and `OptiScaler`'s input).
    pub input: String,
    /// What `OptiScaler` ran.
    pub output: String,
    /// Average frame rate, `OptiScaler` against the game's own.
    pub fps: Verdict,
    /// Change in average frame rate, percent — only meaningful beside `fps`.
    pub fps_change_pct: f64,
    /// 1 % low, the same way; `Inconclusive` when lows were not recorded.
    pub low: Verdict,
    /// Measured runs of the game's own upscaler.
    pub native_runs: usize,
    /// Measured runs of `OptiScaler`.
    pub optiscaler_runs: usize,
}

impl Learned {
    /// Faster here, and the frame-time floor shown to be no worse: worth
    /// recommending. A floor that varied too much to compare is not "no
    /// worse" — see [`Self::faster_floor_unknown`].
    #[must_use]
    pub fn better(&self) -> bool {
        self.fps == Verdict::Improvement
            && matches!(self.low, Verdict::Improvement | Verdict::WithinNoise)
    }

    /// Faster on average, but the 1 % low could not be compared: a gain to
    /// report, not one to recommend on its own.
    #[must_use]
    pub fn faster_floor_unknown(&self) -> bool {
        self.fps == Verdict::Improvement && self.low == Verdict::Inconclusive
    }

    /// Measured, and not better: slower, no faster than the noise, or a
    /// worse 1 % low.
    #[must_use]
    pub fn not_better(&self) -> bool {
        matches!(self.fps, Verdict::Regression | Verdict::WithinNoise)
            || self.low == Verdict::Regression
    }
}

fn pooled(ms: &[&Measurement], pick: fn(&Measurement) -> &Vec<f64>) -> Vec<f64> {
    ms.iter().flat_map(|m| pick(m).iter().copied()).collect()
}

/// Compare `OptiScaler` (taking over `input`) with the game's own `input`,
/// from `measurements` for one game and GPU. Runs of the same setup from
/// several sessions are pooled. `None` when either side was never measured.
#[must_use]
pub fn learned(measurements: &[&Measurement], input: &str) -> Option<Learned> {
    // Presented frames are not throughput: only rendered ones are compared.
    let measurements: Vec<&Measurement> = measurements
        .iter()
        .copied()
        .filter(|m| m.frames == Frames::Rendered)
        .collect();
    let measurements = measurements.as_slice();
    let native: Vec<&Measurement> = measurements
        .iter()
        .copied()
        .filter(|m| matches!(&m.setup, Setup::Native { upscaler } if upscaler == input))
        .collect();
    // The most-measured OptiScaler output for this input.
    let mut outputs: Vec<&str> = measurements
        .iter()
        .filter_map(|m| match &m.setup {
            Setup::OptiScaler { input: i, output } if i == input => Some(output.as_str()),
            _ => None,
        })
        .collect();
    outputs.sort_unstable();
    outputs.dedup();
    let output = outputs.into_iter().max_by_key(|o| {
        measurements
            .iter()
            .filter(|m| matches!(&m.setup, Setup::OptiScaler { output, .. } if output == o))
            .map(|m| m.avg_fps.len())
            .sum::<usize>()
    })?;
    let opti: Vec<&Measurement> = measurements
        .iter()
        .copied()
        .filter(|m| {
            matches!(&m.setup, Setup::OptiScaler { input: i, output: o } if i == input && o == output)
        })
        .collect();
    if native.is_empty() {
        return None;
    }
    let fps = Comparison::new(
        "avg_fps",
        ArmSummary::new("native", pooled(&native, |m| &m.avg_fps))?,
        ArmSummary::new("optiscaler", pooled(&opti, |m| &m.avg_fps))?,
    );
    let low = match (
        ArmSummary::new("native", pooled(&native, |m| &m.low_1pct)),
        ArmSummary::new("optiscaler", pooled(&opti, |m| &m.low_1pct)),
    ) {
        (Some(a), Some(b)) => Comparison::new("low_1pct", a, b).verdict,
        _ => Verdict::Inconclusive,
    };
    Some(Learned {
        input: input.to_owned(),
        output: output.to_owned(),
        native_runs: fps.baseline.runs.len(),
        optiscaler_runs: fps.candidate.runs.len(),
        fps: fps.verdict,
        fps_change_pct: fps.delta_pct,
        low,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(setup: &str, fps: &[f64], low: &[f64]) -> Measurement {
        Measurement {
            date: "2026-09-24".into(),
            game: "steam-750920".into(),
            gpu: "GP107M [GeForce GTX 1050 Ti Mobile]".into(),
            setup: Setup::parse(setup).unwrap(),
            resolution: Some("1920x1080".into()),
            optiscaler_version: None,
            frames: Frames::Rendered,
            avg_fps: fps.to_vec(),
            low_1pct: low.to_vec(),
        }
    }

    #[test]
    fn setups_parse_from_short_names_and_nothing_else() {
        assert_eq!(Setup::parse("none"), Some(Setup::NoUpscaler));
        assert_eq!(
            Setup::parse("native:XeSS"),
            Some(Setup::Native {
                upscaler: "xess".into()
            })
        );
        assert_eq!(
            Setup::parse("optiscaler:xess:fsr"),
            Some(Setup::OptiScaler {
                input: "xess".into(),
                output: "fsr".into()
            })
        );
        for bad in [
            "",
            "native",
            "native:",
            "optiscaler:xess",
            "native:../x",
            "x:y",
        ] {
            assert_eq!(Setup::parse(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn a_clear_gain_with_the_floor_intact_is_better() {
        // The reference machine's numbers (docs/31): XeSS Quality vs OptiScaler
        // FSR from XeSS Quality.
        let ms = [
            m("native:xess", &[93.9, 94.2, 94.4], &[62.3, 66.3, 64.8]),
            m(
                "optiscaler:xess:fsr",
                &[99.0, 98.7, 98.7],
                &[61.0, 60.5, 60.9],
            ),
        ];
        let refs: Vec<&Measurement> = ms.iter().collect();
        let l = learned(&refs, "xess").unwrap();
        assert_eq!(l.fps, Verdict::Improvement);
        assert!((l.fps_change_pct - 4.9).abs() < 0.2, "{}", l.fps_change_pct);
        assert_ne!(l.low, Verdict::Regression);
        assert!(l.better() && !l.not_better());
        assert_eq!(
            (l.output.as_str(), l.native_runs, l.optiscaler_runs),
            ("fsr", 3, 3)
        );
    }

    #[test]
    fn a_gain_with_a_floor_too_scattered_to_compare_is_reported_not_recommended() {
        // The lab laptop's GTX 1050 Ti session (docs/31): two launches of the
        // game's XeSS, one of OptiScaler FSR; average clearly up, 1 % lows
        // varying 9 % run to run.
        let ms = [
            m("native:xess", &[15.9, 15.7, 15.5], &[9.2, 11.0, 10.3]),
            m("native:xess", &[16.6, 16.8], &[11.9, 10.6]),
            m(
                "optiscaler:xess:fsr",
                &[18.2, 18.3, 18.3],
                &[10.5, 10.2, 8.7],
            ),
        ];
        let l = learned(&ms.iter().collect::<Vec<_>>(), "xess").unwrap();
        assert_eq!(
            (l.fps, l.low),
            (Verdict::Improvement, Verdict::Inconclusive)
        );
        assert!(l.faster_floor_unknown());
        assert!(!l.better() && !l.not_better());
        assert_eq!((l.native_runs, l.optiscaler_runs), (5, 3));
    }

    #[test]
    fn noise_a_loss_or_a_worse_floor_is_not_better() {
        let noise = [
            m("native:xess", &[50.0, 52.0, 51.0], &[]),
            m("optiscaler:xess:fsr", &[51.0, 50.5, 51.5], &[]),
        ];
        let l = learned(&noise.iter().collect::<Vec<_>>(), "xess").unwrap();
        assert_eq!(l.fps, Verdict::WithinNoise);
        assert!(!l.better() && l.not_better());

        let floor = [
            m("native:xess", &[50.0, 50.2, 50.1], &[40.0, 40.1, 40.2]),
            m(
                "optiscaler:xess:fsr",
                &[55.0, 55.1, 55.2],
                &[30.0, 30.2, 30.1],
            ),
        ];
        let l = learned(&floor.iter().collect::<Vec<_>>(), "xess").unwrap();
        assert_eq!((l.fps, l.low), (Verdict::Improvement, Verdict::Regression));
        assert!(!l.better() && l.not_better());
    }

    #[test]
    fn one_side_unmeasured_or_one_run_says_nothing() {
        let only_opti = [m("optiscaler:xess:fsr", &[60.0, 61.0], &[])];
        assert!(learned(&only_opti.iter().collect::<Vec<_>>(), "xess").is_none());
        let one_run = [
            m("native:xess", &[50.0], &[]),
            m("optiscaler:xess:fsr", &[60.0], &[]),
        ];
        let l = learned(&one_run.iter().collect::<Vec<_>>(), "xess").unwrap();
        assert_eq!(l.fps, Verdict::Inconclusive);
        assert!(!l.better() && !l.not_better() && !l.faster_floor_unknown());
    }

    #[test]
    fn the_record_appends_and_filters_by_game_and_gpu() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("state/graphics-outcomes.json");
        assert_eq!(load(&p), Outcomes::default());
        record(&p, &[m("native:xess", &[1.0, 2.0], &[])]).unwrap();
        let mut other = m("native:xess", &[3.0, 4.0], &[]);
        other.gpu = "AD104 [GeForce RTX 4070 Ti]".into();
        record(&p, &[other]).unwrap();
        let o = load(&p);
        assert_eq!(o.entries.len(), 2);
        assert_eq!(
            for_game(&o, "steam-750920", "GP107M [GeForce GTX 1050 Ti Mobile]").len(),
            1
        );
        assert!(for_game(&o, "steam-1", "x").is_empty());
        let text = std::fs::read_to_string(&p).unwrap();
        assert!(!text.contains('/'), "no paths in the record:\n{text}");
    }
}
