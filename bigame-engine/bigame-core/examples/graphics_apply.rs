//! Apply AI Graphics' Recommended plan to an installed game, or undo it, the
//! way the page's Apply and Restore buttons do: analyse, then install the plan
//! as a transaction, or roll back what the manifest lists.
//!
//! Usage:
//!   `graphics_apply <process-name>`
//!   `graphics_apply <process-name> --remove`
use bigame_core::graphics::{self, config};

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let process = args
        .first()
        .ok_or_else(|| anyhow::anyhow!("usage: graphics_apply <process-name> [--remove]"))?;
    let target = graphics::target_for_process(process)
        .ok_or_else(|| anyhow::anyhow!("no installed game runs as {process}"))?;

    if args.iter().any(|a| a == "--remove") {
        for outcome in graphics::remove(&target)? {
            println!("{outcome:?}");
        }
        return Ok(());
    }

    let cfg = config::AiGraphicsConfig {
        mode: config::Mode::Recommended,
        ..config::AiGraphicsConfig::default()
    };
    let analysis = graphics::analyze(&target, &cfg);
    println!(
        "PLAN [{:?}] {}",
        analysis.plan.standing, analysis.plan.summary
    );
    let m = graphics::install(&target, &analysis.plan)?;
    println!(
        "installed {} {} into {}",
        m.source.component, m.source.version, target.name
    );
    for e in &m.entries {
        println!(
            "  {}  {}",
            e.path.display(),
            if e.replaced.is_some() {
                "(original backed up)"
            } else {
                "(new)"
            }
        );
    }
    Ok(())
}
