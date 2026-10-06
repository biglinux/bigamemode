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
use super::scan::{AntiCheat, BuiltIn, ComponentKind, Engine, GameScan, Proxy};
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
    /// The name it is sold under, with its maker (`NVIDIA GeForce RTX 4060
    /// Laptop GPU`, `AMD Radeon RX 9060 XT`, `Intel UHD Graphics 630`):
    /// [`product_name`]. `None` when nothing names more than the maker.
    pub product: Option<String>,
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
    /// The family the card belongs to: from its RDNA generation when that
    /// is known (the GPU says it, [`rdna_of`]), from its names otherwise —
    /// the PCI database's first, then the driver's own (`product`), which
    /// knows a card newer than the database.
    #[must_use]
    pub fn family(&self) -> Family {
        if let (GpuVendor::Amd, Some(g)) = (self.vendor, self.rdna) {
            return Family::Rdna(g);
        }
        match family(self.vendor, &self.name) {
            Family::Unknown => self
                .product
                .as_deref()
                .map_or(Family::Unknown, |p| family(self.vendor, p)),
            known => known,
        }
    }

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
        if self.vendor == GpuVendor::Nvidia {
            nvidia_dlss(&self.name).or_else(|| self.product.as_deref().and_then(nvidia_dlss))
        } else {
            Some(false)
        }
    }
}

/// A GPU family, where a family decides what runs: the granularity AI
/// Graphics reasons at, and no finer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Family {
    /// AMD RDNA, by generation (1–4).
    Rdna(u8),
    /// AMD before RDNA (GCN, Vega).
    AmdOlder,
    /// NVIDIA `GeForce` GTX / GT / MX, pre-RTX or without tensor cores.
    Gtx,
    /// NVIDIA RTX 20 (Turing).
    Rtx20,
    /// NVIDIA RTX 30 (Ampere).
    Rtx30,
    /// NVIDIA RTX 40 (Ada).
    Rtx40,
    /// NVIDIA RTX 50 (Blackwell).
    Rtx50,
    /// Intel Arc (Alchemist, Battlemage), with `XeSS` on XMX units.
    Arc,
    /// Intel integrated graphics.
    IntelIntegrated,
    /// Not known from the name.
    Unknown,
}

impl Family {
    /// A short name for the UI: a product name as it is, or words to
    /// translate.
    #[must_use]
    pub fn label(self) -> Text {
        match self {
            Self::Rdna(g) => Text::raw(format!("RDNA {g}")),
            Self::AmdOlder => Text::plain(N_("AMD (before RDNA)")),
            Self::Gtx => Text::raw("GeForce GTX"),
            Self::Rtx20 => Text::raw("GeForce RTX 20"),
            Self::Rtx30 => Text::raw("GeForce RTX 30"),
            Self::Rtx40 => Text::raw("GeForce RTX 40"),
            Self::Rtx50 => Text::raw("GeForce RTX 50"),
            Self::Arc => Text::raw("Intel Arc"),
            Self::IntelIntegrated => Text::plain(N_("Intel integrated")),
            Self::Unknown => Text::plain(N_("unknown")),
        }
    }
}

/// The family of a GPU, from its PCI database name and vendor.
#[must_use]
pub fn family(vendor: GpuVendor, name: &str) -> Family {
    let upper = name.to_ascii_uppercase();
    let chip = upper.split_whitespace().next().unwrap_or_default();
    match vendor {
        GpuVendor::Amd => match rdna_generation(name) {
            Some(g) => Family::Rdna(g),
            None if upper.contains("VEGA")
                || upper.contains("POLARIS")
                || upper.contains("ELLESMERE")
                || upper.contains("BAFFIN")
                || upper.contains("HAWAII")
                || upper.contains("FIJI")
                || upper.contains("RAVEN")
                || upper.contains("PICASSO")
                || upper.contains("RENOIR")
                || upper.contains("CEZANNE") =>
            {
                Family::AmdOlder
            }
            None => Family::Unknown,
        },
        GpuVendor::Nvidia => match nvidia_dlss(name) {
            Some(false) => Family::Gtx,
            _ if chip.starts_with("GB") => Family::Rtx50,
            _ if chip.starts_with("AD") => Family::Rtx40,
            _ if chip.starts_with("GA") => Family::Rtx30,
            _ if chip.starts_with("TU") => Family::Rtx20,
            _ => {
                // No chip code: the model number.
                let model = upper.match_indices("RTX ").find_map(|(i, m)| {
                    let digits: String = upper[i + m.len()..]
                        .chars()
                        .take_while(char::is_ascii_digit)
                        .collect();
                    (digits.len() == 4).then(|| digits[..2].to_owned())
                });
                match model.as_deref() {
                    Some("20") => Family::Rtx20,
                    Some("30") => Family::Rtx30,
                    Some("40") => Family::Rtx40,
                    Some("50") => Family::Rtx50,
                    _ => Family::Unknown,
                }
            }
        },
        GpuVendor::Intel => {
            if upper.contains("ARC")
                || chip.starts_with("DG2")
                || chip.starts_with("BMG")
                || chip.starts_with("DG1")
            {
                Family::Arc
            } else if upper.contains("GRAPHICS") || upper.contains("XE") {
                Family::IntelIntegrated
            } else {
                Family::Unknown
            }
        }
        GpuVendor::Other => Family::Unknown,
    }
}

