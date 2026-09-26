//! The Native backend's one action, as the page's Apply does it: switch the
//! FSR 4 upgrade through Proton on or off for a Steam game, by its launch
//! options (Steam closed; backed up; read back).
//!
//! Usage: `graphics_native <process-name> fsr4 on|off` · `graphics_native <process-name>`
use bigame_core::graphics::{self, fsr4_upgrade};

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(process) = args.first() else {
        anyhow::bail!("usage: graphics_native <process-name> [fsr4 on|off]");
    };
    let target = graphics::target_for_process(process)
        .ok_or_else(|| anyhow::anyhow!("no installed game runs as {process}"))?;
    if args.get(1).map(String::as_str) == Some("fsr4") {
        let on = args.get(2).map(String::as_str) == Some("on");
        println!("{:?}", fsr4_upgrade::apply(target.app_id.as_deref(), on)?);
    }
    println!(
        "{}: FSR4_UPGRADE in Steam launch options: {}",
        target.name,
        fsr4_upgrade::is_enabled(target.app_id.as_deref())
    );
    Ok(())
}
