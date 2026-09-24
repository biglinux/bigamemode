//! Support diagnostics.
//!
//! One report that answers "what is this machine and what is BiGame-mode doing
//! on it", assembled from the detection that already exists rather than from a
//! second set of probes. A support report built from its own parallel code
//! would eventually disagree with the application, and then it would be worse
//! than useless.
//!
//! # Redaction
//!
//! This text is meant to be pasted into a forum or an issue. Everything that
//! identifies a person or a network is removed **as the report is built**,
//! never afterwards as a filtering pass — a filter has to anticipate every
//! field, and the one it misses is the one that leaks.
//!
//! Removed: the username and home directory, hostnames, MAC addresses, public
//! IP addresses, Wi-Fi network names, Steam account ids, and serial numbers.
//! Kept: hardware models, driver and package versions, capabilities, and the
//! state of the knobs this project manages — which is what a supporter needs.

use std::fmt::Write as _;

use crate::capabilities::Capabilities;
use crate::hardware::{Chassis, Hardware, PowerSource, Session};

/// Replace the user's home directory and name with placeholders.
///
/// Paths are the most common way a username escapes into a report, and they
/// appear in library paths, game install directories and log lines.
#[must_use]
pub fn redact_paths(text: &str) -> String {
    let mut out = text.to_owned();
    if let Ok(home) = std::env::var("HOME") {
        if !home.is_empty() && home != "/" {
            out = out.replace(&home, "~");
        }
    }
    if let Ok(user) = std::env::var("USER") {
        // A very short username would match far too much unrelated text.
        if user.len() >= 3 {
            out = out.replace(&user, "<user>");
        }
    }
    out
}

/// Redact an IP address, keeping only enough to be useful.
///
/// Private addresses are kept whole — they say something about the setup and
/// nothing about the person. Anything routable is reduced to its family,
/// because a public address identifies a household.
#[must_use]
pub fn redact_address(addr: &std::net::IpAddr) -> String {
    match addr {
        std::net::IpAddr::V4(v4) => {
            if v4.is_private() || v4.is_loopback() || v4.is_link_local() {
                v4.to_string()
            } else {
                "<public IPv4>".to_owned()
            }
        }
        std::net::IpAddr::V6(v6) => {
            if v6.is_loopback() || (v6.segments()[0] & 0xfe00) == 0xfc00 {
                v6.to_string()
            } else {
                "<public IPv6>".to_owned()
            }
        }
    }
}

/// Build the support report.
///
/// `include_network` runs a DNS benchmark, which takes a few seconds and sends
/// queries; it is opt-in so a diagnostic can be produced offline and without
/// surprising traffic.
#[must_use]
pub fn report(include_network: bool) -> String {
    let hw = Hardware::detect();
    let caps = Capabilities::detect();
    let mut out = String::new();

    let _ = writeln!(out, "BiGame-mode diagnostics");
    let _ = writeln!(out, "version        {}", env!("CARGO_PKG_VERSION"));
    let _ = writeln!(out, "generated      {}", timestamp());
    let _ = writeln!(out);

    section_system(&mut out, &hw);
    section_cpu(&mut out, &hw);
    section_gpu(&mut out, &hw);
    section_display(&mut out, &hw);
    section_stack(&mut out, &caps);
    section_scheduler(&mut out, &caps);
    section_falcond(&mut out);
    section_booster(&mut out);
    if include_network {
        section_network(&mut out);
    }
    section_conflicts(&mut out, &caps);

    redact_paths(&out)
}

fn timestamp() -> String {
    // Date only: a precise time adds nothing to a bug report and is one more
    // thing that can correlate a user across reports.
    std::process::Command::new("date")
        .arg("+%Y-%m-%d")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map_or_else(|| "unknown".into(), |s| s.trim().to_owned())
}

