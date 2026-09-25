//! System health: is everything a game needs here, and does it work?
//!
//! Each check says what it found, why it matters, and what to do about it —
//! a command to copy when there is one. Nothing here changes the system, and
//! nothing offers to "repair" by deleting.
//!
//! Checks are cheap enough to run when the Diagnostics page opens: file reads,
//! pacman's local database, one D-Bus ping. No processes are spawned.

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::capabilities::{Capabilities, Support};
use crate::hardware::{Chassis, GpuVendor, Hardware};

/// How a check came out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Status {
    /// Present and working.
    Ok,
    /// Fine, but worth knowing.
    Info,
    /// Works, but something limits what can be done.
    Warning,
    /// Broken.
    Error,
    /// Not applicable to this hardware.
    NotApplicable,
}

/// What to do about a problem.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum Fix {
    /// A command to copy and run. Never run for the user.
    Command(String),
    /// Something to do in BiGame-mode or elsewhere.
    Advice(String),
}

impl Fix {
    /// The text, whichever kind it is.
    #[must_use]
    pub fn text(&self) -> &str {
        match self {
            Self::Command(s) | Self::Advice(s) => s,
        }
    }
}

/// One check.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Check {
    /// What was checked.
    pub title: String,
    /// The outcome.
    pub status: Status,
    /// What was found, in a sentence.
    pub detail: String,
    /// What to do about it, when anything.
    pub fix: Option<Fix>,
}

fn check(title: &str, status: Status, detail: impl Into<String>, fix: Option<&str>) -> Check {
    Check {
        title: title.to_owned(),
        status,
        detail: detail.into(),
        // Commands are recognisable; everything else is advice.
        fix: fix.map(|f| {
            if f.starts_with("sudo ") || f.starts_with("journalctl ") {
                Fix::Command(f.to_owned())
            } else {
                Fix::Advice(f.to_owned())
            }
        }),
    }
}

/// An installed package's version, from pacman's local database.
///
/// Read from each entry's `desc` rather than inferred from directory names,
/// because names share prefixes (`falcond` and `falcond-profiles`).
#[must_use]
pub fn package_version(db: &Path, name: &str) -> Option<String> {
    let entries = std::fs::read_dir(db).ok()?;
    for entry in entries.flatten() {
        let dir = entry.file_name().to_string_lossy().into_owned();
        if !dir.starts_with(&format!("{name}-")) {
            continue;
        }
        let Ok(desc) = std::fs::read_to_string(entry.path().join("desc")) else {
            continue;
        };
        let field = |key: &str| {
            let mut lines = desc.lines();
            lines.by_ref().find(|l| *l == key)?;
            lines.next().map(str::to_owned)
        };
        if field("%NAME%").as_deref() == Some(name) {
            return field("%VERSION%");
        }
    }
    None
}

const PACMAN_DB: &str = "/var/lib/pacman/local";

/// The 32-bit Vulkan driver a render GPU needs, and whether it is present.
///
/// Many Windows games under Proton, and Steam itself, still load 32-bit
/// Vulkan. Missing, they fail to start or fall back to software rendering.
#[must_use]
pub fn vulkan_32bit(vendor: GpuVendor, lib32: &Path) -> (bool, &'static str) {
    let (file, package) = match vendor {
        GpuVendor::Amd => ("libvulkan_radeon.so", "lib32-vulkan-radeon"),
        GpuVendor::Intel => ("libvulkan_intel.so", "lib32-vulkan-intel"),
        GpuVendor::Nvidia => ("libGLX_nvidia.so.0", "lib32-nvidia-utils"),
        GpuVendor::Other => return (true, ""),
    };
    (lib32.join(file).exists(), package)
}

/// The command that brings power-profiles-daemon back.
///
/// `BigLinux` starts the daemon from its own unit, which also picks the driver,
/// and masks the stock one: enabling the stock unit there starts a second
/// daemon that cannot own the bus name and fails until systemd gives up.
#[must_use]
pub fn power_profiles_fix(unit_dir: &Path) -> &'static str {
    if unit_dir
        .join("power-profiles-daemon-biglinux.service")
        .exists()
    {
        "sudo systemctl enable power-profiles-daemon-biglinux && sudo systemctl restart power-profiles-daemon-biglinux"
    } else {
        "sudo systemctl enable --now power-profiles-daemon"
    }
}

