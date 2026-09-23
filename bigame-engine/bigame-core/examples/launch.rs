//! Build and run a `LaunchPlan` for a real game, through the whole pipeline.
//!
//! Usage: launch [--gamescope] <program> [args...]
use bigame_core::gamescope;
use bigame_core::launcher::LaunchPlan;
use bigame_core::video_config::VideoConfig;

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let force_gs = args.first().is_some_and(|a| a == "--gamescope");
    if force_gs {
        args.remove(0);
    }
    let (program, rest) = args
        .split_first()
        .expect("usage: launch [--gamescope] <program>");

    let video = VideoConfig::default();
    // Deliberately do NOT set video.upscaling.gamescope_enabled: this exercises
    // the Auto decision, where the profile alone says what it wants.
    let profile_gs = if force_gs {
        Some(gamescope::Config {
            render_width: 1920,
            render_height: 1080,
            output_width: 2560,
            output_height: 1080,
            filter: gamescope::Filter::Fsr,
            sharpness: 4,
            frame_limit: gamescope::FrameLimit::NestedRefresh(60),
            mangoapp: false,
            adaptive_sync: false,
            hdr: false,
            fullscreen: false,
        })
    } else {
        None
    };

    let plan = LaunchPlan::build_with_args(program, rest, &video, profile_gs.as_ref());
    println!("program : {}", plan.program);
    println!("args    : {:?}", plan.args);
    println!("env     : {:?}", plan.env);
    println!("steam   : {:?}", plan.as_steam_launch_options());
    println!("\nspawning…");

    let mut child = plan.spawn()?;
    std::thread::sleep(std::time::Duration::from_secs(15));
    let alive = child.try_wait()?.is_none();
    println!("still running after 15s: {alive}");

    bigame_core::launcher::terminate(&mut child)?;
    std::thread::sleep(std::time::Duration::from_millis(800));

    // The point of the process group: nothing of the game survives.
    let leftover = std::process::Command::new("pgrep")
        .args(["-x", "supertuxkart"])
        .output()
        .is_ok_and(|o| !o.stdout.is_empty());
    println!("orphaned game process left behind: {leftover}");
    anyhow::ensure!(alive, "the game exited early");
    anyhow::ensure!(!leftover, "the game was orphaned");
    Ok(())
}
