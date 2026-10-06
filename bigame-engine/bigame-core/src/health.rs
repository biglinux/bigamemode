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
use crate::text::{Arg, N_, Text};

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
    /// A command to copy and run. Never run for the user, never translated.
    Command(String),
    /// Something to do in BiGame-mode or elsewhere, translatable.
    Advice(Text),
}

impl Fix {
    /// The fix in English (a command as it is).
    #[must_use]
    pub fn english(&self) -> String {
        match self {
            Self::Command(s) => s.clone(),
            Self::Advice(t) => t.english(),
        }
    }
}

/// One check. Its sentences are [`Text`]: an English template marked with
/// [`N_`] for the translation catalogue, and the values for its `%s`. The UI
/// translates them; tests and reports use [`Text::english`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Check {
    /// What was checked.
    pub title: Text,
    /// The outcome.
    pub status: Status,
    /// What was found, in a sentence.
    pub detail: Text,
    /// What to do about it, when anything.
    pub fix: Option<Fix>,
}

fn check(title: &'static str, status: Status, detail: impl Into<Text>, fix: Option<Fix>) -> Check {
    Check {
        title: Text::plain(title),
        status,
        detail: detail.into(),
        fix,
    }
}

/// A command to copy. (An `Option`, as every check's fix is.)
#[allow(clippy::unnecessary_wraps)]
fn cmd(command: impl Into<String>) -> Option<Fix> {
    Some(Fix::Command(command.into()))
}

/// Advice, translatable. (An `Option`, as every check's fix is.)
#[allow(clippy::unnecessary_wraps)]
fn advice(template: &'static str) -> Option<Fix> {
    Some(Fix::Advice(Text::plain(template)))
}

/// The command that installs `packages` where they install from this
/// system's repositories; advice where they do not (an Arch system without
/// `BigLinux`'s repositories has falcond in the AUR).
fn install(packages: &[&str]) -> Option<Fix> {
    if packages
        .iter()
        .all(|p| crate::capabilities::in_repositories(p))
    {
        cmd(format!("sudo pacman -S --needed {}", packages.join(" ")))
    } else {
        advice(N_(
            "Not in this system's repositories: install it from your distribution or the AUR",
        ))
    }
}

/// A value shown as it is — a version, a list of names.
fn verbatim(value: impl Into<String>) -> Text {
    Text::raw(value)
}

/// An installed package's version, from pacman's local database.
///
/// Read from each entry's `desc` rather than inferred from directory names,
/// because names share prefixes (`falcond` and `falcond-profiles`).
///
/// A package that only *provides* `name` — a local build such as
/// `falcond-local` with `provides=(falcond)` — counts too: its versioned
/// `provides` entry (`falcond=2.0.14`) when it has one, its own version
/// otherwise. Otherwise such a build reads as "not installed" and loses
/// every version-dependent explanation.
#[must_use]
pub fn package_version(db: &Path, name: &str) -> Option<String> {
    let entries: Vec<_> = std::fs::read_dir(db).ok()?.flatten().collect();
    let desc_of =
        |entry: &std::fs::DirEntry| std::fs::read_to_string(entry.path().join("desc")).ok();
    for entry in &entries {
        let dir = entry.file_name().to_string_lossy().into_owned();
        if !dir.starts_with(&format!("{name}-")) {
            continue;
        }
        let Some(desc) = desc_of(entry) else {
            continue;
        };
        if desc_field(&desc, "%NAME%").first() == Some(&name) {
            return desc_field(&desc, "%VERSION%")
                .first()
                .map(|v| (*v).to_owned());
        }
    }
    // Only a package that names something else can provide it.
    entries.iter().find_map(|entry| {
        let desc = desc_of(entry)?;
        desc_field(&desc, "%PROVIDES%").into_iter().find_map(|p| {
            let (provided, version) = p.split_once('=').unwrap_or((p, ""));
            (provided == name).then(|| {
                if version.is_empty() {
                    desc_field(&desc, "%VERSION%")
                        .first()
                        .map(|v| (*v).to_owned())
                } else {
                    Some(version.to_owned())
                }
            })?
        })
    })
}

/// The lines of one `%FIELD%` section of a pacman `desc` file.
fn desc_field<'a>(desc: &'a str, key: &str) -> Vec<&'a str> {
    let mut lines = desc.lines();
    if lines.by_ref().find(|l| *l == key).is_none() {
        return Vec::new();
    }
    lines.take_while(|l| !l.is_empty()).collect()
}

