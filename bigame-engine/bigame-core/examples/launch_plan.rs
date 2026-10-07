//! Print the command Big Game Mode's launcher would run for an executable,
//! with Tuning's settings and the Turbo preset in force.
//!
//! Usage: `launch_plan <executable> [args…]`
fn main() {
    let mut args = std::env::args().skip(1);
    let Some(exe) = args.next() else {
        eprintln!("usage: launch_plan <executable> [args…]");
        std::process::exit(2);
    };
    let rest: Vec<String> = args.collect();
    let name = std::path::Path::new(&exe)
        .file_name()
        .map_or_else(|| exe.clone(), |n| n.to_string_lossy().into_owned());
    let video = bigame_core::video_config::load();
    let plan = bigame_core::launcher::LaunchPlan::build_for_game(
        &exe,
        &rest,
        &name,
        &video,
        None,
        bigame_core::gamescope::Mode::Auto,
    );
    let mut env: Vec<_> = plan.env.iter().collect();
    env.sort();
    for (k, v) in env {
        println!("env {k}={v}");
    }
    println!("program {}", plan.program);
    for a in &plan.args {
        println!("arg {a}");
    }
}