/// A warning when systemd has had to restart falcond on its own.
///
/// falcond records the machine's state when a game's profile activates and
/// puts it back when the game exits. An instance restarted mid-game (after a
/// crash, or a kill) finds the game already running and records the
/// *boosted* state as the one to restore, so the machine stays boosted after
/// the game.
#[must_use]
pub fn restart_check(restarts: u32) -> Option<Check> {
    (restarts > 0).then(|| {
        check(
            "falcond restarts",
            Status::Warning,
            format!(
                "systemd restarted falcond {restarts} time{} after it stopped unexpectedly; a restart during a game can leave the power profile boosted after the game exits",
                if restarts == 1 { "" } else { "s" }
            ),
            Some("journalctl -u falcond -b"),
        )
    })
}

/// Hybrid graphics: which GPU games go to, and how they get there.
///
/// Nothing to report on a machine with one GPU, or where the games' GPU
/// drives the display itself. On a laptop whose panel belongs to the
/// integrated GPU, Proton games pick the discrete one on their own; an OpenGL
/// game needs PRIME render offload, which BiGame-mode sets for games it
/// starts but cannot set for a game Steam starts.
#[must_use]
pub fn hybrid_check(hw: &Hardware, prime_run: bool) -> Option<Check> {
    let render = crate::hardware::pick_render_gpu(&hw.gpus)?;
    let offload = crate::hardware::offload_for(&hw.gpus, render)?;
    let gpu = &hw.gpus[render];
    let vendor = match gpu.vendor {
        GpuVendor::Nvidia => "NVIDIA",
        GpuVendor::Amd => "AMD",
        GpuVendor::Intel => "Intel",
        GpuVendor::Other => "discrete",
    };
    let steam = if prime_run {
        "a native Linux game started by Steam needs `prime-run %command%` in its launch options"
    } else {
        "a native Linux game started by Steam needs the offload variables in its launch options"
    };
    Some(check(
        "Hybrid graphics",
        Status::Ok,
        format!(
            "games render on the {vendor} GPU ({}); games BiGame-mode starts use {}; Proton games choose it by themselves; {steam}",
            gpu.card,
            offload.label()
        ),
        None,
    ))
}

/// A warning when the CPU has hit its temperature limit since boot.
///
/// The kernel counts every time a core or the package was slowed for heat
/// (`thermal_throttle` under each CPU). A CPU that throttles is capped by its
/// cooling, not by any setting: a performance power profile only adds heat,
/// and a game measured 91 °C on a laptop lost a fifth of its clock. The
/// counts are per core; the package ones are the same on every core.
#[must_use]
pub fn cpu_throttle_check(cpu_dir: &Path) -> Option<Check> {
    let read = |cpu: &Path, name: &str| -> Option<u64> {
        std::fs::read_to_string(cpu.join("thermal_throttle").join(name))
            .ok()?
            .trim()
            .parse()
            .ok()
    };
    let cpus: Vec<PathBuf> = std::fs::read_dir(cpu_dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name().and_then(|n| n.to_str()).is_some_and(|n| {
                n.strip_prefix("cpu")
                    .is_some_and(|d| d.chars().all(|c| c.is_ascii_digit()))
            })
        })
        .collect();
    let mut cores: u64 = 0;
    let mut package: u64 = 0;
    let mut package_ms: u64 = 0;
    let mut any = false;
    for cpu in &cpus {
        if let Some(n) = read(cpu, "core_throttle_count") {
            any = true;
            cores += n;
        }
        package = package.max(read(cpu, "package_throttle_count").unwrap_or(0));
        package_ms = package_ms.max(read(cpu, "package_throttle_total_time_ms").unwrap_or(0));
    }
    if !any {
        return None;
    }
    let events = cores.max(package);
    Some(if events == 0 {
        check(
            "CPU temperature",
            Status::Ok,
            "no thermal throttling since boot",
            None,
        )
    } else {
        check(
            "CPU temperature",
            Status::Warning,
            format!(
                "the CPU hit its temperature limit {events} time{} since boot ({} s slowed in total): its cooling caps its speed, and a performance power profile adds heat",
                if events == 1 { "" } else { "s" },
                package_ms / 1000
            ),
            Some(
                "Keep the vents clear; on a laptop, measure whether Turbo helps this game (Measure the difference) before keeping it on",
            ),
        )
    })
}

