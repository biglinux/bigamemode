//! Apply AI Graphics' Recommended plan to an installed game, or undo it, the
//! way the page's Apply and Restore buttons do: analyse, then install the plan
//! as a transaction, or roll back what the manifest lists.
//!
//! Usage:
//!   `graphics_apply <process-name>`
//!   `graphics_apply <process-name> --fsr`
//!   `graphics_apply <process-name> --frame-generation`
//!   `graphics_apply <process-name> --remove`
//!
//! `--fsr` and `--frame-generation` are the page's Choose yourself: FSR
//! through `OptiScaler`, the second with `OptiScaler`'s frame generation on
//! (experimental). Either is saved to the game's settings as the page would,
//! so the launch rules see it.
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

    let frame_generation = args.iter().any(|a| a == "--frame-generation");
    let cfg = if frame_generation || args.iter().any(|a| a == "--fsr") {
        let cfg = config::AiGraphicsConfig {
            mode: config::Mode::Advanced,
            upscaler: config::Upscaler::Fsr,
            layer: config::Layer::OptiScaler,
            frame_generation: if frame_generation {
                config::FrameGeneration::OptiScaler
            } else {
                config::FrameGeneration::Off
            },
            experimental: frame_generation,
            ..config::AiGraphicsConfig::default()
        };
        let mut settings = bigame_core::game_settings::load(process).unwrap_or_default();
        settings.ai_graphics = cfg.clone();
        bigame_core::game_settings::save(process, &settings)?;
        cfg
    } else {
        config::AiGraphicsConfig {
            mode: config::Mode::Recommended,
            ..config::AiGraphicsConfig::default()
        }
    };
    let analysis = graphics::analyze(&target, &cfg);
    println!(
        "PLAN [{:?}] {}",
        analysis.plan.standing, analysis.plan.summary
    );
    let done = graphics::install(&target, &analysis.plan, &cfg.version)?;
    let m = &done.manifest;
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
    if let Some(applied) = done.game_setting {
        println!("the game's own setting: {applied:?}");
    }
    for c in &m.settings {
        println!(
            "  {}\\{} = {} (was {:?})",
            c.key, c.value, c.set, c.original
        );
    }
    Ok(())
}
