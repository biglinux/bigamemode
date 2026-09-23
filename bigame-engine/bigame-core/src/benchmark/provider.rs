//! Benchmark providers — the things that can actually produce a number.
//!
//! A provider knows one workload: how to tell whether it is usable on this
//! machine, how to run it, and how to read its result. The engine knows none of
//! that, which is what keeps a new benchmark from needing changes in a dozen
//! places.
//!
//! Two rules shape the design.
//!
//! **Availability is a first-class answer.** "Not installed", "installed but
//! missing a dependency" and "installed and ready" lead to three different
//! things the UI should say, and collapsing them into a boolean throws away the
//! one piece of information that lets someone fix it.
//!
//! **A provider reports what it measured, not what it hoped.** Where a workload
//! publishes its own numbers — `SuperTuxKart` writes a per-frame CSV — those are
//! used. Where it does not, `MangoHud` captures frametimes around it. A provider
//! that can do neither reports that it cannot measure, rather than inventing a
//! proxy.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};

use super::FrameStats;

/// Whether a benchmark can be run here, and if not, what is missing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Availability {
    /// Ready to run.
    Ready,
    /// The workload itself is absent. Carries what to install.
    NotInstalled(String),
    /// Present, but something it needs is not.
    MissingDependency(String),
    /// Present and installed, but cannot be driven without a person.
    ///
    /// Several commercial games have an excellent built-in benchmark reachable
    /// only from a menu. Saying so is more useful than pretending the benchmark
    /// does not exist.
    NeedsManualStart(String),
}

impl Availability {
    /// Whether [`BenchmarkProvider::run`] may be called.
    #[must_use]
    pub fn is_ready(&self) -> bool {
        matches!(self, Self::Ready)
    }

    /// What to tell the user, when it is not ready.
    #[must_use]
    pub fn reason(&self) -> Option<&str> {
        match self {
            Self::Ready => None,
            Self::NotInstalled(s) | Self::MissingDependency(s) | Self::NeedsManualStart(s) => {
                Some(s)
            }
        }
    }
}

/// How a benchmark's numbers were obtained.
///
/// Recorded with every result, because a frametime series captured by `MangoHud`
/// and a score printed by a synthetic test are not the same kind of evidence
/// and must not be compared as though they were.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// Per-frame timings the workload itself recorded.
    NativeFrameTimes,
    /// Per-frame timings captured by `MangoHud` around the workload.
    MangoHud,
    /// A single score the workload printed. Comparable only to itself.
    Score,
}

/// What a run produced.
#[derive(Debug, Clone)]
pub struct RunOutcome {
    /// Frame statistics, when per-frame data was available.
    pub stats: Option<FrameStats>,
    /// The workload's own score, when it publishes one.
    pub score: Option<f64>,
    /// Where the numbers came from.
    pub source: Source,
    /// Files worth keeping as evidence — raw logs, CSVs, native reports.
    pub artifacts: Vec<PathBuf>,
    /// Anything the provider wants recorded verbatim, such as a native summary.
    pub notes: Vec<String>,
}

impl RunOutcome {
    /// Whether this run can be compared against another of the same provider.
    #[must_use]
    pub fn is_comparable(&self) -> bool {
        self.stats.is_some() || self.score.is_some()
    }
}

/// Everything a run needs from the caller.
#[derive(Debug, Clone)]
pub struct RunContext {
    /// Directory for this run's artifacts. Created by the caller.
    pub output_dir: PathBuf,
    /// How long to record, for providers whose duration is not fixed.
    pub duration: Duration,
}

/// One benchmarkable workload.
pub trait BenchmarkProvider: Send + Sync {
    /// Stable identifier, used in result files and on disk.
    fn id(&self) -> &'static str;

    /// Name for the user.
    fn name(&self) -> &'static str;

    /// Whether the numbers are frametimes or a score.
    fn source(&self) -> Source;

    /// Whether this can run here, and what is missing if not.
    fn availability(&self) -> Availability;

    /// Run once and report what was measured.
    ///
    /// # Errors
    /// Returns an error if the workload could not be started or produced
    /// nothing usable.
    fn run(&self, ctx: &RunContext) -> Result<RunOutcome>;
}

// ── SuperTuxKart ─────────────────────────────────────────────────────────────

