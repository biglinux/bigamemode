//! Frametime capture and A/B comparison.
//!
//! This is the machinery behind the only claim the project is allowed to make
//! about performance. Without it, a report says
//! [`crate::booster::report::Outcome::NotMeasured`] — and that stays the right
//! answer until a capture actually exists.
//!
//! Three decisions shape everything here.
//!
//! **Frametime is the measurement; FPS is derived.** An averaged FPS counter
//! destroys exactly the information that matters — a run averaging 120 FPS with
//! a hitch every second and one that is perfectly smooth report the same
//! number, and only one of them is pleasant to play.
//!
//! **1% low is reported ahead of average FPS.** It is what players perceive as
//! smoothness, and it is the metric a scheduler or power change is most likely
//! to move.
//!
//! **A difference smaller than the measured noise is not a difference.** The
//! noise floor is measured, not assumed, by comparing two baseline runs against
//! each other. Without that, every comparison finds an improvement.
//!
//! Capture is delegated to `MangoHud`, which is already a dependency and already
//! writes per-frame CSV. Nothing here modifies the game or injects anything of
//! its own.
pub mod lab;
pub mod native;
pub mod result;

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

// ── Capture ──────────────────────────────────────────────────────────────────

/// One recorded run.
#[derive(Debug, Clone, Default)]
pub struct Capture {
    /// Per-frame frametimes, in milliseconds, in the order recorded.
    pub frametimes_ms: Vec<f64>,
    /// GPU utilisation samples, percent.
    pub gpu_load: Vec<f64>,
    /// GPU temperature samples, °C.
    pub gpu_temp: Vec<f64>,
    /// CPU temperature samples, °C.
    pub cpu_temp: Vec<f64>,
    /// GPU power samples, watts.
    pub gpu_power: Vec<f64>,
    /// Wall-clock span of the capture, seconds.
    pub duration_s: f64,
}

impl Capture {
    /// Compute statistics. Returns `None` when the capture is too short.
    #[must_use]
    pub fn stats(&self) -> Option<FrameStats> {
        FrameStats::from_frametimes(&self.frametimes_ms, self.duration_s)
    }
}

/// Fewest frames worth computing statistics from.
pub const MIN_FRAMES: usize = 100;

/// Parse a `MangoHud` per-frame CSV.
///
///
/// The format, as written by `MangoHud` 0.8:
///
/// ```text
/// os,cpu,gpu,ram,kernel,driver,cpuscheduler      <- system header
/// BigLinux…,AMD Ryzen 7 5700G,…                  <- system values
/// fps,frametime,cpu_load,…,elapsed               <- frame header
/// 142.475,7.01876,7.40331,…,13121487             <- samples
/// ```
///
/// Columns are located by name rather than by index, because `MangoHud`'s column
/// set varies with what the machine exposes — a laptop without a power sensor
/// simply has fewer. `frametime` is milliseconds and `elapsed` is nanoseconds.
///
/// # Errors
/// Returns an error if the file is unreadable or has no recognisable frame
/// header.
pub fn parse_mangohud_csv(content: &str) -> Result<Capture> {
    let mut lines = content.lines();

    // Find the frame header — the first line naming a `frametime` column.
    let mut header: Option<Vec<&str>> = None;
    for line in lines.by_ref() {
        let cols: Vec<&str> = line.split(',').map(str::trim).collect();
        if cols.contains(&"frametime") {
            header = Some(cols);
            break;
        }
    }
    let header = header.context("no frametime column found; not a MangoHud frame log")?;
    let index = |name: &str| header.iter().position(|c| *c == name);

    let (i_ft, i_elapsed) = (index("frametime"), index("elapsed"));
    let i_ft = i_ft.context("no frametime column")?;

    let mut capture = Capture::default();
    let mut first_elapsed: Option<f64> = None;
    let mut last_elapsed: Option<f64> = None;

    for line in lines {
        let cols: Vec<&str> = line.split(',').map(str::trim).collect();
        if cols.len() <= i_ft {
            continue;
        }
        let Ok(ft) = cols[i_ft].parse::<f64>() else {
            continue;
        };
        // A non-positive or absurd frametime is a logging artefact, not a frame.
        if !(0.0..=10_000.0).contains(&ft) || ft <= 0.0 {
            continue;
        }
        capture.frametimes_ms.push(ft);

        push_col(&mut capture.gpu_load, &cols, index("gpu_load"));
        push_col(&mut capture.gpu_temp, &cols, index("gpu_temp"));
        push_col(&mut capture.cpu_temp, &cols, index("cpu_temp"));
        push_col(&mut capture.gpu_power, &cols, index("gpu_power"));

        if let Some(i) = i_elapsed {
            if let Some(v) = cols.get(i).and_then(|c| c.parse::<f64>().ok()) {
                first_elapsed.get_or_insert(v);
                last_elapsed = Some(v);
            }
        }
    }

    capture.duration_s = match (first_elapsed, last_elapsed) {
        // `elapsed` is nanoseconds.
        (Some(a), Some(b)) if b > a => (b - a) / 1e9,
        // Without it, fall back to the sum of the frametimes.
        _ => capture.frametimes_ms.iter().sum::<f64>() / 1000.0,
    };

    Ok(capture)
}

