//! Build the launch plan BiGame-mode would use for a program and run it.
//!
//! Usage: `launch_plan [--video] <program> [args…]` — prints the plan, then
//! runs it with the plan's environment and waits. Used to check on real
//! hardware that a plan does what it says (e.g. `launch_plan glxinfo -B` on a
//! hybrid laptop). `--video` uses the saved Video settings (Gamescope, Wine
//! FSR, vkBasalt) instead of the defaults.
fn main() -> anyhow::Result<()> {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let saved = args.first().is_some_and(|a| a == "--video");
    if saved {
        args.remove(0);
    }
    let (program, rest) = args
        .split_first()
        .ok_or_else(|| anyhow::anyhow!("usage: launch_plan [--video] <program> [args…]"))?;
    let video = if saved {
        bigame_core::video_config::load()
    } else {
        bigame_core::video_config::VideoConfig::default()
    };
    let plan = bigame_core::launcher::LaunchPlan::build_with_args(program, rest, &video, None);
    let mut env: Vec<_> = plan.env.iter().collect();
    env.sort();
    for (k, v) in &env {
        println!("env {k}={v}");
    }
    println!("run {} {}", plan.program, plan.args.join(" "));
    let status = std::process::Command::new(&plan.program)
        .args(&plan.args)
        .envs(&plan.env)
        .status()?;
    std::process::exit(status.code().unwrap_or(1));
}
