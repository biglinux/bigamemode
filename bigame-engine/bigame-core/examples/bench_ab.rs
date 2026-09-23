//! Run an A/B frametime comparison using MangoHud capture.
//!
//! Usage: bench_ab <run-label> <log-dir>   — capture one run
//!        bench_ab compare <log-dir>       — analyse captures by label
use bigame_core::benchmark::{self, FrameStats};
use std::path::{Path, PathBuf};

fn stats_for(path: &Path) -> Option<(String, FrameStats)> {
    let capture = benchmark::read_capture(path).ok()?;
    let stats = capture.stats()?;
    Some((path.file_name()?.to_string_lossy().into_owned(), stats))
}

fn main() -> anyhow::Result<()> {
    let mode = std::env::args().nth(1).unwrap_or_default();
    let dir = PathBuf::from(std::env::args().nth(2).unwrap_or_else(|| ".".into()));

    if mode == "compare" {
        let mut runs: Vec<(String, FrameStats)> = std::fs::read_dir(&dir)?
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.extension().is_some_and(|e| e == "csv")
                    && !p.to_string_lossy().ends_with("_summary.csv")
            })
            .filter_map(|p| stats_for(&p))
            .collect();
        runs.sort_by(|a, b| a.0.cmp(&b.0));

        for (name, s) in &runs {
            println!(
                "{name}\n   frames {:>5}  {:.2}s  avg {:>6.1} fps  1% low {:>6.1} fps  \
                 median {:>5.2} ms  p95 {:>5.2}  p99 {:>5.2}  stutters {}",
                s.frames,
                s.duration_s,
                s.avg_fps,
                s.low_1_fps,
                s.median_ms,
                s.p95_ms,
                s.p99_ms,
                s.stutters
            );
        }

        // Group by the label prefix before the first '-'.
        let group = |prefix: &str| -> Vec<&FrameStats> {
            runs.iter()
                .filter(|(n, _)| n.starts_with(prefix))
                .map(|(_, s)| s)
                .collect()
        };
        let a = group("A_");
        let b = group("B_");
        if a.len() < 2 || b.is_empty() {
            println!("\nneed >=2 A runs (for the noise floor) and >=1 B run");
            return Ok(());
        }

        let a_low: Vec<f64> = a.iter().map(|s| s.low_1_fps).collect();
        let noise = benchmark::noise_floor(&a_low).unwrap_or(0.0);
        println!(
            "\nmeasured noise floor from {} baseline runs: {:.1}%",
            a.len(),
            noise * 100.0
        );

        println!("\nA (baseline) vs B (candidate):");
        for outcome in benchmark::compare_all(a[0], b[0], noise) {
            println!("  {}", outcome.describe());
        }
        return Ok(());
    }

    println!("capture mode: point MangoHud at {}", dir.display());
    Ok(())
}
