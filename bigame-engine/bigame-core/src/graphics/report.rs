//! Everything known about one game and this machine that bears on AI
//! Graphics, with how sure each piece is.
//!
//! A value that was only inferred is never presented as a fact: the API of
//! a game that picks its renderer at run time is "unknown" until the game is
//! running, and when a guess has to be made it is labelled as one.

use std::path::{Path, PathBuf};

use serde::Serialize;

use super::manifest::Manifest;
use super::optiscaler::Api;
use super::pe::Machine;
use super::scan::{AntiCheat, ComponentKind, GameScan, Proxy};
use super::text::{N_, Text};
use crate::hardware::{GpuVendor, Hardware};
use crate::running::{GameIdentity, Graphics, Runtime};

/// How a value was established.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Confidence {
    /// Observed directly (the running process, a file's own header).
    Fact,
    /// Read from the game's files (imports, runtimes it ships).
    Detected,
    /// Consistent with what was found, but not shown by it.
    Likely,
    /// A working assumption; nothing supports or contradicts it.
    Assumed,
}

/// The game's graphics API, with how it was found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ApiEvidence {
    /// The API, when anything points to one.
    pub api: Option<Api>,
    /// How sure.
    pub confidence: Confidence,
    /// What pointed there, in words.
    pub evidence: Vec<Text>,
    /// The translation layer seen in the running game (`VKD3D-Proton`, `DXVK`).
    pub translation: Option<&'static str>,
}

/// One GPU, as the report shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GpuInfo {
    /// DRM card (`card1`).
    pub card: String,
    /// Vendor.
    pub vendor: GpuVendor,
    /// Model name from the PCI ID database, or the PCI ID.
    pub name: String,
    /// Kernel driver (`amdgpu`, `nvidia`, …).
    pub driver: String,
    /// Userspace driver and version (`Mesa 26.2.2`, `NVIDIA 580.1`).
    pub userspace: Option<String>,
    /// VRAM in bytes.
    pub vram: Option<u64>,
    /// Discrete (not integrated).
    pub discrete: bool,
    /// AMD RDNA generation (4 = RX 9000), when known.
    pub rdna: Option<u8>,
    /// The running game renders on this card (from its open render node).
    pub renders_game: bool,
}

impl GpuInfo {
    /// Whether FSR 4 can run here under Proton. VKD3D-Proton exposes it only
    /// with native FP8 (`VK_KHR_shader_float8`), which RADV has on RDNA 4.
    #[must_use]
    pub fn fsr4(&self) -> bool {
        self.vendor == GpuVendor::Amd && self.rdna == Some(4)
    }

    /// Whether DLSS Super Resolution runs on this GPU: an NVIDIA RTX card
    /// (tensor cores, Turing or later). `None` when the model does not say.
    #[must_use]
    pub fn dlss(&self) -> Option<bool> {
        (self.vendor == GpuVendor::Nvidia)
            .then(|| nvidia_dlss(&self.name).0)
            .unwrap_or(Some(false))
    }

    /// Whether DLSS Frame Generation runs on this GPU (RTX 40 and later —
    /// Ada and Blackwell). `None` when the model does not say.
    #[must_use]
    pub fn dlss_fg(&self) -> Option<bool> {
        (self.vendor == GpuVendor::Nvidia)
            .then(|| nvidia_dlss(&self.name).1)
            .unwrap_or(Some(false))
    }
}

