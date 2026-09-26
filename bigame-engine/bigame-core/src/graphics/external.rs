//! Neural rendering on AMD through an external component: DLSS-NR-on-AMD
//! (github.com/danielblnc/DLSS-NR-on-AMD), detected and explained, never
//! fetched, placed or removed.
//!
//! Its license (2026) allows personal use and forbids redistribution,
//! bundling in another tool, modification and reverse engineering
//! (`docs/AI_GRAPHICS_LICENSE_AUDIT.md`). So this module reads what the user
//! installed — its proxy DLL, configuration, weights and log beside the
//! game — checks the requirements upstream states, and tells the user what
//! is missing, with the official page to get it from. It also needs NVIDIA's
//! own neural-rendering model (`nvngx_dlssnr.dll`), which BiGame-mode never
//! downloads or points at: the user has it or does not.
//!
//! Every state here is read from files or the running game. Nothing is
//! "active" because it was configured.

use std::path::{Path, PathBuf};

use serde::Serialize;

use super::backend::{self, Availability, Backend};
use super::report::Report;
use super::scan::{ComponentKind, ProxyOwner};
use super::text::{N_, Text};

/// The official release page — the only place the component comes from.
pub const OFFICIAL_URL: &str = "https://github.com/danielblnc/DLSS-NR-on-AMD/releases";

/// The proxy slots its documentation lists, its default first.
pub const PROXY_SLOTS: &[&str] = &[
    "version.dll",
    "winmm.dll",
    "dbghelp.dll",
    "wininet.dll",
    "winhttp.dll",
    "dxgi.dll",
];

/// Its log beside the game.
pub const LOG: &str = "dlssnr_on_amd.log";

/// What the user has installed of it beside the game, from the scan.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Installed {
    /// The slot its proxy DLL is in (`version.dll`), when its contents say
    /// it is this component.
    pub proxy: Option<String>,
    /// The proxy's file version, when it has one.
    pub version: Option<String>,
    /// `dlssnr_on_amd.ini` beside the game.
    pub config: bool,
    /// `dlssnr_on_amd_weights.bin`, converted from the model by its setup.
    pub weights: bool,
    /// NVIDIA's `nvngx_dlssnr.dll`, the model it converts.
    pub model: Option<String>,
}

impl Installed {
    /// Whether any of its files is there.
    #[must_use]
    pub fn any(&self) -> bool {
        self.proxy.is_some() || self.config || self.weights
    }
}

/// Where neural rendering stands for a game — the states the page shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case", tag = "state")]
pub enum Status {
    /// A requirement of the backend is not met here.
    Unavailable {
        /// What is missing.
        missing: Vec<backend::Missing>,
    },
    /// Requirements met; the component is not installed beside the game.
    NotInstalled,
    /// Installed by the user: the proxy is in place. Nothing says yet
    /// whether the game loaded it.
    Installed {
        /// What was found.
        found: Installed,
    },
    /// The game is running and has the proxy mapped; its log has not said
    /// the pass ran.
    Loaded {
        /// What was found.
        found: Installed,
    },
    /// The game is running, the proxy is loaded, and its log written since
    /// the game started says it initialised.
    Active {
        /// What was found.
        found: Installed,
        /// The build its log names, when it does.
        build: Option<String>,
    },
    /// Its log written since the game started reports a failure.
    Failed {
        /// What was found.
        found: Installed,
        /// Its error lines.
        errors: Vec<String>,
    },
    /// Anti-cheat: nothing is injected, whatever is installed.
    Blocked {
        /// The anti-cheat.
        anti_cheat: String,
    },
}

impl Status {
    /// The files found, whatever the state.
    #[must_use]
    pub fn found(&self) -> Option<&Installed> {
        match self {
            Self::Installed { found }
            | Self::Loaded { found }
            | Self::Active { found, .. }
            | Self::Failed { found, .. } => Some(found),
            _ => None,
        }
    }
}

/// What the scan found of the component in `r`.
#[must_use]
pub fn installed(r: &Report) -> Installed {
    let proxy = r
        .proxies
        .iter()
        .find(|p| p.owner == ProxyOwner::DlssNrOnAmd);
    let has = |k: ComponentKind| r.components.iter().any(|c| c.kind == k);
    Installed {
        proxy: proxy.map(|p| p.slot.clone()),
        version: proxy.and_then(|p| p.version.clone()),
        config: has(ComponentKind::DlssNrOnAmdConfig),
        weights: has(ComponentKind::DlssNrOnAmdWeights),
        model: r
            .components
            .iter()
            .find(|c| c.kind == ComponentKind::DlssNeuralRendering)
            .map(|c| c.version.clone().unwrap_or_else(|| "present".into())),
    }
}

