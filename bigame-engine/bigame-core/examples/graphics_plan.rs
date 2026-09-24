//! Run AI Graphics' analysis for an installed game, as the page does, and
//! print the report and the plan (Recommended mode).
//!
//! Usage: `graphics_plan <process-name>`
use bigame_core::graphics::{self, config};

fn main() {
    let Some(process) = std::env::args().nth(1) else {
        eprintln!("usage: graphics_plan <process-name>");
        std::process::exit(2);
    };
    let Some(target) = graphics::target_for_process(&process) else {
        eprintln!("no installed game runs as {process}");
        std::process::exit(1);
    };
    let cfg = config::AiGraphicsConfig {
        mode: config::Mode::Recommended,
        ..config::AiGraphicsConfig::default()
    };
    let t = std::time::Instant::now();
    let a = graphics::analyze(&target, &cfg);
    let r = &a.report;
    println!(
        "{} ({}) — analysed in {:?}",
        target.name,
        target.install_root.display(),
        t.elapsed()
    );
    println!("  executable: {:?} {:?}", r.executable, r.machine);
    println!(
        "  api: {:?} [{:?}] {}",
        r.api.api,
        r.api.confidence,
        r.api
            .evidence
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("; ")
    );
    println!("  native: {:?}", r.native);
    println!(
        "  proxies: {:?}",
        r.proxies
            .iter()
            .map(|p| (&p.slot, &p.owner))
            .collect::<Vec<_>>()
    );
    println!(
        "  anti-cheat: {:?}",
        r.anti_cheat.iter().map(|a| &a.name).collect::<Vec<_>>()
    );
    println!("  gpu: {:?}", r.gpu().map(|g| (&g.name, g.rdna, g.fsr4())));
    println!("PLAN [{:?}] {}", a.plan.standing, a.plan.summary);
    for s in &a.plan.steps {
        println!("  - {}", s.text());
    }
    println!("  files: {:?}", a.plan.files);
    println!("  disable: {:?}", a.plan.disable);
    println!("STATUS {:?}", a.status);
}
