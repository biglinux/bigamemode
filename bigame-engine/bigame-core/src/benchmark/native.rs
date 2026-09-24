//! Results written by games' own built-in benchmarks.
//!
//! Three of the installed titles record every frame of their benchmark to
//! disk, which is better evidence than anything an overlay can capture: the
//! timing is taken by the engine, the pass covers exactly the benchmark and
//! nothing else, and no layer is injected into the game to get it.
//!
//! Two formats cover them:
//!
//! - **Crystal Dynamics / Eidos** (*Shadow of the Tomb Raider*, *Rise of the
//!   Tomb Raider*): a `*_frametimes_*.txt` table of frame, cumulative time and
//!   delta, beside a summary `.txt` whose `Settings:` block lists every
//!   graphics option the run used.
//! - **`REDengine` 4** (*Cyberpunk 2077*): a `benchmark_*` directory holding
//!   `frames.csv` and `summary.json`.
//!
//! Three things are done to the numbers, each for a stated reason:
//!
//! 1. **Scene transitions are set aside, and counted.** *Shadow of the Tomb
//!    Raider* loads between the sections of its benchmark, and the loading
//!    screen arrives as a single frame of several seconds — 7.8 s in a real run
//!    here, which is why the game itself prints `Min FPS: 0.0`. Left in, that
//!    one frame would *be* the 0.1 % low, and its length depends on disk and
//!    cache state rather than on anything a run is meant to compare.
//! 2. **The settings travel with the result.** Two runs at different
//!    resolutions or presets are different experiments. [`settings_differ`]
//!    makes that checkable instead of a thing to remember.
//! 3. **Frame generation is recorded.** Generated frames are not rendered
//!    frames; a run with it on is never comparable to one with it off, and is
//!    labelled so it cannot be mistaken for a throughput result.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::{Context, Result};

use super::Capture;

/// A frame this long is a loading screen, not a rendered frame.
///
/// One second is well clear of any hitch a player would still call gameplay —
/// the longest non-transition frame in a real *Shadow of the Tomb Raider* run
/// here was 62 ms — and well short of the 7.8 s scene load.
pub const TRANSITION_MS: f64 = 1000.0;

/// One run of a game's built-in benchmark, as read from its own files.
#[derive(Debug, Clone, Default)]
pub struct NativeRun {
    /// Rendered frames, transitions removed.
    pub capture: Capture,
    /// Frames set aside as scene transitions.
    pub transitions: usize,
    /// Every setting the game recorded for this run.
    pub settings: BTreeMap<String, String>,
    /// The average the game itself printed, kept to cross-check the parse.
    pub reported_avg_fps: Option<f64>,
    /// Whether any form of frame generation was active.
    pub frame_generation: bool,
    /// The files this was read from.
    pub files: Vec<PathBuf>,
}

impl NativeRun {
    /// A one-line account of the run, for a report or a log.
    #[must_use]
    pub fn describe(&self) -> String {
        let mut parts = vec![format!("{} frames", self.capture.frametimes_ms.len())];
        if self.transitions > 0 {
            parts.push(format!(
                "{} scene transition{} excluded",
                self.transitions,
                if self.transitions == 1 { "" } else { "s" }
            ));
        }
        if let Some(fps) = self.reported_avg_fps {
            parts.push(format!("game reported {fps:.1} fps average"));
        }
        if self.frame_generation {
            parts.push("frame generation ON: presented, not rendered, frames".into());
        }
        parts.join("; ")
    }
}

// ── Crystal Dynamics / Eidos ─────────────────────────────────────────────────

/// Parse a `*_frametimes_*.txt` table.
///
/// ```text
/// Frame, Time (ms), Delta (ms) , Memory (mb)
///     1, 0.000, 0.000,2691
///     2,11.216,11.216,5480
/// ```
///
/// The memory column is present in *Shadow* and absent in *Rise*; columns are
/// found by name. The first row's delta is always zero and is not a frame.
///
/// # Errors
/// Returns an error when no `Delta` column is found.
pub fn parse_crystal_frametimes(content: &str) -> Result<(Capture, usize)> {
    let mut lines = content.lines();
    let header = lines
        .by_ref()
        .find(|l| l.contains("Delta"))
        .context("no Delta column; not a Crystal Dynamics frametime log")?;
    let delta = header
        .split(',')
        .position(|c| c.trim().starts_with("Delta"))
        .context("no Delta column")?;
    let frametimes =
        lines.filter_map(|line| line.split(',').nth(delta)?.trim().parse::<f64>().ok());
    Ok(split_transitions(frametimes))
}

