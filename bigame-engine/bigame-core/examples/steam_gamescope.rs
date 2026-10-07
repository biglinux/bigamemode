//! Write a Steam game's Gamescope wrapper into its launch options, as a
//! profile saved with Gamescope "Always" does, or take Big Game Mode's out.
//! Steam must be closed.
//!
//! `cargo run -p bigame-core --example steam_gamescope -- <process> <on|off>`
fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let usage = || anyhow::anyhow!("usage: steam_gamescope <process> <on|off>");
    let process = args.next().ok_or_else(usage)?;
    let on = match args.next().as_deref() {
        Some("on") => true,
        Some("off") => false,
        _ => return Err(usage()),
    };
    let segment = on
        .then(|| {
            bigame_core::steam_gamescope::segment(
                bigame_core::gamescope::Mode::Enabled,
                &bigame_core::gamescope::Config::default()
                    .with_screen_output(bigame_core::screen::primary_size()),
                bigame_core::capabilities::gamescope_cached().as_ref(),
                bigame_core::hardware::detect_session(),
            )
        })
        .flatten();
    println!("wrapper: {}", segment.as_deref().unwrap_or("none"));
    let applied = bigame_core::steam_gamescope::apply(
        &process,
        bigame_core::steam_gamescope::Wanted {
            gamescope: segment,
            ..Default::default()
        },
    )?;
    println!("{applied:?}");
    Ok(())
}
