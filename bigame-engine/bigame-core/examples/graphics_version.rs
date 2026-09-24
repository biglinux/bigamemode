//! Drive AI Graphics for an installed game through the same calls the page
//! makes — analyse, install, offer, update, go back, repair, remove — with
//! `OptiScaler` FSR chosen in Advanced mode.
//!
//! Usage:
//!   `graphics_version <process> install [<version>]`
//!   `graphics_version <process> offer`
//!   `graphics_version <process> update <version>`
//!   `graphics_version <process> go-back`
//!   `graphics_version <process> repair`
//!   `graphics_version <process> remove`
use bigame_core::graphics::{
    self,
    config::{AiGraphicsConfig, Layer, Mode, Upscaler, VersionPolicy},
    optiscaler, versions,
};

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    anyhow::ensure!(args.len() >= 2, "usage: graphics_version <process> <action> [<version>]");
    let target = graphics::target_for_process(&args[0])
        .ok_or_else(|| anyhow::anyhow!("no installed game runs as {}", args[0]))?;
    let mut cfg = AiGraphicsConfig {
        mode: Mode::Advanced,
        layer: Layer::OptiScaler,
        upscaler: Upscaler::Fsr,
        ..AiGraphicsConfig::default()
    };
    if let Some(v) = args.get(2) {
        cfg.version = VersionPolicy::Pinned(v.clone());
    }
    let plan = || graphics::analyze(&target, &cfg).plan;
    let print = |m: &graphics::manifest::Manifest| {
        println!(
            "{} {} — {} files, previous: {:?}",
            m.source.component,
            m.source.version,
            m.entries.len(),
            m.previous.as_ref().map(|p| &p.version)
        );
    };
    match args[1].as_str() {
        "install" => {
            let p = plan();
            println!("plan: {} [{:?}]", p.summary.english(), p.standing);
            print(&graphics::install(&target, &p, &cfg.version)?);
        }
        "offer" => println!("{:#?}", graphics::update_offer(&target, &cfg)),
        "update" => {
            let v = args.get(2).ok_or_else(|| anyhow::anyhow!("which version?"))?;
            let cache = optiscaler::cache_dir();
            let known = versions::load_fresh(&cache);
            let to = versions::resolve(&cache, &VersionPolicy::Pinned(v.clone()), &known)?;
            print(&graphics::update(&target, &plan(), &to)?);
        }
        "go-back" => print(&graphics::go_back(&target, &plan())?),
        "repair" => println!("put back: {:?}", graphics::repair(&target)?),
        "remove" => {
            for o in graphics::remove(&target)? {
                println!("{o:?}");
            }
        }
        other => anyhow::bail!("unknown action {other}"),
    }
    Ok(())
}