/// What an NVIDIA model can run: (DLSS Super Resolution, DLSS Frame
/// Generation), from its PCI database name — `GP107M [GeForce GTX 1050 Ti
/// Mobile]`, `AD104 [GeForce RTX 4070]`, `TU102GL [Quadro RTX 6000/8000]`.
///
/// DLSS needs tensor cores: every RTX-branded card has them, no GTX, GT, MX
/// or pre-Turing Quadro does, and the GTX 16 series (TU116/TU117) is Turing
/// without them. Frame generation needs Ada's optical-flow hardware or later
/// (AD1xx, GB2xx). A name that fits none of this is unknown, not "yes".
#[must_use]
pub fn nvidia_dlss(name: &str) -> (Option<bool>, Option<bool>) {
    let chip = name
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    let upper = name.to_ascii_uppercase();
    let older_chip = ["GP", "GM", "GK", "GF", "GV"]
        .iter()
        .any(|p| chip.starts_with(p))
        || chip.starts_with("TU116")
        || chip.starts_with("TU117");
    let non_rtx_brand = [
        "GTX", "GEFORCE GT ", "GEFORCE MX", "QUADRO P", "QUADRO M", "QUADRO K", "TITAN X",
        "TITAN V",
    ]
    .iter()
    .any(|b| upper.contains(b));
    let sr = if upper.contains("RTX") {
        Some(true)
    } else if older_chip || non_rtx_brand {
        Some(false)
    } else {
        None
    };
    // A GeForce RTX 40xx/50xx or an "… Ada" workstation card, when the name
    // carries no chip code. ("RTX 4000" alone is also a Turing Quadro.)
    let geforce_40_50 = upper.match_indices("RTX ").any(|(i, m)| {
        let model: String = upper[i + m.len()..]
            .chars()
            .take_while(char::is_ascii_digit)
            .collect();
        model.len() == 4 && (model.starts_with("40") || model.starts_with("50"))
            && upper.contains("GEFORCE")
    });
    let fg = match sr {
        Some(false) => Some(false),
        _ if chip.starts_with("AD") || chip.starts_with("GB") => Some(true),
        _ if chip.starts_with("TU") || chip.starts_with("GA") => Some(false),
        _ if geforce_40_50 || upper.contains(" ADA") => Some(true),
        _ => None,
    };
    (sr, fg)
}

/// The upscalers and frame generators the game ships.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Native {
    /// DLSS Super Resolution, with its version.
    pub dlss: Option<String>,
    /// DLSS Frame Generation.
    pub dlss_fg: Option<String>,
    /// DLSS Ray Reconstruction.
    pub dlss_rr: Option<String>,
    /// Streamline.
    pub streamline: Option<String>,
    /// `XeSS`.
    pub xess: Option<String>,
    /// `XeSS` Frame Generation.
    pub xess_fg: Option<String>,
    /// FSR / `FidelityFX` (version when the DLL has one).
    pub fsr: Option<String>,
    /// FSR frame generation (`ffx_frameinterpolation`,
    /// `amd_fidelityfx_framegeneration`).
    pub fsr_fg: bool,
}

impl Native {
    /// Any frame generation of the game's own.
    #[must_use]
    pub fn frame_gen(&self) -> bool {
        self.dlss_fg.is_some() || self.xess_fg.is_some() || self.fsr_fg
    }
}

/// The report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Report {
    /// Title.
    pub game: String,
    /// Steam app id.
    pub app_id: Option<String>,
    /// Install folder.
    pub install_root: PathBuf,
    /// Executable, relative to the install folder.
    pub executable: Option<PathBuf>,
    /// 32/64-bit.
    pub machine: Option<Machine>,
    /// How it runs (`Proton - Experimental`, native, Wine), when running.
    pub runtime: Option<String>,
    /// The API.
    pub api: ApiEvidence,
    /// What the game ships.
    pub native: Native,
    /// DLLs in proxy slots beside the executable.
    pub proxies: Vec<Proxy>,
    /// Anti-cheat.
    pub anti_cheat: Vec<AntiCheat>,
    /// GPUs.
    pub gpus: Vec<GpuInfo>,
    /// Index into `gpus` of the one the game renders on (or will).
    pub render_gpu: Option<usize>,
    /// What BiGame-mode has placed in this game, if anything.
    pub installed: Option<Manifest>,
    /// The folder scan stopped at its limit.
    pub scan_truncated: bool,
    /// The game's entry in the game list, if it has one.
    pub listed: Option<super::gamedb::Entry>,
}

impl Report {
    /// Take in the game's entry in the game list. Its API fills in only
    /// where detection is weaker than reading the game's files: what the
    /// running game shows, or its files say, is never replaced.
    pub fn with_listing(mut self, entry: Option<super::gamedb::Entry>) -> Self {
        if let Some(e) = &entry {
            if let Some(api) = e.api {
                if self.api.confidence > Confidence::Detected {
                    self.api.api = Some(api);
                    self.api.confidence = Confidence::Detected;
                    self.api.evidence.push(match e.origin {
                        super::gamedb::Origin::Carried => Text::plain(N_(
                            "BiGame-mode's game list names the API this game renders with by default",
                        )),
                        super::gamedb::Origin::User => Text::plain(N_(
                            "your game list names the API this game renders with",
                        )),
                    });
                }
            }
        }
        self.listed = entry;
        self
    }
}