/// Parse the summary `.txt` written beside the frametimes.
///
/// Returns the `Settings:` block as key/value pairs and the game's own average.
#[must_use]
pub fn parse_crystal_summary(content: &str) -> (BTreeMap<String, String>, Option<f64>) {
    let mut settings = BTreeMap::new();
    let mut in_settings = false;
    let mut average = None;
    for line in content.lines() {
        let line = line.trim();
        if line == "Settings:" {
            in_settings = true;
            continue;
        }
        if in_settings {
            if let Some((k, v)) = line.split_once('=') {
                settings.insert(k.trim().to_string(), v.trim().to_string());
            }
        } else if average.is_none() {
            // The first "Average FPS" is the run's; later ones repeat it.
            average = line
                .strip_prefix("Average FPS:")
                .and_then(|v| v.trim().parse().ok());
        }
    }
    (settings, average)
}

/// Read one Crystal Dynamics run from its frametimes file.
///
/// The summary is found by removing `_frametimes` from the name, which is how
/// both games name the pair.
///
/// # Errors
/// Returns an error when the frametimes file cannot be read or parsed.
pub fn read_crystal(frametimes: &Path) -> Result<NativeRun> {
    let content = std::fs::read_to_string(frametimes)
        .with_context(|| format!("read {}", frametimes.display()))?;
    let (capture, transitions) = parse_crystal_frametimes(&content)?;
    let mut run = NativeRun {
        capture,
        transitions,
        files: vec![frametimes.to_path_buf()],
        ..NativeRun::default()
    };
    let name = frametimes.file_name().unwrap_or_default().to_string_lossy();
    let summary = frametimes.with_file_name(name.replace("_frametimes", ""));
    if let Ok(text) = std::fs::read_to_string(&summary) {
        (run.settings, run.reported_avg_fps) = parse_crystal_summary(&text);
        run.frame_generation = [
            "DLSSFrameGeneration",
            "FSRFrameGeneration",
            "FrameGeneration",
        ]
        .iter()
        .any(|k| {
            run.settings
                .get(*k)
                .is_some_and(|v| v != "0" && v != "false")
        });
        run.files.push(summary);
    }
    Ok(run)
}

/// Read one run that the game split across several scene files.
///
/// Rise of the Tomb Raider writes one frametimes/summary pair per scene
/// (Spine of the Mountain, Prophet's Tomb, Geothermal Valley); the run is all
/// of them. Frames are concatenated in the order given, transitions summed,
/// and the settings taken from the first scene — the report refuses a session
/// whose settings differ, so scenes of one run always agree. The game's own
/// per-scene averages are not combined into one figure.
///
/// # Errors
/// Returns an error if any file cannot be read.
pub fn read_crystal_run(files: &[PathBuf]) -> Result<NativeRun> {
    let mut run = NativeRun::default();
    for (i, file) in files.iter().enumerate() {
        let scene = read_crystal(file)?;
        if i == 0 {
            run.settings = scene.settings;
            run.frame_generation = scene.frame_generation;
            run.reported_avg_fps = scene.reported_avg_fps;
        } else {
            run.reported_avg_fps = None;
            run.frame_generation |= scene.frame_generation;
        }
        run.capture
            .frametimes_ms
            .extend(scene.capture.frametimes_ms);
        run.capture.duration_s += scene.capture.duration_s;
        run.transitions += scene.transitions;
        run.files.extend(scene.files);
    }
    Ok(run)
}

/// Every entry in `dir` whose name matches, modified at or after `since`,
/// oldest first.
///
/// # Errors
/// Returns an error when the directory cannot be read.
pub fn all_since(
    dir: &Path,
    matches: impl Fn(&str) -> bool,
    since: SystemTime,
) -> Result<Vec<PathBuf>> {
    let mut found: Vec<(SystemTime, PathBuf)> = std::fs::read_dir(dir)
        .with_context(|| format!("read {}", dir.display()))?
        .flatten()
        .filter(|e| matches(&e.file_name().to_string_lossy()))
        .filter_map(|e| {
            let modified = e.metadata().and_then(|m| m.modified()).ok()?;
            (modified >= since).then(|| (modified, e.path()))
        })
        .collect();
    found.sort();
    Ok(found.into_iter().map(|(_, p)| p).collect())
}

