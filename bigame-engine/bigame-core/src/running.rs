//! The game that is running right now, and what it really is.
//!
//! A Proton game is not one process but a tree, and most of it is not the
//! game:
//!
//! ```text
//! reaper SteamLaunch AppId=750920 -- …
//!  └ srt-bwrap / pv-adverb           pressure-vessel container
//!     └ python3 …/proton waitforexitandrun …
//!        └ steam.exe  SOTTR.exe       Wine's Steam shim
//!           └ SOTTR.exe               the game
//!        wineserver, services.exe, explorer.exe, winedevice.exe, …
//! ```
//!
//! falcond keys a profile on the **basename of `argv[0]`**, splitting on both
//! `/` and `\` — so a Windows path like `S:\…\SOTTR.exe` becomes `SOTTR.exe`.
//! [`falcond_name`] applies exactly that rule, because a profile created from
//! any other spelling would never match. The install directory is never the
//! key: falcond cannot match a profile named after it.
//!
//! Classification is a pure function over a process list, so it is tested
//! against trees copied from real games rather than against a live system.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

// ── Process snapshot ─────────────────────────────────────────────────────────

/// What classification needs to know about one process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Proc {
    /// Process id.
    pub pid: u32,
    /// Parent process id.
    pub ppid: u32,
    /// `argv[0]`, verbatim.
    pub argv0: String,
    /// The whole command line, NUL separators turned into spaces.
    pub cmdline: String,
    /// User + system CPU time, in clock ticks. The game is the busy one.
    pub cpu_ticks: u64,
}

/// Read the current user's processes from `/proc`.
///
/// One directory walk, three small reads per process, no forks.
#[must_use]
#[allow(clippy::similar_names)] // pid and ppid are what /proc calls them
pub fn snapshot() -> Vec<Proc> {
    // SAFETY: getuid cannot fail and has no side effects.
    let uid = unsafe { libc::getuid() };
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|entry| {
            let pid: u32 = entry.file_name().to_string_lossy().parse().ok()?;
            let dir = entry.path();
            let status = std::fs::read_to_string(dir.join("status")).ok()?;
            let owner: u32 = status
                .lines()
                .find_map(|l| l.strip_prefix("Uid:"))?
                .split_whitespace()
                .next()?
                .parse()
                .ok()?;
            if owner != uid {
                return None;
            }
            let stat = std::fs::read_to_string(dir.join("stat")).ok()?;
            let (ppid, state, cpu_ticks) = parse_stat(&stat)?;
            if state == 'Z' {
                return None;
            }
            let raw = std::fs::read(dir.join("cmdline")).ok()?;
            let argv0 = raw
                .split(|b| *b == 0)
                .next()
                .map(|a| String::from_utf8_lossy(a).into_owned())
                .unwrap_or_default();
            let cmdline = String::from_utf8_lossy(
                &raw.iter()
                    .map(|b| if *b == 0 { b' ' } else { *b })
                    .collect::<Vec<u8>>(),
            )
            .trim_end()
            .to_owned();
            Some(Proc {
                pid,
                ppid,
                argv0,
                cmdline,
                cpu_ticks,
            })
        })
        .collect()
}

/// `(ppid, state, utime + stime)` from `/proc/<pid>/stat`.
fn parse_stat(stat: &str) -> Option<(u32, char, u64)> {
    // Fields after the parenthesised comm, which may contain ") ".
    let rest = &stat[stat.rfind(')')? + 1..];
    let fields: Vec<&str> = rest.split_whitespace().collect();
    let state = fields.first()?.chars().next()?;
    let ppid = fields.get(1)?.parse().ok()?;
    let utime: u64 = fields.get(11)?.parse().ok()?;
    let stime: u64 = fields.get(12)?.parse().ok()?;
    Some((ppid, state, utime + stime))
}

/// The name falcond will see for this process: the basename of `argv[0]`,
/// split on both Unix and Windows separators.
#[must_use]
pub fn falcond_name(argv0: &str) -> &str {
    let cut = argv0.rfind(['/', '\\']).map_or(0, |i| i + 1);
    &argv0[cut..]
}

// ── Identity ─────────────────────────────────────────────────────────────────

/// How the game runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "tool")]
pub enum Runtime {
    /// A Linux binary.
    Native,
    /// Windows build under Proton; the tool's directory name.
    Proton(String),
    /// Windows build under Wine outside Steam.
    Wine,
}

/// The graphics path, from what the game has mapped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Graphics {
    /// Direct3D 12 through VKD3D-Proton.
    Vkd3dProton,
    /// Direct3D 8–11 through DXVK.
    Dxvk,
    /// Direct3D through Wine's own OpenGL translation.
    WineD3d,
    /// Native Vulkan.
    Vulkan,
    /// Native OpenGL.
    OpenGl,
    /// Could not be told.
    Unknown,
}

