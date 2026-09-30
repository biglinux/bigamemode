//! Support diagnostics.
//!
//! One report that answers "what is this machine and what is Big Game Mode doing
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

    let _ = writeln!(out, "Big Game Mode diagnostics");
    let _ = writeln!(out, "version        {}", env!("CARGO_PKG_VERSION"));
    let _ = writeln!(out, "generated      {}", timestamp());
    let _ = writeln!(out);

    section_bigame(&mut out);
    section_system(&mut out, &hw);
    section_cpu(&mut out, &hw);
    section_gpu(&mut out, &hw);
    section_display(&mut out, &hw);
    section_stack(&mut out, &caps);
    section_versions(&mut out, std::path::Path::new(crate::health::PACMAN_DB));
    section_scheduler(&mut out, &caps);
    section_falcond(&mut out);
    section_booster(&mut out);
    if include_network {
        section_network(&mut out);
    }
    section_conflicts(&mut out, &caps);
    section_files(&mut out);

    redact_paths(&out)
}

fn timestamp() -> String {
    // SAFETY: `time(NULL)` only reads the clock.
    date(unsafe { libc::time(std::ptr::null_mut()) })
}

/// The local date of `when`, in seconds since the epoch.
fn date(when: libc::time_t) -> String {
    // Date only: a precise time adds nothing to a bug report and is one more
    // thing that can correlate a user across reports.
    // SAFETY: `localtime_r` only reads `when` and writes into the `tm` it is
    // given.
    let tm = unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        if libc::localtime_r(&raw const when, &raw mut tm).is_null() {
            return "unknown".into();
        }
        tm
    };
    format!(
        "{:04}-{:02}-{:02}",
        tm.tm_year + 1900,
        tm.tm_mon + 1,
        tm.tm_mday
    )
}

