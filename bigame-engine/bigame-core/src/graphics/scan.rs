//! What is in a game's folder: upscaler and frame-generation runtimes, DLLs
//! sitting in the slots graphics mods use, mod configuration files, and
//! anti-cheat.
//!
//! Evidence comes from file *contents* where it matters. A `dxgi.dll` next to
//! the executable says nothing on its own — it could be `OptiScaler`, `ReShade`,
//! DXVK, Special K or something else — so its owner is read from the file.
//! An `nvngx.dll` is not "DLSS": NVIDIA's runtime is `nvngx_dlss.dll`, and a
//! bare `nvngx.dll` in a game folder is usually `OptiScaler`'s.
//!
//! The walk is bounded (depth and entry count) and never follows symlinks:
//! game folders can hold hundreds of thousands of files, and this runs
//! whenever a profile is created.
//!
//! The executable a launcher records is not always the one that runs: an
//! Unreal Engine game's `Game.exe` is a small bootstrap that starts
//! `<Project>/Binaries/Win64/<Project>-Win64-Shipping.exe`, and that is the
//! process, the folder for DLL slots, and the file that says which
//! upscalers the game has. Some games build an upscaler into that
//! executable instead of shipping its DLL (Unreal's FSR 2 plugin does), and
//! only the executable's contents show it.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use super::pe;

/// How deep below the install folder the walk goes. Unreal Engine games
/// carry their upscalers as plugins, `Game/Plugins/<plugin>/Binaries/
/// ThirdParty/Win64/` (six levels down), and AMD's FSR 4 plugin keeps its
/// runtime deeper still, under `Source/fidelityfx-sdk/Kits/FidelityFX/
/// signedbin/` (eight). Games pack their assets, so even at this depth an
/// install folder is a few hundred entries.
const MAX_DEPTH: usize = 9;
/// How many directory entries the walk looks at before it stops.
const MAX_ENTRIES: usize = 40_000;
/// How much of a proxy DLL is read to tell who made it.
const OWNER_READ_LIMIT: u64 = 48 << 20;

/// A graphics runtime or mod file the scan recognises.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComponentKind {
    /// NVIDIA DLSS Super Resolution runtime, `nvngx_dlss.dll`.
    DlssSuperResolution,
    /// NVIDIA DLSS Frame Generation runtime, `nvngx_dlssg.dll`.
    DlssFrameGeneration,
    /// NVIDIA DLSS Ray Reconstruction runtime, `nvngx_dlssd.dll`.
    DlssRayReconstruction,
    /// NVIDIA Streamline (`sl.interposer.dll`, `sl.common.dll`, …).
    Streamline,
    /// Intel `XeSS` upscaler, `libxess.dll`.
    Xess,
    /// Intel `XeSS` Frame Generation, `libxess_fg.dll`.
    XessFrameGeneration,
    /// Intel Xe Low Latency, `libxell.dll`.
    XeLowLatency,
    /// AMD `FidelityFX` / FSR runtime (`amd_fidelityfx_*.dll`, `ffx_*.dll`).
    Fsr,
    /// AMD's `FidelityFX` API (`amd_fidelityfx_dx12.dll`, `amd_fidelityfx_vk.dll`,
    /// or the loader newer SDKs ship, `amd_fidelityfx_loader_dx12.dll`): the
    /// FSR 3.1+ entry point a driver provider can take over — FSR 4 on RDNA 4,
    /// through the provider Proton ships.
    FfxApi,
    /// `ReShade` configuration, `ReShade.ini`.
    ReShadeConfig,
    /// `OptiScaler` configuration, `OptiScaler.ini`.
    OptiScalerConfig,
    /// A bare `nvngx.dll`: not NVIDIA's DLSS runtime; usually `OptiScaler`.
    NvngxShim,
    /// NVIDIA's DLSS neural-rendering model, `nvngx_dlssnr.dll` — the input
    /// the external AMD neural backend needs. Detected, never fetched.
    DlssNeuralRendering,
    /// DLSS-NR-on-AMD's configuration, `dlssnr_on_amd.ini`.
    DlssNrOnAmdConfig,
    /// DLSS-NR-on-AMD's converted weights, `dlssnr_on_amd_weights.bin`.
    DlssNrOnAmdWeights,
}

/// A recognised file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Component {
    /// What it is.
    pub kind: ComponentKind,
    /// Path relative to the install folder.
    pub path: PathBuf,
    /// File version from its version resource, when it has one.
    pub version: Option<String>,
}

/// Who a DLL in a proxy slot belongs to, as far as its contents say.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProxyOwner {
    /// `OptiScaler`.
    OptiScaler,
    /// `ReShade` (crosire).
    ReShade,
    /// Special K.
    SpecialK,
    /// DXVK, placed in the game folder rather than the prefix.
    Dxvk,
    /// dgVoodoo 2.
    DgVoodoo,
    /// Ultimate ASI Loader.
    AsiLoader,
    /// DLSS-NR-on-AMD (danielblnc), the external neural-rendering proxy.
    DlssNrOnAmd,
    /// A Microsoft system DLL shipped with the game (redistributable copies of
    /// `dbghelp.dll`, D3D runtimes and the like).
    Microsoft,
    /// Could not be told.
    Unknown,
}

impl ProxyOwner {
    /// A name for the UI; the two that are not product names are marked for
    /// translation.
    #[must_use]
    pub fn label(&self) -> &'static str {
        match self {
            Self::OptiScaler => "OptiScaler",
            Self::ReShade => "ReShade",
            Self::SpecialK => "Special K",
            Self::Dxvk => "DXVK",
            Self::DgVoodoo => "dgVoodoo 2",
            Self::AsiLoader => "Ultimate ASI Loader",
            Self::DlssNrOnAmd => "DLSS-NR-on-AMD",
            Self::Microsoft => super::text::N_("Microsoft system DLL"),
            Self::Unknown => super::text::N_("unknown"),
        }
    }
}

/// A DLL in one of the slots graphics mods load through.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Proxy {
    /// Slot name, lowercase (`dxgi.dll`, `winmm.dll`, …).
    pub slot: String,
    /// Path relative to the install folder.
    pub path: PathBuf,
    /// Who it belongs to.
    pub owner: ProxyOwner,
    /// File version, when it has one.
    pub version: Option<String>,
}

/// An anti-cheat system found in the game's files.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AntiCheat {
    /// Human name (`Easy Anti-Cheat`, `BattlEye`, …).
    pub name: String,
    /// The file or folder that gave it away, relative to the install folder.
    pub evidence: PathBuf,
}

/// A game engine, as a hint for where the executable and its DLL slots are.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Engine {
    /// Unreal Engine: the real executable is `*-Win64-Shipping.exe` under
    /// `Binaries/Win64`, which is where DLL slots are.
    Unreal,
    /// Unity: `UnityPlayer.dll` beside the executable.
    Unity,
}

/// An upscaler the game's executable carries in its own code, with no DLL
/// of its own in the game's folder.
///
/// FSR 2 and the FSR 3.0 SDK can be compiled into a game (Unreal Engine's
/// FSR 2 plugin is). DLSS and `XeSS` cannot: they always run from their
/// DLL (`nvngx_dlss.dll`, `libxess.dll`), so for them this means the game
/// has the code that calls the upscaler and its DLL is missing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BuiltIn {
    /// AMD FSR 2 (`ffxFsr2ContextCreate`).
    Fsr2,
    /// AMD FSR 3.0 (`ffxFsr3ContextCreate`, `ffxFsr3UpscalerContextCreate`).
    Fsr3,
    /// NVIDIA's NGX SDK, which loads DLSS (`NVSDK_NGX_D3D12_Init`, …).
    Dlss,
    /// Intel `XeSS` calls (`xessD3D12CreateContext`, …).
    Xess,
}

