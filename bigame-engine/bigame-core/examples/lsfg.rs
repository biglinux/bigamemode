//! Read or set a game's lsfg-vk frame generation, as the Profiles page does.
//!
//! Usage: `lsfg show <exe>` · `lsfg set <exe> <multiplier> [flow%]` · `lsfg off <exe>`
fn main() -> anyhow::Result<()> {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let exe = a.get(1).map_or("", String::as_str);
    match a.first().map(String::as_str) {
        Some("set") => {
            let m = a.get(2).and_then(|v| v.parse().ok()).unwrap_or(2);
            let f = a.get(3).and_then(|v| v.parse().ok()).unwrap_or(100);
            bigame_core::fg::write_profile(exe, m, f, false, false, 1)?;
        }
        Some("off") => bigame_core::fg::disable_for_game(exe)?,
        _ => {}
    }
    println!("layer installed: {}", bigame_core::fg::layer_installed());
    println!("dll ready: {}", bigame_core::fg::is_lossless_dll_ready());
    println!("{exe}: {:?}", bigame_core::fg::read_profile(exe));
    Ok(())
}
