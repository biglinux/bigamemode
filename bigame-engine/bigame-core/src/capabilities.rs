//! Capability discovery — what this system can actually be asked to do.
//!
//! The rule this module exists to enforce: **never offer, and never plan, an
//! optimization the machine cannot carry out.** Audit finding GS-01 (the project
//! emitted a Gamescope flag removed three releases earlier) and SCX-02 (a
//! scheduler picker on a machine with no `scx_loader`) were both the direct
//! result of assuming instead of probing.
//!
//! Probing is deliberately cheap — `--version` at most, never a benchmark — and
//! results are meant to be cached by the caller for the life of a Booster run.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Whether a feature can be used, and if not, why not.
///
/// The distinction matters for the UI: "your hardware cannot do this" and "the
/// package is missing, here is the install button" are different messages, and
/// neither is "off".
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Support {
    /// Present and usable.
    Available,
    /// The software is not installed. Carries the package name to suggest.
    NotInstalled(String),
    /// Installed, but this machine's hardware cannot use it.
    Unsupported(String),
    /// Installed and supported, but a prerequisite service is not running.
    ServiceDown(String),
}

impl Support {
    /// True only for [`Support::Available`].
    #[must_use]
    pub fn is_available(&self) -> bool {
        matches!(self, Self::Available)
    }

    /// Human-readable reason this is not available, if it is not.
    #[must_use]
    pub fn reason(&self) -> Option<&str> {
        match self {
            Self::Available => None,
            Self::NotInstalled(s) | Self::Unsupported(s) | Self::ServiceDown(s) => Some(s),
        }
    }
}

/// A parsed `major.minor.patch` version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version {
    /// Major component.
    pub major: u32,
    /// Minor component.
    pub minor: u32,
    /// Patch component; `0` when the string had only two parts.
    pub patch: u32,
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// Pull the first `N.N[.N]` out of arbitrary tool output.
#[must_use]
pub fn parse_version(text: &str) -> Option<Version> {
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if !bytes[i].is_ascii_digit() {
            i += 1;
            continue;
        }
        // A version must not start mid-number (e.g. the `16` of "gcc 16.2.1"
        // is fine, but the `0` of "x86_64" must not start one).
        if i > 0 && (bytes[i - 1].is_ascii_alphanumeric() || bytes[i - 1] == b'_') {
            i += 1;
            continue;
        }
        let start = i;
        while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b'.') {
            i += 1;
        }
        let mut parts = text[start..i].split('.').filter(|p| !p.is_empty());
        let major = parts.next()?.parse().ok()?;
        let Some(minor) = parts.next().and_then(|p| p.parse().ok()) else {
            continue; // bare integer — not a version
        };
        return Some(Version {
            major,
            minor,
            patch: parts.next().and_then(|p| p.parse().ok()).unwrap_or(0),
        });
    }
    None
}

/// What the installed Gamescope build accepts.
///
/// Populated by parsing `gamescope --help`, which is authoritative for the
/// binary actually on disk — far safer than mapping a version number to a
/// feature list, because distributions patch Gamescope heavily.
#[derive(Debug, Clone, Default)]
pub struct GamescopeCaps {
    /// Version reported by `gamescope --version`.
    pub version: Option<Version>,
    /// Flags advertised in `--help`, without leading dashes.
    pub flags: Vec<String>,
}

impl GamescopeCaps {
    /// Whether `--<flag>` (or `-<flag>` for short options) appears in `--help`.
    #[must_use]
    pub fn has_flag(&self, flag: &str) -> bool {
        let f = flag.trim_start_matches('-');
        self.flags.iter().any(|k| k == f)
    }

    /// Parse the flag list out of `--help` text.
    #[must_use]
    pub fn parse_help(help: &str) -> Vec<String> {
        let mut flags = Vec::new();
        for line in help.lines() {
            for token in line.split_whitespace() {
                let token = token.trim_end_matches(',');
                let Some(name) = token.strip_prefix("--") else {
                    // Short options: `-F` in "  -F, --filter".
                    if let Some(short) = token.strip_prefix('-') {
                        if short.len() == 1 && short.chars().all(char::is_alphabetic) {
                            let s = short.to_owned();
                            if !flags.contains(&s) {
                                flags.push(s);
                            }
                        }
                    }
                    continue;
                };
                if name.is_empty() || !name.starts_with(|c: char| c.is_ascii_alphabetic()) {
                    continue;
                }
                let name = name.to_owned();
                if !flags.contains(&name) {
                    flags.push(name);
                }
            }
        }
        flags
    }
}

/// sched-ext availability on this kernel.
#[derive(Debug, Clone, Default)]
pub struct SchedExtCaps {
    /// `/sys/kernel/sched_ext` exists — the kernel was built with sched-ext.
    pub kernel_support: bool,
    /// Contents of `/sys/kernel/sched_ext/state` (`disabled`, `enabled`, …).
    pub state: Option<String>,
    /// Scheduler names found as `/usr/bin/scx_*`, without the `scx_` prefix.
    pub installed: Vec<String>,
    /// `scxctl` is on `PATH`.
    pub scxctl: bool,
    /// The `org.scx.Loader` D-Bus service is reachable.
    ///
    /// Without it neither falcond nor this project can switch schedulers, no
    /// matter how many `scx_*` binaries are installed.
    pub loader_service: bool,
}