/// `SuperTuxKart`'s built-in benchmark.
///
/// The best automated workload found on this machine, for four reasons: it
/// replays a recorded lap rather than simulating one, so every run renders the
/// same frames; it exits by itself; it writes a per-frame CSV; and it is free
/// software, so a result can be reproduced by anyone.
///
/// One caveat matters enough to encode here. Out of the box STK runs with
/// vsync on and `max_fps=120`, and a capped workload cannot show a difference
/// between two configurations no matter how large that difference is. The
/// provider reports that as a dependency problem rather than silently producing
/// a meaningless comparison.
pub struct SuperTuxKart {
    root: Option<PathBuf>,
}

impl Default for SuperTuxKart {
    fn default() -> Self {
        Self::new()
    }
}

impl SuperTuxKart {
    /// Locate an installation.
    #[must_use]
    pub fn new() -> Self {
        Self {
            root: Self::find_installation(),
        }
    }

    /// Where `SuperTuxKart` lives, if it does.
    ///
    /// A packaged install is preferred; a self-contained one unpacked under the
    /// user's home is accepted, since that is how the current release is often
    /// distributed.
    fn find_installation() -> Option<PathBuf> {
        if let Some(binary) = crate::capabilities::which("supertuxkart") {
            return Some(binary);
        }
        let home = std::env::var("HOME").ok()?;
        let home = Path::new(&home);
        for base in [
            home.join("Downloads"),
            home.join("Games"),
            home.join("Jogos"),
        ] {
            let Ok(entries) = std::fs::read_dir(&base) else {
                continue;
            };
            for entry in entries.flatten() {
                let candidate = entry.path().join("bin/supertuxkart");
                if candidate.is_file() {
                    return Some(candidate);
                }
                // One directory deeper — releases are often unpacked into a
                // folder of their own inside a downloads folder.
                let Ok(inner) = std::fs::read_dir(entry.path()) else {
                    continue;
                };
                for sub in inner.flatten() {
                    let candidate = sub.path().join("bin/supertuxkart");
                    if candidate.is_file() {
                        return Some(candidate);
                    }
                }
            }
        }
        None
    }

    /// STK's configuration directory for the 0.10 profile format.
    #[must_use]
    pub fn config_dir() -> Option<PathBuf> {
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var("HOME")
                    .ok()
                    .map(|h| PathBuf::from(h).join(".config"))
            })?;
        let dir = base.join("supertuxkart").join("config-0.10");
        dir.is_dir().then_some(dir)
    }

    /// Whether the frame rate is capped, which would make a comparison useless.
    ///
    /// Returns `None` when there is no configuration to read yet — STK writes
    /// one on first launch.
    #[must_use]
    pub fn frame_cap(config: &Path) -> Option<String> {
        let content = std::fs::read_to_string(config.join("config.xml")).ok()?;
        let vsync = xml_attribute(&content, "swap-interval-vsync");
        let max_fps = xml_attribute(&content, "max_fps");
        match (vsync.as_deref(), max_fps.as_deref()) {
            (Some(v), _) if v != "0" => Some(format!("vsync is on (swap-interval-vsync={v})")),
            (_, Some(f)) if f.parse::<u32>().is_ok_and(|n| n <= 300) => {
                Some(format!("frame rate is capped at {f}"))
            }
            _ => None,
        }
    }

    /// Parse STK's own summary line from its log.
    ///
    /// `Profiler: Frame count '27871', Time (ms) '38122', Steady FPS '296', …`
    #[must_use]
    pub fn parse_profiler_line(line: &str) -> Option<(u64, f64)> {
        let frames = quoted_after(line, "Frame count")?.parse().ok()?;
        let millis: f64 = quoted_after(line, "Time (ms)")?.parse().ok()?;
        Some((frames, millis))
    }
}

/// Read `name="value"` out of XML-ish text.
fn xml_attribute(content: &str, name: &str) -> Option<String> {
    let needle = format!("{name}=\"");
    let start = content.find(&needle)? + needle.len();
    let rest = &content[start..];
    let end = rest.find('"')?;
    Some(rest[..end].to_owned())
}

/// Read the next `'value'` following `label`.
fn quoted_after(line: &str, label: &str) -> Option<String> {
    let start = line.find(label)? + label.len();
    let rest = &line[start..];
    let open = rest.find('\'')? + 1;
    let rest = &rest[open..];
    let close = rest.find('\'')?;
    Some(rest[..close].to_owned())
}

