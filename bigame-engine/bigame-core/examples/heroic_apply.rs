//! Write a game's own launch settings into Heroic's settings for it, as a
//! profile save does, or switch Proton's FSR 4 upgrade for it on or off.
//!
//! `cargo run -p bigame-core --example heroic_apply -- <process> [fsr4-on|fsr4-off]`
fn main() -> anyhow::Result<()> {
    let process = std::env::args()
        .nth(1)
        .ok_or_else(|| anyhow::anyhow!("usage: heroic_apply <process> [fsr4-on|fsr4-off]"))?;
    let applied = match std::env::args().nth(2).as_deref() {
        Some("fsr4-on") => bigame_core::optimization::set_heroic_fsr4_upgrade(&process, true)?,
        Some("fsr4-off") => bigame_core::optimization::set_heroic_fsr4_upgrade(&process, false)?,
        _ => bigame_core::optimization::apply_heroic(&process)?,
    };
    println!("{applied:?}");
    for t in bigame_core::heroic_launch::targets(&process) {
        println!("{}", t.file().display());
    }
    Ok(())
}
