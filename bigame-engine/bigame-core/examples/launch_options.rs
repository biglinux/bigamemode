//! Read or set a Steam game's launch options, through the same function the
//! Diagnostics page uses (refused while Steam runs; backed up; read back).
//!
//! Usage: `launch_options <appid>` · `launch_options <appid> <value>` (`""` clears)
fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let app = args
        .first()
        .ok_or_else(|| anyhow::anyhow!("usage: launch_options <appid> [value]"))?;
    let home = std::path::PathBuf::from(std::env::var("HOME")?);
    let configs = glob_localconfigs(&home);
    for c in &configs {
        if let Some(v) = args.get(1) {
            bigame_core::steam::set_launch_options(c, app, v)?;
        }
        println!(
            "{}: {:?}",
            c.display(),
            bigame_core::steam::launch_options(c, app)
        );
    }
    Ok(())
}

fn glob_localconfigs(home: &std::path::Path) -> Vec<std::path::PathBuf> {
    let userdata = home.join(".local/share/Steam/userdata");
    std::fs::read_dir(userdata)
        .map(|d| {
            d.flatten()
                .map(|e| e.path().join("config/localconfig.vdf"))
                .filter(|p| p.is_file())
                .collect()
        })
        .unwrap_or_default()
}
