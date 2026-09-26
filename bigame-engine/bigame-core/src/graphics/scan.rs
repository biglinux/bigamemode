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

use std::path::{Path, PathBuf};

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

/// Everything the scan found.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameScan {
    /// Install folder scanned.
    pub root: PathBuf,
    /// The game's executable, relative to `root`, when it was found.
    pub executable: Option<PathBuf>,
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

/// Walk the tree once, bounded, collecting every file and directory name.
fn walk(root: &Path) -> (Vec<(PathBuf, bool)>, bool) {
    let mut out = Vec::new();
    let mut stack = vec![(root.to_path_buf(), 0usize)];
    let mut seen = 0usize;
    while let Some((dir, depth)) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            seen += 1;
            if seen > MAX_ENTRIES {
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
    if let Some(e) = exes
        .iter()
        .find(|e| lower_name(e).ends_with("-win64-shipping.exe"))
    {
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

/// Scan a game's install folder.
///
/// `exe_hint` is the process name the game runs as, when known.
#[must_use]
pub fn scan(root: &Path, exe_hint: Option<&str>) -> GameScan {
    let (files, truncated) = walk(root);
    let executable = choose_executable(root, &files, exe_hint);
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
    if executable
        .as_ref()
        .is_some_and(|e| lower_name(e).ends_with("-win64-shipping.exe"))
    {
        engine = Some(Engine::Unreal);
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
        executable_pe,
        components,
        proxies,
        anti_cheat,
        engine,
        exe_dir_dlls,
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
}