fn push_col(target: &mut Vec<f64>, cols: &[&str], index: Option<usize>) {
    if let Some(v) = index
        .and_then(|i| cols.get(i))
        .and_then(|c| c.parse::<f64>().ok())
    {
        target.push(v);
    }
}

/// Read and parse a `MangoHud` CSV from disk.
///
/// # Errors
/// Returns an error if the file cannot be read or parsed.
pub fn read_capture(path: &Path) -> Result<Capture> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("read capture: {}", path.display()))?;
    parse_mangohud_csv(&content)
}

/// The newest per-frame CSV in `dir`, ignoring `MangoHud`'s `_summary` files.
///
/// # Errors
/// Returns an error if the directory cannot be read.
pub fn newest_capture_in(dir: &Path) -> Result<Option<PathBuf>> {
    let entries = std::fs::read_dir(dir).with_context(|| format!("read dir: {}", dir.display()))?;
    let mut newest: Option<(std::time::SystemTime, PathBuf)> = None;
    for entry in entries.flatten() {
        let path = entry.path();
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        // `MangoHud` writes a `_summary.csv` alongside each log; it holds
        // pre-aggregated values, not frames.
        if !name.ends_with(".csv") || name.ends_with("_summary.csv") {
            continue;
        }
        let Ok(modified) = entry.metadata().and_then(|m| m.modified()) else {
            continue;
        };
        if newest.as_ref().is_none_or(|(t, _)| modified > *t) {
            newest = Some((modified, path));
        }
    }
    Ok(newest.map(|(_, p)| p))
}

/// Build a `MangoHud` configuration that logs `duration_s` seconds to `folder`.
///
/// Two `MangoHud` 0.8.4 behaviours, neither documented, silently produce **no
/// log at all** when wrong:
///
/// * The settings must reach `MangoHud` through `MANGOHUD_CONFIGFILE`.
///   `MANGOHUD_CONFIG` with the same keys did not produce a log.
/// * `no_display=1` must **not** be set. It suppresses the on-screen overlay
///   and the CSV together, which is exactly the combination a benchmark would
///   reach for first.
///
/// `start_delay_s` is how long to wait before logging begins. It matters more
/// than it looks: a game that spends ten seconds at a menu and loading screen
/// will otherwise have that time inside the capture window, and since the
/// window is a fixed length, each run captures a different mix of menu and
/// gameplay (a `SuperTuxKart` run can vary between 1541 and 3871 frames that
/// way), which is noise no statistic can rescue.
#[must_use]
pub fn mangohud_config(folder: &Path, duration_s: u32, start_delay_s: u32) -> String {
    format!(
        "output_folder={}\nlog_duration={duration_s}\nautostart_log={start_delay_s}\nfps_limit=0\n",
        folder.display()
    )
}

// ── Statistics ───────────────────────────────────────────────────────────────