impl Graphics {
    /// A short label.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Vkd3dProton => "VKD3D-Proton · DX12",
            Self::Dxvk => "DXVK · DX11",
            Self::WineD3d => "WineD3D",
            Self::Vulkan => "Vulkan",
            Self::OpenGl => "OpenGL",
            Self::Unknown => "Unknown",
        }
    }
}

/// A running game, identified.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameIdentity {
    /// Title, from the launcher when known, otherwise the process name.
    pub display_name: String,
    /// Steam application id.
    pub steam_app_id: Option<String>,
    /// Where it was installed, when known.
    pub install_path: Option<PathBuf>,
    /// The Proton prefix, when there is one.
    pub compatdata_path: Option<PathBuf>,
    /// The game's own process.
    pub pid: u32,
    /// The name falcond matches on. This, and only this, is a profile's key.
    pub process_name: String,
    /// `argv[0]` of the game process.
    pub executable: String,
    /// Native, Proton or Wine.
    pub runtime: Runtime,
    /// Graphics path, when it could be told.
    pub graphics: Graphics,
    /// The DRM card it renders on, when it could be told.
    pub render_card: Option<String>,
    /// The tree it was found in, root first, as `(pid, name)`.
    pub tree: Vec<(u32, String)>,
}

/// Processes that are part of the machinery around a game, never the game.
///
/// Names are compared case-insensitively against [`falcond_name`]. falcond's
/// own `system_processes` list (`/usr/share/falcond/system.conf`) covers
/// Wine's services; this adds the container, launcher and helper layers
/// around them.
const INFRASTRUCTURE: &[&str] = &[
    // Steam and its container
    "reaper",
    "steam",
    "steam.exe",
    "steamwebhelper",
    "steamservice.exe",
    "srt-bwrap",
    "pv-adverb",
    "pressure-vessel-wrap",
    "steam-runtime-launcher-service",
    "steam-runtime-launch-client",
    "python3",
    "python",
    "sh",
    "bash",
    // Wine infrastructure
    "wineserver",
    "wine",
    "wine64",
    "wine-preloader",
    "wine64-preloader",
    "services.exe",
    "winedevice.exe",
    "plugplay.exe",
    "svchost.exe",
    "explorer.exe",
    "rpcss.exe",
    "tabtip.exe",
    "conhost.exe",
    "rundll32.exe",
    "wineboot.exe",
    "winemenubuilder.exe",
    "start.exe",
    "cmd.exe",
    "xalia.exe",
    "iexplore.exe",
    "d3ddriverquery64.exe",
    // Steam's installer-script runner: it runs inside the game's Proton tree
    // on a first launch, before the game itself, and must not be offered a
    // profile.
    "iscriptevaluator.exe",
    "installscript.exe",
    // Helpers games ship
    "crashpad_handler.exe",
    "unitycrashhandler64.exe",
    "crashreportclient.exe",
    "rederrorreporter.exe",
    "redprelauncher.exe",
    "easyanticheat.exe",
    "easyanticheat_eos.exe",
    "beservice.exe",
    "epicwebhelper.exe",
    "gameoverlayui",
];

/// falcond's own list of processes that are never games
/// (`/usr/share/falcond/system.conf`), read once.
///
/// Using falcond's list as well as ours means the two can never disagree
/// about what a game is: anything falcond would refuse to profile, BiGame-mode
/// will not offer a profile for either.
fn falcond_system_processes() -> &'static [String] {
    static LIST: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
    LIST.get_or_init(|| {
        std::fs::read_to_string("/usr/share/falcond/system.conf")
            .map(|text| parse_system_processes(&text))
            .unwrap_or_default()
    })
}

/// The quoted names in a `system_processes = [ … ]` array, lowercased.
#[must_use]
pub fn parse_system_processes(text: &str) -> Vec<String> {
    let Some(start) = text.find("system_processes") else {
        return Vec::new();
    };
    let rest = &text[start..];
    let Some(open) = rest.find('[') else {
        return Vec::new();
    };
    let close = rest[open..].find(']').map_or(rest.len(), |c| open + c);
    rest[open + 1..close]
        .split('"')
        .skip(1)
        .step_by(2)
        .map(str::to_ascii_lowercase)
        .collect()
}

/// Whether a process name is machinery rather than a game.
#[must_use]
pub fn is_infrastructure(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    INFRASTRUCTURE.contains(&lower.as_str())
        || falcond_system_processes().contains(&lower)
        || lower.starts_with("wine")
        // Crash handlers by their usual names -- not any name containing
        // "crash", which would also exclude Crash Bandicoot.
        || ["crashhandler", "crash_handler", "crashreport", "crashpad", "crashsender"]
            .iter()
            .any(|n| lower.contains(n))
        || lower.contains("launcher")
        || crate::games::is_support_binary(&lower)
}