impl Report {
    /// The GPU the game renders on.
    #[must_use]
    pub fn gpu(&self) -> Option<&GpuInfo> {
        self.render_gpu.and_then(|i| self.gpus.get(i))
    }
}

/// Decide the API from the running process, the executable's imports, and
/// the renderer-specific DLLs the game ships.
///
/// One linear ladder of evidence, strongest first; splitting it would hide
/// the order, which is the point.
#[must_use]
#[allow(clippy::too_many_lines)]
pub fn api_evidence(scan: &GameScan, running: Option<Graphics>) -> ApiEvidence {
    let mut evidence = Vec::new();
    let translation = match running {
        Some(Graphics::Vkd3dProton) => Some("VKD3D-Proton"),
        Some(Graphics::Dxvk) => Some("DXVK"),
        _ => None,
    };
    match running {
        Some(Graphics::Vkd3dProton) => {
            evidence.push(Text::plain(N_(
                "the running game has VKD3D-Proton (Direct3D 12) loaded",
            )));
            return ApiEvidence {
                api: Some(Api::Dx12),
                confidence: Confidence::Fact,
                evidence,
                translation,
            };
        }
        Some(Graphics::Vulkan) => {
            evidence.push(Text::plain(N_(
                "the running game renders with Vulkan directly",
            )));
            return ApiEvidence {
                api: Some(Api::Vulkan),
                confidence: Confidence::Fact,
                evidence,
                translation,
            };
        }
        Some(Graphics::Dxvk) => {
            // DXVK covers D3D8 to D3D11; only D3D11 matters for upscalers.
            evidence.push(Text::plain(N_(
                "the running game has DXVK (Direct3D 11 or older) loaded",
            )));
        }
        _ => {}
    }
    let links = |dll: &str| scan.executable_pe.as_ref().is_some_and(|p| p.links(dll));
    if links("d3d12.dll") {
        evidence.push(Text::with(N_("the executable links %s"), ["d3d12.dll"]));
        return ApiEvidence {
            api: Some(Api::Dx12),
            confidence: Confidence::Detected,
            evidence,
            translation,
        };
    }
    if links("vulkan-1.dll") {
        evidence.push(Text::with(N_("the executable links %s"), ["vulkan-1.dll"]));
        return ApiEvidence {
            api: Some(Api::Vulkan),
            confidence: Confidence::Detected,
            evidence,
            translation,
        };
    }
    if links("d3d11.dll") || running == Some(Graphics::Dxvk) {
        if links("d3d11.dll") {
            evidence.push(Text::with(N_("the executable links %s"), ["d3d11.dll"]));
        }
        return ApiEvidence {
            api: Some(Api::Dx11),
            confidence: if running == Some(Graphics::Dxvk) {
                Confidence::Fact
            } else {
                Confidence::Detected
            },
            evidence,
            translation,
        };
    }
    // Games that load their renderer at run time (Shadow of the Tomb Raider
    // links neither D3D DLL) still ship DX12-only runtimes beside it.
    let dx12_libs: Vec<&String> = scan
        .exe_dir_dlls
        .iter()
        .filter(|d| d.contains("d3d12") || d.contains("dx12"))
        .collect();
    if !dx12_libs.is_empty() {
        evidence.push(Text::with(
            N_("the game ships Direct3D 12 libraries (%s); it may also have a DX11 renderer"),
            [dx12_libs
                .iter()
                .take(3)
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join(", ")],
        ));
        return ApiEvidence {
            api: Some(Api::Dx12),
            confidence: Confidence::Likely,
            evidence,
            translation,
        };
    }
    evidence.push(Text::plain(N_(
        "nothing in the game's files names its API; DX12 is assumed until the game is seen running",
    )));
    ApiEvidence {
        api: Some(Api::Dx12),
        confidence: Confidence::Assumed,
        evidence,
        translation,
    }
}

