//! The ways AI Graphics can reach a game — the *backends* — and what each
//! one needs, in one place.
//!
//! Three backends, three very different relationships with the game's files:
//!
//! - **Native**: the game's own upscaler, frame generation and, on RDNA 4
//!   under Proton, FSR 4 through the provider Proton itself ships. Nothing is
//!   placed in the game.
//! - **`OptiScaler`**: placed by BiGame-mode as a proxy DLL beside the game,
//!   as a transaction that is backed up and can be undone
//!   ([`super::transaction`]). The only backend BiGame-mode manages files for.
//! - **AMD neural rendering, external**: DLSS-NR-on-AMD, a project whose
//!   license allows neither redistribution nor automated installation
//!   (see `docs/AI_GRAPHICS_LICENSE_AUDIT.md`). BiGame-mode detects it,
//!   explains it, links to it and reports on it; it never downloads, places
//!   or removes its files.
//!
//! What a backend needs is stated as data ([`Capabilities`]) and checked
//! ([`check`]) against one game and one machine, so a page can say exactly
//! what is missing instead of greying a control out — and so `GPU == AMD`
//! is decided here, not in a dozen places.

use serde::Serialize;

use super::optiscaler::Api;
use super::report::{GpuInfo, Report};
use super::text::{N_, Text};
use crate::hardware::GpuVendor;

const OPTISCALER_RISKS: &[&str] = &[
    N_("loaded into the game as a DLL: never for games with anti-cheat"),
    N_("its frame generation is experimental and adds latency"),
];

const AMD_NEURAL_RISKS: &[&str] = &[
    N_("an external component BiGame-mode does not distribute, install or remove"),
    N_("documented for Windows; not established under Proton"),
    N_("takes a DLL slot beside the game and may collide with OptiScaler"),
];

/// A way to bring upscaling, neural rendering or frame generation to a game.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Backend {
    /// The game's own features; no files placed.
    Native,
    /// `OptiScaler`, placed by BiGame-mode.
    #[serde(rename = "optiscaler")]
    OptiScaler,
    /// DLSS-NR-on-AMD, installed by the user, never by BiGame-mode.
    AmdNeuralExternal,
}

impl Backend {
    /// The id written in manifests and logs.
    #[must_use]
    pub fn id(self) -> &'static str {
        match self {
            Self::Native => "native",
            Self::OptiScaler => "optiscaler",
            Self::AmdNeuralExternal => "amd_neural_external",
        }
    }

    /// The backend an id names; unknown ids are `None`.
    #[must_use]
    pub fn from_id(id: &str) -> Option<Self> {
        match id {
            "native" => Some(Self::Native),
            "optiscaler" => Some(Self::OptiScaler),
            "amd_neural_external" => Some(Self::AmdNeuralExternal),
            _ => None,
        }
    }

    /// A name for the UI.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Native => N_("the game's own"),
            Self::OptiScaler => "OptiScaler",
            Self::AmdNeuralExternal => "DLSS-NR-on-AMD",
        }
    }

    /// Every backend, in the order pages list them.
    pub const ALL: [Self; 3] = [Self::Native, Self::OptiScaler, Self::AmdNeuralExternal];

    /// What this backend needs and does.
    #[must_use]
    pub fn capabilities(self) -> Capabilities {
        match self {
            Self::Native => Capabilities {
                backend: self,
                gpu_vendors: &[
                    GpuVendor::Amd,
                    GpuVendor::Nvidia,
                    GpuVendor::Intel,
                    GpuVendor::Other,
                ],
                amd_generations: &[],
                apis: &[Api::Dx11, Api::Dx12, Api::Vulkan],
                windows_game: false,
                needs_ffx_api: false,
                upscaling: true,
                neural_rendering: false,
                frame_generation: true,
                managed: false,
                maturity: Maturity::VerifiedHere,
                risks: &[],
            },
            Self::OptiScaler => Capabilities {
                backend: self,
                gpu_vendors: &[GpuVendor::Amd, GpuVendor::Nvidia, GpuVendor::Intel],
                amd_generations: &[],
                apis: &[Api::Dx11, Api::Dx12, Api::Vulkan],
                windows_game: true,
                needs_ffx_api: false,
                upscaling: true,
                neural_rendering: false,
                frame_generation: true,
                managed: true,
                maturity: Maturity::VerifiedHere,
                risks: OPTISCALER_RISKS,
            },
            // Upstream states Windows, RX 7000/9000, a DirectX 12 game with an
            // FSR path. Linux/Proton is not stated; BiGame-mode treats the
            // pair as unverified until a run here says otherwise.
            Self::AmdNeuralExternal => Capabilities {
                backend: self,
                gpu_vendors: &[GpuVendor::Amd],
                amd_generations: &[3, 4],
                apis: &[Api::Dx12],
                windows_game: true,
                needs_ffx_api: true,
                upscaling: false,
                neural_rendering: true,
                frame_generation: false,
                managed: false,
                maturity: Maturity::Experimental,
                risks: AMD_NEURAL_RISKS,
            },
        }
    }
}