/// Whether an NVIDIA model can run DLSS Super Resolution, from its PCI
/// database name — `GP107M [GeForce GTX 1050 Ti
/// Mobile]`, `AD104 [GeForce RTX 4070]`, `TU102GL [Quadro RTX 6000/8000]`.
///
/// DLSS needs tensor cores: every RTX-branded card has them, no GTX, GT, MX
/// or pre-Turing Quadro does, and the GTX 16 series (TU116/TU117) is Turing
/// without them. A name that fits none of this is unknown, not "yes".
#[must_use]
pub fn nvidia_dlss(name: &str) -> Option<bool> {
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
        "GTX",
        "GEFORCE GT ",
        "GEFORCE MX",
        "QUADRO P",
        "QUADRO M",
        "QUADRO K",
        "TITAN X",
        "TITAN V",
    ]
    .iter()
    .any(|b| upper.contains(b));
    if upper.contains("RTX") {
        Some(true)
    } else if older_chip || non_rtx_brand {
        Some(false)
    } else {
        None
    }
}

/// A [`Native`] version when the component is there but its DLL names none;
/// the UI shows it translated.
pub const PRESENT: &str = N_("present");

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
    /// AMD's `FidelityFX` API (`amd_fidelityfx_dx12.dll`), with its version:
    /// the FSR 3.1+ path a provider upgrades to FSR 4.
    #[serde(default)]
    pub ffx_api: Option<String>,
    /// Upscalers compiled into the executable, with no DLL of their own in
    /// the game's folder ([`BuiltIn`]).
    #[serde(default)]
    pub built_in: Vec<BuiltIn>,
}

impl Native {
    /// Any frame generation of the game's own.
    #[must_use]
    pub fn frame_gen(&self) -> bool {
        self.dlss_fg.is_some() || self.xess_fg.is_some() || self.fsr_fg
    }

    /// The FSR compiled into the executable, when the game has no FSR DLL.
    #[must_use]
    pub fn built_in_fsr(&self) -> Option<BuiltIn> {
        if self.fsr.is_some() {
            return None;
        }
        self.built_in
            .iter()
            .copied()
            .find(|b| b.runs_without_a_dll())
    }
}

/// The Proton prefix a Windows game runs in, and what it holds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProtonInfo {
    /// The prefix (`…/compatdata/<appid>/pfx`).
    pub prefix: PathBuf,
    /// Proton ships AMD's FSR 4 provider (`amdxcffx64.dll`) into the prefix's
    /// `system32`; with it, a game's `FidelityFX` API runs FSR 4 on RDNA 4.
    pub fsr4_provider: bool,
    /// The Windows version the prefix reports (`10`, `11`), when readable.
    pub windows_version: Option<String>,
    /// The Proton build the prefix was last run with (`experimental-11.0-…`,
    /// `GE-Proton10-4`), from `compatdata/<id>/version`.
    pub tool: Option<String>,
    /// AMD's Windows HIP runtime (`amdhip64_7.dll`) is in the prefix's
    /// `system32` — what the external neural backend runs its kernels with.
    /// Proton ships none.
    pub hip_runtime: bool,
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
    /// Executable, relative to the install folder: the one that runs.
    pub executable: Option<PathBuf>,
    /// The bootstrap the launcher starts, when the executable was reached
    /// through one (Unreal Engine's `Game.exe`).
    #[serde(default)]
    pub launcher_stub: Option<PathBuf>,
    /// The engine, when the files say.
    #[serde(default)]
    pub engine: Option<Engine>,
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
    /// The Proton prefix, for a Steam game that has one.
    #[serde(default)]
    pub proton: Option<ProtonInfo>,
    /// Every runtime and mod file the scan recognised, without the ones
    /// BiGame-mode added (the report's own view of the game's files).
    #[serde(default)]
    pub components: Vec<super::scan::Component>,
}

