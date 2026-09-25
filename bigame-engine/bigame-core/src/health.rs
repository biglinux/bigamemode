//! System health: is everything a game needs here, and does it work?
//!
//! Each check says what it found, why it matters, and what to do about it —
//! a command to copy when there is one. Nothing here changes the system, and
//! nothing offers to "repair" by deleting.
//!
//! Checks are cheap enough to run when the Diagnostics page opens: file reads,
//! pacman's local database, one D-Bus ping. No processes are spawned.

use std::path::Path;

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
            format!("{} schedulers, scx_loader running", caps.sched_ext.installed.len()),
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
