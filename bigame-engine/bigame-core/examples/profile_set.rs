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
    let mut p = bigame_core::profiles::load(name)?;
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