/// What Big Game Mode itself is doing: the first thing support asks.
fn section_bigame(out: &mut String) {
    use crate::turbo::Section;

    let _ = writeln!(out, "── Big Game Mode ──");
    let reader = crate::systemd::Reader::shared();
    let unit = reader.and_then(|r| r.unit_state(crate::turbo::BACKEND_UNIT));
    // As Home reads it (turbo::state): falcond's unit when it is installed,
    // Booster's journal without it.
    let turbo = match &unit {
        Some(u) if u.is_installed() => on_off(u.is_active()),
        Some(_) => on_off(crate::booster::BoosterEngine::is_active()),
        None => "unknown (systemd did not answer)",
    };
    let _ = writeln!(out, "  Turbo        {turbo}");
    match &unit {
        Some(u) if u.is_installed() => {
            let _ = writeln!(
                out,
                "  falcond unit {} · {} · {} automatic restart(s)",
                u.active_state,
                u.unit_file_state,
                reader
                    .and_then(|r| r.restarts(crate::turbo::BACKEND_UNIT))
                    .map_or_else(|| "unknown".to_owned(), |n| n.to_string())
            );
        }
        Some(_) => {
            let _ = writeln!(out, "  falcond unit not installed");
        }
        None => {}
    }
    let _ = writeln!(
        out,
        "  preset       {} in force · {} chosen for the next Turbo",
        crate::turbo_preset::active().label(),
        crate::turbo_preset::chosen().label()
    );
    let _ = writeln!(
        out,
        "  owns falcond {}",
        crate::turbo::owned_since()
            .and_then(|t| libc::time_t::try_from(t).ok())
            .map_or_else(|| "no".to_owned(), |t| format!("since {}", date(t)))
    );
    match crate::turbo::Report::load_last() {
        Some(report) => {
            let counts: Vec<String> = [
                (Section::Verified, "verified"),
                (Section::Restored, "restored"),
                (Section::ManagedPerGame, "managed per game"),
                (Section::Skipped, "skipped"),
                (Section::Unavailable, "unavailable"),
                (Section::ConflictAvoided, "conflict avoided"),
                (Section::Failed, "failed"),
            ]
            .into_iter()
            .filter_map(|(section, name)| {
                let n = report.count(section);
                (n > 0).then(|| format!("{n} {name}"))
            })
            .collect();
            let _ = writeln!(
                out,
                "  last Turbo   switched {} {} · {}",
                on_off(report.turned_on),
                libc::time_t::try_from(report.at).map_or_else(|_| "unknown".to_owned(), date),
                if counts.is_empty() {
                    "nothing to do".to_owned()
                } else {
                    counts.join(", ")
                }
            );
            // What failed is what a supporter asks about next; the detail
            // is kept in English for exactly this.
            for item in report.items.iter().filter(|i| i.section == Section::Failed) {
                let _ = writeln!(out, "     failed: {} ({})", item.detail, item.owner);
            }
        }
        None => {
            let _ = writeln!(out, "  last Turbo   no report yet");
        }
    }
    let _ = writeln!(out);
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
    // The model's name and the userspace driver's version, as AI Graphics
    // reads them: which Mesa or NVIDIA release is the usual first question.
    let (infos, _) = crate::graphics::report::gpu_infos(hw, None);
    for (i, gpu) in hw.gpus.iter().enumerate() {
        let role = if hw.render_gpu == Some(i) {
            "  <- renders games"
        } else {
            ""
        };
        let _ = writeln!(
            out,
            "  {} {:?} {} driver {}{}",
            gpu.node(),
            gpu.vendor,
            gpu.pci_id,
            gpu.driver,
            role
        );
        if let Some(info) = infos.get(i) {
            let _ = writeln!(
                out,
                "     {} · {}",
                info.name,
                info.userspace
                    .as_deref()
                    .unwrap_or("userspace driver unknown")
            );
        }
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

/// The packages that decide how a game runs. The version a problem was seen
/// with is the first thing a fix is checked against.
const PACKAGES: [&str; 13] = [
    "bigame-mode",
    "falcond",
    "falcond-profiles",
    "gamescope",
    "mangohud",
    "vkbasalt",
    "lsfg-vk",
    "scx-scheds",
    "scx-tools",
    "power-profiles-daemon",
    "mesa",
    "gtk4",
    "libadwaita",
];

/// Listed only when installed: each belongs to one vendor or setup, and
/// "not installed" would read as a fault on every other machine.
const OPTIONAL_PACKAGES: [&str; 10] = [
    "lib32-mangohud",
    "lib32-vkbasalt",
    "vulkan-radeon",
    "lib32-vulkan-radeon",
    "vulkan-intel",
    "lib32-vulkan-intel",
    "nvidia-utils",
    "lib32-nvidia-utils",
    "steam",
    "gamemode",
];

/// Package versions, from pacman's database: what is installed, which is not
/// always what runs (the kernel and falcond's own status say that).
fn section_versions(out: &mut String, db: &std::path::Path) {
    let _ = writeln!(out, "── Versions ──");
    if !db.is_dir() {
        let _ = writeln!(out, "  no pacman database at {}", db.display());
        let _ = writeln!(out);
        return;
    }
    for name in PACKAGES {
        let version = crate::health::package_version(db, name);
        let _ = writeln!(
            out,
            "  {name:<22} {}",
            version.as_deref().unwrap_or("not installed")
        );
    }
    for name in OPTIONAL_PACKAGES {
        if let Some(version) = crate::health::package_version(db, name) {
            let _ = writeln!(out, "  {name:<22} {version}");
        }
    }
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
            let _ = writeln!(
                out,
                "  live         performance mode {} · scx {} · vcache {} · screen kept awake {}",
                on_off(status.perf_mode_active),
                or_none(&status.current_scx),
                or_none(&status.current_vcache),
                yes_no(status.screensaver_inhibited)
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
        let _ = writeln!(out, "    skipped: {}", skipped_line(skipped));
    }
    let _ = writeln!(out);
}

/// A knob the plan left alone, and why, as one line of the report.
fn skipped_line(skipped: &crate::booster::plan::Skipped) -> String {
    use crate::booster::plan::Skipped;
    match skipped {
        Skipped::Unsupported { knob, detail } => {
            format!("{}: not supported, {}", knob.english(), detail.english())
        }
        Skipped::AlreadyOptimal { knob, value } => {
            format!("{}: already {value}", knob.english())
        }
        Skipped::NotBeneficial { knob, detail } => {
            format!("{}: not changed, {}", knob.english(), detail.english())
        }
        Skipped::OwnedBy {
            knob,
            owner,
            detail,
        } => format!("{}: owned by {owner}, {}", knob.english(), detail.english()),
        Skipped::NotRestorable { knob } => {
            format!(
                "{}: its value could not be read, so it could not be put back",
                knob.english()
            )
        }
    }
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
             restore the same state independently, so each can undo the other's changes."
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

/// Where Big Game Mode keeps its files and where its logs go, so a supporter
/// can ask for the right one.
fn section_files(out: &mut String) {
    let _ = writeln!(out, "── Files and logs ──");
    let config = crate::paths::config_home().join("bigame-mode");
    let state = crate::paths::state_home().join("bigame-mode");
    let _ = writeln!(out, "  settings     {}", listing(&config));
    let _ = writeln!(out, "  state        {}", listing(&state));
    let _ = writeln!(
        out,
        "  environment  {}",
        present(&crate::video_config::env_file_path())
    );
    let _ = writeln!(out, "  lsfg-vk      {}", present(&crate::fg::config_path()));
    let _ = writeln!(
        out,
        "  falcond      {} · profiles in {}",
        present(std::path::Path::new(crate::config::CONFIG_PATH)),
        crate::profiles::USER_PROFILES_DIR
    );
    let _ = writeln!(
        out,
        "  logs         journalctl -b -u falcond -u bigame-daemon -u scx_loader -u power-profiles-daemon"
    );
    let _ = writeln!(
        out,
        "               journalctl -b -t bigame-ui -t gamescope"
    );
    let _ = writeln!(out, "  this report  bigame-ui --diagnostics [--network]");
    let _ = writeln!(out);
}

/// A folder and the names in it, never what they hold.
fn listing(dir: &std::path::Path) -> String {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return format!("{} (missing)", dir.display());
    };
    let mut names: Vec<String> = entries
        .flatten()
        .map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            if e.file_type().is_ok_and(|t| t.is_dir()) {
                format!("{name}/")
            } else {
                name
            }
        })
        .collect();
    names.sort();
    format!("{} ({})", dir.display(), names.join(", "))
}