impl BuiltIn {
    /// Its product name.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Fsr2 => "FSR 2",
            Self::Fsr3 => "FSR 3",
            Self::Dlss => "DLSS",
            Self::Xess => "XeSS",
        }
    }

    /// Whether the executable runs it by itself (FSR); DLSS and `XeSS`
    /// need their DLL.
    #[must_use]
    pub fn runs_without_a_dll(self) -> bool {
        matches!(self, Self::Fsr2 | Self::Fsr3)
    }
}

/// Everything the scan found.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameScan {
    /// Install folder scanned.
    pub root: PathBuf,
    /// The game's executable, relative to `root`, when it was found: the
    /// one that runs, after a bootstrap ([`Self::launcher_stub`]).
    pub executable: Option<PathBuf>,
    /// The bootstrap the launcher starts, when the executable was reached
    /// through one (Unreal Engine's `Game.exe`), relative to `root`.
    #[serde(default)]
    pub launcher_stub: Option<PathBuf>,
    /// Upscalers whose code the executable carries, once
    /// [`GameScan::read_built_in`] has looked. Which of them are really
    /// built in — no DLL of their own in the game's folder — is
    /// [`GameScan::built_in_only`]'s to say: a game that calls FSR 2 from
    /// `ffx_fsr2_api_x64.dll` names the same functions.
    #[serde(default)]
    pub built_in: Vec<BuiltIn>,
    /// What the executable links, when it could be read.
    #[serde(skip)]
    pub executable_pe: Option<pe::PeInfo>,
    /// Recognised runtimes and mod files, anywhere in the tree.
    pub components: Vec<Component>,
    /// DLLs in proxy slots beside the executable.
    pub proxies: Vec<Proxy>,
    /// Anti-cheat found.
    pub anti_cheat: Vec<AntiCheat>,
    /// Engine hint.
    pub engine: Option<Engine>,
    /// Names of the DLLs beside the executable, lowercase — evidence for the
    /// renderers a game ships (`gfsdk_ssao_d3d12.win64.dll`, …).
    pub exe_dir_dlls: Vec<String>,
    /// VKD3D-Proton's pipeline cache (`vkd3d-proton.cache`) is beside the
    /// executable or at the top of the install folder: the game has run
    /// with Direct3D 12 under Proton here.
    #[serde(default)]
    pub vkd3d_cache: bool,
    /// The walk stopped at its entry limit before seeing everything.
    pub truncated: bool,
}

impl GameScan {
    /// The folder DLL slots are relative to: the executable's.
    #[must_use]
    pub fn executable_dir(&self) -> Option<PathBuf> {
        self.executable
            .as_ref()
            .map(|e| self.root.join(e.parent().unwrap_or_else(|| Path::new(""))))
    }

    /// Whether a component of `kind` was found.
    #[must_use]
    pub fn has(&self, kind: ComponentKind) -> bool {
        self.components.iter().any(|c| c.kind == kind)
    }

    /// Look inside the executable for upscalers compiled into it
    /// ([`Self::built_in`]) — only when the game ships no DLL for one of
    /// them, since the answer changes nothing otherwise. A game executable
    /// can be hundreds of megabytes (Black Myth: Wukong's is 700), so this
    /// is not part of [`scan`], which runs whenever a profile is made; the
    /// AI Graphics analysis asks for it, and the answer is kept for as long
    /// as the file is unchanged.
    pub fn read_built_in(&mut self) {
        let every_dll = self.has(ComponentKind::DlssSuperResolution)
            && self.has(ComponentKind::Xess)
            && (self.has(ComponentKind::Fsr) || self.has(ComponentKind::FfxApi));
        if every_dll {
            return;
        }
        if let Some(found) = self
            .executable
            .as_ref()
            .and_then(|e| built_in_in_file(&self.root.join(e)))
        {
            self.built_in = found;
        }
    }

    /// The upscalers in [`Self::built_in`] with no DLL of their own among
    /// [`Self::components`].
    #[must_use]
    pub fn built_in_only(&self) -> Vec<BuiltIn> {
        self.built_in
            .iter()
            .copied()
            .filter(|b| match b {
                BuiltIn::Fsr2 | BuiltIn::Fsr3 => {
                    !self.has(ComponentKind::Fsr) && !self.has(ComponentKind::FfxApi)
                }
                BuiltIn::Dlss => !self.has(ComponentKind::DlssSuperResolution),
                BuiltIn::Xess => !self.has(ComponentKind::Xess),
            })
            .collect()
    }

    /// The first component of `kind`.
    #[must_use]
    pub fn component(&self, kind: ComponentKind) -> Option<&Component> {
        self.components.iter().find(|c| c.kind == kind)
    }
}

/// DLL names graphics mods load through, lowercase. A file with one of these
/// names beside the executable is loaded by the game in place of the system
/// DLL (given the Wine override for it), which is what makes the slot
/// valuable — and contested.
pub const PROXY_SLOTS: &[&str] = &[
    "dxgi.dll",
    "d3d9.dll",
    "d3d10.dll",
    "d3d11.dll",
    "d3d12.dll",
    "winmm.dll",
    "version.dll",
    "dinput8.dll",
    "dbghelp.dll",
    "wininet.dll",
    "winhttp.dll",
    "opengl32.dll",
];

/// Whether `name` ends in `.ext`, ignoring case.
fn has_ext(name: &str, ext: &str) -> bool {
    Path::new(name)
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case(ext))
}

fn component_kind(file_lower: &str) -> Option<ComponentKind> {
    use ComponentKind as K;
    Some(match file_lower {
        "nvngx_dlss.dll" => K::DlssSuperResolution,
        "nvngx_dlssg.dll" => K::DlssFrameGeneration,
        "nvngx_dlssd.dll" => K::DlssRayReconstruction,
        "nvngx.dll" => K::NvngxShim,
        "nvngx_dlssnr.dll" => K::DlssNeuralRendering,
        "dlssnr_on_amd.ini" => K::DlssNrOnAmdConfig,
        "dlssnr_on_amd_weights.bin" => K::DlssNrOnAmdWeights,
        "libxess.dll" => K::Xess,
        "libxess_fg.dll" => K::XessFrameGeneration,
        "libxell.dll" => K::XeLowLatency,
        "reshade.ini" => K::ReShadeConfig,
        "optiscaler.ini" => K::OptiScalerConfig,
        n if n.starts_with("sl.") && has_ext(n, "dll") => K::Streamline,
        "amd_fidelityfx_dx12.dll"
        | "amd_fidelityfx_vk.dll"
        | "amd_fidelityfx_loader_dx12.dll"
        | "amd_fidelityfx_loader_vk.dll" => K::FfxApi,
        n if (n.starts_with("amd_fidelityfx") || n.starts_with("ffx_")) && has_ext(n, "dll") => {
            K::Fsr
        }
        _ => return None,
    })
}

