//! Why AI Graphics is or is not working for a game, as the page's
//! Diagnose shows it.
//!
//! Usage: `graphics_diagnose <process-name>`
use bigame_core::graphics::{self, config, diagnose};

fn main() {
    let Some(process) = std::env::args().nth(1) else {
        eprintln!("usage: graphics_diagnose <process-name>");
        std::process::exit(2);
    };
    let Some(target) = graphics::target_for_process(&process) else {
        eprintln!("no installed game runs as {process}");
        std::process::exit(1);
    };
    let cfg = bigame_core::game_settings::load(&target.process)
        .map(|s| s.ai_graphics)
        .unwrap_or_default();
    let cfg = if cfg.mode == config::Mode::Off {
        config::AiGraphicsConfig {
            mode: config::Mode::Recommended,
            ..cfg
        }
    } else {
        cfg
    };
    let a = graphics::analyze(&target, &cfg);
    print!("{}", diagnose::render(&diagnose::diagnose(&a)));
}