fn present(path: &std::path::Path) -> String {
    if path.exists() {
        path.display().to_string()
    } else {
        format!("{} (missing)", path.display())
    }
}

/// falcond leaves a live value empty while no game runs.
fn or_none(value: &str) -> &str {
    if value.is_empty() { "none" } else { value }
}

fn on_off(v: bool) -> &'static str {
    if v { "on" } else { "off" }
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
            "── Big Game Mode ──",
            "── System ──",
            "── CPU ──",
            "── GPU ──",
            "── Displays ──",
            "── Gaming stack ──",
            "── Versions ──",
            "── sched-ext ──",
            "── falcond ──",
            "── Booster ──",
            "── Conflicts ──",
            "── Files and logs ──",
        ] {
            assert!(text.contains(heading), "missing section {heading}");
        }
        assert!(text.contains("Big Game Mode diagnostics"));
    }

    #[test]
    fn versions_come_from_the_package_database_and_absence_is_said() {
        let db = tempfile::tempdir().unwrap();
        let entry = db.path().join("falcond-2.0.3-1");
        std::fs::create_dir(&entry).unwrap();
        std::fs::write(
            entry.join("desc"),
            "%NAME%\nfalcond\n\n%VERSION%\n2.0.3-1\n",
        )
        .unwrap();
        let mut out = String::new();
        section_versions(&mut out, db.path());
        assert!(out.contains("falcond                2.0.3-1"), "{out}");
        assert!(
            out.contains("gamescope              not installed"),
            "{out}"
        );
        // A vendor's package is not a fault on another vendor's machine.
        assert!(!out.contains("nvidia-utils"), "{out}");

        let mut out = String::new();
        section_versions(&mut out, &db.path().join("nowhere"));
        assert!(out.contains("no pacman database"), "{out}");
    }

    #[test]
    fn a_folder_is_listed_by_name_only() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("settings.toml"), "secret = 1").unwrap();
        std::fs::create_dir(dir.path().join("games")).unwrap();
        let text = listing(dir.path());
        assert!(text.ends_with("(games/, settings.toml)"), "{text}");
        assert!(!text.contains("secret"));
        assert!(listing(&dir.path().join("gone")).ends_with("(missing)"));
    }

    #[test]
    fn the_network_section_is_opt_in() {
        assert!(!report(false).contains("── Network ──"));
        assert!(report(true).contains("── Network ──"));
    }
}