pub(crate) const PACMAN_DB: &str = "/var/lib/pacman/local";

/// `NTSync`: the kernel's Windows synchronisation driver, which Wine and
/// Proton use instead of fsync/esync when `/dev/ntsync` can be opened.
///
/// Read from the device node, `/sys/module` and the running kernel's module
/// directory; nothing is loaded and no variable is set — Proton picks it up
/// by itself. On Arch the module is loaded at boot by `ntsync-autoload`,
/// which only `wine` pulls in, so a Steam-only installation may have a kernel
/// with `NTSync` and no device.
#[must_use]
pub fn ntsync_check(dev: &Path, sys_module: &Path, modules_dir: &Path) -> Check {
    use std::os::unix::fs::FileTypeExt as _;
    let device = std::fs::metadata(dev).is_ok_and(|m| m.file_type().is_char_device());
    if device {
        let usable = std::ffi::CString::new(dev.as_os_str().as_encoded_bytes())
            // SAFETY: access(2) reads only the NUL-terminated path.
            .is_ok_and(|c| unsafe { libc::access(c.as_ptr(), libc::R_OK | libc::W_OK) } == 0);
        return if usable {
            check(
                N_("NTSync"),
                Status::Ok,
                N_(
                    "available: Proton and Wine synchronise game threads through the kernel when they support it",
                ),
                None,
            )
        } else {
            check(
                N_("NTSync"),
                Status::Warning,
                N_(
                    "/dev/ntsync exists, but this user cannot open it, so games fall back to fsync or esync",
                ),
                cmd("ls -l /dev/ntsync"),
            )
        };
    }
    let builtin = std::fs::read_to_string(modules_dir.join("modules.builtin"))
        .is_ok_and(|b| b.lines().any(|l| l.ends_with("/ntsync.ko")));
    let module = builtin
        || sys_module.exists()
        || std::fs::read_dir(modules_dir.join("kernel/drivers/misc")).is_ok_and(|d| {
            d.flatten()
                .any(|e| e.file_name().to_string_lossy().starts_with("ntsync.ko"))
        });
    if module {
        check(
            N_("NTSync"),
            Status::Warning,
            N_("the kernel has NTSync, but it is not loaded, so games fall back to fsync or esync"),
            cmd("sudo pacman -S --needed ntsync-autoload && sudo modprobe ntsync"),
        )
    } else {
        check(
            N_("NTSync"),
            Status::NotApplicable,
            N_("this kernel has no NTSync; games use fsync or esync"),
            None,
        )
    }
}

/// The scheduler `scx_loader` starts by itself at boot (`default_sched`, and
/// `default_mode`), from the first of its configuration files that exists.
#[must_use]
pub fn scx_loader_default(files: &[&Path]) -> Option<(String, String)> {
    let text = files.iter().find_map(|f| std::fs::read_to_string(f).ok())?;
    let table = text.parse::<toml::Table>().ok()?;
    let sched = table.get("default_sched")?.as_str()?.trim();
    if sched.is_empty() {
        return None;
    }
    let mode = table
        .get("default_mode")
        .and_then(toml::Value::as_str)
        .unwrap_or("Auto");
    Some((sched.to_owned(), mode.to_owned()))
}

/// A scheduler `scx_loader` loads at boot is a second owner of the CPU
/// scheduler: it runs with Turbo off and on battery, and falcond's choice
/// replaces it for a game while Turbo is on. Said, never changed.
#[must_use]
pub fn scx_loader_default_check(default: Option<(String, String)>) -> Option<Check> {
    let (sched, mode) = default?;
    Some(check(
        N_("Scheduler at boot"),
        Status::Info,
        Text::with(
            N_(
                "scx_loader starts %s (%s mode) at every boot, with Turbo off too: a second owner of the CPU scheduler. While Turbo is on, a game's profile can replace it.",
            ),
            [sched, mode],
        ),
        None,
    ))
}