/// Tell who made a DLL from what it contains.
///
/// Order matters: `OptiScaler` builds embed the names of the upscalers they
/// wrap, and `ReShade` add-on DLLs mention `ReShade`, so the most specific
/// markers are tried first.
#[must_use]
pub fn identify_owner(bytes: &[u8]) -> ProxyOwner {
    const MARKERS: &[(&str, ProxyOwner)] = &[
        ("dlssnr_amd", ProxyOwner::DlssNrOnAmd),
        ("DLSS-NR on AMD", ProxyOwner::DlssNrOnAmd),
        ("dlssnr_on_amd", ProxyOwner::DlssNrOnAmd),
        ("OptiScaler", ProxyOwner::OptiScaler),
        ("crosire", ProxyOwner::ReShade),
        ("ReShade", ProxyOwner::ReShade),
        ("SpecialK", ProxyOwner::SpecialK),
        ("Special K", ProxyOwner::SpecialK),
        ("dgVoodoo", ProxyOwner::DgVoodoo),
        ("Ultimate ASI Loader", ProxyOwner::AsiLoader),
        ("DXVK", ProxyOwner::Dxvk),
        ("dxvk", ProxyOwner::Dxvk),
    ];
    for (marker, owner) in MARKERS {
        if pe::contains_marker(bytes, marker) {
            return owner.clone();
        }
    }
    if pe::contains_marker(bytes, "Microsoft Corporation") {
        return ProxyOwner::Microsoft;
    }
    ProxyOwner::Unknown
}

/// Anti-cheat markers in an install folder.
///
/// From the vendors' own layouts and `SteamDB`'s file-detection rules (MIT).
/// Easy Anti-Cheat is flagged by its own folder or launcher only: the Epic
/// Online Services SDK (`EOSSDK-Win64-Shipping.dll`) on its own is not
/// anti-cheat — Shadow of the Tomb Raider ships it. Erring towards a marker
/// costs a disabled injection; erring away costs an account, so doubtful
/// markers (mhyprot's driver names are not confirmed by its vendor) stay in.
fn anti_cheat_marker(name_lower: &str, is_dir: bool) -> Option<&'static str> {
    if is_dir {
        return match name_lower {
            "easyanticheat" | "easyanticheat_eos" => Some("Easy Anti-Cheat"),
            "battleye" => Some("BattlEye"),
            "gameguard" => Some("nProtect GameGuard"),
            "eaanticheat" => Some("EA Javelin Anticheat"),
            "xigncode" | "xigncode3" => Some("XIGNCODE3"),
            "equ8" => Some("EQU8"),
            "anticheatexpert" | "aceantibotclient" => Some("Tencent ACE"),
            "hshield" => Some("AhnLab HackShield"),
            _ => None,
        };
    }
    match name_lower {
        "start_protected_game.exe"
        | "easyanticheat_eos_setup.exe"
        | "easyanticheat_setup.exe"
        | "easyanticheat.dll"
        | "easyanticheat_x64.dll"
        | "easyanticheat_x64.so" => Some("Easy Anti-Cheat"),
        "beservice.exe"
        | "beservice_x64.exe"
        | "install_battleye.bat"
        | "uninstall_battleye.bat"
        | "beclient.dll"
        | "beclient_x64.dll" => Some("BattlEye"),
        n if n.ends_with("_be.exe") => Some("BattlEye"),
        "ggsetup.exe" | "gameguard.des" => Some("nProtect GameGuard"),
        "eaanticheat.installer.exe" => Some("EA Javelin Anticheat"),
        n if n.starts_with("eaanticheat") => Some("EA Javelin Anticheat"),
        n if has_ext(n, "xem") => Some("XIGNCODE3"),
        "equ8_conf.json" => Some("EQU8"),
        "randgrid.sys" => Some("Ricochet"),
        "pnkbstra.exe" | "pbsvc.exe" | "pbsv.dll" => Some("PunkBuster"),
        "neacsafe64.sys" | "nep2.dll" => Some("NetEase anti-cheat"),
        "blackcall.aes" | "blackcall64.aes" | "blackcat64.sys" => Some("Nexon BlackCipher"),
        "hsinst.dll" => Some("AhnLab HackShield"),
        "mhyprot2.sys" | "mhyprot3.sys" | "mhypbase.dll" => Some("mhyprot"),
        "vgc.exe" | "vgk.sys" => Some("Riot Vanguard"),
        _ => None,
    }
}

/// Games whose anti-cheat leaves no marker in the install folder, by
/// executable name (lowercase).
fn known_protected_executable(exe_lower: &str) -> Option<&'static str> {
    match exe_lower {
        "valorant.exe"
        | "valorant-win64-shipping.exe"
        | "leagueclient.exe"
        | "league of legends.exe" => Some("Riot Vanguard"),
        "overwatch.exe" => Some("Blizzard anti-cheat"),
        "wow.exe" | "wowclassic.exe" => Some("Blizzard Warden"),
        "cs2.exe" | "csgo.exe" => Some("Valve Anti-Cheat"),
        "cod.exe" | "cod24-cod.exe" | "blackops6.exe" => Some("Ricochet"),
        _ => None,
    }
}

fn lower_name(p: &Path) -> String {
    p.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
}

/// How deep below the install folder the anti-cheat check looks, and how
/// deep below each folder on the way to the executable: deep enough for
/// an Unreal game's `<Project>/Binaries/Win64/BattlEye/`, from the top.
const ANTI_CHEAT_DEPTH: usize = 4;
const ANTI_CHEAT_DEPTH_NEAR_EXE: usize = 2;

/// Anti-cheat markers by name alone, with no entry limit: in the install
/// folder and a few levels below it, and around every folder on the way to
/// the executable. The walk that reads the rest stops at [`MAX_ENTRIES`],
/// and a game must never look unprotected because it is large. Names only:
/// nothing is read, and links are not followed.
fn anti_cheat_near(root: &Path, exe: Option<&Path>) -> Vec<AntiCheat> {
    let mut starts = vec![(PathBuf::new(), ANTI_CHEAT_DEPTH)];
    let mut on_the_way = PathBuf::new();
    for c in exe
        .and_then(Path::parent)
        .into_iter()
        .flat_map(Path::components)
    {
        on_the_way.push(c);
        starts.push((on_the_way.clone(), ANTI_CHEAT_DEPTH_NEAR_EXE));
    }
    let mut found: Vec<AntiCheat> = Vec::new();
    // Each folder listed once, from wherever it is reached with the most
    // depth left.
    let mut listed: HashMap<PathBuf, usize> = HashMap::new();
    let mut stack = starts;
    while let Some((rel, left)) = stack.pop() {
        if left == 0 || listed.get(&rel).is_some_and(|&l| l >= left) {
            continue;
        }
        listed.insert(rel.clone(), left);
        let Ok(entries) = std::fs::read_dir(root.join(&rel)) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(ft) = entry.file_type() else { continue };
            let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
            let child = rel.join(entry.file_name());
            if let Some(ac) = anti_cheat_marker(&name, ft.is_dir()) {
                if !found.iter().any(|a| a.name == ac) {
                    found.push(AntiCheat {
                        name: ac.to_owned(),
                        evidence: child.clone(),
                    });
                }
            }
            if ft.is_dir() {
                stack.push((child, left - 1));
            }
        }
    }
    found
}