/// Model name for a PCI `vendor:device` from the system's PCI ID database.
#[must_use]
pub fn pci_name(db: &str, pci_id: &str) -> Option<String> {
    let (vendor, device) = pci_id.split_once(':')?;
    let mut in_vendor = false;
    for line in db.lines() {
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        if !line.starts_with('\t') {
            in_vendor = line
                .split_whitespace()
                .next()
                .is_some_and(|v| v.eq_ignore_ascii_case(vendor));
            continue;
        }
        if in_vendor && !line.starts_with("\t\t") {
            let mut parts = line.trim_start().splitn(2, char::is_whitespace);
            if parts.next().is_some_and(|d| d.eq_ignore_ascii_case(device)) {
                return parts.next().map(|n| n.trim().to_owned());
            }
        }
    }
    None
}

/// AMD RDNA generation from a model name (`Navi 44 [Radeon RX 9060 XT]`).
#[must_use]
pub fn rdna_generation(name: &str) -> Option<u8> {
    let navi = name.split("Navi ").nth(1)?;
    match navi.chars().next()? {
        '4' => Some(4),
        '3' => Some(3),
        '2' => Some(2),
        '1' => Some(1),
        _ => None,
    }
}

/// The name the report gives the GPU games render on — the key measurements
/// are recorded under ([`super::outcomes`]).
#[must_use]
pub fn render_gpu_name(hw: &Hardware) -> Option<String> {
    let (gpus, render) = gpu_infos(hw, None);
    render.and_then(|i| gpus.into_iter().nth(i)).map(|g| g.name)
}

fn gpu_infos(hw: &Hardware, render_card: Option<&str>) -> (Vec<GpuInfo>, Option<usize>) {
    let db = std::fs::read_to_string("/usr/share/hwdata/pci.ids").unwrap_or_default();
    let pacman = Path::new("/var/lib/pacman/local");
    let gpus: Vec<GpuInfo> = hw
        .gpus
        .iter()
        .map(|g| {
            let name = pci_name(&db, &g.pci_id).unwrap_or_else(|| g.pci_id.clone());
            let userspace = match g.vendor {
                GpuVendor::Nvidia => std::fs::read_to_string("/proc/driver/nvidia/version")
                    .ok()
                    .and_then(|v| {
                        v.split_whitespace()
                            .find(|w| {
                                w.chars().next().is_some_and(|c| c.is_ascii_digit())
                                    && w.contains('.')
                            })
                            .map(|w| format!("NVIDIA {w}"))
                    }),
                _ => crate::health::package_version(pacman, "mesa").map(|v| {
                    format!(
                        "Mesa {}",
                        v.split('-').next().unwrap_or(&v).trim_start_matches("1:")
                    )
                }),
            };
            GpuInfo {
                card: g.card.clone(),
                vendor: g.vendor,
                rdna: (g.vendor == GpuVendor::Amd)
                    .then(|| rdna_generation(&name))
                    .flatten(),
                name,
                driver: g.driver.clone(),
                userspace,
                vram: g.vram_total_bytes,
                discrete: g.discrete,
                renders_game: render_card.is_some_and(|c| c == g.card),
            }
        })
        .collect();
    // The card the running game has open wins over the one games would pick.
    let render = gpus.iter().position(|g| g.renders_game).or(hw.render_gpu);
    (gpus, render)
}

