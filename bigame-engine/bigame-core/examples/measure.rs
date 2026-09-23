//! Run a real A/B measurement of this machine's Booster plan.
//!
//! Usage: measure <log-dir> <seconds> <runs-per-arm> -- <workload> [args...]
use bigame_core::booster::BoosterEngine;
use bigame_core::booster::measure::{MeasureProgress, MeasurementPlan};

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let sep = args.iter().position(|a| a == "--").unwrap_or(args.len());
    let (head, tail) = args.split_at(sep);
    anyhow::ensure!(
        head.len() >= 3,
        "usage: measure <dir> <secs> <runs> -- <workload>"
    );

    let plan = MeasurementPlan {
        command: tail.iter().skip(1).cloned().collect(),
        duration_s: head[1].parse()?,
        runs_per_arm: head[2].parse()?,
    };
    let engine = BoosterEngine::detect();

    let (_, opt) = engine.dry_run();
    println!("optimization plan ({} changes):", opt.changes.len());
    for c in &opt.changes {
        println!("   {}: {} -> {}", c.knob.id(), c.from, c.to);
    }
    println!();

    let m = engine
        .measure(&plan, &std::path::PathBuf::from(&head[0]), |p| match p {
            MeasureProgress::Running {
                arm,
                run,
                total,
                warmup,
            } => println!(
                "  [{run}/{total}] {arm:?}{}",
                if warmup { "  (warm-up, discarded)" } else { "" }
            ),
            MeasureProgress::Switching { to } => println!("  switching to {to:?}"),
            MeasureProgress::Analysing => println!("  analysing"),
        })
        .await?;

    println!("\n=== RESULT ===");
    for (label, runs) in [("baseline", &m.baseline), ("optimized", &m.optimized)] {
        for s in runs {
            println!(
                "  {label:<10} {:>6} frames  avg {:>8.1} fps  1% low {:>8.1} fps  p99 {:>6.2} ms",
                s.frames, s.avg_fps, s.low_1_fps, s.p99_ms
            );
        }
    }
    println!("\n  measured noise floor: {:.1}%", m.noise_floor * 100.0);
    for o in &m.outcomes {
        println!("  {}", o.describe());
    }
    Ok(())
}
