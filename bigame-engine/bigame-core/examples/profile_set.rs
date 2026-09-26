//! Change a game's falcond profile the way the Profiles page saves it: through
//! the helper (Polkit asks for the administrator password), then falcond reloads.
//!
//! Usage: `profile_set <process> key=value …`
//! Keys: `scx_sched`, `scx_sched_props`, `performance_mode`, `idle_inhibit`, `fg_multiplier`
fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let name = args
        .first()
        .ok_or_else(|| anyhow::anyhow!("usage: profile_set <process> key=value …"))?;
    // A game without a profile yet starts from what the recommendation writes
    // (performance mode, idle inhibit, no scheduler, no V-Cache preference).
    let mut p = match bigame_core::profiles::load(name) {
        Ok(p) => p,
        Err(_) => toml::from_str::<bigame_core::profiles::GameProfile>(&format!(
            "name = {name:?}\nperformance_mode = true\nidle_inhibit = true\n\
             scx_sched = \"none\"\nscx_sched_props = \"default\"\nvcache_mode = \"none\"\n"
        ))?,
    };
    for kv in &args[1..] {
        let (k, v) = kv
            .split_once('=')
            .ok_or_else(|| anyhow::anyhow!("not key=value: {kv}"))?;
        match k {
            "scx_sched" => v.clone_into(&mut p.scx_sched),
            "scx_sched_props" => v.clone_into(&mut p.scx_sched_props),
            "performance_mode" => p.performance_mode = v == "true",
            "idle_inhibit" => p.idle_inhibit = v == "true",
            "fg_multiplier" => p.fg_multiplier = v.parse()?,
            other => anyhow::bail!("unknown key {other}"),
        }
    }
    bigame_core::profiles::save(&p)?;
    println!(
        "{}",
        std::fs::read_to_string(format!("/usr/share/falcond/profiles/user/{name}.conf"))?
    );
    Ok(())
}
