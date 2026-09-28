//! Download the Nara Linux shaders into a folder and print the style's file.
//!
//! `cargo run -p bigame-core --example vkbasalt_style -- <dir>`
fn main() -> anyhow::Result<()> {
    use bigame_core::vkbasalt::{self, Style};
    let dir = std::path::PathBuf::from(
        std::env::args()
            .nth(1)
            .unwrap_or_else(|| vkbasalt::shader_dir().to_string_lossy().into_owned()),
    );
    vkbasalt::fetch_shaders(&dir)?;
    eprintln!("shaders ready: {}", vkbasalt::shaders_ready_in(&dir));
    print!(
        "{}",
        vkbasalt::style_config(Style::NaraLinux, &dir).unwrap_or_default()
    );
    Ok(())
}