/// What its log says about one run.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LogFindings {
    /// `dlssnr_amd <version> (build <b>) loaded into <exe> as <slot>`.
    pub loaded: bool,
    /// The build it names.
    pub build: Option<String>,
    /// Lines that say something failed.
    pub errors: Vec<String>,
}

/// Read `dlssnr_on_amd.log`. Its lines are matched on the messages its
/// binary carries; a line naming a missing runtime or weights is a failure.
#[must_use]
pub fn read_log(text: &str) -> LogFindings {
    let mut f = LogFindings::default();
    for line in text.lines() {
        if line.contains("loaded into") && line.contains("dlssnr_amd") {
            f.loaded = true;
            if let Some(rest) = line.split("(build ").nth(1) {
                f.build = rest.split(')').next().map(str::to_owned);
            }
        }
        let lower = line.to_ascii_lowercase();
        if lower.contains("was not found")
            || lower.contains("not found next to")
            || lower.contains("failed")
            || lower.contains("error")
        {
            let msg = line.trim().to_owned();
            if !f.errors.contains(&msg) {
                f.errors.push(msg);
            }
        }
    }
    f
}

/// The requirements upstream states, checked against `r`, plus the two
/// only a Windows machine meets today: AMD's HIP runtime in the prefix and
/// the user's own model beside the game.
#[must_use]
pub fn availability(r: &Report) -> Availability {
    let mut missing = match backend::check(Backend::AmdNeuralExternal, r) {
        Availability::Available => Vec::new(),
        Availability::Unavailable { missing } => missing,
    };
    if r.proton.as_ref().is_none_or(|p| !p.hip_runtime) {
        missing.push(backend::Missing {
            what: N_("AMD HIP runtime"),
            detail: Text::plain(N_(
                "the component runs its kernels through AMD's Windows HIP runtime (amdhip64_7.dll), which comes with the Adrenalin driver; Proton ships none, and this prefix has none",
            )),
        });
    }
    if installed(r).model.is_none() {
        missing.push(backend::Missing {
            what: N_("Neural-rendering model"),
            detail: Text::plain(N_(
                "your own copy of NVIDIA's DLSS neural-rendering model (nvngx_dlssnr.dll) beside the game; BiGame-mode does not download it or say where to get it",
            )),
        });
    }
    if missing.is_empty() {
        Availability::Available
    } else {
        Availability::Unavailable { missing }
    }
}

/// The status for a game.
///
/// `running` is the game's pid, the executable's folder (absolute) and how
/// long it has run; `maps` and `log` read the process and the log, passed in
/// so this can be tested without a game.
#[must_use]
pub fn status(
    r: &Report,
    running: Option<(u32, &Path, std::time::Duration)>,
    maps: &dyn Fn(u32) -> Option<String>,
    log: &dyn Fn(&Path, std::time::Duration) -> Option<String>,
) -> Status {
    if let Some(ac) = r.anti_cheat.first() {
        return Status::Blocked {
            anti_cheat: ac.name.clone(),
        };
    }
    let found = installed(r);
    if !found.any() {
        return match availability(r) {
            Availability::Available => Status::NotInstalled,
            Availability::Unavailable { missing } => Status::Unavailable { missing },
        };
    }
    let Some((pid, exe_dir, age)) = running else {
        return Status::Installed { found };
    };
    let proxy_path: Option<PathBuf> = found.proxy.as_ref().and_then(|slot| {
        r.proxies
            .iter()
            .find(|p| &p.slot == slot)
            .map(|p| r.install_root.join(&p.path))
    });
    let loaded = proxy_path.as_ref().is_some_and(|p| {
        maps(pid).is_some_and(|text| super::runtime::mapped_paths(&text).iter().any(|q| q == p))
    });
    if let Some(f) = log(exe_dir, age).map(|t| read_log(&t)) {
        if !f.errors.is_empty() {
            return Status::Failed {
                found,
                errors: f.errors,
            };
        }
        if f.loaded {
            return Status::Active {
                found,
                build: f.build,
            };
        }
    }
    if loaded {
        Status::Loaded { found }
    } else {
        Status::Installed { found }
    }
}