/// Run every check.
///
/// One flat list, top to bottom, so each check reads on its own.
#[must_use]
#[allow(clippy::too_many_lines)]
pub fn collect() -> Vec<Check> {
    let hw = Hardware::detect();
    let caps = Capabilities::detect();
    let status = crate::status::read();
    let systemd = crate::systemd::Reader::system();
    let backend = systemd
        .as_ref()
        .and_then(|r| r.unit_state(crate::turbo::BACKEND_UNIT));
    let db = Path::new(PACMAN_DB);
    let mut out = Vec::new();

    // falcond
    let version = package_version(db, "falcond");
    let v = version.as_deref().unwrap_or("?");
    out.push(match (&backend, caps.falcond_installed) {
        (_, false) => check(
            "falcond",
            Status::Error,
            "not installed: there is no per-game optimization",
            Some("sudo pacman -S falcond falcond-profiles"),
        ),
        (Some(u), true) if u.active_state == "failed" => check(
            "falcond",
            Status::Error,
            format!("{v} · the service failed; see Logs for why"),
            Some("journalctl -u falcond -n 50"),
        ),
        (Some(u), true) if u.is_active() => check(
            "falcond",
            Status::Ok,
            format!("{v} · running (Turbo on)"),
            None,
        ),
        _ => check(
            "falcond",
            Status::Info,
            format!("{v} · stopped (Turbo off)"),
            None,
        ),
    });
    if backend
        .as_ref()
        .is_some_and(crate::systemd::UnitState::is_active)
    {
        if let Some(c) = systemd
            .as_ref()
            .and_then(|r| r.restarts(crate::turbo::BACKEND_UNIT))
            .and_then(restart_check)
        {
            out.push(c);
        }
    }
    if status.as_ref().is_some_and(|s| s.dmem_cgroup.is_none()) && caps.falcond_installed {
        let kernel_can = Path::new("/sys/fs/cgroup/dmem.capacity").exists();
        out.push(check(
            "falcond features",
            Status::Info,
            if kernel_can {
                "this falcond predates VRAM protection (DMEM) and split-lock handling; the kernel supports DMEM, so a newer falcond could use it"
            } else {
                "this falcond predates VRAM protection (DMEM) and split-lock handling"
            },
            None,
        ));
    }
    if let Some(s) = &status {
        if s.profile_mode == "handheld" && !matches!(hw.chassis, Chassis::Handheld) {
            out.push(check(
                "falcond profile set",
                Status::Warning,
                "handheld profiles on a machine that is not a handheld: games run in power-saving mode",
                Some("Turn Turbo on — or off and on again if it is already on — to switch falcond to its desktop profiles"),
            ));
        }
    }

    // sched-ext
    out.push(match caps.sched_ext.switchable() {
        Support::Available => check(
            "sched-ext",
            Status::Ok,
            format!("{} schedulers, scx_loader available", caps.sched_ext.installed.len()),
            None,
        ),
        Support::Unsupported(why) => check("sched-ext", Status::NotApplicable, why, None),
        Support::NotInstalled(package) if package == "scx-tools" => check(
            "sched-ext",
            Status::Warning,
            format!(
                "{} schedulers installed, but scx-tools (scx_loader) is not, so game profiles cannot switch scheduler",
                caps.sched_ext.installed.len()
            ),
            Some("sudo pacman -S scx-tools && sudo systemctl enable --now scx_loader"),
        ),
        Support::NotInstalled(package) => check(
            "sched-ext",
            Status::Warning,
            format!("{package} is not installed"),
            Some("sudo pacman -S scx-scheds scx-tools"),
        ),
        Support::ServiceDown(why) => check(
            "sched-ext",
            Status::Warning,
            why,
            Some("sudo systemctl enable --now scx_loader"),
        ),
    });

    // Power profiles
    out.push(if caps.power_profiles {
        check(
            "power-profiles-daemon",
            Status::Ok,
            caps.power_profiles_available.join(", "),
            None,
        )
    } else {
        check(
            "power-profiles-daemon",
            Status::Warning,
            "not reachable: game profiles cannot switch the power profile",
            Some(power_profiles_fix(Path::new("/usr/lib/systemd/system"))),
        )
    });

    // GameMode
    out.push(if caps.gamemode {
        check(
            "Feral GameMode",
            Status::Warning,
            "installed alongside falcond; BiGame-mode does not use it, because two controllers would save and restore the same settings",
            None,
        )
    } else {
        check("Feral GameMode", Status::Ok, "not installed · no conflict with falcond", None)
    });

    // Hybrid graphics
    if let Some(c) = hybrid_check(&hw, crate::capabilities::which("prime-run").is_some()) {
        out.push(c);
    }

    // CPU cooling
    if let Some(c) = cpu_throttle_check(Path::new("/sys/devices/system/cpu")) {
        out.push(c);
    }

    // Graphics
    match hw.render_gpu() {
        Some(gpu) => {
            let (ok, package) = vulkan_32bit(gpu.vendor, Path::new("/usr/lib32"));
            out.push(if ok {
                check("32-bit Vulkan", Status::Ok, "present", None)
            } else {
                check(
                    "32-bit Vulkan",
                    Status::Error,
                    "missing: many Proton games and Steam itself need it",
                    Some(&format!("sudo pacman -S {package}")),
                )
            });
        }
        None => out.push(check(
            "GPU",
            Status::Error,
            "no render GPU was identified",
            None,
        )),
    }

    // Tools
    for (name, present, package, why) in [
        (
            "Steam",
            caps.steam,
            "steam",
            "the launcher for most games here",
        ),
        (
            "MangoHud",
            caps.mangohud,
            "mangohud",
            "frame-time capture and the in-game overlay",
        ),
        (
            "Gamescope",
            caps.gamescope.is_some(),
            "gamescope",
            "the micro-compositor used for scaling and frame limiting",
        ),
    ] {
        out.push(if present {
            let detail = match (name, &caps.gamescope) {
                ("Gamescope", Some(g)) => g.version.map_or_else(
                    || "present".to_owned(),
                    |v| format!("{}.{}.{}", v.major, v.minor, v.patch),
                ),
                _ => package_version(db, package).unwrap_or_else(|| "present".into()),
            };
            check(name, Status::Ok, detail, None)
        } else {
            check(
                name,
                Status::Info,
                format!("not installed · {why}"),
                Some(&format!("sudo pacman -S {package}")),
            )
        });
    }

    // Hardware-specific
    out.push(if hw.cpu.vcache.is_some() {
        check(
            "3D V-Cache",
            Status::Ok,
            "present; game profiles can prefer the cache CCD",
            None,
        )
    } else {
        check(
            "3D V-Cache",
            Status::NotApplicable,
            "this CPU has none",
            None,
        )
    });

    // Our own helper
    let helper = crate::dbus_client::daemon_proxy_blocking()
        .ok()
        .and_then(|p| p.ping().ok());
    out.push(match helper {
        Some(_) => check("BiGame-mode helper", Status::Ok, "reachable", None),
        None => check(
            "BiGame-mode helper",
            Status::Error,
            "not reachable: Turbo and profile changes cannot be made",
            Some("sudo systemctl restart bigame-daemon"),
        ),
    });

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_restarted_falcond_is_a_warning_and_an_untouched_one_is_silent() {
        assert_eq!(restart_check(0), None);
        let one = restart_check(1).unwrap();
        assert_eq!(one.status, Status::Warning);
        assert!(one.detail.contains("1 time after"), "{}", one.detail);
        assert!(restart_check(3).unwrap().detail.contains("3 times"));
        assert_eq!(
            one.fix,
            Some(Fix::Command("journalctl -u falcond -b".into()))
        );
    }

    #[test]
    fn power_profiles_are_restarted_through_biglinux_s_own_unit_where_it_has_one() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            power_profiles_fix(dir.path()),
            "sudo systemctl enable --now power-profiles-daemon"
        );
        std::fs::write(
            dir.path().join("power-profiles-daemon-biglinux.service"),
            "",
        )
        .unwrap();
        let fix = power_profiles_fix(dir.path());
        assert!(
            fix.contains("restart power-profiles-daemon-biglinux"),
            "{fix}"
        );
        assert!(!fix.contains("--now power-profiles-daemon"), "{fix}");
    }

    #[test]
    fn a_throttled_cpu_is_a_warning_and_a_cool_one_is_ok() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            cpu_throttle_check(dir.path()),
            None,
            "no counters, no check"
        );
        for (cpu, core, pkg, ms) in [("cpu0", 0, 0, 0), ("cpu1", 0, 0, 0)] {
            let t = dir.path().join(cpu).join("thermal_throttle");
            std::fs::create_dir_all(&t).unwrap();
            std::fs::write(t.join("core_throttle_count"), core.to_string()).unwrap();
            std::fs::write(t.join("package_throttle_count"), pkg.to_string()).unwrap();
            std::fs::write(t.join("package_throttle_total_time_ms"), ms.to_string()).unwrap();
        }
        std::fs::create_dir_all(dir.path().join("cpufreq")).unwrap();
        let cool = cpu_throttle_check(dir.path()).unwrap();
        assert_eq!(cool.status, Status::Ok);
        // The lab laptop after three benchmark runs.
        let t = dir.path().join("cpu1").join("thermal_throttle");
        std::fs::write(t.join("core_throttle_count"), "1799").unwrap();
        std::fs::write(t.join("package_throttle_count"), "47").unwrap();
        std::fs::write(t.join("package_throttle_total_time_ms"), "14300").unwrap();
        let hot = cpu_throttle_check(dir.path()).unwrap();
        assert_eq!(hot.status, Status::Warning);
        assert!(
            hot.detail.contains("1799 times") && hot.detail.contains("14 s"),
            "{}",
            hot.detail
        );
        assert!(matches!(hot.fix, Some(Fix::Advice(_))));
    }

    #[test]
    fn a_hybrid_laptop_is_told_where_games_render_and_how() {
        use crate::hardware::{Gpu, Session};
        let g = |card: &str, driver: &str, vendor: GpuVendor, discrete: bool, out: &[&str]| Gpu {
            card: card.into(),
            device_path: std::path::PathBuf::new(),
            vendor,
            pci_id: String::new(),
            pci_slot: "0000:01:00.0".into(),
            driver: driver.into(),
            hwmon: None,
            connected_outputs: out.iter().map(|s| (*s).to_owned()).collect(),
            vram_total_bytes: None,
            discrete,
            dpm_level_path: None,
        };
        let mut hw = Hardware::detect();
        hw.session = Session::Wayland;
        hw.gpus = vec![
            g("card0", "nvidia", GpuVendor::Nvidia, true, &[]),
            g("card1", "i915", GpuVendor::Intel, false, &["eDP-1"]),
        ];
        let c = hybrid_check(&hw, true).unwrap();
        assert_eq!(c.status, Status::Ok);
        assert!(
            c.detail.contains("NVIDIA PRIME render offload"),
            "{}",
            c.detail
        );
        assert!(c.detail.contains("prime-run %command%"), "{}", c.detail);
        // A desktop whose dGPU drives the monitor: nothing to say.
        hw.gpus = vec![g("card0", "nvidia", GpuVendor::Nvidia, true, &["DP-1"])];
        assert_eq!(hybrid_check(&hw, true), None);
    }

    #[test]
    fn a_package_is_found_by_its_name_not_a_prefix() {
        let db = std::env::temp_dir().join(format!("bgm-pacdb-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&db);
        for (dir, name, version) in [
            (
                "falcond-profiles-r23.a3e0e63-1",
                "falcond-profiles",
                "r23.a3e0e63-1",
            ),
            ("falcond-2.0.2-2", "falcond", "2.0.2-2"),
        ] {
            std::fs::create_dir_all(db.join(dir)).unwrap();
            std::fs::write(
                db.join(dir).join("desc"),
                format!("%NAME%\n{name}\n\n%VERSION%\n{version}\n"),
            )
            .unwrap();
        }
        assert_eq!(package_version(&db, "falcond").as_deref(), Some("2.0.2-2"));
        assert_eq!(
            package_version(&db, "falcond-profiles").as_deref(),
            Some("r23.a3e0e63-1")
        );
        assert_eq!(package_version(&db, "mangohud"), None);
        let _ = std::fs::remove_dir_all(&db);
    }

    #[test]
    fn missing_32bit_vulkan_names_the_package() {
        let empty = std::env::temp_dir().join(format!("bgm-lib32-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&empty);
        assert_eq!(
            vulkan_32bit(GpuVendor::Amd, &empty),
            (false, "lib32-vulkan-radeon")
        );
        assert_eq!(
            vulkan_32bit(GpuVendor::Nvidia, &empty).1,
            "lib32-nvidia-utils"
        );
        let _ = std::fs::remove_dir_all(&empty);
    }

    #[test]
    fn no_check_offers_to_delete_anything() {
        for c in collect() {
            if let Some(fix) = &c.fix {
                assert!(!fix.text().contains("rm "), "{}: {fix:?}", c.title);
            }
        }
    }
}