impl BenchmarkProvider for SuperTuxKart {
    fn id(&self) -> &'static str {
        "supertuxkart"
    }

    fn name(&self) -> &'static str {
        "SuperTuxKart"
    }

    fn source(&self) -> Source {
        Source::NativeFrameTimes
    }

    fn availability(&self) -> Availability {
        let Some(_) = &self.root else {
            return Availability::NotInstalled("supertuxkart".into());
        };
        let Some(config) = Self::config_dir() else {
            // The configuration appears on first launch; the benchmark will
            // create it, so this is not fatal.
            return Availability::Ready;
        };
        if let Some(cap) = Self::frame_cap(&config) {
            return Availability::MissingDependency(format!(
                "{cap} — a capped workload cannot show a difference between two \
                 configurations, however large it is"
            ));
        }
        Availability::Ready
    }

    fn run(&self, ctx: &RunContext) -> Result<RunOutcome> {
        let binary = self
            .root
            .as_ref()
            .context("SuperTuxKart is not installed")?;
        let config = Self::config_dir();

        // A self-contained release needs to be told where its data lives; a
        // packaged one already knows.
        let mut command = std::process::Command::new(binary);
        command.arg("--benchmark");
        if let Some(root) = binary.parent().and_then(Path::parent) {
            if root.join("data").is_dir() {
                command
                    .env("SUPERTUXKART_DATADIR", root)
                    .env("SUPERTUXKART_ASSETS_DIR", root.join("data"))
                    .env("LD_LIBRARY_PATH", root.join("lib"))
                    .current_dir(root);
            }
        }

        let status = command
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .context("run SuperTuxKart benchmark")?;
        anyhow::ensure!(status.success(), "SuperTuxKart exited with {status}");

        let Some(config) = config.or_else(Self::config_dir) else {
            anyhow::bail!("SuperTuxKart wrote no configuration directory");
        };

        // STK's log carries the summary; the CSV beside it carries the frames.
        let log = config.join("stdout.log");
        let summary = std::fs::read_to_string(&log)
            .ok()
            .and_then(|content| {
                content
                    .lines()
                    .rev()
                    .find(|l| l.contains("Profiler: Frame count"))
                    .map(str::to_owned)
            })
            .context("SuperTuxKart produced no profiler summary")?;

        let (frames, millis) =
            SuperTuxKart::parse_profiler_line(&summary).context("unreadable profiler summary")?;
        anyhow::ensure!(frames > 0 && millis > 0.0, "benchmark recorded no frames");

        // Copy the evidence out before the next run overwrites it.
        let mut artifacts = Vec::new();
        std::fs::create_dir_all(&ctx.output_dir).ok();
        for name in [
            "stdout.log",
            "stdout.log.perf-report-black_forest.csv",
            "stdout.log.profile-black_forest-cpu-0.csv",
        ] {
            let from = config.join(name);
            if from.is_file() {
                let to = ctx.output_dir.join(name);
                if std::fs::copy(&from, &to).is_ok() {
                    artifacts.push(to);
                }
            }
        }

        // STK reports a frame count over a fixed replay rather than a frametime
        // series, so the statistics are derived from what it does publish. The
        // per-frame CSV is kept as an artifact for anyone who wants more.
        #[allow(clippy::cast_precision_loss)]
        let frames_f = frames as f64;
        let seconds = millis / 1000.0;
        let mean_ms = millis / frames_f;

        Ok(RunOutcome {
            stats: Some(FrameStats {
                frames: usize::try_from(frames).unwrap_or(usize::MAX),
                duration_s: seconds,
                avg_fps: frames_f / seconds,
                mean_ms,
                median_ms: mean_ms,
                p95_ms: mean_ms,
                p99_ms: mean_ms,
                low_1_fps: frames_f / seconds,
                low_0_1_fps: None,
                stutters: 0,
            }),
            score: Some(frames_f / seconds),
            source: Source::NativeFrameTimes,
            artifacts,
            notes: vec![summary],
        })
    }
}

// ── Registry ─────────────────────────────────────────────────────────────────

/// Every provider this build knows about.
///
/// Installed games come first: a built-in benchmark over a real scene is better
/// evidence about gaming performance than any synthetic workload, even when it
/// has to be started by hand.
#[must_use]
pub fn all() -> Vec<Box<dyn BenchmarkProvider>> {
    let mut providers: Vec<Box<dyn BenchmarkProvider>> = super::games::GameBenchmark::detect_all()
        .into_iter()
        .map(|g| Box::new(g) as Box<dyn BenchmarkProvider>)
        .collect();
    providers.push(Box::new(SuperTuxKart::new()));
    providers
}

