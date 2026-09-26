//! What this machine can do for AI Graphics, backend by backend — and, with
//! a game named, what each backend is missing for it.
//!
//! Usage: `graphics_capabilities [<process-name>]`
use bigame_core::graphics::backend::{Availability, Backend};
use bigame_core::graphics::{self, config, external, report};
use bigame_core::hardware::Hardware;

fn main() {
    let hw = Hardware::detect();
    let (gpus, render) = report::gpu_infos(&hw, None);
    println!("GPUs");
    for (i, g) in gpus.iter().enumerate() {
        println!(
            "  {}{} · {} · {}{}",
            report::display_name(&g.name),
            if Some(i) == render {
                " (games render here)"
            } else {
                ""
            },
            g.family().label(),
            g.userspace.clone().unwrap_or_else(|| g.driver.clone()),
            match (g.vendor, g.fsr4(), g.dlss()) {
                (bigame_core::hardware::GpuVendor::Amd, true, _) => " · FSR 4 (FP8)",
                (bigame_core::hardware::GpuVendor::Nvidia, _, Some(true)) => " · DLSS",
                (bigame_core::hardware::GpuVendor::Nvidia, _, Some(false)) => " · no DLSS",
                _ => "",
            }
        );
    }
    println!("\nBackends");
    for b in Backend::ALL {
        let c = b.capabilities();
        println!(
            "  {:<22} upscaling {} · neural {} · frame generation {} · managed by BiGame-mode {} · {:?}",
            b.id(),
            yn(c.upscaling),
            yn(c.neural_rendering),
            yn(c.frame_generation),
            yn(c.managed),
            c.maturity
        );
    }
    println!("\nFrame generation");
    println!(
        "  lsfg-vk: layer {} · Lossless.dll {}",
        yn(bigame_core::fg::layer_installed()),
        yn(bigame_core::fg::is_lossless_dll_ready())
    );
    println!("  OptiScaler frame generation: with OptiScaler, DirectX 12, experimental");

    let Some(process) = std::env::args().nth(1) else {
        return;
    };
    let Some(target) = graphics::target_for_process(&process) else {
        eprintln!("no installed game runs as {process}");
        std::process::exit(1);
    };
    let cfg = config::AiGraphicsConfig {
        mode: config::Mode::Recommended,
        ..config::AiGraphicsConfig::default()
    };
    let a = graphics::analyze(&target, &cfg);
    println!("\n{}", target.name);
    for b in Backend::ALL {
        let avail = if b == Backend::AmdNeuralExternal {
            external::availability(&a.report)
        } else {
            bigame_core::graphics::backend::check(b, &a.report)
        };
        match avail {
            Availability::Available => println!("  {:<22} available", b.id()),
            Availability::Unavailable { missing } => {
                println!("  {:<22} unavailable", b.id());
                for m in missing {
                    println!("      missing {}: {}", m.what, m.detail);
                }
            }
        }
    }
    println!(
        "  native FSR 4 path: {} · plan: {} [{:?}] via {} · frame generation {:?}",
        yn(a.report.native_fsr4_path()),
        a.plan.summary,
        a.plan.standing,
        a.plan.backend.id(),
        a.plan.frame_generation
    );
    println!("  neural rendering: {:?}", a.neural);
}

fn yn(b: bool) -> &'static str {
    if b { "yes" } else { "no" }
}
