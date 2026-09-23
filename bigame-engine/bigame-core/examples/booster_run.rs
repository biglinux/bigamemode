//! Exercise the full Booster pipeline against the live machine.
//!
//! Usage: `booster_run plan | on | off | status`
use bigame_core::booster::{BoosterEngine, Progress};

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();
    let mode = std::env::args().nth(1).unwrap_or_else(|| "plan".into());
    let engine = BoosterEngine::detect();

    match mode.as_str() {
        "status" => {
            println!("active: {:?}", BoosterEngine::active_summary());
            for knob in engine.relevant_knobs() {
                println!("  {:<24} = {:?}", knob.id(), knob.read());
            }
        }
        "plan" => {
            let (snapshot, plan) = engine.dry_run();
            println!("--- BASELINE ---");
            for (id, c) in &snapshot.entries {
                println!("  {id:<24} = {:?}", c.value);
            }
            println!("--- PLAN ({} changes) ---", plan.changes.len());
            for c in &plan.changes {
                println!("  {} : {} -> {}  [{:?}]", c.knob.id(), c.from, c.to, c.risk);
                println!("      {}", c.rationale);
            }
            println!("--- SKIPPED ({}) ---", plan.skipped.len());
            for s in &plan.skipped {
                println!("  {s:?}");
            }
        }
        "on" => {
            let report = engine
                .activate(|p| match p {
                    Progress::Applying { knob, index, total } => {
                        println!("[{index}/{total}] applying {knob}");
                    }
                    Progress::Verifying { knob } => println!("         verifying {knob}"),
                    other => println!("[stage] {other:?}"),
                })
                .await?;
            println!("\n=== REPORT ===");
            println!("{}", report.headline());
            println!("machine: {}", report.machine);
            for a in &report.applied {
                println!(
                    "  {}  verification={:?} error={:?}",
                    a.summary(),
                    a.verification,
                    a.error
                );
            }
            println!(
                "verified={} failed={}",
                report.verified_count(),
                report.failed_count()
            );
            println!("performance: {}", report.performance_claim());
        }
        "off" => {
            for o in BoosterEngine::deactivate().await? {
                println!("  restore {} -> {} : {:?}", o.knob.id(), o.target, o.status);
            }
            println!("active after: {:?}", BoosterEngine::active_summary());
        }
        other => anyhow::bail!("unknown mode {other}"),
    }
    Ok(())
}