/// Build the report for a scanned game.
///
/// `running` is the game's identity when it is running now — which turns the
/// API and the render GPU into facts.
#[must_use]
pub fn build(
    name: &str,
    app_id: Option<&str>,
    scan: &GameScan,
    running: Option<&GameIdentity>,
    hw: &Hardware,
    installed: Option<Manifest>,
) -> Report {
    let version = |k: ComponentKind| {
        scan.component(k)
            .map(|c| c.version.clone().unwrap_or_else(|| "present".into()))
    };
    let native = Native {
        dlss: version(ComponentKind::DlssSuperResolution),
        dlss_fg: version(ComponentKind::DlssFrameGeneration),
        dlss_rr: version(ComponentKind::DlssRayReconstruction),
        streamline: version(ComponentKind::Streamline),
        xess: version(ComponentKind::Xess),
        xess_fg: version(ComponentKind::XessFrameGeneration),
        fsr: scan
            .components
            .iter()
            .filter(|c| c.kind == ComponentKind::Fsr)
            .find_map(|c| c.version.clone())
            .or_else(|| scan.has(ComponentKind::Fsr).then(|| "present".into())),
        fsr_fg: scan.components.iter().any(|c| {
            c.kind == ComponentKind::Fsr && {
                let n = c.path.to_string_lossy().to_ascii_lowercase();
                n.contains("frameinterpolation") || n.contains("framegeneration")
            }
        }),
    };
    let (gpus, render_gpu) = gpu_infos(hw, running.and_then(|r| r.render_card.as_deref()));
    Report {
        game: name.to_owned(),
        app_id: app_id.map(str::to_owned),
        install_root: scan.root.clone(),
        executable: scan.executable.clone(),
        machine: scan.executable_pe.as_ref().and_then(|p| p.machine),
        runtime: running.map(|r| match &r.runtime {
            Runtime::Native => "native".to_owned(),
            Runtime::Proton(tool) => tool.clone(),
            Runtime::Wine => "Wine".to_owned(),
        }),
        api: api_evidence(scan, running.map(|r| r.graphics)),
        native,
        proxies: scan.proxies.clone(),
        anti_cheat: scan.anti_cheat.clone(),
        gpus,
        render_gpu,
        installed,
        listed: None,
        scan_truncated: scan.truncated,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graphics::pe::PeInfo;

    fn scan_with(imports: &[&str], dlls: &[&str]) -> GameScan {
        GameScan {
            executable: Some("Game.exe".into()),
            executable_pe: Some(PeInfo {
                machine: Some(Machine::X64),
                imports: imports.iter().map(|s| (*s).to_owned()).collect(),
                delay_imports: vec![],
            }),
            exe_dir_dlls: dlls.iter().map(|s| (*s).to_owned()).collect(),
            ..GameScan::default()
        }
    }

    #[test]
    fn the_running_game_makes_the_api_a_fact() {
        let e = api_evidence(&scan_with(&[], &[]), Some(Graphics::Vkd3dProton));
        assert_eq!(
            (e.api, e.confidence, e.translation),
            (Some(Api::Dx12), Confidence::Fact, Some("VKD3D-Proton"))
        );
    }

    #[test]
    fn imports_are_detected_not_facts() {
        let e = api_evidence(&scan_with(&["kernel32.dll", "d3d11.dll"], &[]), None);
        assert_eq!(
            (e.api, e.confidence),
            (Some(Api::Dx11), Confidence::Detected)
        );
    }

    #[test]
    fn a_game_that_picks_its_renderer_at_run_time_is_only_likely_dx12() {
        // Shadow of the Tomb Raider: no D3D import, DX12 libraries beside it.
        let e = api_evidence(
            &scan_with(
                &["kernel32.dll", "libxess.dll"],
                &["gfsdk_ssao_d3d12.win64.dll", "libxess.dll"],
            ),
            None,
        );
        assert_eq!((e.api, e.confidence), (Some(Api::Dx12), Confidence::Likely));
        assert!(e.evidence[0].english().contains("gfsdk_ssao_d3d12"));
    }

    #[test]
    fn with_no_evidence_the_assumption_says_it_is_one() {
        let e = api_evidence(&scan_with(&["kernel32.dll"], &[]), None);
        assert_eq!(e.confidence, Confidence::Assumed);
        assert!(e.evidence[0].english().contains("assumed"));
    }

    #[test]
    fn gpu_names_come_from_the_pci_database_and_give_the_rdna_generation() {
        let db = "# comment\n1002  Advanced Micro Devices, Inc. [AMD/ATI]\n\t1638  Cezanne [Radeon Vega Series / Radeon Vega Mobile Series]\n\t7590  Navi 44 [Radeon RX 9060 XT]\n\t\t1002 0001  some subsystem\n10de  NVIDIA Corporation\n\t7590  not this one\n";
        let n = pci_name(db, "1002:7590").unwrap();
        assert_eq!(n, "Navi 44 [Radeon RX 9060 XT]");
        assert_eq!(rdna_generation(&n), Some(4));
        assert_eq!(
            rdna_generation(&pci_name(db, "1002:1638").unwrap()),
            None,
            "Vega is not RDNA"
        );
        assert_eq!(pci_name(db, "10de:7590").as_deref(), Some("not this one"));
        assert_eq!(pci_name(db, "8086:1234"), None);
        assert_eq!(rdna_generation("Navi 31 [Radeon RX 7900 XTX]"), Some(3));
    }

    #[test]
    fn dlss_needs_an_rtx_card_and_frame_generation_needs_ada_or_later() {
        // Names exactly as /usr/share/hwdata/pci.ids has them.
        for (name, sr, fg) in [
            ("GP107M [GeForce GTX 1050 Ti Mobile]", Some(false), Some(false)),
            ("GP104 [GeForce GTX 1080]", Some(false), Some(false)),
            ("GP108 [GeForce GT 1030]", Some(false), Some(false)),
            ("TU117 [GeForce GTX 1650]", Some(false), Some(false)),
            ("TU117M [GeForce GTX 1650 Ti Mobile]", Some(false), Some(false)),
            ("TU106 [GeForce RTX 2060 Rev. A]", Some(true), Some(false)),
            ("TU102GL [Quadro RTX 6000/8000]", Some(true), Some(false)),
            ("GA102 [GeForce RTX 3090]", Some(true), Some(false)),
            ("GA106M [GeForce RTX 3060 Mobile / Max-Q]", Some(true), Some(false)),
            ("GA102GL [RTX A6000]", Some(true), Some(false)),
            ("AD102 [GeForce RTX 4090]", Some(true), Some(true)),
            ("AD104 [GeForce RTX 4070 Ti]", Some(true), Some(true)),
            ("AD104GL [RTX 4000 SFF Ada Generation]", Some(true), Some(true)),
            ("GB202 [GeForce RTX 5090]", Some(true), Some(true)),
            ("GB206 [GeForce RTX 5060 Ti]", Some(true), Some(true)),
            // No chip code: the brand alone.
            ("NVIDIA GeForce RTX 4070", Some(true), Some(true)),
            ("NVIDIA GeForce RTX 2080", Some(true), None),
            ("NVIDIA GeForce GTX 1050 Ti", Some(false), Some(false)),
            // Nothing to go on: unknown, never "yes".
            ("10de:9999", None, None),
        ] {
            assert_eq!(nvidia_dlss(name), (sr, fg), "{name}");
        }
    }

    #[test]
    fn a_listed_api_fills_in_uncertainty_but_never_replaces_what_was_seen() {
        let entry = crate::graphics::gamedb::GameDb::from_texts(None)
            .lookup(Some("750920"), "SOTTR.exe")
            .cloned();
        let base = |api, confidence| Report {
            game: "g".into(),
            app_id: Some("750920".into()),
            install_root: "/g".into(),
            executable: None,
            machine: None,
            runtime: None,
            api: ApiEvidence {
                api,
                confidence,
                evidence: vec![],
                translation: None,
            },
            native: Native::default(),
            proxies: vec![],
            anti_cheat: vec![],
            gpus: vec![],
            render_gpu: None,
            installed: None,
            scan_truncated: false,
            listed: None,
        };
        // SotTR from its files alone is only "likely DX12".
        let r = base(Some(Api::Dx12), Confidence::Likely).with_listing(entry.clone());
        assert_eq!((r.api.api, r.api.confidence), (Some(Api::Dx12), Confidence::Detected));
        assert_eq!(r.api.evidence.len(), 1);
        // Seen running with DXVK (the DX11 renderer): that stays.
        let r = base(Some(Api::Dx11), Confidence::Fact).with_listing(entry);
        assert_eq!((r.api.api, r.api.confidence), (Some(Api::Dx11), Confidence::Fact));
        assert!(r.listed.is_some());
    }

    #[test]
    fn only_nvidia_cards_run_dlss() {
        let g = |vendor, name: &str| GpuInfo {
            card: "card0".into(),
            vendor,
            name: name.into(),
            driver: String::new(),
            userspace: None,
            vram: None,
            discrete: true,
            rdna: None,
            renders_game: false,
        };
        assert_eq!(g(GpuVendor::Amd, "Navi 44 [Radeon RX 9060 XT]").dlss(), Some(false));
        assert_eq!(g(GpuVendor::Intel, "DG2 [Arc A770]").dlss_fg(), Some(false));
        assert_eq!(
            g(GpuVendor::Nvidia, "GP107M [GeForce GTX 1050 Ti Mobile]").dlss(),
            Some(false)
        );
        assert_eq!(g(GpuVendor::Nvidia, "AD102 [GeForce RTX 4090]").dlss_fg(), Some(true));
    }
}
