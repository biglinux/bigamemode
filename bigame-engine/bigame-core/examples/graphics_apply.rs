//! Install `OptiScaler` into one game through the AI Graphics core, or remove
//! it, exactly as the product does: scan, choose the slot, build the payload,
//! apply as a transaction (or roll it back).
//!
//! Usage:
//!   `graphics_apply <install-dir> <steam-appid> <process-name> [xess|dlss|fsr] [--watermark]`
//!   `graphics_apply <install-dir> <steam-appid> <process-name> --remove`
use bigame_core::graphics::{manifest, optiscaler, scan, transaction};
use std::path::{Path, PathBuf};

fn state_dir() -> PathBuf {
    std::env::var_os("XDG_STATE_HOME")
        .map_or_else(
            || PathBuf::from(std::env::var_os("HOME").unwrap()).join(".local/state"),
            PathBuf::from,
        )
        .join("bigame-mode/graphics")
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    anyhow::ensure!(
        args.len() >= 3,
        "usage: graphics_apply <install-dir> <appid> <process> [...]"
    );
    let root = Path::new(&args[0]);
    let s = scan::scan(root, Some(&args[2]));
    let exe = s
        .executable
        .clone()
        .ok_or_else(|| anyhow::anyhow!("no executable found"))?;
    let key = manifest::game_key(Some(&args[1]), &args[2], root);
    let state = state_dir();
    if args.iter().any(|a| a == "--remove") {
        for o in transaction::remove(&state, &key)? {
            println!("{o:?}");
        }
        return Ok(());
    }
    anyhow::ensure!(
        s.anti_cheat.is_empty(),
        "anti-cheat found: {:?}",
        s.anti_cheat
    );
    let slot = optiscaler::choose_slot(&s)
        .map_err(|t| anyhow::anyhow!("{} is taken by {:?}", t.slot, t.owner))?;
    let input = match args.get(3).map(String::as_str) {
        Some("dlss") => optiscaler::Input::Dlss,
        Some("fsr") => optiscaler::Input::Fsr,
        _ => optiscaler::Input::Xess,
    };
    let o = optiscaler::Options {
        proxy: slot,
        api: optiscaler::Api::Dx12,
        input,
        output: optiscaler::Output::Fsr,
        frame_gen: optiscaler::FrameGen::Off,
        nvidia: false,
        dlss: false,
        watermark: args.iter().any(|a| a == "--watermark"),
    };
    let cached = optiscaler::fetch(
        &optiscaler::cache_dir(),
        &optiscaler::Release::recommended(),
    )?;
    let exe_dir = exe.parent().unwrap_or(Path::new("")).to_path_buf();
    let files = optiscaler::payload(&cached, &o, &exe_dir, &state.join(&key).join("staging"))?;
    let game = transaction::Game {
        key: &key,
        root,
        process: Some(&args[2]),
        title: None,
    };
    let m = transaction::apply(
        &state,
        &game,
        cached.source(),
        &files,
        &[exe_dir.join("OptiScaler.log")],
    )?;
    println!(
        "installed {} {} into {} as {}",
        m.source.component, m.source.version, key, o.proxy
    );
    for e in &m.entries {
        println!(
            "  {}  {}",
            e.path.display(),
            if e.replaced.is_some() {
                "(original backed up)"
            } else {
                "(new)"
            }
        );
    }
    Ok(())
}