/// The Steam app id a reaper was started for.
fn reaper_app_id(proc: &Proc) -> Option<String> {
    if falcond_name(&proc.argv0) != "reaper" {
        return None;
    }
    let id = proc
        .cmdline
        .split_whitespace()
        .find_map(|w| w.strip_prefix("AppId="))?;
    id.chars()
        .all(|c| c.is_ascii_digit())
        .then(|| id.to_owned())
}

/// The Proton tool name from a proton script's path, e.g. `Proton - Experimental`.
fn proton_tool(tree: &[&Proc]) -> Option<String> {
    tree.iter().find_map(|p| {
        let path = p.cmdline.split(" waitforexitandrun").next()?;
        let path = path
            .strip_prefix("python3 ")
            .or_else(|| path.strip_prefix("python "))?;
        let dir = Path::new(path.trim()).parent()?;
        (Path::new(path.trim()).file_name()? == "proton")
            .then(|| dir.file_name().map(|n| n.to_string_lossy().into_owned()))
            .flatten()
    })
}

/// Everything below `root`, root first.
fn descendants<'a>(
    root: u32,
    by_parent: &HashMap<u32, Vec<&'a Proc>>,
    all: &'a [Proc],
) -> Vec<&'a Proc> {
    let mut out: Vec<&Proc> = all.iter().filter(|p| p.pid == root).collect();
    let mut i = 0;
    while i < out.len() {
        if let Some(children) = by_parent.get(&out[i].pid) {
            out.extend(children.iter().copied());
        }
        i += 1;
    }
    out
}

/// Find the running games in a process list.
///
/// Steam games are found from their reaper, which names the app id; within
/// that tree the game is the busiest process that is not machinery — a
/// launcher can briefly be the only candidate, and it is excluded by name.
/// Wine games outside Steam are found as busy `.exe` processes under Wine.
#[must_use]
pub fn identify(procs: &[Proc]) -> Vec<GameIdentity> {
    identify_with(procs, &HashMap::new())
}

/// [`identify`], also recognising native games outside Steam.
///
/// `native` maps the executable names of games this machine knows about
/// ([`known_native_games`]) to their display names. Without it a native
/// game started from the application menu (`SuperTuxKart` from the
/// repositories, say) is never taken for a game: Home keeps saying *waiting
/// for games* and no profile is offered.
#[must_use]
pub fn identify_with<S: std::hash::BuildHasher>(
    procs: &[Proc],
    native: &HashMap<String, String, S>,
) -> Vec<GameIdentity> {
    let mut by_parent: HashMap<u32, Vec<&Proc>> = HashMap::new();
    for p in procs {
        by_parent.entry(p.ppid).or_default().push(p);
    }
    let mut found = Vec::new();
    let mut claimed: std::collections::HashSet<u32> = std::collections::HashSet::new();

    for reaper in procs.iter().filter(|p| reaper_app_id(p).is_some()) {
        let tree = descendants(reaper.pid, &by_parent, procs);
        claimed.extend(tree.iter().map(|p| p.pid));
        let proton = proton_tool(&tree);
        let candidates: Vec<&&Proc> = tree
            .iter()
            .filter(|p| !is_infrastructure(falcond_name(&p.argv0)))
            .collect();
        // In a Proton tree the game is a Windows binary. A Linux helper inside
        // the container -- an overlay, a wrapper -- must not outrank a game
        // that is still loading and has burned little CPU yet.
        let windows: Vec<&&Proc> = candidates
            .iter()
            .copied()
            .filter(|p| p.argv0.to_ascii_lowercase().ends_with(".exe"))
            .collect();
        let pool = if proton.is_some() && !windows.is_empty() {
            windows
        } else {
            candidates
        };
        let Some(game) = pool.into_iter().max_by_key(|p| p.cpu_ticks) else {
            continue;
        };
        found.push(GameIdentity {
            display_name: falcond_name(&game.argv0).to_owned(),
            steam_app_id: reaper_app_id(reaper),
            install_path: None,
            compatdata_path: None,
            pid: game.pid,
            process_name: falcond_name(&game.argv0).to_owned(),
            executable: game.argv0.clone(),
            runtime: match proton {
                Some(tool) => Runtime::Proton(tool),
                None if game.argv0.to_ascii_lowercase().ends_with(".exe") => {
                    Runtime::Proton(String::new())
                }
                None => Runtime::Native,
            },
            graphics: Graphics::Unknown,
            render_card: None,
            tree: tree
                .iter()
                .map(|p| (p.pid, falcond_name(&p.argv0).to_owned()))
                .collect(),
        });
    }

    // Wine outside Steam: a busy .exe that is not machinery.
    for p in procs.iter().filter(|p| !claimed.contains(&p.pid)) {
        let name = falcond_name(&p.argv0);
        if !name.to_ascii_lowercase().ends_with(".exe") || is_infrastructure(name) {
            continue;
        }
        // Idle Windows helpers are not games; a game burns CPU from its
        // first seconds.
        if p.cpu_ticks < 100 {
            continue;
        }
        found.push(GameIdentity {
            display_name: name.to_owned(),
            steam_app_id: None,
            install_path: None,
            compatdata_path: None,
            pid: p.pid,
            process_name: name.to_owned(),
            executable: p.argv0.clone(),
            runtime: Runtime::Wine,
            graphics: Graphics::Unknown,
            render_card: None,
            tree: vec![(p.pid, name.to_owned())],
        });
    }

    // Native games outside Steam: an executable the machine lists as a game.
    // Only the busiest process of each name counts, so a game that forks
    // helpers under its own name is still one game.
    let mut native_found: HashMap<&str, &Proc> = HashMap::new();
    for p in procs.iter().filter(|p| !claimed.contains(&p.pid)) {
        let name = falcond_name(&p.argv0);
        if !native.contains_key(name) || is_infrastructure(name) || p.cpu_ticks < 100 {
            continue;
        }
        let best = native_found.entry(name).or_insert(p);
        if p.cpu_ticks > best.cpu_ticks {
            *best = p;
        }
    }
    for (name, p) in native_found {
        found.push(GameIdentity {
            display_name: native[name].clone(),
            steam_app_id: None,
            install_path: None,
            compatdata_path: None,
            pid: p.pid,
            process_name: name.to_owned(),
            executable: p.argv0.clone(),
            runtime: Runtime::Native,
            graphics: Graphics::Unknown,
            render_card: None,
            tree: vec![(p.pid, name.to_owned())],
        });
    }
    found
}