/// Walk the tree once, bounded, collecting every file and directory name.
fn walk(root: &Path, max_entries: usize) -> (Vec<(PathBuf, bool)>, bool) {
    let mut out = Vec::new();
    let mut stack = vec![(root.to_path_buf(), 0usize)];
    let mut seen = 0usize;
    while let Some((dir, depth)) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            seen += 1;
            if seen > max_entries {
                return (out, true);
            }
            // `file_type` does not follow symlinks: a link is neither a file
            // nor a directory here, and is skipped.
            let Ok(ft) = entry.file_type() else { continue };
            let path = entry.path();
            let rel = path.strip_prefix(root).unwrap_or(&path).to_path_buf();
            if ft.is_dir() {
                out.push((rel, true));
                if depth + 1 < MAX_DEPTH {
                    stack.push((path, depth + 1));
                }
            } else if ft.is_file() {
                out.push((rel, false));
            }
        }
    }
    (out, false)
}

/// Pick the game's executable among the `.exe` files found.
///
/// `hint` is the process name the game runs as, when known (from the running
/// process, or the launcher's candidates) — that settles it. Otherwise the
/// Unreal shipping binary, then the largest executable that is not an
/// installer, crash handler, launcher or anti-cheat.
fn choose_executable(
    root: &Path,
    files: &[(PathBuf, bool)],
    hint: Option<&str>,
) -> Option<PathBuf> {
    const NOT_THE_GAME: &[&str] = &[
        "unins",
        "setup",
        "install",
        "redist",
        "vcredist",
        "dxsetup",
        "crash",
        "report",
        "launcher",
        "easyanticheat",
        "battleye",
        "beservice",
        "helper",
        "update",
        "dotnet",
        "ue4prereq",
        "prereq",
        "cefprocess",
        "webhelper",
        "profilefixer",
    ];
    let exes: Vec<&PathBuf> = files
        .iter()
        .filter(|(p, dir)| !dir && has_ext(&lower_name(p), "exe"))
        .map(|(p, _)| p)
        .collect();
    if let Some(hint) = hint.map(str::to_ascii_lowercase) {
        if let Some(e) = exes.iter().find(|e| lower_name(e) == hint) {
            return Some((*e).clone());
        }
    }
    // Unreal's crash reporter is a `-Win64-Shipping.exe` too.
    if let Some(e) = exes.iter().find(|e| {
        let n = lower_name(e);
        n.ends_with("-win64-shipping.exe") && !NOT_THE_GAME.iter().any(|w| n.contains(w))
    }) {
        return Some((*e).clone());
    }
    exes.into_iter()
        .filter(|e| {
            let n = lower_name(e);
            !NOT_THE_GAME.iter().any(|w| n.contains(w))
                && !e.components().any(|c| {
                    let c = c.as_os_str().to_string_lossy().to_ascii_lowercase();
                    c.contains("redist") || c == "_commonredist" || c.contains("anticheat")
                })
        })
        .max_by_key(|e| std::fs::metadata(root.join(e)).map_or(0, |m| m.len()))
        .cloned()
}

/// A bootstrap is small: Unreal's is about 200 KB. Anything larger is a
/// game, and is not read whole to find out.
const BOOTSTRAP_MAX: u64 = 8 << 20;

/// What a file's identity is for the caches below: its path, size and
/// modification time. A game update changes one of them.
type FileKey = (PathBuf, u64, Option<SystemTime>);

fn file_key(path: &Path) -> Option<FileKey> {
    // Not a symlink: the scan never follows one, and neither does this.
    let m = std::fs::symlink_metadata(path).ok()?;
    m.is_file()
        .then(|| (path.to_path_buf(), m.len(), m.modified().ok()))
}

/// `value` for `path`, computed once per version of the file.
fn cached<T: Clone>(
    cache: &Mutex<Option<HashMap<FileKey, T>>>,
    path: &Path,
    compute: impl FnOnce(&FileKey) -> T,
) -> Option<T> {
    let key = file_key(path)?;
    if let Some(v) = cache
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .as_ref()
        .and_then(|m| m.get(&key))
    {
        return Some(v.clone());
    }
    // Computed without the lock: reading a large file must not hold up
    // another thread asking about a different one.
    let value = compute(&key);
    cache
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get_or_insert_with(HashMap::new)
        .insert(key, value.clone());
    Some(value)
}

/// `rel` under `base`, each part matched without regard to case as Windows
/// (and Wine) do, and never through a symlink. `None` when a part is not
/// there, or is `..`, `.`, empty or a drive.
fn resolve_ci(base: &Path, rel: &str) -> Option<PathBuf> {
    let mut at = PathBuf::new();
    for part in rel.split(['\\', '/']) {
        if part.is_empty() || part == "." || part == ".." || part.contains(':') {
            return None;
        }
        let exact = base.join(&at).join(part);
        let found = if std::fs::symlink_metadata(&exact).is_ok() {
            part.to_owned()
        } else {
            std::fs::read_dir(base.join(&at))
                .ok()?
                .flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .find(|n| n.eq_ignore_ascii_case(part))?
        };
        at.push(found);
        if std::fs::symlink_metadata(base.join(&at)).is_ok_and(|m| m.file_type().is_symlink()) {
            return None;
        }
    }
    Some(at)
}

/// The UTF-16 strings in `bytes` that contain `needle` (ASCII), whole: a
/// Windows resource keeps paths as UTF-16, one printable unit after another
/// between two NULs.
fn wide_strings_with(bytes: &[u8], needle: &str) -> Vec<String> {
    let wide: Vec<u8> = needle.encode_utf16().flat_map(u16::to_le_bytes).collect();
    let printable = |b: &[u8]| b.len() == 2 && b[1] == 0 && (0x20..0x7f).contains(&b[0]);
    let mut out = Vec::new();
    for hit in memchr::memmem::find_iter(bytes, &wide) {
        let mut start = hit;
        while start >= 2 && printable(&bytes[start - 2..start]) {
            start -= 2;
        }
        let mut end = hit + wide.len();
        while end + 2 <= bytes.len() && printable(&bytes[end..end + 2]) {
            end += 2;
        }
        let s: String = bytes[start..end]
            .chunks_exact(2)
            .map(|c| char::from(c[0]))
            .collect();
        if !out.contains(&s) {
            out.push(s);
        }
    }
    out
}