fn section_system(out: &mut String, hw: &Hardware) {
    let _ = writeln!(out, "── System ──");
    let _ = writeln!(out, "  distribution {}", distribution());
    let _ = writeln!(out, "  kernel       {}", hw.kernel);
    let _ = writeln!(
        out,
        "  session      {}",
        match hw.session {
            Session::Wayland => "Wayland",
            Session::X11 => "X11",
            Session::Tty => "none (tty)",
        }
    );
    let _ = writeln!(
        out,
        "  desktop      {}",
        std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_else(|_| "unknown".into())
    );
    let _ = writeln!(
        out,
        "  chassis      {}",
        match hw.chassis {
            Chassis::Desktop => "desktop",
            Chassis::Laptop => "laptop",
            Chassis::Handheld => "handheld",
            Chassis::Unknown => "unknown",
        }
    );
    let _ = writeln!(
        out,
        "  power        {}",
        match hw.power_source {
            PowerSource::Ac => "AC",
            PowerSource::Battery => "battery",
            PowerSource::Unknown => "unknown",
        }
    );
    let _ = writeln!(out);
}

/// Distribution name, from `/etc/os-release`. Hostname is deliberately omitted.
fn distribution() -> String {
    std::fs::read_to_string("/etc/os-release")
        .ok()
        .and_then(|content| {
            content.lines().find_map(|line| {
                line.strip_prefix("PRETTY_NAME=")
                    .map(|v| v.trim_matches('"').to_owned())
            })
        })
        .unwrap_or_else(|| "unknown".into())
}

fn section_cpu(out: &mut String, hw: &Hardware) {
    let cpu = &hw.cpu;
    let _ = writeln!(out, "── CPU ──");
    let _ = writeln!(out, "  model        {}", cpu.model);
    let _ = writeln!(
        out,
        "  topology     {} cores / {} threads, SMT {}, hybrid {}",
        cpu.physical_cores,
        cpu.logical_cpus,
        if cpu.smt { "on" } else { "off" },
        if cpu.hybrid { "yes" } else { "no" }
    );
    let _ = writeln!(
        out,
        "  scaling      {} (governors: {})",
        cpu.scaling_driver.as_deref().unwrap_or("none"),
        if cpu.available_governors.is_empty() {
            "none".to_owned()
        } else {
            cpu.available_governors.join(", ")
        }
    );
    let _ = writeln!(
        out,
        "  governor     {}",
        cpu.current_governor.as_deref().unwrap_or("unknown")
    );
    let _ = writeln!(
        out,
        "  EPP          {}",
        cpu.current_epp.as_deref().unwrap_or("not available")
    );
    if let Some(status) = &cpu.amd_pstate_status {
        let _ = writeln!(out, "  amd_pstate   {status}");
    }
    let _ = writeln!(
        out,
        "  3D V-Cache   {}",
        cpu.vcache.as_ref().map_or_else(
            || "not present".to_owned(),
            |v| v.current_mode.clone().unwrap_or_else(|| "present".into())
        )
    );
    let _ = writeln!(out);
}

fn section_gpu(out: &mut String, hw: &Hardware) {
    let _ = writeln!(out, "── GPU ──");
    if hw.gpus.is_empty() {
        let _ = writeln!(out, "  none detected");
    }
    for (i, gpu) in hw.gpus.iter().enumerate() {
        let role = if hw.render_gpu == Some(i) {
            "  <- renders games"
        } else {
            ""
        };
        let _ = writeln!(
            out,
            "  {} {:?} {} driver {}{}",
            gpu.card, gpu.vendor, gpu.pci_id, gpu.driver, role
        );
        let _ = writeln!(
            out,
            "     vram {} · {} · dpm {}",
            gpu.vram_total_bytes.map_or_else(
                || "unknown".to_owned(),
                |b| format!("{} MiB", b / 1_048_576)
            ),
            if gpu.discrete {
                "discrete"
            } else {
                "integrated"
            },
            gpu.dpm_level().unwrap_or_else(|| "n/a".into())
        );
        if let Some(t) = gpu.hwmon_u64("temp1_input") {
            let _ = write!(out, "     {} °C", t / 1000);
            if let Some(p) = gpu.hwmon_u64("power1_average") {
                let _ = write!(out, " · {} W", p / 1_000_000);
            }
            if let Some(b) = gpu.busy_percent() {
                let _ = write!(out, " · {b}% busy");
            }
            let _ = writeln!(out);
        }
    }
    let _ = writeln!(out);
}