// ── Native games the machine knows about ─────────────────────────────────────

/// Executable name → display name for every native game this machine lists:
/// the menu entries in the `Game` category ([`crate::games::menu_games`]),
/// and the process names falcond has a profile for — a profile is falcond's
/// own statement that a process is a game.
///
/// Read at most once a minute: detection runs every few seconds, and a game
/// installed meanwhile is picked up on the next read.
#[must_use]
pub fn known_native_games() -> HashMap<String, String> {
    use std::sync::Mutex;
    use std::time::{Duration, Instant};
    static CACHE: Mutex<Option<(Instant, HashMap<String, String>)>> = Mutex::new(None);
    let mut cache = CACHE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some((at, games)) = cache.as_ref() {
        if at.elapsed() < Duration::from_secs(60) {
            return games.clone();
        }
    }
    let games = read_native_games();
    *cache = Some((Instant::now(), games.clone()));
    games
}

fn read_native_games() -> HashMap<String, String> {
    let mut games = HashMap::new();
    // Profile names first, so a menu entry's friendlier name wins.
    let base = Path::new(crate::profiles::SYSTEM_PROFILES_DIR);
    for dir in [base.to_path_buf(), base.join("user")] {
        for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let Ok(content) = std::fs::read_to_string(entry.path()) else {
                continue;
            };
            if let Some(name) = profile_name_field(&content) {
                if name != "Proton"
                    && !name.to_ascii_lowercase().ends_with(".exe")
                    && !is_infrastructure(&name)
                {
                    games.insert(name.clone(), name);
                }
            }
        }
    }
    for game in crate::games::menu_games() {
        games.insert(game.program, game.name);
    }
    games
}

// ── Enrichment (reads the live system for one process) ───────────────────────

/// The graphics path, from the libraries a process has mapped.
///
/// Under Proton the translation layers are mapped from the prefix's
/// `system32` under their Windows names — `d3d12.dll` is VKD3D-Proton,
/// `d3d11.dll`/`d3d9.dll` are DXVK — not from `vkd3d-proton/` or `dxvk/`
/// directories, as was observed in Shadow of the Tomb Raider. The highest API
/// wins, because DX12 games map `d3d11.dll` as well. `libGL` is always mapped
/// by Wine's display driver, so OpenGL and Vulkan only count for a process
/// with no Windows DLLs at all.
#[must_use]
pub fn graphics_from_maps(maps: &str) -> Graphics {
    let libraries: std::collections::HashSet<String> = maps
        .lines()
        .filter_map(|line| line.split_whitespace().nth(5))
        .map(|path| falcond_name(path).to_ascii_lowercase())
        .collect();
    let has = |name: &str| libraries.contains(name);
    if has("d3d12.dll") || has("d3d12core.dll") {
        Graphics::Vkd3dProton
    } else if has("wined3d.dll") {
        Graphics::WineD3d
    } else if has("d3d11.dll") || has("d3d10core.dll") || has("d3d9.dll") || has("d3d8.dll") {
        Graphics::Dxvk
    } else if libraries.iter().any(|l| {
        Path::new(l)
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("dll"))
    }) {
        Graphics::Unknown
    } else if libraries.iter().any(|l| l.starts_with("libvulkan_")) || has("libvulkan.so.1") {
        Graphics::Vulkan
    } else if libraries
        .iter()
        .any(|l| l.starts_with("libgl.so") || l.starts_with("libglx") || l.starts_with("libegl"))
    {
        Graphics::OpenGl
    } else {
        Graphics::Unknown
    }
}