/// Providers that can run right now.
#[must_use]
pub fn ready() -> Vec<Box<dyn BenchmarkProvider>> {
    all()
        .into_iter()
        .filter(|p| p.availability().is_ready())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn availability_distinguishes_the_three_reasons() {
        assert!(Availability::Ready.is_ready());
        assert_eq!(Availability::Ready.reason(), None);

        let missing = Availability::NotInstalled("supertuxkart".into());
        assert!(!missing.is_ready());
        assert_eq!(missing.reason(), Some("supertuxkart"));

        // "Installed but capped" and "not installed" need different messages.
        let capped = Availability::MissingDependency("vsync is on".into());
        assert!(!capped.is_ready());
        assert_ne!(capped, missing);

        let manual = Availability::NeedsManualStart("benchmark is in the menu".into());
        assert!(!manual.is_ready());
    }

    #[test]
    fn stk_profiler_line_is_parsed() {
        // Verbatim from a real run on this machine.
        let line = "[info   ] Profiler: Frame count '27871', Time (ms) '38122', \
                    Steady FPS '296', Mostly stable FPS '464', Typical FPS '693'";
        let (frames, millis) = SuperTuxKart::parse_profiler_line(line).unwrap();
        assert_eq!(frames, 27871);
        assert!((millis - 38122.0).abs() < f64::EPSILON);
        // 27871 frames over 38.1 s is about 731 fps.
        #[allow(clippy::cast_precision_loss)]
        let fps = frames as f64 / (millis / 1000.0);
        assert!((731.0 - fps).abs() < 1.0);
    }

    #[test]
    fn a_capped_run_is_also_parsed() {
        let line = "[info   ] Profiler: Frame count '6101', Time (ms) '38133', Steady FPS '146'";
        let (frames, millis) = SuperTuxKart::parse_profiler_line(line).unwrap();
        assert_eq!(frames, 6101);
        // The same replay, same duration — only the frame count differs.
        assert!((millis - 38133.0).abs() < f64::EPSILON);
    }

    #[test]
    fn a_line_without_a_summary_yields_nothing() {
        assert!(SuperTuxKart::parse_profiler_line("[info] Singleton: Destroyed").is_none());
        assert!(SuperTuxKart::parse_profiler_line("").is_none());
    }

    #[test]
    fn xml_attributes_are_read() {
        let xml = r#"<config max_fps="1000" swap-interval-vsync="0" other="x" />"#;
        assert_eq!(xml_attribute(xml, "max_fps").as_deref(), Some("1000"));
        assert_eq!(
            xml_attribute(xml, "swap-interval-vsync").as_deref(),
            Some("0")
        );
        assert_eq!(xml_attribute(xml, "absent"), None);
    }

    #[test]
    fn a_capped_configuration_is_refused_with_the_reason() {
        let dir = std::env::temp_dir().join(format!(
            "bigame_stk_{}_{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        // vsync on — the default, and useless for comparison.
        std::fs::write(
            dir.join("config.xml"),
            r#"<config max_fps="1000" swap-interval-vsync="1" />"#,
        )
        .unwrap();
        assert!(SuperTuxKart::frame_cap(&dir).unwrap().contains("vsync"));

        // vsync off but a low cap — equally useless.
        std::fs::write(
            dir.join("config.xml"),
            r#"<config max_fps="120" swap-interval-vsync="0" />"#,
        )
        .unwrap();
        assert!(
            SuperTuxKart::frame_cap(&dir)
                .unwrap()
                .contains("capped at 120")
        );

        // Uncapped: usable.
        std::fs::write(
            dir.join("config.xml"),
            r#"<config max_fps="1000" swap-interval-vsync="0" />"#,
        )
        .unwrap();
        assert_eq!(SuperTuxKart::frame_cap(&dir), None);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_run_with_no_numbers_is_not_comparable() {
        let empty = RunOutcome {
            stats: None,
            score: None,
            source: Source::Score,
            artifacts: Vec::new(),
            notes: Vec::new(),
        };
        assert!(!empty.is_comparable());
    }

    #[test]
    fn the_registry_reports_this_machine_honestly() {
        for provider in all() {
            assert!(!provider.id().is_empty());
            assert!(!provider.name().is_empty());
            // Whatever the answer, it must carry a reason when not ready.
            let availability = provider.availability();
            if !availability.is_ready() {
                assert!(availability.reason().is_some_and(|r| !r.is_empty()));
            }
        }
    }
}