/// How far a backend has been taken on BiGame-mode's own machines.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Maturity {
    /// Ran, rendered, was measured on a BiGame-mode test machine.
    VerifiedHere,
    /// Documented upstream; not verified by BiGame-mode.
    Documented,
    /// Reported to work, or not established at all.
    Experimental,
}

/// What a backend needs and offers, as data.
// Independent facts a page reads one by one; grouping them would only hide
// which is which.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Capabilities {
    /// Which backend.
    pub backend: Backend,
    /// GPU vendors it runs on.
    pub gpu_vendors: &'static [GpuVendor],
    /// AMD RDNA generations it runs on; empty means any AMD card.
    pub amd_generations: &'static [u8],
    /// Graphics APIs of the game it works with.
    pub apis: &'static [Api],
    /// Needs a Windows game (Proton or Wine): it is a Windows DLL.
    pub windows_game: bool,
    /// Needs the game to ship AMD's `FidelityFX` API (`amd_fidelityfx_dx12.dll`),
    /// the FSR 3.1+ path a provider can take over.
    pub needs_ffx_api: bool,
    /// Provides upscaling.
    pub upscaling: bool,
    /// Provides neural rendering.
    pub neural_rendering: bool,
    /// Provides frame generation.
    pub frame_generation: bool,
    /// BiGame-mode places and removes its files (through a transaction).
    pub managed: bool,
    /// How established it is.
    pub maturity: Maturity,
    /// What to know before choosing it.
    pub risks: &'static [&'static str],
}

/// Something a backend needs that this game or machine does not have.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Missing {
    /// A short name of the requirement (`GPU`, `Graphics API`, …), marked
    /// for translation.
    pub what: &'static str,
    /// What is needed and what was found, in a sentence.
    pub detail: Text,
}

/// Whether a backend can be used for a game on this machine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum Availability {
    /// Every requirement is met.
    Available,
    /// Every requirement that is not.
    Unavailable {
        /// What is missing, in the order it was checked.
        missing: Vec<Missing>,
    },
}

impl Availability {
    /// Whether every requirement is met.
    #[must_use]
    pub fn is_available(&self) -> bool {
        matches!(self, Self::Available)
    }
}

/// Check `backend`'s requirements against `r`.
///
/// Anti-cheat is not a requirement but a veto, and it is applied by the
/// planner to everything that injects; this only answers "could it run".
#[must_use]
pub fn check(backend: Backend, r: &Report) -> Availability {
    let c = backend.capabilities();
    let mut missing = Vec::new();
    let gpu: Option<&GpuInfo> = r.gpu();
    match gpu {
        None => missing.push(Missing {
            what: N_("GPU"),
            detail: Text::plain(N_("no GPU was identified for this game")),
        }),
        Some(g) => {
            if !c.gpu_vendors.contains(&g.vendor) {
                missing.push(Missing {
                    what: N_("GPU"),
                    detail: Text::with(
                        N_("needs %s; this game renders on %s"),
                        [vendors(c.gpu_vendors), g.name.clone()],
                    ),
                });
            } else if g.vendor == GpuVendor::Amd && !c.amd_generations.is_empty() {
                match g.rdna {
                    Some(generation) if c.amd_generations.contains(&generation) => {}
                    Some(generation) => missing.push(Missing {
                        what: N_("GPU"),
                        detail: Text::with(
                            N_("needs an AMD RDNA %s card; %s is RDNA %s"),
                            [
                                generations(c.amd_generations),
                                g.name.clone(),
                                generation.to_string(),
                            ],
                        ),
                    }),
                    None => missing.push(Missing {
                        what: N_("GPU"),
                        detail: Text::with(
                            N_("needs an AMD RDNA %s card; the generation of %s is not known"),
                            [generations(c.amd_generations), g.name.clone()],
                        ),
                    }),
                }
            }
        }
    }
    if c.windows_game && (r.executable.is_none() || r.runtime.as_deref() == Some("native")) {
        missing.push(Missing {
            what: N_("Game"),
            detail: Text::plain(N_(
                "a Windows game under Proton or Wine; this is a native Linux game",
            )),
        });
    }
    if c.windows_game && r.machine == Some(super::pe::Machine::X86) {
        missing.push(Missing {
            what: N_("Game"),
            detail: Text::plain(N_("a 64-bit game; this one is 32-bit")),
        });
    }
    if let Some(api) = r.api.api {
        if !c.apis.contains(&api) {
            missing.push(Missing {
                what: N_("Graphics API"),
                detail: Text::with(
                    N_("needs %s; this game renders with %s"),
                    [apis(c.apis), api_name(api).to_owned()],
                ),
            });
        }
    }
    if c.needs_ffx_api && r.native.ffx_api.is_none() {
        missing.push(Missing {
            what: N_("FSR path"),
            detail: Text::plain(N_(
                "the game must ship AMD's FidelityFX API (FSR 3.1 or newer); it does not",
            )),
        });
    }
    if missing.is_empty() {
        Availability::Available
    } else {
        Availability::Unavailable { missing }
    }
}