/// The DRM card a process renders on, from its open render node.
fn render_card(pid: u32) -> Option<String> {
    let fds = std::fs::read_dir(format!("/proc/{pid}/fd")).ok()?;
    let node = fds.flatten().find_map(|fd| {
        let target = std::fs::read_link(fd.path()).ok()?;
        let name = target.file_name()?.to_string_lossy().into_owned();
        name.starts_with("renderD").then_some(name)
    })?;
    // /sys/class/drm/renderD128/device → the PCI device; its drm/cardN is the card.
    let device = std::fs::canonicalize(format!("/sys/class/drm/{node}/device")).ok()?;
    std::fs::read_dir(device.join("drm"))
        .ok()?
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .find(|n| n.starts_with("card"))
}

/// Fill in what the launcher and the live process can say.
fn enrich(mut game: GameIdentity) -> GameIdentity {
    if let Ok(maps) = std::fs::read_to_string(format!("/proc/{}/maps", game.pid)) {
        game.graphics = graphics_from_maps(&maps);
    }
    game.render_card = render_card(game.pid);
    if let Some(id) = game.steam_app_id.clone() {
        if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
            for library in crate::games::steam_libraries(&home) {
                let steamapps = library.join("steamapps");
                let Ok(text) =
                    std::fs::read_to_string(steamapps.join(format!("appmanifest_{id}.acf")))
                else {
                    continue;
                };
                if let Some(name) = crate::games::acf_value(&text, "name") {
                    game.display_name = name;
                }
                if let Some(dir) = crate::games::acf_value(&text, "installdir") {
                    game.install_path = Some(steamapps.join("common").join(dir));
                }
                // The prefix beside the manifest, not the first compatdata/<id>
                // found: Steam leaves stale ones behind when a game moves.
                let prefix = steamapps.join("compatdata").join(&id);
                game.compatdata_path = prefix.is_dir().then_some(prefix);
                break;
            }
        }
    }
    game
}

/// The running game, if there is one.
///
/// When more than one is found, the busiest wins: that is the one being
/// played, and it is also the one falcond will be holding a profile for.
#[must_use]
pub fn detect() -> Option<GameIdentity> {
    let procs = snapshot();
    let ticks: HashMap<u32, u64> = procs.iter().map(|p| (p.pid, p.cpu_ticks)).collect();
    identify_with(&procs, &known_native_games())
        .into_iter()
        .max_by_key(|g| ticks.get(&g.pid).copied().unwrap_or(0))
        .map(enrich)
}

/// How long a process has been running, in seconds.
///
/// From its start time in `/proc/<pid>/stat` (clock ticks since boot) and the
/// system's uptime.
#[must_use]
pub fn running_for(pid: u32) -> Option<u64> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let rest = &stat[stat.rfind(')')? + 1..];
    // starttime is proc(5) field 22; counted from the state field (3) it is
    // index 19.
    let start_ticks: u64 = rest.split_whitespace().nth(19)?.parse().ok()?;
    let uptime: f64 = std::fs::read_to_string("/proc/uptime")
        .ok()?
        .split_whitespace()
        .next()?
        .parse()
        .ok()?;
    // SAFETY: sysconf only reads a static configuration value.
    let hz = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
    if hz <= 0 {
        return None;
    }
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss
    )]
    let started = start_ticks as f64 / hz as f64;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    Some((uptime - started).max(0.0) as u64)
}

// ── Profiles, as falcond sees them ───────────────────────────────────────────

/// A falcond profile that matches a process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileMatch {
    /// The profile's `name` field.
    pub name: String,
    /// The file it came from.
    pub path: PathBuf,
    /// Whether it is a user profile (`profiles/user/`).
    pub user: bool,
}

/// The `name = "…"` field of a falcond profile.
#[must_use]
pub fn profile_name_field(content: &str) -> Option<String> {
    content.lines().find_map(|line| {
        let rest = line.trim().strip_prefix("name")?.trim_start();
        let value = rest.strip_prefix('=')?.trim();
        Some(value.trim_matches('"').to_owned())
    })
}