fn section_display(out: &mut String, hw: &Hardware) {
    let _ = writeln!(out, "── Displays ──");
    if hw.displays.is_empty() {
        let _ = writeln!(out, "  none connected");
    }
    for d in &hw.displays {
        let _ = writeln!(
            out,
            "  {} on {} · max {} · VRR {}",
            d.connector,
            d.card,
            d.max_mode
                .map_or_else(|| "unknown".to_owned(), |(w, h)| format!("{w}x{h}")),
            match d.vrr_capable {
                Some(true) => "yes",
                Some(false) => "no",
                None => "not reported by the kernel",
            }
        );
    }
    let _ = writeln!(out);
}

fn section_stack(out: &mut String, caps: &Capabilities) {
    let _ = writeln!(out, "── Gaming stack ──");
    match &caps.gamescope {
        Some(gs) => {
            let _ = writeln!(
                out,
                "  gamescope    {} ({} options)",
                gs.version
                    .map_or_else(|| "unknown version".to_owned(), |v| v.to_string()),
                gs.flags.len()
            );
            let _ = writeln!(
                out,
                "               -F {} · --adaptive-sync {} · --hdr-enabled {}",
                yes_no(gs.has_flag("F")),
                yes_no(gs.has_flag("adaptive-sync")),
                yes_no(gs.has_flag("hdr-enabled"))
            );
        }
        None => {
            let _ = writeln!(out, "  gamescope    not installed");
        }
    }
    let _ = writeln!(out, "  mangohud     {}", yes_no(caps.mangohud));
    let _ = writeln!(out, "  mangoapp     {}", yes_no(caps.mangoapp));
    let _ = writeln!(out, "  vkBasalt     {}", yes_no(caps.vkbasalt));
    let _ = writeln!(out, "  lsfg-vk      {}", yes_no(caps.lsfg_vk));
    let _ = writeln!(out, "  steam        {}", yes_no(caps.steam));
    let _ = writeln!(out, "  GameMode     {}", yes_no(caps.gamemode));
    let _ = writeln!(
        out,
        "  power-profiles-daemon {} ({})",
        yes_no(caps.power_profiles),
        if caps.power_profiles_available.is_empty() {
            "no profiles".to_owned()
        } else {
            caps.power_profiles_available.join(", ")
        }
    );
    let _ = writeln!(
        out,
        "  active profile        {}",
        crate::dbus::power_profile_get().unwrap_or_else(|| "unknown".into())
    );
    let _ = writeln!(out);
}

fn section_scheduler(out: &mut String, caps: &Capabilities) {
    let scx = &caps.sched_ext;
    let _ = writeln!(out, "── sched-ext ──");
    let _ = writeln!(out, "  kernel support {}", yes_no(scx.kernel_support));
    let _ = writeln!(
        out,
        "  state          {}",
        scx.state.as_deref().unwrap_or("unknown")
    );
    let _ = writeln!(out, "  scxctl         {}", yes_no(scx.scxctl));
    let _ = writeln!(out, "  scx_loader     {}", yes_no(scx.loader_service));
    let _ = writeln!(
        out,
        "  installed ({})  {}",
        scx.installed.len(),
        if scx.installed.is_empty() {
            "none".to_owned()
        } else {
            scx.installed.join(", ")
        }
    );
    let support = scx.switchable();
    let _ = writeln!(
        out,
        "  switchable     {}",
        support.reason().unwrap_or("yes")
    );
    let _ = writeln!(out);
}