fn vendors(v: &[GpuVendor]) -> String {
    v.iter()
        .map(|g| match g {
            GpuVendor::Amd => "AMD",
            GpuVendor::Nvidia => "NVIDIA",
            GpuVendor::Intel => "Intel",
            GpuVendor::Other => "other",
        })
        .collect::<Vec<_>>()
        .join(" / ")
}

fn generations(g: &[u8]) -> String {
    g.iter().map(u8::to_string).collect::<Vec<_>>().join(" / ")
}

/// The API's name as people write it.
#[must_use]
pub fn api_name(api: Api) -> &'static str {
    match api {
        Api::Dx11 => "DirectX 11",
        Api::Dx12 => "DirectX 12",
        Api::Vulkan => "Vulkan",
    }
}

fn apis(a: &[Api]) -> String {
    a.iter()
        .map(|x| api_name(*x))
        .collect::<Vec<_>>()
        .join(" / ")
}

/// The three jobs AI Graphics tells apart. One technology owns each job for
/// a game; two in series is a conflict ([`super::rules`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Job {
    /// Rendering below the output resolution and reconstructing the image.
    Upscaling,
    /// Neural post-processing of the rendered image.
    NeuralRendering,
    /// Presenting frames that were not rendered.
    FrameGeneration,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graphics::report::{ApiEvidence, Confidence, Native};

    fn report(vendor: GpuVendor, rdna: Option<u8>, api: Option<Api>, ffx: bool) -> Report {
        Report {
            game: "G".into(),
            app_id: None,
            install_root: "/g".into(),
            executable: Some("G.exe".into()),
            machine: Some(super::super::pe::Machine::X64),
            runtime: None,
            api: ApiEvidence {
                api,
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
                vendor,
                name: "GPU".into(),
                driver: String::new(),
                userspace: None,
                vram: None,
                discrete: true,
                rdna,
                renders_game: true,
            }],
            render_gpu: Some(0),
            installed: None,
            scan_truncated: false,
            listed: None,
            proton: None,
            components: vec![],
        }
    }

    #[test]
    fn the_external_amd_backend_needs_rdna_3_or_4_dx12_and_an_ffx_path() {
        let ok = report(GpuVendor::Amd, Some(4), Some(Api::Dx12), true);
        assert!(check(Backend::AmdNeuralExternal, &ok).is_available());
        let missing = |r: &Report| match check(Backend::AmdNeuralExternal, r) {
            Availability::Unavailable { missing } => {
                missing.iter().map(|m| m.what).collect::<Vec<_>>()
            }
            Availability::Available => vec![],
        };
        assert_eq!(
            missing(&report(GpuVendor::Nvidia, None, Some(Api::Dx12), true)),
            ["GPU"]
        );
        assert_eq!(
            missing(&report(GpuVendor::Amd, Some(2), Some(Api::Dx12), true)),
            ["GPU"]
        );
        assert_eq!(
            missing(&report(GpuVendor::Amd, Some(4), Some(Api::Dx11), false)),
            ["Graphics API", "FSR path"]
        );
        // A native Linux game.
        let mut native = report(GpuVendor::Amd, Some(4), Some(Api::Vulkan), false);
        native.runtime = Some("native".into());
        let m = missing(&native);
        assert!(m.contains(&"Game") && m.contains(&"Graphics API"));
    }

    #[test]
    fn optiscaler_runs_on_every_vendor_but_only_for_64_bit_windows_games() {
        assert!(
            check(
                Backend::OptiScaler,
                &report(GpuVendor::Intel, None, Some(Api::Dx11), false)
            )
            .is_available()
        );
        let mut x86 = report(GpuVendor::Amd, Some(4), Some(Api::Dx12), false);
        x86.machine = Some(super::super::pe::Machine::X86);
        assert!(!check(Backend::OptiScaler, &x86).is_available());
        // The game's own features have no requirements beyond a GPU.
        assert!(check(Backend::Native, &x86).is_available());
    }

    #[test]
    fn ids_round_trip_and_only_optiscaler_is_managed() {
        for b in Backend::ALL {
            assert_eq!(Backend::from_id(b.id()), Some(b));
            assert_eq!(b.capabilities().managed, b == Backend::OptiScaler);
        }
        assert_eq!(Backend::from_id("dlss5"), None);
        assert!(Backend::AmdNeuralExternal.capabilities().neural_rendering);
        assert!(!Backend::OptiScaler.capabilities().neural_rendering);
    }
}