/// The profile falcond would apply to `process_name`, other than its generic
/// Proton fallback.
///
/// Looks where falcond looks — the directory for the configured profile
/// mode, then `user/` — and matches the `name` field, not the file name, the
/// way falcond does: exactly, then case-insensitively.
#[must_use]
pub fn matching_profile(process_name: &str, profile_mode: &str) -> Option<ProfileMatch> {
    let base = Path::new(crate::profiles::SYSTEM_PROFILES_DIR);
    let mode_dir = match profile_mode {
        "handheld" | "htpc" => base.join(profile_mode),
        _ => base.to_path_buf(),
    };
    let mut candidates = Vec::new();
    for (dir, user) in [(mode_dir, false), (base.join("user"), true)] {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_none_or(|e| e != "conf") {
                continue;
            }
            let Ok(content) = std::fs::read_to_string(&path) else {
                continue;
            };
            if let Some(name) = profile_name_field(&content) {
                if name != "Proton" {
                    candidates.push(ProfileMatch { name, path, user });
                }
            }
        }
    }
    candidates
        .iter()
        .find(|c| c.name == process_name)
        .or_else(|| {
            candidates
                .iter()
                .find(|c| c.name.eq_ignore_ascii_case(process_name))
        })
        .cloned()
}

#[cfg(test)]
// PIDs are copied verbatim from a real process list; pid and ppid are the
// /proc names.
#[allow(clippy::unreadable_literal, clippy::similar_names)]
mod tests {
    use super::*;

    /// `"argv0|arguments"`: argv0 is given explicitly because Wine paths
    /// contain spaces, and the kernel separates arguments with NUL, not space.
    fn p(pid: u32, ppid: u32, line: &str, cpu: u64) -> Proc {
        let (argv0, args) = line.split_once('|').unwrap_or((line, ""));
        Proc {
            pid,
            ppid,
            argv0: argv0.to_owned(),
            cmdline: format!("{argv0} {args}").trim_end().to_owned(),
            cpu_ticks: cpu,
        }
    }

    /// A real Shadow of the Tomb Raider process tree (from `pgrep -a`), plus
    /// the Wine services a prefix always has.
    fn sottr_tree() -> Vec<Proc> {
        vec![
            p(1, 0, "/usr/lib/systemd/systemd|--user", 5000),
            p(
                10,
                1,
                "/home/u/.local/share/Steam/ubuntu12_32/steam|-srt-logger-opened",
                90_000,
            ),
            p(
                2237909,
                10,
                "/home/u/.local/share/Steam/ubuntu12_32/reaper|SteamLaunch AppId=750920 -- /home/u/.local/share/Steam/steamapps/common/SteamLinuxRuntime_4/_v2-entry-point",
                1,
            ),
            p(
                2237912,
                2237909,
                "/home/u/.local/share/Steam/steamapps/common/SteamLinuxRuntime_4/pressure-vessel/libexec/steam-runtime-tools-0/srt-bwrap|--args 26",
                2,
            ),
            p(
                2237979,
                2237912,
                "/usr/lib/pressure-vessel/from-host/libexec/steam-runtime-tools-0/pv-adverb|--prefix=/usr/lib/pressure-vessel/from-host",
                3,
            ),
            p(
                2238013,
                2237979,
                "python3|/home/u/.local/share/Steam/steamapps/common/Proton - Experimental/proton waitforexitandrun /run/media/u/Games/steamapps/common/Shadow of the Tomb Raider/SOTTR.exe",
                40,
            ),
            p(
                2238018,
                2238013,
                "c:\\windows\\system32\\steam.exe|/run/media/u/Games/steamapps/common/Shadow of the Tomb Raider/SOTTR.exe",
                30,
            ),
            p(
                2238202,
                2238018,
                "S:\\steamapps\\common\\Shadow of the Tomb Raider\\SOTTR.exe|",
                1_570_000,
            ),
            p(
                2238030,
                2238013,
                "/home/u/.local/share/Steam/steamapps/common/Proton - Experimental/files/bin/wineserver|",
                20_000,
            ),
            p(
                2238040,
                2238018,
                "C:\\windows\\system32\\services.exe|",
                500,
            ),
            p(
                2238041,
                2238018,
                "C:\\windows\\system32\\winedevice.exe|",
                900,
            ),
            p(
                2238042,
                2238018,
                "C:\\windows\\system32\\explorer.exe|/desktop",
                800,
            ),
            p(
                2238043,
                2238202,
                "S:\\steamapps\\common\\Shadow of the Tomb Raider\\crashpad_handler.exe|",
                20,
            ),
        ]
    }

    #[test]
    fn the_name_falcond_sees_splits_on_both_separators() {
        assert_eq!(
            falcond_name("S:\\steamapps\\common\\Shadow of the Tomb Raider\\SOTTR.exe"),
            "SOTTR.exe"
        );
        assert_eq!(
            falcond_name("/opt/game/bin/Game-Linux-Shipping"),
            "Game-Linux-Shipping"
        );
        assert_eq!(falcond_name("cs2"), "cs2");
        // Mixed, as Wine sometimes reports: the last separator of either kind.
        assert_eq!(falcond_name("/run/media/g/x\\Bin\\Game.exe"), "Game.exe");
    }