/// Read the component's log beside the game if it was written after a
/// process that has run for `age` started.
#[must_use]
pub fn fresh_log(exe_dir: &Path, age: std::time::Duration) -> Option<String> {
    let path = exe_dir.join(LOG);
    let modified = std::fs::metadata(&path).ok()?.modified().ok()?;
    let started = std::time::SystemTime::now().checked_sub(age)?;
    if modified + std::time::Duration::from_secs(2) < started {
        return None;
    }
    std::fs::read_to_string(path).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graphics::report::{ApiEvidence, Confidence, GpuInfo, Native, ProtonInfo};
    use crate::graphics::scan::{AntiCheat, Component, Proxy};
    use crate::hardware::GpuVendor;
    use std::time::Duration;

    fn report(ffx: bool, hip: bool, model: bool) -> Report {
        let mut components = vec![];
        if model {
            components.push(Component {
                kind: ComponentKind::DlssNeuralRendering,
                path: "nvngx_dlssnr.dll".into(),
                version: Some("310.8.0.0".into()),
            });
        }
        Report {
            game: "G".into(),
            app_id: Some("1".into()),
            install_root: "/g".into(),
            executable: Some("G.exe".into()),
            machine: Some(crate::graphics::pe::Machine::X64),
            runtime: None,
            api: ApiEvidence {
                api: Some(crate::graphics::optiscaler::Api::Dx12),
                confidence: Confidence::Detected,
                evidence: vec![],
                translation: None,
            },
            native: Native {
                ffx_api: ffx.then(|| "1.0.1".into()),
                ..Native::default()
            },
            proxies: vec![],
            anti_cheat: vec![],
            gpus: vec![GpuInfo {
                card: "card1".into(),
                vendor: GpuVendor::Amd,
                name: "Navi 44 [Radeon RX 9060 XT]".into(),
                driver: "amdgpu".into(),
                userspace: None,
                vram: None,
                discrete: true,
                rdna: Some(4),
                renders_game: true,
            }],
            render_gpu: Some(0),
            installed: None,
            scan_truncated: false,
            listed: None,
            proton: Some(ProtonInfo {
                prefix: "/pfx".into(),
                fsr4_provider: true,
                windows_version: Some("10".into()),
                tool: None,
                hip_runtime: hip,
            }),
            components,
        }
    }

    fn no_maps(_: u32) -> Option<String> {
        None
    }
    fn no_log(_: &Path, _: Duration) -> Option<String> {
        None
    }

    #[test]
    fn the_reference_desktop_is_told_exactly_what_is_missing() {
        // RDNA 4, DX12, FFX API: upstream's requirements met; Proton has no
        // HIP runtime and the user has no model.
        let s = status(&report(true, false, false), None, &no_maps, &no_log);
        let Status::Unavailable { missing } = s else {
            panic!("{s:?}");
        };
        let what: Vec<&str> = missing.iter().map(|m| m.what).collect();
        assert_eq!(what, ["AMD HIP runtime", "Neural-rendering model"]);
        // Everything there: not installed, with the page to get it from.
        assert_eq!(
            status(&report(true, true, true), None, &no_maps, &no_log),
            Status::NotInstalled
        );
    }

    #[test]
    fn installed_loaded_active_and_failed_are_read_not_assumed() {
        let mut r = report(true, true, true);
        r.proxies.push(Proxy {
            slot: "version.dll".into(),
            path: "version.dll".into(),
            owner: ProxyOwner::DlssNrOnAmd,
            version: Some("0.4.0".into()),
        });
        r.components.push(Component {
            kind: ComponentKind::DlssNrOnAmdConfig,
            path: "dlssnr_on_amd.ini".into(),
            version: None,
        });
        let found = installed(&r);
        assert_eq!(found.proxy.as_deref(), Some("version.dll"));
        assert!(found.config && found.model.is_some());
        assert_eq!(
            status(&r, None, &no_maps, &no_log),
            Status::Installed {
                found: found.clone()
            }
        );
        let running = Some((1, Path::new("/g"), Duration::from_secs(60)));
        let mapped = |_: u32| Some("7f00 r-xp 0 0:1 1 /g/version.dll\n".to_owned());
        assert_eq!(
            status(&r, running, &mapped, &no_log),
            Status::Loaded {
                found: found.clone()
            }
        );
        let active = |_: &Path, _: Duration| {
            Some("dlssnr_amd 0.4.0 (build 2026-09-25) loaded into G.exe as version.dll from /g; log x; settings y\n".to_owned())
        };
        assert_eq!(
            status(&r, running, &mapped, &active),
            Status::Active {
                found: found.clone(),
                build: Some("2026-09-25".into())
            }
        );
        let failed = |_: &Path, _: Duration| {
            Some("The AMD HIP runtime (amdhip64_7.dll) was not found\n".to_owned())
        };
        assert!(matches!(
            status(&r, running, &mapped, &failed),
            Status::Failed { .. }
        ));
    }

    #[test]
    fn anti_cheat_blocks_it_whatever_is_installed() {
        let mut r = report(true, true, true);
        r.anti_cheat.push(AntiCheat {
            name: "Easy Anti-Cheat".into(),
            evidence: "EasyAntiCheat".into(),
        });
        assert_eq!(
            status(&r, None, &no_maps, &no_log),
            Status::Blocked {
                anti_cheat: "Easy Anti-Cheat".into()
            }
        );
    }
}