fn section_falcond(out: &mut String) {
    let _ = writeln!(out, "── falcond ──");
    let path = crate::status::status_path();
    let _ = writeln!(out, "  status file  {}", path.display());
    let _ = writeln!(
        out,
        "  trusted      {}",
        yes_no(crate::status::is_trustworthy(path))
    );
    match crate::status::read() {
        Some(status) => {
            let _ = writeln!(
                out,
                "  performance  {}",
                yes_no(status.performance_available)
            );
            let _ = writeln!(out, "  profile mode {}", status.profile_mode);
            let _ = writeln!(out, "  global scx   {}", status.config_scx);
            let _ = writeln!(out, "  global vcache {}", status.config_vcache);
            let _ = writeln!(out, "  profiles     {}", status.loaded_profiles);
            let _ = writeln!(
                out,
                "  active       {}",
                status.active_profile.as_deref().unwrap_or("none")
            );
        }
        None => {
            let _ = writeln!(out, "  status       unavailable (falcond not running?)");
        }
    }
    let _ = writeln!(
        out,
        "  config       {}",
        if std::path::Path::new(crate::config::CONFIG_PATH).exists() {
            crate::config::CONFIG_PATH
        } else {
            "missing"
        }
    );
    let _ = writeln!(out);
}

fn section_booster(out: &mut String) {
    let _ = writeln!(out, "── Booster ──");
    match crate::booster::BoosterEngine::active_summary() {
        Some(n) => {
            let _ = writeln!(out, "  active       yes, {n} change(s) in force");
        }
        None => {
            let _ = writeln!(out, "  active       no");
        }
    }
    let engine = crate::booster::BoosterEngine::detect();
    let (snapshot, plan) = engine.dry_run();
    let _ = writeln!(out, "  current state:");
    for (id, captured) in &snapshot.entries {
        let _ = writeln!(
            out,
            "    {id:<22} {}",
            captured.value.as_deref().unwrap_or("unreadable")
        );
    }
    let _ = writeln!(out, "  plan ({} change(s)):", plan.changes.len());
    for change in &plan.changes {
        let _ = writeln!(
            out,
            "    {:<22} {} -> {}",
            change.knob.id(),
            change.from,
            change.to
        );
    }
    for skipped in &plan.skipped {
        let _ = writeln!(out, "    skipped: {skipped:?}");
    }
    let _ = writeln!(out);
}

fn section_network(out: &mut String) {
    let _ = writeln!(out, "── Network ──");
    match crate::network::primary_link() {
        Some(link) => {
            let _ = writeln!(
                out,
                "  interface    {} ({:?})",
                // The name itself is not identifying; the MAC and addresses are,
                // and neither is included.
                link.name,
                link.medium
            );
            let _ = writeln!(
                out,
                "  link         {} · MTU {}",
                link.speed_mbps
                    .map_or_else(|| "unknown speed".to_owned(), |s| format!("{s} Mb/s")),
                link.mtu
                    .map_or_else(|| "unknown".to_owned(), |m| m.to_string())
            );
            let _ = writeln!(
                out,
                "  qdisc        {} ({})",
                link.qdisc.as_deref().unwrap_or("unknown"),
                if link.has_modern_qdisc() {
                    "latency-managing"
                } else {
                    "not latency-managing"
                }
            );
            if let Some(gw) = link.gateway {
                let _ = writeln!(out, "  gateway      {}", redact_address(&gw));
            }
        }
        None => {
            let _ = writeln!(out, "  no default route");
        }
    }
    let resolvers = crate::network::system_resolvers();
    let _ = writeln!(
        out,
        "  resolvers    {}",
        if resolvers.is_empty() {
            "none configured".to_owned()
        } else {
            resolvers
                .iter()
                .map(redact_address)
                .collect::<Vec<_>>()
                .join(", ")
        }
    );
    let _ = writeln!(out);
}