    #[test]
    fn the_real_game_is_found_in_a_proton_tree() {
        let games = identify(&sottr_tree());
        assert_eq!(games.len(), 1, "{games:?}");
        let g = &games[0];
        assert_eq!(g.process_name, "SOTTR.exe");
        assert_eq!(g.pid, 2238202);
        assert_eq!(g.steam_app_id.as_deref(), Some("750920"));
        assert_eq!(g.runtime, Runtime::Proton("Proton - Experimental".into()));
        // The tree is kept, root first, for the details view.
        assert_eq!(g.tree.first().map(|t| t.1.as_str()), Some("reaper"));
    }

    #[test]
    fn machinery_is_never_the_game_however_busy() {
        // wineserver and the Steam shim can out-burn a game that is loading;
        // they must still never be chosen.
        let mut tree = sottr_tree();
        for proc in &mut tree {
            if proc.pid == 2238202 {
                proc.cpu_ticks = 10;
            }
        }
        let games = identify(&tree);
        assert_eq!(games[0].process_name, "SOTTR.exe");
        for name in [
            "wineserver",
            "steam.exe",
            "services.exe",
            "explorer.exe",
            "crashpad_handler.exe",
            "steamwebhelper",
            "REDprelauncher.exe",
        ] {
            assert!(is_infrastructure(name), "{name}");
        }
        assert!(!is_infrastructure("Cyberpunk2077.exe"));
        assert!(
            !is_infrastructure("CrashBandicoot.exe"),
            "a game, not a crash handler"
        );
        assert!(!is_infrastructure("DeadByDaylight-Win64-Shipping.exe"));
    }

    #[test]
    fn steams_installer_script_is_not_the_game() {
        // Rise of the Tomb Raider's first launch runs iscriptevaluator.exe in
        // the game's tree before the game.
        let tree = vec![
            p(
                1,
                0,
                "/h/.local/share/Steam/ubuntu12_32/reaper|SteamLaunch AppId=391220 --",
                1,
            ),
            p(
                2,
                1,
                "python3|/s/steamapps/common/Proton - Experimental/proton waitforexitandrun x",
                5,
            ),
            p(
                3,
                2,
                "C:\\Program Files (x86)\\Steam\\bin\\iscriptevaluator.exe|--get-current-step 391220",
                900,
            ),
        ];
        assert!(
            identify(&tree).is_empty(),
            "the installer helper is not a game"
        );
    }

    #[test]
    fn falconds_system_process_list_is_parsed() {
        let conf = "system_processes = [\n  \"steam.exe\",\n  \"iscriptevaluator.exe\",\n  \"SteelSeriesGG.exe\",\n]\n";
        assert_eq!(
            parse_system_processes(conf),
            vec!["steam.exe", "iscriptevaluator.exe", "steelseriesgg.exe"]
        );
        assert!(parse_system_processes("nothing here").is_empty());
    }

    #[test]
    fn a_launcher_alone_is_not_a_game() {
        let tree = vec![
            p(
                1,
                0,
                "/home/u/.local/share/Steam/ubuntu12_32/reaper|SteamLaunch AppId=1091500 --",
                1,
            ),
            p(
                2,
                1,
                "python3|/s/steamapps/common/Proton 9.0/proton waitforexitandrun x",
                5,
            ),
            p(3, 2, "C:\\Games\\Cyberpunk 2077\\REDprelauncher.exe|", 4000),
        ];
        assert!(
            identify(&tree).is_empty(),
            "the launcher is not what falcond should key on"
        );
    }

    fn known() -> HashMap<String, String> {
        HashMap::from([("supertuxkart".to_owned(), "SuperTuxKart".to_owned())])
    }

    #[test]
    fn a_native_game_outside_steam_is_found_by_its_menu_entry() {
        let procs = vec![
            p(900, 1, "/usr/bin/kwin_wayland", 90_000),
            p(2188074, 2000, "/usr/bin/supertuxkart", 7_600),
        ];
        assert!(
            identify(&procs).is_empty(),
            "without the list nothing is known"
        );
        let found = identify_with(&procs, &known());
        assert_eq!(found.len(), 1);
        let g = &found[0];
        assert_eq!(g.process_name, "supertuxkart");
        assert_eq!(g.display_name, "SuperTuxKart");
        assert_eq!(g.runtime, Runtime::Native);
        assert_eq!(g.pid, 2188074);
    }