/// Summary of one run's frametimes.
#[derive(Debug, Clone, PartialEq)]
pub struct FrameStats {
    /// Frames recorded.
    pub frames: usize,
    /// Capture span, seconds.
    pub duration_s: f64,
    /// Frames per second over the whole run.
    pub avg_fps: f64,
    /// Mean frametime, ms.
    pub mean_ms: f64,
    /// Median frametime, ms.
    pub median_ms: f64,
    /// 95th percentile frametime, ms.
    pub p95_ms: f64,
    /// 99th percentile frametime, ms.
    pub p99_ms: f64,
    /// 1% low, expressed as FPS.
    pub low_1_fps: f64,
    /// 0.1% low as FPS, when there were enough frames to mean anything.
    pub low_0_1_fps: Option<f64>,
    /// Frames taking more than twice the median.
    pub stutters: usize,
}

impl FrameStats {
    /// Compute statistics from raw frametimes.
    ///
    /// Returns `None` below [`MIN_FRAMES`].
    #[must_use]
    pub fn from_frametimes(frametimes_ms: &[f64], duration_s: f64) -> Option<Self> {
        if frametimes_ms.len() < MIN_FRAMES {
            return None;
        }
        let mut sorted = frametimes_ms.to_vec();
        sorted.sort_by(f64::total_cmp);

        let n = sorted.len();
        #[allow(clippy::cast_precision_loss)]
        let n_f = n as f64;
        let total_ms: f64 = sorted.iter().sum();
        let mean_ms = total_ms / n_f;

        // Prefer the recorded span; fall back to the frametime sum.
        let span = if duration_s > 0.0 {
            duration_s
        } else {
            total_ms / 1000.0
        };

        Some(Self {
            avg_fps: if span > 0.0 { n_f / span } else { 0.0 },
            mean_ms,
            median_ms: percentile(&sorted, 50.0),
            p95_ms: percentile(&sorted, 95.0),
            p99_ms: percentile(&sorted, 99.0),
            low_1_fps: low_fps(&sorted, 0.01),
            // Below a thousand frames the 0.1% low describes the single worst
            // frame rather than a population, so it is withheld.
            low_0_1_fps: (n >= 1000).then(|| low_fps(&sorted, 0.001)),
            stutters: count_stutters(frametimes_ms, percentile(&sorted, 50.0)),
            frames: n,
            duration_s: span,
        })
    }
}

/// Nearest-rank percentile of a pre-sorted slice.
#[must_use]
pub fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss
    )]
    let rank = ((p / 100.0) * sorted.len() as f64).ceil() as usize;
    sorted[rank.saturating_sub(1).min(sorted.len() - 1)]
}

/// "N% low", as FPS, from frametimes sorted ascending.
///
/// Defined the way benchmarking practice does: take the slowest `fraction` of
/// frames, average their frametimes, and convert. That is a property of the
/// worst frames rather than a percentile of an FPS series, which is why it
/// tracks perceived smoothness.
#[must_use]
pub fn low_fps(sorted_ascending: &[f64], fraction: f64) -> f64 {
    if sorted_ascending.is_empty() {
        return 0.0;
    }
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss
    )]
    let count = (((sorted_ascending.len() as f64) * fraction).ceil() as usize).max(1);
    let slowest = &sorted_ascending[sorted_ascending.len() - count..];
    #[allow(clippy::cast_precision_loss)]
    let mean_ms = slowest.iter().sum::<f64>() / slowest.len() as f64;
    if mean_ms > 0.0 { 1000.0 / mean_ms } else { 0.0 }
}

/// Frames taking more than twice the median — a visible hitch.
#[must_use]
pub fn count_stutters(frametimes_ms: &[f64], median_ms: f64) -> usize {
    if median_ms <= 0.0 {
        return 0;
    }
    frametimes_ms
        .iter()
        .filter(|ft| **ft > median_ms * 2.0)
        .count()
}

// ── Comparison ───────────────────────────────────────────────────────────────