/// The executable an Unreal Engine bootstrap at `exe` (relative to `root`)
/// starts, relative to `root`; `None` when `exe` is not one.
///
/// The bootstrap (`BootstrapPackagedGame`) carries the relative path of the
/// game it starts as a UTF-16 resource,
/// `Indiana\Binaries\Win64\IndianaEpicGameStore-Win64-Shipping.exe` in The
/// Outer Worlds: Spacer's Choice Edition. That path is taken only when it
/// names a `-Shipping.exe` under a `Binaries` folder that exists inside the
/// game's folder. A bootstrap whose path cannot be read falls back to the
/// one `<Project>/Binaries/Win64/*-Shipping.exe` beside it.
fn follow_bootstrap(root: &Path, exe: &Path) -> Option<PathBuf> {
    static CACHE: Mutex<Option<HashMap<FileKey, Option<PathBuf>>>> = Mutex::new(None);
    let full = root.join(exe);
    let exe_dir = exe.parent().unwrap_or_else(|| Path::new("")).to_path_buf();
    let base = root.join(&exe_dir);
    cached(&CACHE, &full, |(_, len, _)| {
        if *len > BOOTSTRAP_MAX {
            return None;
        }
        let bytes = pe::read_prefix(&full, BOOTSTRAP_MAX).ok()?;
        let bootstrap = pe::contains_marker(&bytes, "BootstrapPackagedGame");
        // Hogwarts Legacy's bootstrap starts
        // `Phoenix\Binaries\Win64\HogwartsLegacy.exe`: without the
        // bootstrap's own name in the file, only a `-Shipping.exe` is taken.
        let named = wide_strings_with(&bytes, "\\Binaries\\")
            .into_iter()
            .filter(|s| {
                let s = s.to_ascii_lowercase();
                s.ends_with("-shipping.exe") || bootstrap && has_ext(&s, "exe")
            })
            .find_map(|s| resolve_ci(&base, &s));
        if let Some(rel) = named {
            return Some(exe_dir.join(rel));
        }
        if !bootstrap {
            return None;
        }
        let mut found = Vec::new();
        for project in std::fs::read_dir(&base).ok()?.flatten() {
            let name = project.file_name().to_string_lossy().into_owned();
            if name.eq_ignore_ascii_case("Engine") || !project.file_type().is_ok_and(|t| t.is_dir())
            {
                continue;
            }
            let Some(bin) = resolve_ci(&base, &format!("{name}/Binaries/Win64")) else {
                continue;
            };
            for f in std::fs::read_dir(base.join(&bin))
                .into_iter()
                .flatten()
                .flatten()
            {
                let n = f.file_name().to_string_lossy().into_owned();
                if n.to_ascii_lowercase().ends_with("-shipping.exe")
                    && f.file_type().is_ok_and(|t| t.is_file())
                {
                    found.push(bin.join(n));
                }
            }
        }
        (found.len() == 1).then(|| exe_dir.join(found.remove(0)))
    })
    .flatten()
}

/// The process names a game recorded as `process` runs as: `process`
/// itself and, when it is an Unreal Engine bootstrap at the top of `root`,
/// the game it starts. Cheap: one small file is read, once per version.
#[must_use]
pub fn runs_as(root: &Path, process: &str) -> Vec<String> {
    let mut names = vec![process.to_owned()];
    if let Some(real) = resolve_ci(root, process)
        .and_then(|exe| follow_bootstrap(root, &exe))
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
    {
        if !real.eq_ignore_ascii_case(process) {
            names.push(real);
        }
    }
    names
}

/// The markers of an upscaler's code, and what they mean. Function names of
/// each SDK's API, which a game that compiles the SDK in keeps as strings
/// (Unreal's FSR 2 plugin: `ffxFsr2ContextCreate`, `ffxFsr2GetInterfaceDX12`).
const BUILT_IN_MARKERS: &[(&str, BuiltIn)] = &[
    ("ffxFsr2ContextCreate", BuiltIn::Fsr2),
    ("ffxFsr3ContextCreate", BuiltIn::Fsr3),
    ("ffxFsr3UpscalerContextCreate", BuiltIn::Fsr3),
    ("NVSDK_NGX_D3D12_Init", BuiltIn::Dlss),
    ("NVSDK_NGX_D3D11_Init", BuiltIn::Dlss),
    ("NVSDK_NGX_VULKAN_Init", BuiltIn::Dlss),
    ("xessD3D12CreateContext", BuiltIn::Xess),
    ("xessD3D11CreateContext", BuiltIn::Xess),
    ("xessVKCreateContext", BuiltIn::Xess),
];
/// How much of an executable is searched for them. Game executables reach
/// a few hundred megabytes; the markers sit in its read-only data.
const BUILT_IN_READ_LIMIT: u64 = 1 << 30;

/// Which upscalers' code `bytes` carries.
#[must_use]
pub fn built_in_markers(bytes: &[u8]) -> Vec<BuiltIn> {
    let mut found: Vec<BuiltIn> = BUILT_IN_MARKERS
        .iter()
        .filter(|(m, _)| memchr::memmem::find(bytes, m.as_bytes()).is_some())
        .map(|(_, k)| *k)
        .collect();
    found.sort_unstable();
    found.dedup();
    found
}

/// [`built_in_markers`] for a file on disk, read in pieces (an executable
/// can be larger than is worth holding in memory), once per version;
/// `None` when there is no such file.
fn built_in_in_file(path: &Path) -> Option<Vec<BuiltIn>> {
    use std::io::Read as _;
    static CACHE: Mutex<Option<HashMap<FileKey, Vec<BuiltIn>>>> = Mutex::new(None);
    cached(&CACHE, path, |_| {
        const PIECE: usize = 8 << 20;
        let overlap = BUILT_IN_MARKERS
            .iter()
            .map(|(m, _)| m.len())
            .max()
            .unwrap_or(0);
        let Ok(file) = std::fs::File::open(path) else {
            return Vec::new();
        };
        let mut file = file.take(BUILT_IN_READ_LIMIT);
        let mut found = Vec::new();
        let mut buf = Vec::with_capacity(PIECE + overlap);
        loop {
            let kept = buf.len();
            buf.resize(kept + PIECE, 0);
            let n = file.read(&mut buf[kept..]).unwrap_or(0);
            buf.truncate(kept + n);
            found.extend(built_in_markers(&buf));
            if n == 0 {
                break;
            }
            // The end of this piece starts the next, so a marker cut in two
            // is still seen.
            let from = buf.len().saturating_sub(overlap);
            buf.drain(..from);
        }
        found.sort_unstable();
        found.dedup();
        found
    })
}

/// Whether VKD3D-Proton left its cache where it keeps it: the game's
/// working folder, which is the executable's or the install folder.
fn has_vkd3d_cache(files: &[(PathBuf, bool)], exe_dir: Option<&Path>) -> bool {
    files.iter().any(|(rel, is_dir)| {
        let dir = rel.parent().unwrap_or_else(|| Path::new(""));
        !is_dir
            && lower_name(rel) == "vkd3d-proton.cache"
            && (dir.as_os_str().is_empty() || exe_dir == Some(dir))
    })
}

/// The game's executable and, when a bootstrap hands over to it, the
/// bootstrap: the scan goes where the game runs.
fn executable_of(
    root: &Path,
    files: &[(PathBuf, bool)],
    hint: Option<&str>,
) -> (Option<PathBuf>, Option<PathBuf>) {
    let chosen = choose_executable(root, files, hint);
    match chosen.as_ref().and_then(|e| follow_bootstrap(root, e)) {
        Some(real) => (Some(real), chosen),
        None => (chosen, None),
    }
}

/// Scan a game's install folder.
///
/// `exe_hint` is the process name the game runs as, when known.
#[must_use]
pub fn scan(root: &Path, exe_hint: Option<&str>) -> GameScan {
    scan_limited(root, exe_hint, MAX_ENTRIES)
}