impl SchedExtCaps {
    /// Can a scheduler actually be switched right now?
    #[must_use]
    pub fn switchable(&self) -> Support {
        if !self.kernel_support {
            return Support::Unsupported("kernel has no sched_ext support".into());
        }
        if self.installed.is_empty() {
            return Support::NotInstalled("scx-scheds".into());
        }
        if !self.loader_service && !self.scxctl {
            return Support::ServiceDown("scx_loader service is not running".into());
        }
        Support::Available
    }
}

/// Everything the Booster planner needs to know about installed software.
///
/// The many booleans are deliberate: this is a flat roster of independent
/// "is this present?" answers, not a state machine. Grouping them into
/// sub-structs would add indirection without removing a single question.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone)]
pub struct Capabilities {
    /// Gamescope, when installed.
    pub gamescope: Option<GamescopeCaps>,
    /// `MangoHud` binary present.
    pub mangohud: bool,
    /// `mangoapp` present — required for Gamescope's `--mangoapp` overlay.
    pub mangoapp: bool,
    /// falcond binary present.
    pub falcond_installed: bool,
    /// falcond currently running.
    pub falcond_running: bool,
    /// Feral `GameMode` present. Relevant because it and falcond contend for the
    /// same knobs; see `docs/02-PERFORMANCE-AUTHORITY.md`.
    pub gamemode: bool,
    /// power-profiles-daemon reachable on the system bus.
    pub power_profiles: bool,
    /// Profiles power-profiles-daemon offers.
    pub power_profiles_available: Vec<String>,
    /// sched-ext state.
    pub sched_ext: SchedExtCaps,
    /// lsfg-vk Vulkan layer manifest installed.
    pub lsfg_vk: bool,
    /// vkBasalt layer installed.
    pub vkbasalt: bool,
    /// Steam client present.
    pub steam: bool,
}

impl Capabilities {
    /// Probe the system.
    #[must_use]
    pub fn detect() -> Self {
        Self {
            gamescope: detect_gamescope(),
            mangohud: which("mangohud").is_some(),
            mangoapp: which("mangoapp").is_some(),
            falcond_installed: which("falcond").is_some(),
            falcond_running: falcond_running(),
            gamemode: which("gamemoderun").is_some() || which("gamemoded").is_some(),
            power_profiles: crate::dbus::power_profile_get().is_some(),
            power_profiles_available: crate::dbus::power_profiles_available(),
            sched_ext: detect_sched_ext(),
            lsfg_vk: vulkan_layer_installed("VkLayer_LS_frame_generation"),
            vkbasalt: vulkan_layer_installed("vkBasalt"),
            steam: which("steam").is_some(),
        }
    }

    /// Gamescope support status, with a reason when unavailable.
    #[must_use]
    pub fn gamescope_support(&self) -> Support {
        match &self.gamescope {
            Some(_) => Support::Available,
            None => Support::NotInstalled("gamescope".into()),
        }
    }
}

// ── Probes ───────────────────────────────────────────────────────────────────

/// Find an executable on `PATH`.
#[must_use]
pub fn which(binary: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(binary))
        .find(|p| p.is_file())
}

fn detect_gamescope() -> Option<GamescopeCaps> {
    which("gamescope")?;
    // Gamescope prints its banner on stderr and the option list on stdout;
    // merge both so neither layout surprises us.
    let out = Command::new("gamescope").arg("--help").output().ok()?;
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    text.push('\n');
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    let version = text
        .lines()
        .find(|l| l.contains("gamescope version"))
        .and_then(parse_version);
    Some(GamescopeCaps {
        version,
        flags: GamescopeCaps::parse_help(&text),
    })
}

fn falcond_running() -> bool {
    Command::new("systemctl")
        .args(["is-active", "--quiet", "falcond"])
        .status()
        .is_ok_and(|s| s.success())
}

fn detect_sched_ext() -> SchedExtCaps {
    let kernel_support = Path::new("/sys/kernel/sched_ext").is_dir();
    let mut installed: Vec<String> = Vec::new();
    if let Ok(entries) = std::fs::read_dir("/usr/bin") {
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if let Some(sched) = name.strip_prefix("scx_") {
                if !sched.is_empty() {
                    installed.push(sched.to_owned());
                }
            }
        }
    }
    installed.sort();
    installed.dedup();
    SchedExtCaps {
        kernel_support,
        state: std::fs::read_to_string("/sys/kernel/sched_ext/state")
            .ok()
            .map(|s| s.trim().to_owned()),
        installed,
        scxctl: which("scxctl").is_some(),
        loader_service: crate::dbus::system_service_running("org.scx.Loader"),
    }
}