    #[test]
    fn a_native_game_is_one_game_and_an_idle_one_is_not_yet() {
        let forks = vec![
            p(10, 1, "/usr/bin/supertuxkart", 5_000),
            p(11, 10, "/usr/bin/supertuxkart", 40),
        ];
        let found = identify_with(&forks, &known());
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].pid, 10);
        let idle = vec![p(10, 1, "/usr/bin/supertuxkart", 20)];
        assert!(identify_with(&idle, &known()).is_empty());
    }

    #[test]
    fn a_steam_tree_is_not_counted_twice_as_a_native_game() {
        let procs = vec![
            p(100, 1, "reaper|SteamLaunch AppId=4242 -- supertuxkart", 1),
            p(
                101,
                100,
                "/home/g/steamapps/common/STK/bin/supertuxkart",
                9_000,
            ),
        ];
        let found = identify_with(&procs, &known());
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].steam_app_id.as_deref(), Some("4242"));
    }

    #[test]
    fn a_native_steam_game_is_native() {
        let tree = vec![
            p(
                1,
                0,
                "/home/u/.local/share/Steam/ubuntu12_32/reaper|SteamLaunch AppId=1234 -- /g/bin/game",
                1,
            ),
            p(2, 1, "/g/steamapps/common/Game/bin/game_x64|", 9000),
        ];
        let g = &identify(&tree)[0];
        assert_eq!(g.runtime, Runtime::Native);
        assert_eq!(g.process_name, "game_x64");
    }

    #[test]
    fn a_wine_game_outside_steam_is_found_when_busy() {
        let procs = vec![
            p(1, 0, "/usr/bin/lutris|", 500),
            p(2, 1, "/home/u/Games/wine/bin/wine64-preloader|", 100),
            p(3, 2, "C:\\Program Files\\Game\\Game.exe|", 12_000),
            p(4, 2, "C:\\windows\\system32\\services.exe|", 800),
            p(5, 2, "C:\\Program Files\\Tool\\idle.exe|", 3),
        ];
        let games = identify(&procs);
        assert_eq!(games.len(), 1, "{games:?}");
        assert_eq!(games[0].process_name, "Game.exe");
        assert_eq!(games[0].runtime, Runtime::Wine);
    }

    #[test]
    fn graphics_are_told_from_mapped_libraries() {
        // Copied from Shadow of the Tomb Raider under Proton Experimental:
        // DX12, which also maps d3d11.dll, and libGL from winex11.
        let sottr = "\
7f01 r-xp 0 00:00 1 /run/host/usr/lib/libGL.so.1.7.0
7f02 r-xp 0 00:00 1 /run/host/usr/lib/libvulkan_radeon.so
7f03 r-xp 0 00:00 1 /g/compatdata/750920/pfx/drive_c/windows/system32/d3d11.dll
7f04 r-xp 0 00:00 1 /g/compatdata/750920/pfx/drive_c/windows/system32/d3d12core.dll
7f05 r-xp 0 00:00 1 /g/compatdata/750920/pfx/drive_c/windows/system32/d3d12.dll
7f06 r-xp 0 00:00 1 /g/compatdata/750920/pfx/drive_c/windows/system32/dxgi.dll
";
        assert_eq!(graphics_from_maps(sottr), Graphics::Vkd3dProton);
        let dx11 = "7f00 r-xp 0 00:00 1 /p/drive_c/windows/system32/d3d11.dll\n7f01 r-xp 0 00:00 1 /usr/lib/libGL.so.1\n";
        assert_eq!(graphics_from_maps(dx11), Graphics::Dxvk);
        let native = "7f00 r-xp 0 00:00 1 /usr/lib/libvulkan_radeon.so\n7f01 r-xp 0 00:00 1 /usr/lib/libGL.so.1\n";
        assert_eq!(graphics_from_maps(native), Graphics::Vulkan);
        assert_eq!(
            graphics_from_maps("7f00 r-xp 0 00:00 1 /usr/lib/libGLX_mesa.so.0\n"),
            Graphics::OpenGl
        );
    }

    #[test]
    fn this_process_has_been_running_a_short_while() {
        let secs = running_for(std::process::id()).expect("readable");
        assert!(secs < 3600, "{secs}");
    }

    #[test]
    fn stat_is_parsed_past_a_comm_with_parentheses() {
        let stat = "2238202 (SOTTR.exe (1)) S 2238018 2238018 0 0 -1 4194304 1 0 0 0 1200 370 0 0 20 0 60 0";
        assert_eq!(parse_stat(stat), Some((2238018, 'S', 1570)));
    }

    #[test]
    fn a_profile_is_matched_on_its_name_field() {
        assert_eq!(
            profile_name_field("name = \"Cyberpunk2077.exe\"\nscx_sched = none\n").as_deref(),
            Some("Cyberpunk2077.exe")
        );
        assert_eq!(profile_name_field("  name=\"cs2\"").as_deref(), Some("cs2"));
        assert_eq!(profile_name_field("scx_sched = none\n"), None);
    }
}