/// falcond's VRAM protection (DMEM, falcond 2.0.14): unavailable on this
/// system, or failing with the error falcond reports. Nothing when it works
/// or this falcond does not report it.
#[must_use]
pub fn dmem_check(status: Option<&crate::status::FalcondStatus>) -> Option<Check> {
    let status = status?;
    if let Some(error) = &status.dmem_last_error {
        return Some(check(
            N_("VRAM protection"),
            Status::Warning,
            Text::with(
                N_("falcond could not protect a game's VRAM: %s"),
                [error.clone()],
            ),
            advice(N_(
                "falcond protects a game's VRAM only when the dmem controller is delegated to your session (the dmemcg-booster service does that); without it games run as before, without the protection",
            )),
        ));
    }
    (status.dmem_cgroup == Some(false)).then(|| {
        check(
            N_("VRAM protection"),
            Status::Info,
            N_("this kernel or its cgroup setup does not let falcond protect a game's VRAM (DMEM); games run without that protection"),
            None,
        )
    })
}

/// A falcond directory that someone other than root can write: falcond, a
/// root service, applies what is in it, and Big Game Mode writes there only
/// after an administrator's password. A group-writable profile directory (as
/// some setup scripts make for falcond-gui) skips that password.
#[must_use]
pub fn falcond_dirs_check(dirs: &[&Path]) -> Option<Check> {
    use std::os::unix::fs::MetadataExt as _;
    let open: Vec<String> = dirs
        .iter()
        .filter(|d| {
            std::fs::symlink_metadata(d).is_ok_and(|m| m.uid() != 0 || m.mode() & 0o022 != 0)
        })
        .map(|d| d.display().to_string())
        .collect();
    let first = open.first()?;
    Some(check(
        N_("falcond's files"),
        Status::Warning,
        Text::with(
            N_(
                "%s can be changed without the administrator password, yet falcond applies what it holds as root",
            ),
            [open.join(", ")],
        ),
        cmd(format!(
            "sudo chown root:root '{first}' && sudo chmod 755 '{first}'"
        )),
    ))
}

/// `MangoHud` without its 32-bit build: 32-bit games (and 32-bit Vulkan in
/// Proton) get no overlay and no capture for Measure the difference.
#[must_use]
pub fn mangohud_32bit_check(mangohud: bool, lib32: &Path) -> Option<Check> {
    (mangohud && !lib32.join("mangohud/libMangoHud.so").exists()).then(|| {
        check(
            N_("MangoHud (32-bit)"),
            Status::Info,
            N_("only the 64-bit MangoHud is installed: 32-bit games show no overlay and cannot be measured"),
            cmd("sudo pacman -S lib32-mangohud"),
        )
    })
}

/// A `Lossless.dll` the installed lsfg-vk cannot generate frames with: the
/// profile loads, the swapchain does not, and a game may close at start.
/// Asked of lsfg-vk itself ([`crate::fg::check_dll`], saved until the DLL or
/// lsfg-vk changes).
fn lossless_dll_check() -> Option<Check> {
    let dll = crate::fg::read_global_dll()?;
    match crate::fg::check_dll(Path::new(&dll))? {
        crate::fg::DllCheck::Works => None,
        crate::fg::DllCheck::Unusable(reason) => Some(check(
            N_("Frame generation (lsfg-vk)"),
            Status::Warning,
            Text::with(
                N_(
                    "lsfg-vk cannot generate frames with this Lossless.dll (%s): update Lossless Scaling and choose its Lossless.dll again",
                ),
                [reason],
            ),
            None,
        )),
    }
}

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