// ── REDengine 4 ──────────────────────────────────────────────────────────────

/// Parse `Cyberpunk 2077`'s `frames.csv`.
///
/// ```text
/// Frame index, Frame time (ms), DLSS Frame Generation (ms), FSR3 Frame Generation (ms), …
/// 0, 31.00, 31.00, 31.00, 31.00
/// ```
///
/// Only `Frame time (ms)` is read. The per-vendor frame generation columns
/// repeat it when generation is off, and whether it was on is taken from
/// `summary.json`, which states it, rather than inferred from them.
///
/// # Errors
/// Returns an error when the header has no frame time column.
pub fn parse_cyberpunk_frames(content: &str) -> Result<(Capture, usize)> {
    let mut lines = content.lines();
    let header = lines.next().context("empty frames.csv")?;
    let column = header
        .split(',')
        .position(|c| c.trim() == "Frame time (ms)")
        .context("no 'Frame time (ms)' column; not a Cyberpunk 2077 benchmark")?;
    let frametimes =
        lines.filter_map(|line| line.split(',').nth(column)?.trim().parse::<f64>().ok());
    Ok(split_transitions(frametimes))
}

/// Summary fields that decide whether two runs are the same experiment.
const CYBERPUNK_SETTINGS: &[&str] = &[
    "gameVersion",
    "presetName",
    "textureQualityPresetLocalizedName",
    "renderWidth",
    "renderHeight",
    "windowMode",
    "verticalSync",
    "fpsClamp",
    "upscalingType",
    "frameGenerationType",
    "DLSSEnabled",
    "FSR2Enabled",
    "FSR2Quality",
    "FSR3Enabled",
    "FSR3Quality",
    "FSR4Enabled",
    "XeSSEnabled",
    "DRSEnabled",
    "rayTracingEnabled",
    "rayTracedPathTracingEnabled",
    "rayTracedLightingQuality",
];

/// Parse `summary.json`: the settings that define the experiment, the game's
/// own average, and whether frame generation was on.
///
/// # Errors
/// Returns an error when the file is not the expected JSON document.
pub fn parse_cyberpunk_summary(
    json: &str,
) -> Result<(BTreeMap<String, String>, Option<f64>, bool)> {
    let doc: serde_json::Value = serde_json::from_str(json).context("summary.json is not JSON")?;
    let data = doc.get("Data").context("summary.json has no Data object")?;
    let mut settings = BTreeMap::new();
    for key in CYBERPUNK_SETTINGS {
        if let Some(v) = data.get(*key) {
            let text = v.as_str().map_or_else(|| v.to_string(), str::to_string);
            settings.insert((*key).to_string(), text);
        }
    }
    let frame_generation = data
        .get("frameGenerationType")
        .and_then(serde_json::Value::as_i64)
        .is_some_and(|t| t != 0)
        || [
            "DLSSFrameGenEnabled",
            "DLSSMultiFrameGenEnabled",
            "FSR3FrameGenEnabled",
            "XeSSFrameGenEnabled",
        ]
        .iter()
        .any(|k| data.get(*k).and_then(serde_json::Value::as_bool) == Some(true));
    let average = data.get("averageFps").and_then(serde_json::Value::as_f64);
    Ok((settings, average, frame_generation))
}

/// Read one `Cyberpunk 2077` run from its `benchmark_*` directory.
///
/// # Errors
/// Returns an error when `frames.csv` is missing or unparseable. A missing or
/// malformed `summary.json` is also an error: without it, neither the settings
/// nor the frame generation state is known, and a run whose conditions are
/// unknown cannot be compared with anything.
pub fn read_cyberpunk(dir: &Path) -> Result<NativeRun> {
    let frames = dir.join("frames.csv");
    let summary = dir.join("summary.json");
    let content =
        std::fs::read_to_string(&frames).with_context(|| format!("read {}", frames.display()))?;
    let (capture, transitions) = parse_cyberpunk_frames(&content)?;
    let json =
        std::fs::read_to_string(&summary).with_context(|| format!("read {}", summary.display()))?;
    let (settings, reported_avg_fps, frame_generation) = parse_cyberpunk_summary(&json)?;
    Ok(NativeRun {
        capture,
        transitions,
        settings,
        reported_avg_fps,
        frame_generation,
        files: vec![frames, summary],
    })
}

// ── Shared ───────────────────────────────────────────────────────────────────