/// Whether a metric improves when it goes up or when it goes down.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// Higher is better — FPS, 1% low.
    HigherIsBetter,
    /// Lower is better — frametime, P99.
    LowerIsBetter,
}

/// Measure the noise floor from repeated baseline runs.
///
/// The largest relative spread between any two runs of the *same*
/// configuration. Anything smaller than this is not a result, and an engine
/// that skips this step will report an improvement every single time.
///
/// Returns `None` with fewer than two runs — with one sample there is no
/// evidence about repeatability at all.
#[must_use]
pub fn noise_floor(values: &[f64]) -> Option<f64> {
    if values.len() < 2 {
        return None;
    }
    let min = values.iter().copied().fold(f64::INFINITY, f64::min);
    let max = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    if min <= 0.0 {
        return None;
    }
    Some((max - min) / min)
}

/// The metrics reported, in the order they are reported.
///
/// 1% low first, because it is what is felt; average FPS last, because it is
/// the easiest number to move without improving anything.
type MetricReader = fn(&FrameStats) -> f64;

/// name, unit, direction, and how to read it from a run.
type Metric = (&'static str, &'static str, Direction, MetricReader);

const METRICS: &[Metric] = &[
    ("1% low", "fps", Direction::HigherIsBetter, |s| s.low_1_fps),
    ("P99 frametime", "ms", Direction::LowerIsBetter, |s| {
        s.p99_ms
    }),
    ("P95 frametime", "ms", Direction::LowerIsBetter, |s| {
        s.p95_ms
    }),
    ("Average FPS", "fps", Direction::HigherIsBetter, |s| {
        s.avg_fps
    }),
];