/// The command that brings power-profiles-daemon back, or installs it when
/// its unit is not there.
///
/// `BigLinux` starts the daemon from its own unit, which also picks the driver,
/// and masks the stock one: enabling the stock unit there starts a second
/// daemon that cannot own the bus name and fails until systemd gives up.
#[must_use]
pub fn power_profiles_fix(unit_dir: &Path) -> &'static str {
    if !unit_dir.join("power-profiles-daemon.service").exists() {
        "sudo pacman -S --needed power-profiles-daemon"
    } else if unit_dir
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
            N_("falcond restarts"),
            Status::Warning,
            if restarts == 1 {
                Text::plain(N_("systemd restarted falcond once after it stopped unexpectedly; a restart during a game can leave the power profile boosted after the game exits"))
            } else {
                Text::with(
                    N_("systemd restarted falcond %s times after it stopped unexpectedly; a restart during a game can leave the power profile boosted after the game exits"),
                    [restarts.to_string()],
                )
            },
            cmd("journalctl -u falcond -b"),
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
        GpuVendor::Other => "PCI",
    };
    let values = [
        Arg::from(vendor),
        Arg::from(gpu.node()),
        Arg::Text(offload.label()),
    ];
    Some(check(
        N_("Hybrid graphics"),
        Status::Ok,
        if prime_run {
            Text::with(
                N_(
                    "games render on the %s GPU (%s); games Big Game Mode starts use %s; Proton games choose it by themselves; a native Linux game started by Steam needs `prime-run %command%` in its launch options",
                ),
                values,
            )
        } else {
            Text::with(
                N_(
                    "games render on the %s GPU (%s); games Big Game Mode starts use %s; Proton games choose it by themselves; a native Linux game started by Steam needs the offload variables in its launch options",
                ),
                values,
            )
        },
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
            N_("CPU temperature"),
            Status::Ok,
            N_("no thermal throttling since boot"),
            None,
        )
    } else {
        let slowed = (package_ms / 1000).to_string();
        check(
            N_("CPU temperature"),
            Status::Warning,
            if events == 1 {
                Text::with(
                    N_(
                        "the CPU hit its temperature limit once since boot (%s s slowed in total): its cooling caps its speed, and a performance power profile adds heat",
                    ),
                    [slowed],
                )
            } else {
                Text::with(
                    N_(
                        "the CPU hit its temperature limit %s times since boot (%s s slowed in total): its cooling caps its speed, and a performance power profile adds heat",
                    ),
                    [events.to_string(), slowed],
                )
            },
            advice(N_(
                "Keep the vents clear; on a laptop, measure whether Turbo helps this game (Measure the difference) before keeping it on",
            )),
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
    let backend =
        crate::systemd::Reader::shared().and_then(|r| r.unit_state(crate::turbo::BACKEND_UNIT));
    let db = Path::new(PACMAN_DB);
    let mut out = Vec::new();

    // falcond
    let version = package_version(db, "falcond");
    let v = version.as_deref().unwrap_or("?");
    out.push(match (&backend, caps.falcond_installed) {
        (_, false) => check(
            N_("falcond"),
            Status::Warning,
            N_("not installed: no per-game performance profiles; Turbo applies only the general settings"),
            install(&["falcond", "falcond-profiles"]),
        ),
        (Some(u), true) if u.active_state == "failed" => check(
            N_("falcond"),
            Status::Error,
            Text::with(N_("%s · the service failed; see Logs for why"), [v]),
            cmd("journalctl -u falcond -n 50"),
        ),
        (Some(u), true) if u.is_active() => check(
            N_("falcond"),
            Status::Ok,
            Text::with(N_("%s · running (Turbo on)"), [v]),
            None,
        ),
        _ => check(
            N_("falcond"),
            Status::Info,
            Text::with(N_("%s · stopped (Turbo off)"), [v]),
            None,
        ),
    });
    if backend
        .as_ref()
        .is_some_and(crate::systemd::UnitState::is_active)
    {
        if let Some(c) = crate::systemd::Reader::shared()
            .and_then(|r| r.restarts(crate::turbo::BACKEND_UNIT))
            .and_then(restart_check)
        {
            out.push(c);
        }
    }
    if status.as_ref().is_some_and(|s| s.dmem_cgroup.is_none()) && caps.falcond_installed {
        let kernel_can = Path::new("/sys/fs/cgroup/dmem.capacity").exists();
        out.push(check(
            N_("falcond features"),
            Status::Info,
            if kernel_can {
                N_("this falcond predates VRAM protection (DMEM) and split-lock handling; the kernel supports DMEM, so a newer falcond could use it")
            } else {
                N_("this falcond predates VRAM protection (DMEM) and split-lock handling")
            },
            None,
        ));
    }
    if let Some(s) = &status {
        if s.profile_mode == "handheld" && !matches!(hw.chassis, Chassis::Handheld) {
            out.push(check(
                N_("falcond profile set"),
                Status::Warning,
                N_("handheld profiles on a machine that is not a handheld: games run in power-saving mode"),
                advice(N_("Turn Turbo on — or off and on again if it is already on — to switch falcond to its desktop profiles")),
            ));
        }
    }

    // sched-ext
    out.push(match caps.sched_ext.switchable() {
        Support::Available => check(
            N_("sched-ext"),
            Status::Ok,
            Text::with(
                N_("%s schedulers, scx_loader available"),
                [caps.sched_ext.installed.len().to_string()],
            ),
            None,
        ),
        Support::Unsupported(why) => check(N_("sched-ext"), Status::NotApplicable, why, None),
        Support::NotInstalled(package) if package == "scx-tools" => check(
            N_("sched-ext"),
            Status::Warning,
            Text::with(
                N_("%s schedulers installed, but scx-tools (scx_loader) is not, so game profiles cannot switch scheduler"),
                [caps.sched_ext.installed.len().to_string()],
            ),
            cmd("sudo pacman -S scx-tools && sudo systemctl enable --now scx_loader"),
        ),
        Support::NotInstalled(package) => check(
            N_("sched-ext"),
            Status::Warning,
            Text::with(N_("%s is not installed"), [package]),
            cmd("sudo pacman -S scx-scheds scx-tools"),
        ),
        Support::ServiceDown(why) => check(
            N_("sched-ext"),
            Status::Warning,
            why,
            cmd("sudo systemctl enable --now scx_loader"),
        ),
    });

    out.extend(dmem_check(status.as_ref()));
    if caps.falcond_installed {
        out.extend(falcond_dirs_check(&[
            Path::new("/etc/falcond"),
            Path::new("/usr/share/falcond/profiles/user"),
        ]));
    }
    out.extend(mangohud_32bit_check(caps.mangohud, Path::new("/usr/lib32")));
    out.extend(lossless_dll_check());
    if let Some(c) = scx_loader_default_check(scx_loader_default(&[
        Path::new("/etc/scx_loader.toml"),
        Path::new("/etc/scx_loader/config.toml"),
        Path::new("/usr/share/scx_loader/config.toml"),
    ])) {
        out.push(c);
    }

    // NTSync
    let release = std::fs::read_to_string("/proc/sys/kernel/osrelease").unwrap_or_default();
    out.push(ntsync_check(
        Path::new("/dev/ntsync"),
        Path::new("/sys/module/ntsync"),
        &Path::new("/lib/modules").join(release.trim()),
    ));

    // Power profiles
    out.push(if caps.power_profiles {
        check(
            N_("power-profiles-daemon"),
            Status::Ok,
            verbatim(caps.power_profiles_available.join(", ")),
            None,
        )
    } else {
        check(
            N_("power-profiles-daemon"),
            Status::Warning,
            N_("not reachable: game profiles cannot switch the power profile"),
            cmd(power_profiles_fix(Path::new("/usr/lib/systemd/system"))),
        )
    });

    // GameMode
    out.push(if caps.gamemode {
        check(
            N_("Feral GameMode"),
            Status::Warning,
            N_("installed alongside falcond; Big Game Mode does not use it, because two controllers would save and restore the same settings"),
            None,
        )
    } else {
        check(N_("Feral GameMode"), Status::Ok, N_("not installed · no conflict with falcond"), None)
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
                check(N_("32-bit Vulkan"), Status::Ok, N_("present"), None)
            } else {
                check(
                    N_("32-bit Vulkan"),
                    Status::Error,
                    N_("missing: many Proton games and Steam itself need it"),
                    cmd(format!("sudo pacman -S {package}")),
                )
            });
        }
        None => out.push(check(
            N_("GPU"),
            Status::Error,
            N_("no render GPU was identified"),
            None,
        )),
    }

    // Tools
    for (name, present, package, missing) in [
        (
            N_("Steam"),
            caps.steam,
            "steam",
            N_("not installed · the launcher for most games here"),
        ),
        (
            N_("MangoHud"),
            caps.mangohud,
            "mangohud",
            N_("not installed · frame-time capture and the in-game overlay"),
        ),
        (
            N_("Gamescope"),
            caps.gamescope.is_some(),
            "gamescope",
            N_("not installed · the micro-compositor used for scaling and frame limiting"),
        ),
    ] {
        out.push(if present {
            let version = match (name, &caps.gamescope) {
                ("Gamescope", Some(g)) => g
                    .version
                    .map(|v| format!("{}.{}.{}", v.major, v.minor, v.patch)),
                _ => package_version(db, package),
            };
            let detail = version.map_or_else(|| Text::plain(N_("present")), verbatim);
            check(name, Status::Ok, detail, None)
        } else {
            check(
                name,
                Status::Info,
                missing,
                cmd(format!("sudo pacman -S {package}")),
            )
        });
    }

    // MangoHud inside a Flatpak launcher: the system's mangohud is not
    // visible there, and Heroic refuses to start a game with its MangoHud
    // switch on and none on its PATH.
    if caps.mangohud {
        for (app, title) in [
            (
                "com.heroicgameslauncher.hgl",
                N_("MangoHud in Heroic (Flatpak)"),
            ),
            ("net.lutris.Lutris", N_("MangoHud in Lutris (Flatpak)")),
        ] {
            if let Some(command) = crate::mangohud::missing_flatpak_extension(app) {
                out.push(check(
                    title,
                    Status::Warning,
                    N_("this launcher runs as a Flatpak and cannot see the system's MangoHud: games it starts get no overlay (Heroic refuses to start one with its MangoHud switch on) until MangoHud's Flatpak extension for its runtime is installed"),
                    cmd(command),
                ));
            }
        }
    }

    // Hardware-specific
    out.push(if hw.cpu.vcache.is_some() {
        check(
            N_("3D V-Cache"),
            Status::Ok,
            N_("present; game profiles can prefer the cache CCD"),
            None,
        )
    } else {
        check(
            N_("3D V-Cache"),
            Status::NotApplicable,
            N_("this CPU has none"),
            None,
        )
    });

    if let Some(c) = hw.render_gpu().and_then(resizable_bar_of) {
        out.push(c);
    }

    // Our own helper
    let helper = crate::dbus_client::daemon_proxy_blocking()
        .ok()
        .and_then(|p| p.ping().ok());
    out.push(match helper {
        Some(_) => check(
            N_("Big Game Mode helper"),
            Status::Ok,
            N_("reachable"),
            None,
        ),
        None => check(
            N_("Big Game Mode helper"),
            Status::Error,
            N_("not reachable: Turbo and profile changes cannot be made"),
            cmd("sudo systemctl restart bigame-daemon"),
        ),
    });

    out
}