fn section_conflicts(out: &mut String, caps: &Capabilities) {
    let _ = writeln!(out, "── Conflicts ──");
    let mut found = 0usize;

    if caps.gamemode && caps.falcond_running {
        found += 1;
        let _ = writeln!(
            out,
            "  ! Feral GameMode and falcond are both present. Both snapshot and \
             restore the same state independently; see docs/02."
        );
    }

    // A Steam launch option naming a program that is not installed stops the
    // game from starting, with nothing in Steam's UI to explain it.
    if let Ok(home) = std::env::var("HOME") {
        for user in crate::steam::users(std::path::Path::new(&home)) {
            for broken in crate::steam::broken_launch_options(&user.config) {
                found += 1;
                let _ = writeln!(
                    out,
                    "  ! Steam app {} has launch options calling '{}', which is not \
                     installed — that game will not start.",
                    broken.app_id, broken.missing
                );
            }
        }
    }

    if found == 0 {
        let _ = writeln!(out, "  none detected");
    }
    let _ = writeln!(out);
}

fn yes_no(v: bool) -> &'static str {
    if v { "yes" } else { "no" }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_home_directory_never_appears() {
        let home = std::env::var("HOME").unwrap_or_default();
        if home.is_empty() {
            return;
        }
        let text = format!("game at {home}/Games/thing and lib at {home}/.local/lib");
        let redacted = redact_paths(&text);
        assert!(!redacted.contains(&home), "home leaked: {redacted}");
        assert!(redacted.contains("~/Games/thing"));
    }

    #[test]
    fn the_username_never_appears() {
        let Ok(user) = std::env::var("USER") else {
            return;
        };
        if user.len() < 3 {
            return;
        }
        let redacted = redact_paths(&format!("owned by {user} in group {user}"));
        assert!(!redacted.contains(&user));
        assert!(redacted.contains("<user>"));
    }

    #[test]
    fn private_addresses_are_kept_and_public_ones_are_not() {
        let private: std::net::IpAddr = "192.168.0.1".parse().unwrap();
        assert_eq!(redact_address(&private), "192.168.0.1");

        let loopback: std::net::IpAddr = "127.0.0.1".parse().unwrap();
        assert_eq!(redact_address(&loopback), "127.0.0.1");

        // A public address identifies a household; the family is enough.
        let public: std::net::IpAddr = "1.1.1.1".parse().unwrap();
        assert_eq!(redact_address(&public), "<public IPv4>");

        let public6: std::net::IpAddr = "2804:14c::1".parse().unwrap();
        assert_eq!(redact_address(&public6), "<public IPv6>");

        // Unique-local IPv6 is the private equivalent and is kept.
        let ula: std::net::IpAddr = "fd7a:115c:a1e0::1".parse().unwrap();
        assert_eq!(redact_address(&ula), "fd7a:115c:a1e0::1");
    }

    #[test]
    fn the_report_carries_no_personal_identifiers() {
        let text = report(false);

        if let Ok(home) = std::env::var("HOME") {
            assert!(!text.contains(&home), "home directory leaked");
        }
        if let Ok(user) = std::env::var("USER") {
            if user.len() >= 3 {
                assert!(!text.contains(&user), "username leaked");
            }
        }
        // The hostname is never collected, so it must not appear either.
        if let Ok(host) = std::fs::read_to_string("/etc/hostname") {
            let host = host.trim();
            if host.len() >= 4 {
                assert!(!text.contains(host), "hostname leaked");
            }
        }
    }

    #[test]
    fn the_report_answers_the_questions_support_asks() {
        let text = report(false);
        for heading in [
            "── System ──",
            "── CPU ──",
            "── GPU ──",
            "── Displays ──",
            "── Gaming stack ──",
            "── sched-ext ──",
            "── falcond ──",
            "── Booster ──",
            "── Conflicts ──",
        ] {
            assert!(text.contains(heading), "missing section {heading}");
        }
        assert!(text.contains("BiGame-mode diagnostics"));
    }

    #[test]
    fn the_network_section_is_opt_in() {
        assert!(!report(false).contains("── Network ──"));
        assert!(report(true).contains("── Network ──"));
    }
}