impl Report {
    /// Whether the game's own FSR can run FSR 4 here: it ships the
    /// `FidelityFX` API, the GPU is RDNA 4, and the prefix has Proton's
    /// provider. Expected, not proven: the game must also offer FSR 4 in its
    /// menu, and only the running game shows what it loaded.
    #[must_use]
    pub fn native_fsr4_path(&self) -> bool {
        self.native.ffx_api.is_some()
            && self.gpu().is_some_and(GpuInfo::fsr4)
            && self.proton.as_ref().is_some_and(|p| p.fsr4_provider)
    }

    /// Take in the game's entry in the game list. Its API fills in only
    /// where detection is weaker than reading the game's files: what the
    /// running game shows, or its files say, is never replaced.
    #[must_use]
    pub fn with_listing(mut self, entry: Option<super::gamedb::Entry>) -> Self {
        if let Some(e) = &entry {
            if let Some(api) = e.api {
                if self.api.confidence > Confidence::Detected {
                    self.api.api = Some(api);
                    self.api.confidence = Confidence::Detected;
                    self.api.evidence.push(match e.origin {
                        super::gamedb::Origin::Carried => Text::plain(N_(
                            "Big Game Mode's game list names the API this game renders with by default",
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
    // VKD3D-Proton left its pipeline cache: the game ran with Direct3D 12
    // here before (a game with a DX11 renderer too may have switched since).
    if scan.vkd3d_cache {
        evidence.push(Text::plain(N_(
            "VKD3D-Proton's cache (vkd3d-proton.cache) is in the game's folder: it has run with Direct3D 12 here",
        )));
        return ApiEvidence {
            api: Some(Api::Dx12),
            confidence: Confidence::Detected,
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
    // Direct3D 12 games are 64-bit: a 32-bit one (Tomb Raider, 2013) renders
    // with D3D11 or older, which the running game confirmed.
    if scan
        .executable_pe
        .as_ref()
        .is_some_and(|p| p.machine == Some(super::pe::Machine::X86))
    {
        evidence.push(Text::plain(N_(
            "the executable is 32-bit, and Direct3D 12 games are 64-bit: DX11 or older is assumed until the game is seen running",
        )));
        return ApiEvidence {
            api: Some(Api::Dx11),
            confidence: Confidence::Assumed,
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

/// The device name `/usr/share/hwdata/pci.ids` gives `pci_id`. The 1.6 MB
/// database is read once per device and the answer kept for the process:
/// the same GPUs are named on every page.
#[must_use]
pub fn device_name(pci_id: &str) -> Option<String> {
    static NAMES: std::sync::Mutex<Option<std::collections::HashMap<String, Option<String>>>> =
        std::sync::Mutex::new(None);
    let mut names = NAMES
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    names
        .get_or_insert_with(std::collections::HashMap::new)
        .entry(pci_id.to_owned())
        .or_insert_with(|| {
            let db = std::fs::read_to_string("/usr/share/hwdata/pci.ids").unwrap_or_default();
            pci_name(&db, pci_id)
        })
        .clone()
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

/// The name people know a GPU by, from its PCI database name: the product in
/// brackets without the chip code (`Navi 44 [Radeon RX 9060 XT]` → `Radeon RX
/// 9060 XT`). Where the database lists a family of products for one chip, as
/// it does for APUs (`Cezanne [Radeon Vega Series / Radeon Vega Mobile
/// Series]`), the first one without "Series", with the chip to tell it apart:
/// `Radeon Vega (Cezanne)`. Names without brackets are kept as they are.
///
/// For display only: measurements are keyed on the database name.
#[must_use]
pub fn display_name(pci_name: &str) -> String {
    let (Some(a), Some(b)) = (pci_name.find('['), pci_name.rfind(']')) else {
        return pci_name.to_owned();
    };
    if b <= a + 1 {
        return pci_name.to_owned();
    }
    let product = pci_name[a + 1..b].trim();
    let chip = pci_name[..a].trim();
    if !product.contains(" / ") {
        return product.to_owned();
    }
    let first = product.split(" / ").next().unwrap_or(product).trim();
    let first = first.strip_suffix(" Series").unwrap_or(first);
    if chip.is_empty() {
        first.to_owned()
    } else {
        format!("{first} ({chip})")
    }
}

/// The name a GPU is sold under, with its maker.
///
/// NVIDIA's driver names its GPUs itself, as the box does, in
/// `/proc/driver/nvidia/gpus/<slot>/information` (`NVIDIA GeForce RTX 4060
/// Laptop GPU`): read without running `nvidia-smi`, which may be missing or
/// hang with the GPU asleep. Otherwise — nouveau, amdgpu, i915, xe, or an
/// NVIDIA driver too old to say — the PCI database's product
/// ([`display_name`]; for NVIDIA the product alone, without the chip),
/// led by the maker. `None` when the database does not know the device
/// either (a `hwdata` older than the card).
#[must_use]
pub fn product_name(vendor: GpuVendor, pci_slot: &str, pci_name: Option<&str>) -> Option<String> {
    if vendor == GpuVendor::Nvidia && !pci_slot.is_empty() {
        let info =
            std::fs::read_to_string(format!("/proc/driver/nvidia/gpus/{pci_slot}/information"));
        if let Some(model) = info.ok().as_deref().and_then(nvidia_model) {
            return Some(with_maker(vendor, &model));
        }
    }
    let pci_name = pci_name?;
    let product = if vendor == GpuVendor::Nvidia {
        nvidia_product(pci_name)
    } else {
        display_name(pci_name)
    };
    Some(with_maker(vendor, &product))
}

/// The `Model:` line of the NVIDIA driver's `information` file.
fn nvidia_model(information: &str) -> Option<String> {
    information.lines().find_map(|l| {
        let model = l.strip_prefix("Model:")?.trim();
        (!model.is_empty()).then(|| model.to_owned())
    })
}

/// An NVIDIA product from its PCI database name: `GA106M [GeForce RTX 3060
/// Mobile / Max-Q]` → `GeForce RTX 3060 Mobile`. The chip code tells NVIDIA
/// products apart no better than their names do.
fn nvidia_product(pci_name: &str) -> String {
    match (pci_name.find('['), pci_name.rfind(']')) {
        (Some(a), Some(b)) if b > a + 1 => pci_name[a + 1..b]
            .split(" / ")
            .next()
            .unwrap_or_default()
            .trim()
            .to_owned(),
        _ => pci_name.to_owned(),
    }
}

/// `product` led by its maker's name, unless it already is.
#[must_use]
pub fn with_maker(vendor: GpuVendor, product: &str) -> String {
    let maker = match vendor {
        GpuVendor::Nvidia => "NVIDIA",
        GpuVendor::Amd => "AMD",
        GpuVendor::Intel => "Intel",
        GpuVendor::Other => return product.to_owned(),
    };
    let lower = product.to_ascii_lowercase();
    if lower.starts_with(&maker.to_ascii_lowercase()) || lower.starts_with("ati ") {
        product.to_owned()
    } else {
        format!("{maker} {product}")
    }
}

/// The RDNA generation of the AMD card `card` (`card1`): from the graphics
/// IP version the GPU reports ([`gc_version`]), and from its PCI database
/// name only when it reports none (an older kernel). The name knows only the dGPUs' `Navi`
/// chips: APUs (Phoenix, Strix, Rembrandt, Van Gogh) and any card newer
/// than the system's `hwdata` have none.
#[must_use]
pub fn rdna_of(card: &str, pci_name: &str) -> Option<u8> {
    match gc_version(&Path::new("/sys/class/drm").join(card).join("device")) {
        Some((major, minor)) => rdna_from_gc(major, minor),
        None => rdna_generation(pci_name),
    }
}

/// The graphics IP version a GPU reports under `device` (a DRM card's
/// `device` folder): `ip_discovery/die/0/GC/0/{major,minor}`.
#[must_use]
pub fn gc_version(device: &Path) -> Option<(u32, u32)> {
    let gc = device.join("ip_discovery/die/0/GC/0");
    let read = |f: &str| -> Option<u32> {
        std::fs::read_to_string(gc.join(f))
            .ok()?
            .trim()
            .parse()
            .ok()
    };
    Some((read("major")?, read("minor")?))
}

/// The RDNA generation of graphics IP `major.minor`: GC 12 is RDNA 4, 11
/// (and 11.5, RDNA 3.5) RDNA 3, 10.3 RDNA 2, 10.1 RDNA 1; 9 and below came
/// before RDNA.
#[must_use]
pub fn rdna_from_gc(major: u32, minor: u32) -> Option<u8> {
    match (major, minor) {
        (12, _) => Some(4),
        (11, _) => Some(3),
        (10, 3..) => Some(2),
        (10, _) => Some(1),
        _ => None,
    }
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

/// The GPU games render on, as the report describes it, and how many GPUs
/// the machine has.
#[must_use]
pub fn render_gpu(hw: &Hardware) -> Option<GpuInfo> {
    let (gpus, render) = gpu_infos(hw, None);
    render.and_then(|i| gpus.into_iter().nth(i))
}

/// Every GPU as the report describes it (name from the PCI database,
/// userspace driver), and which one renders games: `render_card` when a
/// running game has it open, otherwise the expected one.
#[must_use]
pub fn gpu_infos(hw: &Hardware, render_card: Option<&str>) -> (Vec<GpuInfo>, Option<usize>) {
    let pacman = Path::new("/var/lib/pacman/local");
    let gpus: Vec<GpuInfo> = hw
        .gpus
        .iter()
        .map(|g| {
            let pci_name = device_name(&g.pci_id);
            let product = product_name(g.vendor, &g.pci_slot, pci_name.as_deref());
            let name = pci_name.unwrap_or_else(|| g.pci_id.clone());
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
                    .then(|| rdna_of(&g.card, &name))
                    .flatten(),
                name,
                product,
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

/// `scan` without the files BiGame-mode added to the game: an `OptiScaler`
/// install brings AMD's FSR DLLs, and a game does not "ship FSR" because
/// BiGame-mode put them there. A file BiGame-mode *replaced* stays — the
/// game had its own there.
fn without_added(scan: &GameScan, installed: Option<&Manifest>) -> GameScan {
    let mut s = scan.clone();
    if let Some(m) = installed {
        let added: Vec<String> = m
            .entries
            .iter()
            .filter(|e| e.replaced.is_none())
            .map(|e| e.path.to_string_lossy().to_ascii_lowercase())
            .collect();
        s.components
            .retain(|c| !added.contains(&c.path.to_string_lossy().to_ascii_lowercase()));
    }
    s
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
    proton: Option<ProtonInfo>,
) -> Report {
    let mut scan = without_added(scan, installed.as_ref());
    // Only now, without BiGame-mode's own files: FSR DLLs an OptiScaler
    // install brought do not make the game's built-in FSR 2 go away.
    scan.read_built_in();
    let scan = &scan;
    let version = |k: ComponentKind| {
        scan.component(k)
            .map(|c| c.version.clone().unwrap_or_else(|| PRESENT.into()))
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
            .filter(|c| matches!(c.kind, ComponentKind::Fsr | ComponentKind::FfxApi))
            .find_map(|c| c.version.clone())
            .or_else(|| {
                (scan.has(ComponentKind::Fsr) || scan.has(ComponentKind::FfxApi))
                    .then(|| PRESENT.into())
            }),
        ffx_api: version(ComponentKind::FfxApi),
        fsr_fg: scan.components.iter().any(|c| {
            c.kind == ComponentKind::Fsr && {
                let n = c.path.to_string_lossy().to_ascii_lowercase();
                n.contains("frameinterpolation") || n.contains("framegeneration")
            }
        }),
        built_in: scan.built_in_only(),
    };
    let (gpus, render_gpu) = gpu_infos(hw, running.and_then(|r| r.render_card.as_deref()));
    Report {
        game: name.to_owned(),
        app_id: app_id.map(str::to_owned),
        install_root: scan.root.clone(),
        executable: scan.executable.clone(),
        launcher_stub: scan.launcher_stub.clone(),
        engine: scan.engine,
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
        proton,
        components: scan.components.clone(),
    }
}

/// What a Proton prefix holds that bears on AI Graphics.
#[must_use]
pub fn proton_info(prefix: &Path) -> Option<ProtonInfo> {
    // `compatdata/<id>` and `compatdata/<id>/pfx` both name the prefix.
    let pfx = prefix.join("pfx");
    let prefix = if pfx.join("drive_c").is_dir() {
        pfx.as_path()
    } else {
        prefix
    };
    if !prefix.join("drive_c").is_dir() {
        return None;
    }
    let system32 = prefix.join("drive_c/windows/system32");
    let fsr4_provider = system32.join("amdxcffx64.dll").is_file();
    let hip_runtime = system32.join("amdhip64_7.dll").is_file();
    let tool = prefix
        .parent()
        .and_then(|d| std::fs::read_to_string(d.join("version")).ok())
        .map(|v| v.split_whitespace().last().unwrap_or("").to_owned())
        .filter(|v| !v.is_empty());
    // `system.reg` carries the version Wine reports; the key is read, not
    // parsed as a registry, since only one value matters.
    let windows_version = std::fs::read_to_string(prefix.join("system.reg"))
        .ok()
        .and_then(|reg| {
            let after = reg.split(r#""CurrentBuild"=""#).nth(1)?;
            let build: u32 = after.split('"').next()?.parse().ok()?;
            Some(if build >= 22000 { "11" } else { "10" }.to_owned())
        });
    Some(ProtonInfo {
        prefix: prefix.to_path_buf(),
        fsr4_provider,
        windows_version,
        tool,
        hip_runtime,
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn display_names_are_the_products_people_know() {
        assert_eq!(
            display_name("Navi 44 [Radeon RX 9060 XT]"),
            "Radeon RX 9060 XT"
        );
        assert_eq!(
            display_name("Cezanne [Radeon Vega Series / Radeon Vega Mobile Series]"),
            "Radeon Vega (Cezanne)"
        );
        assert_eq!(
            display_name("GP107M [GeForce GTX 1050 Ti Mobile]"),
            "GeForce GTX 1050 Ti Mobile"
        );
        assert_eq!(display_name("HD Graphics 630"), "HD Graphics 630");
        assert_eq!(display_name("1002:7590"), "1002:7590");
        assert_eq!(display_name("Odd []"), "Odd []");
    }

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
    fn a_32_bit_game_is_not_assumed_dx12() {
        let mut scan = scan_with(&["kernel32.dll"], &[]);
        if let Some(pe) = scan.executable_pe.as_mut() {
            pe.machine = Some(Machine::X86);
        }
        let e = api_evidence(&scan, None);
        assert_eq!(
            (e.api, e.confidence),
            (Some(Api::Dx11), Confidence::Assumed)
        );
        assert!(e.evidence[0].english().contains("32-bit"));
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
    fn a_gpu_is_named_as_it_is_sold() {
        // The NVIDIA driver's own name wins; the PCI database is the fallback.
        let info = "Model: \t\t NVIDIA GeForce RTX 4060 Laptop GPU\nIRQ:   \t\t 180\n";
        assert_eq!(
            nvidia_model(info).as_deref(),
            Some("NVIDIA GeForce RTX 4060 Laptop GPU")
        );
        assert_eq!(nvidia_model("Model: \nIRQ: 1\n"), None);
        for (vendor, pci, shown) in [
            (
                GpuVendor::Nvidia,
                "GA106M [GeForce RTX 3060 Mobile / Max-Q]",
                "NVIDIA GeForce RTX 3060 Mobile",
            ),
            (
                GpuVendor::Nvidia,
                "GP104 [GeForce GTX 1070]",
                "NVIDIA GeForce GTX 1070",
            ),
            (
                GpuVendor::Amd,
                "Navi 44 [Radeon RX 9060 XT]",
                "AMD Radeon RX 9060 XT",
            ),
            (
                GpuVendor::Amd,
                "Cezanne [Radeon Vega Series / Radeon Vega Mobile Series]",
                "AMD Radeon Vega (Cezanne)",
            ),
            (
                GpuVendor::Intel,
                "CometLake-S GT2 [UHD Graphics 630]",
                "Intel UHD Graphics 630",
            ),
            (
                GpuVendor::Intel,
                "Alder Lake-P GT2 [Iris Xe Graphics]",
                "Intel Iris Xe Graphics",
            ),
        ] {
            // An empty slot: nothing read from this machine's driver.
            assert_eq!(
                product_name(vendor, "", Some(pci)).as_deref(),
                Some(shown),
                "{pci}"
            );
        }
        assert_eq!(product_name(GpuVendor::Nvidia, "", None), None);
        assert_eq!(
            with_maker(GpuVendor::Amd, "AMD Radeon 780M"),
            "AMD Radeon 780M"
        );
    }

    #[test]
    fn dlss_needs_an_rtx_card_and_frame_generation_needs_ada_or_later() {
        // Names exactly as /usr/share/hwdata/pci.ids has them.
        for (name, sr) in [
            ("GP107M [GeForce GTX 1050 Ti Mobile]", Some(false)),
            ("GP104 [GeForce GTX 1080]", Some(false)),
            ("GP108 [GeForce GT 1030]", Some(false)),
            ("TU117 [GeForce GTX 1650]", Some(false)),
            ("TU117M [GeForce GTX 1650 Ti Mobile]", Some(false)),
            ("TU106 [GeForce RTX 2060 Rev. A]", Some(true)),
            ("TU102GL [Quadro RTX 6000/8000]", Some(true)),
            ("GA102 [GeForce RTX 3090]", Some(true)),
            ("GA106M [GeForce RTX 3060 Mobile / Max-Q]", Some(true)),
            ("GA102GL [RTX A6000]", Some(true)),
            ("AD102 [GeForce RTX 4090]", Some(true)),
            ("AD104 [GeForce RTX 4070 Ti]", Some(true)),
            ("AD104GL [RTX 4000 SFF Ada Generation]", Some(true)),
            ("GB202 [GeForce RTX 5090]", Some(true)),
            ("GB206 [GeForce RTX 5060 Ti]", Some(true)),
            // No chip code: the brand alone.
            ("NVIDIA GeForce RTX 4070", Some(true)),
            ("NVIDIA GeForce RTX 2080", Some(true)),
            ("NVIDIA GeForce GTX 1050 Ti", Some(false)),
            // Nothing to go on: unknown, never "yes".
            ("10de:9999", None),
        ] {
            assert_eq!(nvidia_dlss(name), sr, "{name}");
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
            launcher_stub: None,
            engine: None,
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
            proton: None,
            components: vec![],
        };
        // SotTR from its files alone is only "likely DX12".
        let r = base(Some(Api::Dx12), Confidence::Likely).with_listing(entry.clone());
        assert_eq!(
            (r.api.api, r.api.confidence),
            (Some(Api::Dx12), Confidence::Detected)
        );
        assert_eq!(r.api.evidence.len(), 1);
        // Seen running with DXVK (the DX11 renderer): that stays.
        let r = base(Some(Api::Dx11), Confidence::Fact).with_listing(entry);
        assert_eq!(
            (r.api.api, r.api.confidence),
            (Some(Api::Dx11), Confidence::Fact)
        );
        assert!(r.listed.is_some());
    }

    #[test]
    fn files_bigame_mode_added_are_not_the_games_own_upscalers() {
        use crate::graphics::manifest::{Backup, Entry, FileKind, Source, State};
        use crate::graphics::scan::Component;
        let comp = |kind, path: &str| Component {
            kind,
            path: path.into(),
            version: None,
        };
        let scan = GameScan {
            components: vec![
                comp(ComponentKind::Xess, "libxess.dll"),
                comp(ComponentKind::Fsr, "amd_fidelityfx_dx12.dll"),
                comp(ComponentKind::DlssSuperResolution, "nvngx_dlss.dll"),
            ],
            ..GameScan::default()
        };
        let entry = |path: &str, replaced: bool| Entry {
            path: path.into(),
            sha256: String::new(),
            kind: FileKind::Binary,
            replaced: replaced.then(|| Backup {
                path: "/b".into(),
                sha256: String::new(),
                size: 0,
            }),
        };
        let m = Manifest {
            schema: 1,
            game_key: "steam-750920".into(),
            process: None,
            title: None,
            install_root: "/g".into(),
            source: Source::default(),
            started_at: 0,
            state: State::Installed,
            // FSR added by an OptiScaler install; XeSS replaced by a newer one.
            entries: vec![
                entry("AMD_FidelityFX_DX12.dll", false),
                entry("libxess.dll", true),
            ],
            managed: true,
            settings: Vec::new(),
            created_dirs: vec![],
            generated: vec![],
            previous: None,
        };
        let kinds = |s: &GameScan| s.components.iter().map(|c| c.kind).collect::<Vec<_>>();
        assert_eq!(
            kinds(&without_added(&scan, Some(&m))),
            [ComponentKind::Xess, ComponentKind::DlssSuperResolution]
        );
        assert_eq!(kinds(&without_added(&scan, None)).len(), 3);

        // A built-in FSR 2 stays the game's own after OptiScaler brought
        // AMD's DLLs: they are BiGame-mode's, not the game's.
        let tow = GameScan {
            executable: Some("Game.exe".into()),
            components: vec![comp(ComponentKind::FfxApi, "amd_fidelityfx_dx12.dll")],
            built_in: vec![super::BuiltIn::Fsr2],
            ..GameScan::default()
        };
        assert_eq!(tow.built_in_only(), [], "with the DLLs counted");
        assert_eq!(
            without_added(&tow, Some(&m)).built_in_only(),
            [super::BuiltIn::Fsr2]
        );
        let n = Native {
            built_in: vec![super::BuiltIn::Fsr2],
            ..Native::default()
        };
        assert_eq!(n.built_in_fsr(), Some(super::BuiltIn::Fsr2));
        assert!(n.fsr.is_none());
    }

    #[test]
    fn vkd3d_protons_cache_says_the_game_ran_with_dx12() {
        let mut s = scan_with(&[], &[]);
        s.vkd3d_cache = true;
        let e = api_evidence(&s, None);
        assert_eq!(
            (e.api, e.confidence),
            (Some(Api::Dx12), Confidence::Detected)
        );
        // What the executable links is stronger.
        let mut s = scan_with(&["d3d11.dll"], &[]);
        s.vkd3d_cache = true;
        assert_eq!(api_evidence(&s, None).api, Some(Api::Dx11));
    }

    #[test]
    fn the_rdna_generation_comes_from_the_gpus_graphics_ip() {
        let dir = tempfile::tempdir().unwrap();
        let device = dir.path();
        assert_eq!(gc_version(device), None, "an older kernel says nothing");
        let gc = device.join("ip_discovery/die/0/GC/0");
        std::fs::create_dir_all(&gc).unwrap();
        // As the reference desktop's cards report it: the RX 9060 XT, then
        // the Vega of its Cezanne APU, then APUs pci.ids names no Navi for.
        for (major, minor, want) in [
            ("12", "0", Some(4)),
            ("9", "3", None),
            ("11", "5", Some(3)),
            ("11", "0", Some(3)),
            ("10", "3", Some(2)),
            ("10", "1", Some(1)),
        ] {
            std::fs::write(gc.join("major"), format!("{major}\n")).unwrap();
            std::fs::write(gc.join("minor"), format!("{minor}\n")).unwrap();
            let (ma, mi) = gc_version(device).unwrap();
            assert_eq!(rdna_from_gc(ma, mi), want, "GC {major}.{minor}");
        }
        // Phoenix's name has no Navi in it; its GC 11 makes it RDNA 3.
        assert_eq!(rdna_generation("Phoenix1"), None);
        let apu = GpuInfo {
            card: "card0".into(),
            vendor: GpuVendor::Amd,
            name: "Phoenix1".into(),
            product: None,
            driver: "amdgpu".into(),
            userspace: None,
            vram: None,
            discrete: false,
            rdna: rdna_from_gc(11, 0),
            renders_game: false,
        };
        assert_eq!(apu.family(), Family::Rdna(3));
    }

    #[test]
    fn a_card_newer_than_the_pci_database_is_known_by_the_drivers_name() {
        // hwdata older than the card: only the id. NVIDIA's driver names it.
        let card = GpuInfo {
            card: "card1".into(),
            vendor: GpuVendor::Nvidia,
            name: "10DE:2D83".into(),
            product: Some("NVIDIA GeForce RTX 5050".into()),
            driver: "nvidia".into(),
            userspace: None,
            vram: None,
            discrete: true,
            rdna: None,
            renders_game: false,
        };
        assert_eq!(card.dlss(), Some(true));
        assert_eq!(card.family(), Family::Rtx50);
        // Without a name from the driver it stays unknown, never "yes".
        let bare = GpuInfo {
            product: None,
            ..card
        };
        assert_eq!((bare.dlss(), bare.family()), (None, Family::Unknown));
    }

    #[test]
    fn only_nvidia_cards_run_dlss() {
        let g = |vendor, name: &str| GpuInfo {
            card: "card0".into(),
            vendor,
            name: name.into(),
            product: None,
            driver: String::new(),
            userspace: None,
            vram: None,
            discrete: true,
            rdna: None,
            renders_game: false,
        };
        assert_eq!(
            g(GpuVendor::Amd, "Navi 44 [Radeon RX 9060 XT]").dlss(),
            Some(false)
        );
        assert_eq!(
            g(GpuVendor::Amd, "Navi 44 [Radeon RX 9060 XT]").family(),
            Family::Rdna(4)
        );
        assert_eq!(
            g(GpuVendor::Amd, "Cezanne [Radeon Vega Series]").family(),
            Family::AmdOlder
        );
        assert_eq!(g(GpuVendor::Intel, "DG2 [Arc A770]").family(), Family::Arc);
        assert_eq!(
            g(GpuVendor::Intel, "Alder Lake-P GT2 [Iris Xe Graphics]").family(),
            Family::IntelIntegrated
        );
        for (name, want) in [
            ("GP107M [GeForce GTX 1050 Ti Mobile]", Family::Gtx),
            ("TU116 [GeForce GTX 1660]", Family::Gtx),
            ("TU104 [GeForce RTX 2080]", Family::Rtx20),
            ("GA102 [GeForce RTX 3080]", Family::Rtx30),
            ("AD102 [GeForce RTX 4090]", Family::Rtx40),
            ("GB203 [GeForce RTX 5080]", Family::Rtx50),
            ("GeForce RTX 3060", Family::Rtx30),
        ] {
            assert_eq!(g(GpuVendor::Nvidia, name).family(), want, "{name}");
        }
        assert_eq!(g(GpuVendor::Intel, "DG2 [Arc A770]").dlss(), Some(false));
        assert_eq!(
            g(GpuVendor::Nvidia, "GP107M [GeForce GTX 1050 Ti Mobile]").dlss(),
            Some(false)
        );
        assert_eq!(
            g(GpuVendor::Nvidia, "AD102 [GeForce RTX 4090]").dlss(),
            Some(true)
        );
    }
}