// One pass over the walked names, in the order the scan reads them.
#[allow(clippy::too_many_lines)]
fn scan_limited(root: &Path, exe_hint: Option<&str>, max_entries: usize) -> GameScan {
    let (files, truncated) = walk(root, max_entries);
    let (mut executable, mut launcher_stub) = executable_of(root, &files, exe_hint);
    if executable.is_none() && truncated {
        // The walk stopped before the executable: the name the game runs
        // as, at the top of its folder, is looked for directly.
        if let Some(exe) = exe_hint.and_then(|h| resolve_ci(root, h)) {
            let listed = [(exe, false)];
            (executable, launcher_stub) = executable_of(root, &listed, exe_hint);
        }
    }
    let executable_pe = executable
        .as_ref()
        .and_then(|e| pe::parse_file(&root.join(e), 64 << 20).ok());
    let exe_dir = executable
        .as_ref()
        .map(|e| e.parent().unwrap_or_else(|| Path::new("")).to_path_buf());

    let mut components = Vec::new();
    let mut proxies = Vec::new();
    let mut anti_cheat = Vec::new();
    let mut engine = None;
    let mut exe_dir_dlls = Vec::new();
    for (rel, is_dir) in &files {
        let name = lower_name(rel);
        if let Some(ac) = anti_cheat_marker(&name, *is_dir) {
            if !anti_cheat.iter().any(|a: &AntiCheat| a.name == ac) {
                anti_cheat.push(AntiCheat {
                    name: ac.to_owned(),
                    evidence: rel.clone(),
                });
            }
        }
        if *is_dir {
            continue;
        }
        if name == "unityplayer.dll" {
            engine = Some(Engine::Unity);
        }
        if let Some(kind) = component_kind(&name) {
            let version = has_ext(&name, "dll")
                .then(|| pe::read_file_version(&root.join(rel)))
                .flatten();
            components.push(Component {
                kind,
                path: rel.clone(),
                version,
            });
        }
        let beside_exe = exe_dir
            .as_ref()
            .is_some_and(|d| rel.parent().unwrap_or_else(|| Path::new("")) == d.as_path());
        if beside_exe && has_ext(&name, "dll") {
            exe_dir_dlls.push(name.clone());
        }
        if beside_exe && PROXY_SLOTS.contains(&name.as_str()) {
            let full = root.join(rel);
            let block = pe::read_version_block(&full);
            // The version resource names the product in a few hundred bytes;
            // the whole file is searched only when it says nothing useful.
            let owner = match block.as_deref().map(identify_owner) {
                Some(owner) if owner != ProxyOwner::Unknown => owner,
                _ => pe::read_prefix(&full, OWNER_READ_LIMIT)
                    .map_or(ProxyOwner::Unknown, |b| identify_owner(&b)),
            };
            proxies.push(Proxy {
                slot: name.clone(),
                path: rel.clone(),
                owner,
                version: block.as_deref().and_then(pe::file_version),
            });
        }
    }
    // Only Unreal hands over from a bootstrap.
    if launcher_stub.is_some()
        || executable
            .as_ref()
            .is_some_and(|e| lower_name(e).ends_with("-win64-shipping.exe"))
    {
        engine = Some(Engine::Unreal);
    }
    for ac in anti_cheat_near(root, executable.as_deref()) {
        if !anti_cheat.iter().any(|a| a.name == ac.name) {
            anti_cheat.push(ac);
        }
    }
    if let Some(ac) = executable
        .as_ref()
        .and_then(|e| known_protected_executable(&lower_name(e)))
    {
        if !anti_cheat.iter().any(|a| a.name == ac) {
            anti_cheat.push(AntiCheat {
                name: ac.to_owned(),
                evidence: executable.clone().unwrap_or_default(),
            });
        }
    }
    components.sort_by(|a, b| a.kind.cmp(&b.kind).then_with(|| a.path.cmp(&b.path)));
    proxies.sort_by(|a, b| a.slot.cmp(&b.slot));
    GameScan {
        root: root.to_path_buf(),
        executable,
        launcher_stub,
        built_in: Vec::new(),
        executable_pe,
        components,
        proxies,
        anti_cheat,
        engine,
        exe_dir_dlls,
        vkd3d_cache: has_vkd3d_cache(&files, exe_dir.as_deref()),
        truncated,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graphics::pe::fixture;

    fn put(root: &Path, rel: &str, bytes: &[u8]) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, bytes).unwrap();
    }

    fn exe(imports: &[&str], size_pad: usize) -> Vec<u8> {
        let mut b = fixture::pe(0x8664, true, imports, &[], b"");
        b.resize(b.len() + size_pad, 0);
        b
    }

    #[test]
    fn a_game_like_sottr_is_read_for_what_it_ships() {
        let dir = tempfile::tempdir().unwrap();
        let r = dir.path();
        put(r, "SOTTR.exe", &exe(&["kernel32.dll", "d3d12.dll"], 4096));
        put(r, "unins000.exe", &exe(&["kernel32.dll"], 99_999)); // bigger, but not the game
        put(r, "nvngx_dlss.dll", b"MZ");
        put(r, "libxess.dll", b"MZ");
        put(r, "EOSSDK-Win64-Shipping.dll", b"MZ"); // Epic Online Services is not anti-cheat
        let s = scan(r, None);
        assert_eq!(s.executable.as_deref(), Some(Path::new("SOTTR.exe")));
        assert!(s.executable_pe.as_ref().unwrap().links("d3d12.dll"));
        assert!(s.has(ComponentKind::DlssSuperResolution) && s.has(ComponentKind::Xess));
        assert!(!s.has(ComponentKind::NvngxShim));
        assert!(s.anti_cheat.is_empty(), "{:?}", s.anti_cheat);
        assert!(s.proxies.is_empty());
    }

    #[test]
    fn a_bare_nvngx_is_not_dlss() {
        let dir = tempfile::tempdir().unwrap();
        put(dir.path(), "Game.exe", &exe(&["kernel32.dll"], 0));
        put(dir.path(), "nvngx.dll", b"MZ...OptiScaler...");
        let s = scan(dir.path(), None);
        assert!(s.has(ComponentKind::NvngxShim));
        assert!(!s.has(ComponentKind::DlssSuperResolution));
    }

    #[test]
    fn proxy_owners_are_read_from_contents_and_only_beside_the_executable_count() {
        let dir = tempfile::tempdir().unwrap();
        let r = dir.path();
        put(
            r,
            "Game/Binaries/Win64/Game-Win64-Shipping.exe",
            &exe(&["d3d12.dll"], 0),
        );
        put(r, "Game.exe", &exe(&["kernel32.dll"], 50_000)); // the launcher stub UE ships
        put(
            r,
            "Game/Binaries/Win64/dxgi.dll",
            b"MZ ... ReShade by crosire ...",
        );
        let wide: Vec<u8> = "OptiScaler"
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect();
        put(
            r,
            "Game/Binaries/Win64/winmm.dll",
            &[b"MZ".as_slice(), &wide].concat(),
        );
        put(r, "Game/Binaries/Win64/version.dll", b"MZ nobody knows");
        put(
            r,
            "Engine/Binaries/ThirdParty/dbghelp.dll",
            b"MZ Microsoft Corporation",
        );
        let s = scan(r, None);
        assert_eq!(s.engine, Some(Engine::Unreal));
        assert_eq!(
            s.executable.as_deref(),
            Some(Path::new("Game/Binaries/Win64/Game-Win64-Shipping.exe"))
        );
        let owners: Vec<(&str, &ProxyOwner)> = s
            .proxies
            .iter()
            .map(|p| (p.slot.as_str(), &p.owner))
            .collect();
        assert_eq!(
            owners,
            [
                ("dxgi.dll", &ProxyOwner::ReShade),
                ("version.dll", &ProxyOwner::Unknown),
                ("winmm.dll", &ProxyOwner::OptiScaler),
            ]
        );
    }

    #[test]
    fn the_running_process_name_settles_which_executable_is_the_game() {
        let dir = tempfile::tempdir().unwrap();
        put(dir.path(), "big.exe", &exe(&[], 90_000));
        put(dir.path(), "bin/real.exe", &exe(&[], 10));
        let s = scan(dir.path(), Some("REAL.exe"));
        assert_eq!(s.executable.as_deref(), Some(Path::new("bin/real.exe")));
    }

    #[test]
    fn anti_cheat_is_found_by_folder_file_and_executable() {
        for (rel, want) in [
            ("EasyAntiCheat/Settings.json", "Easy Anti-Cheat"),
            ("start_protected_game.exe", "Easy Anti-Cheat"),
            ("BattlEye/BEClient_x64.dll", "BattlEye"),
            ("Game_BE.exe", "BattlEye"),
            ("x3.xem", "XIGNCODE3"),
            ("EasyAntiCheat_EOS_Setup.exe", "Easy Anti-Cheat"),
            ("Randgrid.sys", "Ricochet"),
            ("EAAntiCheat.Installer.exe", "EA Javelin Anticheat"),
            ("AntiCheatExpert/x.dat", "Tencent ACE"),
            ("pbsvc.exe", "PunkBuster"),
        ] {
            let dir = tempfile::tempdir().unwrap();
            put(dir.path(), "Game.exe", &exe(&[], 0));
            put(dir.path(), rel, b"x");
            let s = scan(dir.path(), None);
            assert_eq!(
                s.anti_cheat.first().map(|a| a.name.as_str()),
                Some(want),
                "{rel}"
            );
        }
        let dir = tempfile::tempdir().unwrap();
        put(dir.path(), "game/bin/win64/cs2.exe", &exe(&[], 0));
        assert_eq!(
            scan(dir.path(), None).anti_cheat[0].name,
            "Valve Anti-Cheat"
        );
    }

    #[test]
    fn unreal_plugin_upscalers_are_found() {
        // Bodycam's layout: each upscaler is a plugin whose folder name has a
        // space, its runtime six levels down, FSR 4's eight.
        let dir = tempfile::tempdir().unwrap();
        let r = dir.path();
        put(
            r,
            "Game/Binaries/Win64/Game-Win64-Shipping.exe",
            &exe(&[], 0),
        );
        let plugins = "Game/Plugins";
        put(
            r,
            &format!("{plugins}/DLSS v8.8.0/Binaries/ThirdParty/Win64/nvngx_dlss.dll"),
            b"MZ",
        );
        put(
            r,
            &format!("{plugins}/XeSS v3.0.5/Binaries/ThirdParty/Win64/libxess.dll"),
            b"MZ",
        );
        put(
            r,
            &format!(
                "{plugins}/FSR v4.1.1/Source/fidelityfx-sdk/Kits/FidelityFX/signedbin/amd_fidelityfx_loader_dx12.dll"
            ),
            b"MZ",
        );
        let s = scan(r, Some("Game-Win64-Shipping.exe"));
        assert!(s.has(ComponentKind::DlssSuperResolution));
        assert!(s.has(ComponentKind::Xess));
        assert!(s.has(ComponentKind::FfxApi));
    }

    fn utf16(s: &str) -> Vec<u8> {
        s.encode_utf16().flat_map(u16::to_le_bytes).collect()
    }

    /// An Unreal bootstrap as The Outer Worlds: Spacer's Choice Edition has
    /// it: the program's name, and the game's path as a UTF-16 resource.
    fn bootstrap(target: &str) -> Vec<u8> {
        let mut extra = b"BootstrapPackagedGame-Win64-Shipping.pdb\0\0".to_vec();
        extra.extend(utf16(target));
        extra.extend([0, 0]);
        fixture::pe(0x8664, true, &["kernel32.dll"], &[], &extra)
    }

    /// A game executable with `markers` in its data.
    fn game_exe(markers: &[&str]) -> Vec<u8> {
        let extra: Vec<u8> = markers
            .iter()
            .flat_map(|m| [m.as_bytes(), b"\0"].concat())
            .collect();
        fixture::pe(0x8664, true, &["kernel32.dll"], &[], &extra)
    }

    #[test]
    fn an_unreal_bootstrap_is_followed_to_the_game_it_starts() {
        let dir = tempfile::tempdir().unwrap();
        let r = dir.path();
        let real = "Indiana/Binaries/Win64/IndianaEpicGameStore-Win64-Shipping.exe";
        put(
            r,
            "TheOuterWorldsSpacersChoiceEdition.exe",
            &bootstrap(r"Indiana\Binaries\Win64\IndianaEpicGameStore-Win64-Shipping.exe"),
        );
        put(
            r,
            real,
            &game_exe(&[
                "bFSR2Enabled",
                "ffxFsr2ContextCreate",
                "ffxFsr2GetInterfaceDX12",
            ]),
        );
        put(r, "Indiana/Binaries/Win64/vkd3d-proton.cache", b"x");
        put(
            r,
            "Engine/Binaries/Win64/CrashReportClient-Win64-Shipping.exe",
            &exe(&[], 0),
        );
        // The launcher records the bootstrap; the scan goes to the game.
        let mut s = scan(r, Some("TheOuterWorldsSpacersChoiceEdition.exe"));
        assert!(s.built_in.is_empty(), "not read by the scan itself");
        s.read_built_in();
        assert_eq!(s.executable.as_deref(), Some(Path::new(real)));
        assert_eq!(
            s.launcher_stub.as_deref(),
            Some(Path::new("TheOuterWorldsSpacersChoiceEdition.exe"))
        );
        assert_eq!(s.engine, Some(Engine::Unreal));
        assert!(s.vkd3d_cache);
        assert_eq!(s.built_in_only(), [BuiltIn::Fsr2]);
        // And the running game is known by either name.
        assert_eq!(
            runs_as(r, "theouterworldsspacerschoiceedition.EXE"),
            [
                "theouterworldsspacerschoiceedition.EXE",
                "IndianaEpicGameStore-Win64-Shipping.exe"
            ]
        );
    }

    #[test]
    fn a_bootstrap_may_start_a_game_not_named_shipping() {
        // Hogwarts Legacy: `HogwartsLegacy.exe` (290 KB) starts
        // `Phoenix\Binaries\Win64\HogwartsLegacy.exe`.
        let dir = tempfile::tempdir().unwrap();
        let r = dir.path();
        let real = "Phoenix/Binaries/Win64/HogwartsLegacy.exe";
        put(
            r,
            "HogwartsLegacy.exe",
            &bootstrap(r"Phoenix\Binaries\Win64\HogwartsLegacy.exe"),
        );
        put(r, real, &exe(&["d3d12.dll"], 0));
        let s = scan(r, Some("HogwartsLegacy.exe"));
        assert_eq!(s.executable.as_deref(), Some(Path::new(real)));
        assert_eq!(s.engine, Some(Engine::Unreal));
        // Without the bootstrap's name, a path that is not a Shipping
        // binary is only a string in some program.
        let other = tempfile::tempdir().unwrap();
        let mut extra = utf16(r"Phoenix\Binaries\Win64\HogwartsLegacy.exe");
        extra.extend([0, 0]);
        put(
            other.path(),
            "Launcher2.exe",
            &fixture::pe(0x8664, true, &[], &[], &extra),
        );
        put(other.path(), real, &exe(&[], 0));
        let s = scan(other.path(), Some("Launcher2.exe"));
        assert_eq!(s.executable.as_deref(), Some(Path::new("Launcher2.exe")));
    }

    #[test]
    fn a_bootstrap_without_a_readable_path_takes_the_one_shipping_binary_beside_it() {
        let dir = tempfile::tempdir().unwrap();
        let r = dir.path();
        put(r, "Game.exe", &game_exe(&["BootstrapPackagedGame"]));
        put(
            r,
            "Proj/Binaries/Win64/Proj-Win64-Shipping.exe",
            &exe(&[], 0),
        );
        put(
            r,
            "Engine/Binaries/Win64/Other-Win64-Shipping.exe",
            &exe(&[], 0),
        );
        let s = scan(r, Some("Game.exe"));
        assert_eq!(
            s.executable.as_deref(),
            Some(Path::new("Proj/Binaries/Win64/Proj-Win64-Shipping.exe"))
        );
        // Two projects: which one is not guessed.
        let dir2 = tempfile::tempdir().unwrap();
        for f in [
            "Proj/Binaries/Win64/Proj-Win64-Shipping.exe",
            "Other/Binaries/Win64/Other-Win64-Shipping.exe",
        ] {
            put(dir2.path(), f, &exe(&[], 0));
        }
        put(
            dir2.path(),
            "Game.exe",
            &game_exe(&["BootstrapPackagedGame"]),
        );
        let s = scan(dir2.path(), Some("Game.exe"));
        assert_eq!(s.executable.as_deref(), Some(Path::new("Game.exe")));
        assert_eq!(s.launcher_stub, None);
    }

    #[test]
    fn an_ordinary_executable_and_a_path_outside_the_game_are_not_followed() {
        let dir = tempfile::tempdir().unwrap();
        let r = dir.path();
        // A small game that happens to name a Binaries path, which is not
        // there: it stays the game.
        let mut extra = utf16(r"Proj\Binaries\Win64\Proj-Win64-Shipping.exe");
        extra.extend([0, 0]);
        put(r, "Small.exe", &fixture::pe(0x8664, true, &[], &[], &extra));
        assert_eq!(
            scan(r, Some("Small.exe")).executable.as_deref(),
            Some(Path::new("Small.exe"))
        );
        // A path that climbs out of the game's folder is never taken.
        let outside = tempfile::tempdir().unwrap();
        put(
            outside.path(),
            "x/Binaries/Win64/x-Win64-Shipping.exe",
            &exe(&[], 0),
        );
        let climb = format!(
            r"..\{}\x\Binaries\Win64\x-Win64-Shipping.exe",
            outside.path().file_name().unwrap().to_string_lossy()
        );
        let game = outside.path().join("game");
        put(&game, "Game.exe", &bootstrap(&climb));
        let s = scan(&game, Some("Game.exe"));
        assert_eq!(s.executable.as_deref(), Some(Path::new("Game.exe")));
        assert_eq!(runs_as(&game, "Game.exe"), ["Game.exe"]);
    }

    #[test]
    fn upscalers_compiled_into_the_executable_are_told_from_their_dlls() {
        assert_eq!(
            built_in_markers(
                b"..ffxFsr3UpscalerContextCreate..NVSDK_NGX_D3D12_Init..xessVKCreateContext"
            ),
            [BuiltIn::Fsr3, BuiltIn::Dlss, BuiltIn::Xess]
        );
        // "FSR2" on its own is a setting's name, not the SDK.
        assert!(built_in_markers(b"bFSR2Enabled EFSR2Mode::Quality").is_empty());

        let dir = tempfile::tempdir().unwrap();
        let r = dir.path();
        put(
            r,
            "Game.exe",
            &game_exe(&["ffxFsr2ContextCreate", "NVSDK_NGX_D3D12_Init"]),
        );
        // DLSS code with no DLSS runtime: built in (and the game cannot run
        // it); FSR 2 with its DLL: the DLL's.
        put(r, "ffx_fsr2_api_x64.dll", b"MZ");
        let mut s = scan(r, None);
        s.read_built_in();
        assert_eq!(s.built_in, [BuiltIn::Fsr2, BuiltIn::Dlss]);
        assert_eq!(s.built_in_only(), [BuiltIn::Dlss]);
        // A game with every DLL has nothing built in worth reading for.
        put(r, "nvngx_dlss.dll", b"MZ");
        put(r, "libxess.dll", b"MZ");
        let mut s = scan(r, None);
        s.read_built_in();
        assert!(s.built_in.is_empty());
        assert!(!BuiltIn::Dlss.runs_without_a_dll() && BuiltIn::Fsr2.runs_without_a_dll());
    }

    #[test]
    fn a_marker_across_two_pieces_of_a_large_executable_is_found() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("Big.exe");
        let mut bytes = vec![0u8; (8 << 20) - 7];
        bytes.extend_from_slice(b"ffxFsr2ContextCreate");
        bytes.resize(bytes.len() + 4096, 0);
        std::fs::write(&path, &bytes).unwrap();
        assert_eq!(built_in_in_file(&path), Some(vec![BuiltIn::Fsr2]));
        assert_eq!(built_in_in_file(&dir.path().join("none.exe")), None);
    }

    #[test]
    fn symlinks_are_not_followed_and_the_walk_is_bounded() {
        let dir = tempfile::tempdir().unwrap();
        let r = dir.path();
        put(r, "Game.exe", &exe(&[], 0));
        std::fs::create_dir_all(r.join("deep/a/b/c/d/e/f/g/h/i")).unwrap();
        put(r, "deep/a/b/c/d/e/f/g/h/i/nvngx_dlss.dll", b"MZ"); // beyond MAX_DEPTH
        std::os::unix::fs::symlink("/usr", r.join("usr-link")).unwrap();
        let s = scan(r, None);
        assert!(!s.has(ComponentKind::DlssSuperResolution));
        assert!(!s.truncated);
    }

    #[test]
    fn anti_cheat_is_found_however_large_the_folder() {
        let dir = tempfile::tempdir().unwrap();
        let r = dir.path();
        // More entries at the top than the walk is allowed to read, and
        // BattlEye beside an Unreal game's executable, four levels down.
        for i in 0..120 {
            put(r, &format!("pak{i:03}.dat"), b"x");
        }
        put(r, "Game.exe", &exe(&[], 0));
        put(r, "Proj/Binaries/Win64/BattlEye/BEClient_x64.dll", b"MZ");
        let s = scan_limited(r, Some("Game.exe"), 50);
        assert!(s.truncated);
        assert_eq!(s.executable.as_deref(), Some(Path::new("Game.exe")));
        assert_eq!(
            s.anti_cheat
                .iter()
                .map(|a| a.name.as_str())
                .collect::<Vec<_>>(),
            ["BattlEye"]
        );
        // Beside a deeper executable too, past the depth from the top.
        let dir = tempfile::tempdir().unwrap();
        let r = dir.path();
        put(r, "a/b/c/d/Game.exe", &exe(&[], 0));
        put(r, "a/b/c/d/x/EasyAntiCheat_x64.dll", b"MZ");
        let found = anti_cheat_near(r, Some(Path::new("a/b/c/d/Game.exe")));
        assert_eq!(found[0].name, "Easy Anti-Cheat");
        assert_eq!(
            found[0].evidence,
            Path::new("a/b/c/d/x/EasyAntiCheat_x64.dll")
        );
    }
}