fn vulkan_layer_installed(stem: &str) -> bool {
    const DIRS: &[&str] = &[
        "/usr/share/vulkan/implicit_layer.d",
        "/usr/local/share/vulkan/implicit_layer.d",
        "/etc/vulkan/implicit_layer.d",
    ];
    DIRS.iter().any(|dir| {
        std::fs::read_dir(dir).is_ok_and(|entries| {
            entries.flatten().any(|e| {
                e.file_name()
                    .to_string_lossy()
                    .to_ascii_lowercase()
                    .starts_with(&stem.to_ascii_lowercase())
            })
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_gamescope_banner() {
        let v = parse_version("[gamescope] Info console: gamescope version 3.16.28 (gcc 16.2.1)");
        assert_eq!(
            v,
            Some(Version {
                major: 3,
                minor: 16,
                patch: 28
            })
        );
    }

    #[test]
    fn parses_two_component_versions() {
        assert_eq!(
            parse_version("mangohud 0.8"),
            Some(Version {
                major: 0,
                minor: 8,
                patch: 0
            })
        );
    }

    #[test]
    fn ignores_bare_integers_and_arch_suffixes() {
        assert_eq!(parse_version("build 12345"), None);
        // `x86_64` must not be read as version 64.something.
        assert_eq!(parse_version("linux x86_64"), None);
        assert_eq!(parse_version("no numbers here"), None);
    }

    /// Trimmed from the real `gamescope 3.16.28 --help` on the bench.
    const HELP: &str = "\
usage: gamescope [options...] -- [command...]

Options:
  --help                         show help message
  -W, --output-width             output width
  -H, --output-height            output height
  -w, --nested-width             game width
  -F, --filter                   upscaler filter (linear, nearest, fsr, nis, pixel)
  --sharpness, --fsr-sharpness   upscaler sharpness from 0 (max) to 20 (min)
  --backend                      select rendering backend
  --hdr-enabled                  enable HDR output
  --framerate-limit              Set a simple framerate limit.
  --mangoapp                     Launch with the mangoapp overlay enabled.
  --adaptive-sync                Enable adaptive sync if available
  -f, --fullscreen               make the window fullscreen
  -b, --borderless               make the window borderless
";

    #[test]
    fn detects_flags_that_exist() {
        let caps = GamescopeCaps {
            version: None,
            flags: GamescopeCaps::parse_help(HELP),
        };
        for flag in [
            "filter",
            "fsr-sharpness",
            "adaptive-sync",
            "hdr-enabled",
            "mangoapp",
            "framerate-limit",
            "backend",
            "output-width",
        ] {
            assert!(caps.has_flag(flag), "expected --{flag}");
        }
        // Short options must be picked up too — `-F` is how the filter is set.
        assert!(caps.has_flag("F"));
        assert!(caps.has_flag("f"));
        assert!(caps.has_flag("b"));
    }

    #[test]
    fn rejects_the_flag_that_caused_gs_01() {
        let caps = GamescopeCaps {
            version: None,
            flags: GamescopeCaps::parse_help(HELP),
        };
        // `--fsr` was removed from Gamescope; the old builder emitted it anyway
        // and every launch failed. Capability detection is what stops that.
        assert!(!caps.has_flag("fsr"));
        assert!(!caps.has_flag("nis"));
        assert!(!caps.has_flag("invented-flag"));
    }

    #[test]
    fn leading_dashes_are_optional_when_querying() {
        let caps = GamescopeCaps {
            version: None,
            flags: GamescopeCaps::parse_help(HELP),
        };
        assert!(caps.has_flag("--filter"));
        assert!(caps.has_flag("filter"));
    }

    #[test]
    fn support_reports_reasons() {
        assert!(Support::Available.is_available());
        assert_eq!(Support::Available.reason(), None);
        let s = Support::NotInstalled("gamescope".into());
        assert!(!s.is_available());
        assert_eq!(s.reason(), Some("gamescope"));
    }

    #[test]
    fn sched_ext_is_not_switchable_without_a_loader() {
        // Exactly the bench state: kernel support yes, 16 binaries installed,
        // but org.scx.Loader is absent and scxctl is not installed.
        let caps = SchedExtCaps {
            kernel_support: true,
            state: Some("disabled".into()),
            installed: vec!["lavd".into(), "bpfland".into()],
            scxctl: false,
            loader_service: false,
        };
        assert_eq!(
            caps.switchable(),
            Support::ServiceDown("scx_loader service is not running".into())
        );
    }

    #[test]
    fn sched_ext_switchable_when_loader_present() {
        let caps = SchedExtCaps {
            kernel_support: true,
            state: Some("disabled".into()),
            installed: vec!["lavd".into()],
            scxctl: true,
            loader_service: false,
        };
        assert!(caps.switchable().is_available());
    }

    #[test]
    fn sched_ext_unsupported_without_kernel() {
        let caps = SchedExtCaps::default();
        assert!(matches!(caps.switchable(), Support::Unsupported(_)));
    }

    #[test]
    fn sched_ext_missing_binaries() {
        let caps = SchedExtCaps {
            kernel_support: true,
            installed: Vec::new(),
            ..SchedExtCaps::default()
        };
        assert!(matches!(caps.switchable(), Support::NotInstalled(_)));
    }
}