/// Compare every run of two configurations, metric by metric, with the test
/// `docs/BENCHMARKS.md` describes: both arms need two runs or more and a
/// coefficient of variation within 5 %, and a difference counts only above
/// the noise and past Welch's t-test at 95 % ([`crate::benchmark::result`]).
/// This is what "Measure the difference" reports.
#[must_use]
pub fn compare_arms(
    baseline_runs: &[FrameStats],
    candidate_runs: &[FrameStats],
) -> Vec<crate::booster::report::Outcome> {
    use crate::benchmark::result::{ArmSummary, Comparison, Verdict};
    use crate::booster::report::Outcome;
    METRICS
        .iter()
        .map(|(name, unit, direction, get)| {
            let base = ArmSummary::new("baseline", baseline_runs.iter().map(get).collect());
            let cand = ArmSummary::new("candidate", candidate_runs.iter().map(get).collect());
            let (Some(base), Some(cand)) = (base, cand) else {
                return Outcome::NotMeasured;
            };
            let (before, after) = (base.mean, cand.mean);
            match Comparison::new(*name, base, cand).verdict {
                Verdict::Inconclusive => Outcome::Inconclusive {
                    metric: (*name).to_owned(),
                },
                Verdict::WithinNoise => Outcome::NoChange {
                    metric: (*name).to_owned(),
                },
                Verdict::Improvement | Verdict::Regression => {
                    let better = match direction {
                        Direction::HigherIsBetter => after > before,
                        Direction::LowerIsBetter => after < before,
                    };
                    let (metric, unit) = ((*name).to_owned(), (*unit).to_owned());
                    if better {
                        Outcome::Improved {
                            metric,
                            before,
                            after,
                            unit,
                        }
                    } else {
                        Outcome::Regressed {
                            metric,
                            before,
                            after,
                            unit,
                        }
                    }
                }
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Trimmed from a real `MangoHud` 0.8.4 capture: `mangohud vkcube`, RX 9060
    /// XT, 6 seconds, 959 frames.
    const REAL_CSV: &str = "\
os,cpu,gpu,ram,kernel,driver,cpuscheduler
BigLinux based on Manjaro Linux,AMD Ryzen 7 5700G with Radeon Graphics,AMD Radeon RX 9060 XT (RADV GFX1200),48677468,7.2.6-x64v3-xanmod1-1,,performance
fps,frametime,cpu_load,cpu_power,gpu_load,cpu_temp,gpu_temp,gpu_core_clock,gpu_mem_clock,gpu_vram_used,gpu_power,ram_used,swap_used,process_rss,cpu_mhz,elapsed
142.475,7.01876,7.40331,19.3663,2,53,39,400,6,0.0164833,65,17.4671,0,0,4553,13121487
184.086,5.43223,7.40331,19.3663,2,53,39,400,6,0.0164833,65,17.4671,0,0,4553,18554181
150.895,6.62711,7.40331,19.3663,2,53,39,400,6,0.0164833,65,17.4671,0,0,4553,25183789
158.869,6.29448,7.15072,16.6758,0,51,38,400,6,0.0164833,65,17.431,0,0,4419,5995032514
159.877,6.2548,7.15072,16.6758,0,51,38,400,6,0.0164833,65,17.431,0,0,4419,6001287651
";

    #[test]
    fn parses_a_real_mangohud_capture() {
        let capture = parse_mangohud_csv(REAL_CSV).unwrap();
        assert_eq!(capture.frametimes_ms.len(), 5);
        assert!((capture.frametimes_ms[0] - 7.01876).abs() < 1e-6);
        assert_eq!(capture.gpu_temp.len(), 5);
        assert!((capture.gpu_temp[0] - 39.0).abs() < f64::EPSILON);
        assert!((capture.gpu_power[0] - 65.0).abs() < f64::EPSILON);
        // elapsed is nanoseconds: 13121487 -> 6001287651 is a little under 6 s.
        assert!(
            (capture.duration_s - 5.988).abs() < 0.01,
            "duration was {}",
            capture.duration_s
        );
    }

    #[test]
    fn columns_are_located_by_name_not_position() {
        // A machine without power or temperature sensors writes fewer columns.
        let csv = "fps,frametime,elapsed\n100,10.0,0\n120,8.333,10000000\n";
        let capture = parse_mangohud_csv(csv).unwrap();
        assert_eq!(capture.frametimes_ms.len(), 2);
        assert!(capture.gpu_power.is_empty());
        assert!(capture.gpu_temp.is_empty());
    }

    #[test]
    fn a_file_that_is_not_a_frame_log_is_an_error() {
        // MangoHud's `_summary.csv` has no frametime column.
        let summary = "0.1% Min FPS,1% Min FPS,Average FPS\n133.6,139.4,160.0\n";
        assert!(parse_mangohud_csv(summary).is_err());
        assert!(parse_mangohud_csv("").is_err());
    }

    #[test]
    fn malformed_rows_are_skipped_not_fatal() {
        let csv =
            "fps,frametime,elapsed\n100,10.0,0\ngarbage\n120,notanumber,1\n90,11.1,20000000\n";
        let capture = parse_mangohud_csv(csv).unwrap();
        assert_eq!(capture.frametimes_ms.len(), 2);
    }

    #[test]
    fn nonsensical_frametimes_are_discarded() {
        let csv = "fps,frametime\n100,10.0\n0,0\n1,-5\n0,99999\n100,10.0\n";
        let capture = parse_mangohud_csv(csv).unwrap();
        assert_eq!(capture.frametimes_ms.len(), 2);
    }

    /// A steady run at ~6.25 ms with a periodic hitch.
    fn synthetic(frames: usize, base_ms: f64, hitch_every: usize, hitch_ms: f64) -> Vec<f64> {
        (0..frames)
            .map(|i| {
                if hitch_every > 0 && i % hitch_every == 0 {
                    hitch_ms
                } else {
                    base_ms
                }
            })
            .collect()
    }

    #[test]
    fn short_captures_report_nothing_rather_than_a_number() {
        assert!(FrameStats::from_frametimes(&synthetic(99, 6.25, 0, 0.0), 1.0).is_none());
        assert!(FrameStats::from_frametimes(&[], 1.0).is_none());
        assert!(FrameStats::from_frametimes(&synthetic(100, 6.25, 0, 0.0), 1.0).is_some());
    }

    #[test]
    fn the_0_1_percent_low_is_withheld_until_it_means_something() {
        // Below 1000 frames it describes the single worst frame.
        let short = FrameStats::from_frametimes(&synthetic(500, 6.25, 0, 0.0), 3.1).unwrap();
        assert_eq!(short.low_0_1_fps, None);
        let long = FrameStats::from_frametimes(&synthetic(1000, 6.25, 0, 0.0), 6.25).unwrap();
        assert!(long.low_0_1_fps.is_some());
    }

    #[test]
    fn one_percent_low_reflects_the_worst_frames_not_the_average() {
        // 1000 frames at 6.25 ms (160 fps) with 1% of them at 50 ms (20 fps).
        let mut frames = vec![6.25; 990];
        frames.extend(std::iter::repeat_n(50.0, 10));
        let stats = FrameStats::from_frametimes(&frames, 6.69).unwrap();

        // Average FPS barely notices the hitches...
        assert!(stats.avg_fps > 140.0, "avg_fps was {}", stats.avg_fps);
        // ...while the 1% low reports them plainly.
        assert!(
            (stats.low_1_fps - 20.0).abs() < 0.5,
            "low_1_fps was {}",
            stats.low_1_fps
        );
        // This gap is the whole reason 1% low is reported first.
        assert!(stats.avg_fps > stats.low_1_fps * 5.0);
    }

    #[test]
    fn a_perfectly_smooth_run_has_matching_statistics() {
        let stats = FrameStats::from_frametimes(&synthetic(1000, 8.0, 0, 0.0), 8.0).unwrap();
        assert!((stats.mean_ms - 8.0).abs() < 1e-9);
        assert!((stats.median_ms - 8.0).abs() < 1e-9);
        assert!((stats.p99_ms - 8.0).abs() < 1e-9);
        assert!((stats.low_1_fps - 125.0).abs() < 1e-6);
        assert_eq!(stats.stutters, 0);
    }

    #[test]
    fn stutters_are_counted_against_the_median() {
        // Every 10th frame takes 4x the median.
        let frames = synthetic(1000, 6.0, 10, 24.0);
        let stats = FrameStats::from_frametimes(&frames, 7.8).unwrap();
        assert_eq!(stats.stutters, 100);
    }

    #[test]
    fn percentiles_use_nearest_rank_and_lows_handle_edges() {
        let s = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0];
        assert!((percentile(&s, 50.0) - 5.0).abs() < f64::EPSILON);
        assert!((percentile(&s, 95.0) - 10.0).abs() < f64::EPSILON);
        assert!((percentile(&s, 100.0) - 10.0).abs() < f64::EPSILON);
        assert!((percentile(&s, 0.0) - 1.0).abs() < f64::EPSILON);
        assert!((percentile(&[], 50.0)).abs() < f64::EPSILON);
        assert!((low_fps(&[], 0.01)).abs() < f64::EPSILON);
        // A single frame: the 1% low is that frame.
        assert!((low_fps(&[10.0], 0.01) - 100.0).abs() < 1e-9);
    }

    // ── Comparison ───────────────────────────────────────────────────────

    #[test]
    fn noise_floor_needs_repeats() {
        // One run says nothing about repeatability.
        assert_eq!(noise_floor(&[100.0]), None);
        assert_eq!(noise_floor(&[]), None);

        // Two baseline runs 4% apart set the bar at 4%.
        let floor = noise_floor(&[100.0, 104.0]).unwrap();
        assert!((floor - 0.04).abs() < 1e-9);

        // The widest pair sets it, not the closest.
        let floor = noise_floor(&[100.0, 104.0, 110.0]).unwrap();
        assert!((floor - 0.10).abs() < 1e-9);
    }

    #[test]
    fn mangohud_config_is_well_formed() {
        let cfg = mangohud_config(Path::new("/tmp/logs"), 30, 12);
        assert!(cfg.contains("output_folder=/tmp/logs"));
        assert!(cfg.contains("log_duration=30"));
        assert!(cfg.contains("autostart_log=12"));
        // no_display=1 suppresses the CSV as well as the overlay on 0.8.4,
        // so it must never appear here.
        assert!(!cfg.contains("no_display"));
    }
}