/// Separate rendered frames from scene transitions.
///
/// The capture's duration is the sum of the rendered frames, so the average
/// frame rate describes rendering and not loading.
fn split_transitions(frametimes: impl Iterator<Item = f64>) -> (Capture, usize) {
    let mut capture = Capture::default();
    let mut transitions = 0;
    for ft in frametimes {
        if ft <= 0.0 || !ft.is_finite() {
            continue;
        }
        if ft >= TRANSITION_MS {
            transitions += 1;
        } else {
            capture.frametimes_ms.push(ft);
        }
    }
    capture.duration_s = capture.frametimes_ms.iter().sum::<f64>() / 1000.0;
    (capture, transitions)
}

/// Settings that differ between two runs, as `key: a → b`.
///
/// Window position and similar keys that change nothing about rendering are
/// ignored, so moving a window between runs does not void a comparison.
#[must_use]
pub fn settings_differ(a: &BTreeMap<String, String>, b: &BTreeMap<String, String>) -> Vec<String> {
    const COSMETIC: &[&str] = &["WindowLeft", "WindowTop", "WindowMaximized", "Monitor"];
    let keys: std::collections::BTreeSet<&String> = a.keys().chain(b.keys()).collect();
    keys.into_iter()
        .filter(|k| !COSMETIC.contains(&k.as_str()))
        .filter_map(|k| {
            let (x, y) = (a.get(k), b.get(k));
            (x != y).then(|| {
                format!(
                    "{k}: {} → {}",
                    x.map_or("(absent)", String::as_str),
                    y.map_or("(absent)", String::as_str)
                )
            })
        })
        .collect()
}