/// Whether the CPU can map all of an AMD card's VRAM (Resizable BAR, "Smart
/// Access Memory"), from the size of its VRAM aperture (BAR 0 on amdgpu)
/// against its VRAM. Only amdgpu discrete cards: NVIDIA and Intel put the
/// aperture in another BAR. Firmware decides it (Above 4G Decoding and
/// Re-Size BAR in the setup program); BiGame-mode only reports it.
fn resizable_bar_of(gpu: &crate::hardware::Gpu) -> Option<Check> {
    if gpu.driver != "amdgpu" || !gpu.discrete || gpu.pci_slot.is_empty() {
        return None;
    }
    let vram = gpu.vram_total_bytes?;
    let resource =
        std::fs::read_to_string(format!("/sys/bus/pci/devices/{}/resource", gpu.pci_slot)).ok()?;
    let bar0 = resource.lines().next().and_then(|l| {
        let mut it = l
            .split_whitespace()
            .map(|v| u64::from_str_radix(v.trim_start_matches("0x"), 16));
        let (start, end) = (it.next()?.ok()?, it.next()?.ok()?);
        (end > start).then(|| end - start + 1)
    })?;
    Some(resizable_bar(bar0, vram))
}

/// [`resizable_bar_of`]'s verdict for an aperture of `bar` bytes and `vram`
/// bytes of VRAM.
fn resizable_bar(bar: u64, vram: u64) -> Check {
    let size = |b: u64| {
        if b >= 1 << 30 {
            format!("{} GiB", (b + (1 << 29)) >> 30)
        } else {
            format!("{} MiB", b >> 20)
        }
    };
    // Apertures are powers of two; 16 GiB of VRAM is reported a little short.
    if bar.saturating_mul(10) >= vram.saturating_mul(9) {
        check(
            N_("Resizable BAR"),
            Status::Ok,
            Text::with(
                N_("on: the CPU can reach all %s of video memory"),
                [size(vram)],
            ),
            None,
        )
    } else {
        check(
            N_("Resizable BAR"),
            Status::Info,
            Text::with(
                N_("off: the CPU reaches %s of the card's %s of video memory at a time"),
                [size(bar), size(vram)],
            ),
            advice(N_(
                "It is set in the computer's firmware (Above 4G Decoding and Re-Size BAR). Some games gain from it; it has not been measured on this machine, and Big Game Mode does not change firmware settings",
            )),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dmem_is_reported_only_when_unavailable_or_failing() {
        let mut s = crate::status::FalcondStatus::default();
        assert!(dmem_check(None).is_none());
        assert!(
            dmem_check(Some(&s)).is_none(),
            "an older falcond says nothing"
        );
        s.dmem_cgroup = Some(true);
        assert!(dmem_check(Some(&s)).is_none());
        s.dmem_cgroup = Some(false);
        assert_eq!(dmem_check(Some(&s)).unwrap().status, Status::Info);
        s.dmem_cgroup = Some(true);
        s.dmem_last_error = Some(
            "dmem exists, but the source cgroup hierarchy does not expose dmem to the game scope"
                .into(),
        );
        let c = dmem_check(Some(&s)).unwrap();
        assert_eq!(c.status, Status::Warning);
        assert!(c.detail.english().contains("does not expose dmem"));
    }

    #[test]
    fn a_falcond_directory_writable_without_root_is_named() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = std::env::temp_dir().join(format!("bgm-fdirs-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // Owned by this (non-root) user: writable without the password.
        let c = falcond_dirs_check(&[&dir]).unwrap();
        assert_eq!(c.status, Status::Warning);
        assert!(c.fix.unwrap().english().contains("chmod 755"));
        // A root-owned 0755 directory is fine; a missing one is not checked.
        assert!(falcond_dirs_check(&[Path::new("/usr"), &dir.join("absent")]).is_none());
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn missing_32bit_mangohud_is_named_only_with_mangohud() {
        let dir = std::env::temp_dir().join(format!("bgm-lib32mh-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("mangohud")).unwrap();
        assert!(mangohud_32bit_check(false, &dir).is_none());
        assert!(mangohud_32bit_check(true, &dir).is_some());
        std::fs::write(dir.join("mangohud/libMangoHud.so"), b"").unwrap();
        assert!(mangohud_32bit_check(true, &dir).is_none());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_scheduler_scx_loader_starts_at_boot_is_named() {
        let dir = std::env::temp_dir().join(format!("bgm-scxl-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (etc, share) = (dir.join("etc.toml"), dir.join("share.toml"));
        // The packaged file: every default commented out.
        std::fs::write(
            &share,
            "# default_sched = \"scx_bpfland\"\n[scheds.scx_cake]\n",
        )
        .unwrap();
        assert_eq!(scx_loader_default(&[&etc, &share]), None);
        assert!(scx_loader_default_check(None).is_none());
        std::fs::write(
            &etc,
            "default_sched = \"scx_beerland\"\ndefault_mode = \"Auto\"\n",
        )
        .unwrap();
        let d = scx_loader_default(&[&etc, &share]);
        assert_eq!(d, Some(("scx_beerland".to_owned(), "Auto".to_owned())));
        let c = scx_loader_default_check(d).unwrap();
        assert_eq!(c.status, Status::Info);
        assert!(c.detail.english().contains("scx_beerland (Auto mode)"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn ntsync_is_told_apart_by_device_module_and_kernel() {
        let dir = std::env::temp_dir().join(format!("bgm-ntsync-{}", std::process::id()));
        let modules = dir.join("modules");
        std::fs::create_dir_all(modules.join("kernel/drivers/misc")).unwrap();
        let none = dir.join("absent");
        // No device and no module: a kernel without it.
        let c = ntsync_check(&none, &none, &modules);
        assert_eq!(c.status, Status::NotApplicable);
        // The module file, not loaded: say so, with the package that loads it.
        std::fs::write(modules.join("kernel/drivers/misc/ntsync.ko.zst"), b"").unwrap();
        let c = ntsync_check(&none, &none, &modules);
        assert_eq!(c.status, Status::Warning);
        assert!(c.fix.unwrap().english().contains("ntsync-autoload"));
        // A regular file where the device should be is not a device.
        let fake = dir.join("ntsync");
        std::fs::write(&fake, b"").unwrap();
        assert_eq!(ntsync_check(&fake, &none, &modules).status, Status::Warning);
        // Built into the kernel counts as present too.
        std::fs::remove_file(modules.join("kernel/drivers/misc/ntsync.ko.zst")).unwrap();
        std::fs::write(
            modules.join("modules.builtin"),
            "kernel/drivers/misc/ntsync.ko\n",
        )
        .unwrap();
        assert_eq!(ntsync_check(&none, &none, &modules).status, Status::Warning);
        // A character device the user can open: /dev/null stands in for it.
        assert_eq!(
            ntsync_check(Path::new("/dev/null"), &none, &modules).status,
            Status::Ok
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn resizable_bar_off_and_on() {
        // The reference desktop: RX 9060 XT, 15.92 GiB of VRAM, 256 MiB BAR 0.
        let vram = 17_095_983_104;
        let off = resizable_bar(256 << 20, vram);
        assert_eq!(off.status, Status::Info);
        assert_eq!(
            off.detail.english(),
            "off: the CPU reaches 256 MiB of the card's 16 GiB of video memory at a time"
        );
        let on = resizable_bar(16 << 30, vram);
        assert_eq!(on.status, Status::Ok);
        assert_eq!(
            on.detail.english(),
            "on: the CPU can reach all 16 GiB of video memory"
        );
    }

    #[test]
    fn a_restarted_falcond_is_a_warning_and_an_untouched_one_is_silent() {
        assert_eq!(restart_check(0), None);
        let one = restart_check(1).unwrap();
        assert_eq!(one.status, Status::Warning);
        assert!(
            one.detail.english().contains("once after"),
            "{}",
            one.detail
        );
        assert!(
            restart_check(3)
                .unwrap()
                .detail
                .english()
                .contains("3 times")
        );
        assert_eq!(
            one.fix,
            Some(Fix::Command("journalctl -u falcond -b".into()))
        );
    }

    #[test]
    fn power_profiles_are_restarted_through_biglinux_s_own_unit_where_it_has_one() {
        let dir = tempfile::tempdir().unwrap();
        // Not installed at all: enabling a unit that is not there fails.
        assert_eq!(
            power_profiles_fix(dir.path()),
            "sudo pacman -S --needed power-profiles-daemon"
        );
        std::fs::write(dir.path().join("power-profiles-daemon.service"), "").unwrap();
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
            hot.detail.english().contains("1799 times") && hot.detail.english().contains("14 s"),
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
            c.detail.english().contains("NVIDIA PRIME render offload"),
            "{}",
            c.detail
        );
        assert!(
            c.detail.english().contains("prime-run %command%"),
            "{}",
            c.detail
        );
        // A desktop whose dGPU drives the monitor: nothing to say.
        hw.gpus = vec![g("card0", "nvidia", GpuVendor::Nvidia, true, &["DP-1"])];
        assert_eq!(hybrid_check(&hw, true), None);
    }

    #[test]
    fn every_sentence_is_a_marked_template_ready_for_translation() {
        // A value never lands in a template: the placeholders carry it, so
        // the catalogue sees one sentence per message, whatever the numbers.
        let c = restart_check(4).unwrap();
        assert!(c.detail.template.contains("%s"), "{}", c.detail.template);
        assert_eq!(c.detail.args, ["4"]);
        assert_eq!(c.title.template, "falcond restarts");
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
        // A local build that provides it: its versioned provides, or its own
        // version.
        for (dir, desc) in [
            (
                "falcond-local-2.0.14.r3.gabc-1",
                "%NAME%\nfalcond-local\n\n%VERSION%\n2.0.14.r3.gabc-1\n\n%PROVIDES%\nfalcond=2.0.14\nfalcond-git\n",
            ),
            (
                "vkbasalt-git-r1-1",
                "%NAME%\nvkbasalt-git\n\n%VERSION%\nr1-1\n\n%PROVIDES%\nvkbasalt\n",
            ),
        ] {
            std::fs::create_dir_all(db.join(dir)).unwrap();
            std::fs::write(db.join(dir).join("desc"), desc).unwrap();
        }
        assert_eq!(
            package_version(&db, "falcond").as_deref(),
            Some("2.0.2-2"),
            "the real package first"
        );
        assert_eq!(
            package_version(&db, "falcond-git").as_deref(),
            Some("2.0.14.r3.gabc-1")
        );
        assert_eq!(package_version(&db, "vkbasalt").as_deref(), Some("r1-1"));
        std::fs::remove_dir_all(db.join("falcond-2.0.2-2")).unwrap();
        assert_eq!(package_version(&db, "falcond").as_deref(), Some("2.0.14"));
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
                assert!(!fix.english().contains("rm "), "{}: {fix:?}", c.title);
            }
        }
    }
}