/// The newest entry in `dir` whose name matches, modified at or after `since`.
///
/// Used to pick up the run a person has just finished, and to ignore every
/// run from before the session started.
///
/// # Errors
/// Returns an error when the directory cannot be read.
pub fn newest_since(
    dir: &Path,
    matches: impl Fn(&str) -> bool,
    since: SystemTime,
) -> Result<Option<PathBuf>> {
    let mut newest: Option<(SystemTime, PathBuf)> = None;
    for entry in std::fs::read_dir(dir)
        .with_context(|| format!("read {}", dir.display()))?
        .flatten()
    {
        let name = entry.file_name();
        if !matches(&name.to_string_lossy()) {
            continue;
        }
        let Ok(modified) = entry.metadata().and_then(|m| m.modified()) else {
            continue;
        };
        if modified >= since && newest.as_ref().is_none_or(|(t, _)| modified > *t) {
            newest = Some((modified, entry.path()));
        }
    }
    Ok(newest.map(|(_, p)| p))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A shortened real log: the header and first rows of a *Shadow of the
    /// Tomb Raider* run on this machine, with its scene-load frame.
    const SOTTR: &str = "Frame, Time (ms), Delta (ms) , Memory (mb)\n\
            1, 0.000, 0.000,2691\n\
            2,11.216,11.216,5480\n\
            3,23.247,12.031,5480\n\
         6131,88169.203,7813.936,5480\n\
         6132,88181.000,11.797,5480\n";

    #[test]
    fn crystal_frametimes_are_read_by_column_name() {
        let (capture, transitions) = parse_crystal_frametimes(SOTTR).unwrap();
        assert_eq!(capture.frametimes_ms, vec![11.216, 12.031, 11.797]);
        assert_eq!(transitions, 1, "the 7.8 s scene load is counted, not kept");
        let expected = (11.216 + 12.031 + 11.797) / 1000.0;
        assert!((capture.duration_s - expected).abs() < 1e-9);
    }

    #[test]
    fn rise_of_the_tomb_raider_has_no_memory_column_and_still_parses() {
        let rottr = " Frame, Time (ms), Delta (ms)\n    1, 0.000, 0.000\n    2, 3.281, 3.281\n";
        let (capture, transitions) = parse_crystal_frametimes(rottr).unwrap();
        assert_eq!(capture.frametimes_ms, vec![3.281]);
        assert_eq!(transitions, 0);
    }

    #[test]
    fn a_file_without_a_delta_column_is_refused() {
        assert!(parse_crystal_frametimes("fps,frametime\n60,16.6\n").is_err());
    }

    #[test]
    fn crystal_summary_yields_settings_and_the_games_own_average() {
        let text = "Raw Benchmark Statistics for 1 run:\n\n\
                    \tBenchmark Statistics (Run No. 1):\n\
                    \t\tMin FPS: 0.0\n\t\tAverage FPS: 77.9\n\n\
                    Average Benchmark Statistics for 1 runs with 0 extremes excluded:\n\
                    \tAverage FPS: 99.9\n\n\
                    Settings:\n\nFullscreenWidth=3440\nFullscreenHeight=1440\nVSync=false\n";
        let (settings, average) = parse_crystal_summary(text);
        assert_eq!(average, Some(77.9), "the run's average, not a later repeat");
        assert_eq!(settings["FullscreenWidth"], "3440");
        assert_eq!(settings["VSync"], "false");
    }

    #[test]
    fn cyberpunk_frames_ignore_the_generation_columns() {
        let csv = "Frame index, Frame time (ms), DLSS Frame Generation (ms), FSR3 Frame Generation (ms)\n\
                   0, 31.00, 15.50, 15.50\n1, 30.04, 15.02, 15.02\n";
        let (capture, _) = parse_cyberpunk_frames(csv).unwrap();
        assert_eq!(capture.frametimes_ms, vec![31.00, 30.04]);
    }

    #[test]
    fn cyberpunk_summary_carries_the_experiment_and_frame_generation() {
        let json = r#"{"RootType":"worldBenchmarkSummary","Data":{
            "gameVersion":"2.31","presetName":"RayTracingUltra",
            "renderWidth":3440,"renderHeight":1440,"verticalSync":false,
            "averageFps":34.63,"frameGenerationType":0,"FSR3FrameGenEnabled":false}}"#;
        let (settings, average, fg) = parse_cyberpunk_summary(json).unwrap();
        assert_eq!(average, Some(34.63));
        assert!(!fg);
        assert_eq!(settings["presetName"], "RayTracingUltra");
        assert_eq!(settings["renderWidth"], "3440");

        let with_fg = json.replace(
            "\"FSR3FrameGenEnabled\":false",
            "\"FSR3FrameGenEnabled\":true",
        );
        assert!(parse_cyberpunk_summary(&with_fg).unwrap().2);
    }

    #[test]
    fn different_settings_are_named_and_window_moves_are_not() {
        let a: BTreeMap<String, String> = [("FullscreenWidth", "3440"), ("WindowLeft", "0")]
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect();
        let mut b = a.clone();
        b.insert("WindowLeft".into(), "760".into());
        assert!(settings_differ(&a, &b).is_empty());
        b.insert("FullscreenWidth".into(), "1920".into());
        assert_eq!(
            settings_differ(&a, &b),
            vec!["FullscreenWidth: 3440 → 1920"]
        );
    }

    #[test]
    fn a_run_split_across_scenes_is_one_capture() {
        let dir = std::env::temp_dir().join(format!("bgm-scenes-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let a = dir.join("SpineOfTheMountain_H_frametimes_1.txt");
        let b = dir.join("ProphetsTomb_H_frametimes_2.txt");
        std::fs::write(
            &a,
            " Frame, Time (ms), Delta (ms)\n1, 0, 0\n2, 5, 5.0\n3, 10, 5.0\n",
        )
        .unwrap();
        std::fs::write(
            &b,
            " Frame, Time (ms), Delta (ms)\n1, 0, 0\n2, 1500, 1500.0\n3, 1510, 10.0\n",
        )
        .unwrap();
        let run = read_crystal_run(&[a, b]).unwrap();
        assert_eq!(run.capture.frametimes_ms, vec![5.0, 5.0, 10.0]);
        assert_eq!(
            run.transitions, 1,
            "the scene load in the second file is set aside"
        );
        assert!((run.capture.duration_s - 0.020).abs() < 1e-9);
        assert_eq!(
            run.reported_avg_fps, None,
            "per-scene averages are not combined"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn only_runs_after_the_session_started_are_picked_up() {
        let dir = std::env::temp_dir().join(format!("bgm-native-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("old_frametimes_1.txt"), "").unwrap();
        let since = SystemTime::now() + std::time::Duration::from_secs(3600);
        let picked = newest_since(&dir, |n| n.contains("_frametimes_"), since).unwrap();
        assert!(
            picked.is_none(),
            "a run from before the session is not this run"
        );
        let picked =
            newest_since(&dir, |n| n.contains("_frametimes_"), SystemTime::UNIX_EPOCH).unwrap();
        assert_eq!(picked, Some(dir.join("old_frametimes_1.txt")));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
