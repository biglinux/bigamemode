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

use crate::text::N_;

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
/// One directory walk, the owner from the directory itself, two small reads
/// per process of the user's, no forks.
#[must_use]
#[allow(clippy::similar_names)] // pid and ppid are what /proc calls them
pub fn snapshot() -> Vec<Proc> {
    snapshot_in(Path::new("/proc"))
}

/// [`snapshot`] of a `/proc`-like tree at `root`.
///
/// A process's directory belongs to its effective user, which is what
/// `status`'s `Uid:` line was read for; the `stat` of the directory costs
/// no open and no parse, for the hundreds of processes that are not the
/// user's.
#[allow(clippy::similar_names)]
fn snapshot_in(root: &Path) -> Vec<Proc> {
    use std::os::unix::fs::MetadataExt;
    // SAFETY: getuid cannot fail and has no side effects.
    let uid = unsafe { libc::getuid() };
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|entry| {
            let pid: u32 = entry.file_name().to_string_lossy().parse().ok()?;
            let dir = entry.path();
            if entry.metadata().ok()?.uid() != uid {
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
            // No command line: a kernel thread, or a process on its way
            // out. Neither is a game, and an exiting one in a game's tree
            // was once reported as the game, with no name.
            if argv0.is_empty() {
                return None;
            }
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
    /// A short label: product names, and a word marked for translation when
    /// the API is not known.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Vkd3dProton => "VKD3D-Proton · DX12",
            Self::Dxvk => "DXVK · DX11",
            Self::WineD3d => "WineD3D",
            Self::Vulkan => "Vulkan",
            Self::OpenGl => "OpenGL",
            Self::Unknown => N_("Unknown"),
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
    "steam-launch-wrapper",
    "python3",
    "python",
    "sh",
    "bash",
    // pressure-vessel rebuilds the container's library cache with it before
    // the game starts; Debian-based runtimes name the binary ldconfig.real.
    "ldconfig",
    "ldconfig.real",
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
    // The web pages launchers draw their windows with (REDlauncher's Qt,
    // CEF, WebView2): never a game, even while the launcher is the only
    // thing running.
    "qtwebengineprocess.exe",
    "cefsharp.browsersubprocess.exe",
    "msedgewebview2.exe",
    "unrealcefsubprocess.exe",
    // Store clients a game starts through, or that run beside it under
    // Wine: they sign in and update for longer than a profile offer waits.
    "upc.exe",
    "ubisoftconnect.exe",
    "playgtav.exe",
    "galaxyclient.exe",
    "galaxyclient helper.exe",
    "gog galaxy notifications renderer.exe",
    "origin.exe",
    "battle.net.exe",
    "battle.net helper.exe",
    "agent.exe",
    "start_protected_game.exe",
    // Linux launchers, wrappers and the compositing around a game: the menu
    // does not list them as games and the running game is never one of
    // them. Big Game Mode itself is in the menu's Game category too.
    "bigame-ui",
    "lutris",
    "heroic",
    "heroic-run",
    "legendary",
    "gogdl",
    "nile",
    "umu-run",
    "faugus-launcher",
    "bottles",
    "bottles-cli",
    "itch",
    "minigalaxy",
    "gamehub",
    "playonlinux",
    "protonup-qt",
    "protonplus",
    "gamescope",
    "gamescope-wl",
    "gamescopereaper",
    "xwayland",
    "mangohud",
    "mangoapp",
    "mangojuice",
    "goverlay",
    "obs-gamecapture",
    "bwrap",
    "flatpak",
    "flatpak-spawn",
    "xdg-open",
    "steamtinkerlaunch",
    // Game streaming: the game runs on another machine.
    "moonlight",
    "sunshine",
    "big-remote-play",
    "chiaki",
    "chiaki-ng",
    "greenlight",
    "nvidia geforce now",
    "geforcenow",
];

/// Name prefixes of store clients and their services, without `.exe`: EA
/// (`EADesktop`, `EABackgroundService`, `EALocalHostSvc`, `Link2EA`),
/// Ubisoft (`UplayWebCore`, `UbisoftConnect…`), GOG Galaxy, Rockstar
/// (`RockstarService`, `SocialClubHelper`), Epic Online Services, and
/// `BattlEye`'s service under any build name (`BEService_x64`).
const STORE_CLIENT_PREFIXES: &[&str] = &[
    "eadesktop",
    "eabackgroundservice",
    "ealocalhostsvc",
    "eaconnect",
    "eacefsubprocess",
    "link2ea",
    "igoproxy",
    "uplay",
    "ubisoftconnect",
    "upc_",
    "galaxyclient",
    "gogalaxy",
    "socialclub",
    "rockstarservice",
    "rockstarsteamhelper",
    "epiconlineservices",
    "eosoverlayrenderer",
    "beservice",
];

/// Name prefixes of the Steam Linux Runtime's own programs (pressure-vessel
/// and steam-runtime-tools): the container, its logger, and the probes it
/// runs while the container starts, which are named after the architecture
/// they check (`i386-linux-gnu-capsule-capture-libs`, …). While they run they
/// are the only non-Wine processes in the game's tree, and the busiest.
const STEAM_RUNTIME_PREFIXES: &[&str] = &[
    "pressure-vessel-",
    "pv-",
    "srt-",
    "steam-runtime-",
    "x86_64-linux-gnu-",
    "i386-linux-gnu-",
    "aarch64-linux-gnu-",
];

/// falcond's own list of processes that are never games
/// (`/usr/share/falcond/system.conf`), read once.
///
/// Using falcond's list as well as ours means the two can never disagree
/// about what a game is: anything falcond would refuse to profile, Big Game Mode
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
        || STEAM_RUNTIME_PREFIXES.iter().any(|p| lower.starts_with(p))
        // Crash handlers by their usual names -- not any name containing
        // "crash", which would also exclude Crash Bandicoot.
        || ["crashhandler", "crash_handler", "crashreport", "crashpad", "crashsender"]
            .iter()
            .any(|n| lower.contains(n))
        || lower.contains("launcher")
        // REDupdater.exe, EA's EADesktopUpdater…: they update, they do not play.
        || lower.contains("updater")
        || STORE_CLIENT_PREFIXES.iter().any(|p| lower.starts_with(p))
        // BattlEye's starters beside protected games (`DuneSandbox_BE.exe`).
        || lower.ends_with("_be.exe")
        || crate::games::is_support_binary(&lower)
}

/// The processes of a Steam tree the game can be, out of `candidates`.
fn game_pool<'a>(candidates: Vec<&'a &'a Proc>, proton: bool) -> Vec<&'a &'a Proc> {
    // In a Proton tree the game is a Windows binary. A Linux helper inside
    // the container -- an overlay, a wrapper -- must not outrank a game
    // that is still loading and has burned little CPU yet.
    let windows: Vec<&&Proc> = candidates
        .iter()
        .copied()
        .filter(|p| p.argv0.to_ascii_lowercase().ends_with(".exe"))
        .collect();
    if proton && !windows.is_empty() {
        // Steam runs its games from the library. A launcher the game
        // installed in the prefix (CD Projekt's REDlauncher, in
        // C:\users\…\AppData) runs beside it, and its web page
        // (`QtWebEngineProcess.exe`) can be the busiest `.exe` there:
        // while one runs from the library, only those can be the game.
        let library: Vec<&&Proc> = windows
            .iter()
            .copied()
            .filter(|p| in_steam_library(&p.argv0))
            .collect();
        if library.is_empty() { windows } else { library }
    } else {
        candidates
    }
}

/// Whether a Windows process runs from a Steam library
/// (`S:\steamapps\common\…`, `Z:\…/steamapps/common/…`).
fn in_steam_library(argv0: &str) -> bool {
    argv0
        .to_ascii_lowercase()
        .replace('/', "\\")
        .contains("\\steamapps\\common\\")
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
    // `/proc` is not read at one instant: a pid reused while it is read can
    // make a process its own ancestor, and the walk must still end.
    let mut seen: std::collections::HashSet<u32> = out.iter().map(|p| p.pid).collect();
    let mut i = 0;
    while i < out.len() {
        if let Some(children) = by_parent.get(&out[i].pid) {
            out.extend(children.iter().copied().filter(|c| seen.insert(c.pid)));
        }
        i += 1;
    }
    out
}

/// [`identify_with`] knowing no native game, as most tests need.
#[cfg(test)]
#[must_use]
pub fn identify(procs: &[Proc]) -> Vec<GameIdentity> {
    identify_with(procs, &HashMap::new())
}

/// Find the running games in a process list.
///
/// Steam games are found from their reaper, which names the app id; within
/// that tree the game is the busiest process that is not machinery — a
/// launcher can briefly be the only candidate, and it is excluded by name.
/// Wine games outside Steam are found as busy `.exe` processes under Wine.
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
    identify_ranked(procs, native, &Activity::default())
}

/// What tells the process being played from the others beside it: the CPU
/// time each has used since the previous look, and whether it submits GPU
/// work.
///
/// Total CPU time since a process started favours whatever has been open
/// longest: Ubisoft Connect's web page, open for an hour, outweighs the game
/// started a minute ago. And the busiest process is not always the one
/// drawing: the one that submits GPU work is.
#[derive(Default)]
pub struct Activity {
    /// CPU ticks of each process at the previous look.
    pub previous: HashMap<u32, u64>,
    /// Whether a process submits GPU work; `None` when it cannot be told,
    /// so CPU time decides alone.
    pub renders: Option<Box<dyn Fn(u32) -> bool>>,
}

impl Activity {
    /// CPU ticks since the previous look; all of them for a process not seen
    /// then — or seen with more, so a pid reused since.
    fn recent(&self, p: &Proc) -> u64 {
        match self.previous.get(&p.pid) {
            Some(&before) if before <= p.cpu_ticks => p.cpu_ticks - before,
            _ => p.cpu_ticks,
        }
    }

    /// The process being played among `pool`: the one drawing, then the
    /// busiest of late. The GPU is asked only when there is a choice.
    fn choose<'a>(&self, pool: Vec<&'a Proc>) -> Option<&'a Proc> {
        if pool.len() > 1
            && let Some(renders) = &self.renders
        {
            return pool
                .into_iter()
                .map(|p| (renders(p.pid), self.recent(p), p))
                .max_by_key(|(drawing, recent, p)| (*drawing, *recent, p.cpu_ticks))
                .map(|(_, _, p)| p);
        }
        pool.into_iter()
            .max_by_key(|p| (self.recent(p), p.cpu_ticks))
    }
}

/// [`identify_with`], choosing within a tree by [`Activity`].
#[must_use]
pub fn identify_ranked<S: std::hash::BuildHasher>(
    procs: &[Proc],
    native: &HashMap<String, String, S>,
    activity: &Activity,
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
            .filter(|p| !p.argv0.is_empty() && !is_infrastructure(falcond_name(&p.argv0)))
            .collect();
        let pool = game_pool(candidates, proton.is_some());
        let Some(game) = activity.choose(pool.into_iter().copied().collect()) else {
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
    if let Some((at, games)) = cache.as_ref()
        && at.elapsed() < Duration::from_secs(60)
    {
        return games.clone();
    }
    let games = read_native_games();
    *cache = Some((Instant::now(), games.clone()));
    games
}

fn read_native_games() -> HashMap<String, String> {
    let mut profiles = Vec::new();
    let base = Path::new(crate::profiles::SYSTEM_PROFILES_DIR);
    for dir in [base.to_path_buf(), base.join("user")] {
        for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let Ok(content) = std::fs::read_to_string(entry.path()) else {
                continue;
            };
            profiles.extend(profile_name_field(&content));
        }
    }
    native_games_from(&profiles, &crate::games::detect_all())
}

/// Names too common among native programs to say which game runs: a
/// script's or an engine's (`run`, `AppRun`, NW.js's `nw`, LÖVE's `love`).
const GENERIC_NATIVE_NAMES: &[&str] = &[
    "game", "run", "start", "main", "app", "apprun", "nw", "love", "godot", "java",
];

/// The native-game map of [`known_native_games`] from falcond's profile
/// names and the library: menu games and Flatpaks by their program, and the
/// Linux programs of Lutris, Heroic and Faugus games that run here directly.
/// Steam's are found by their reaper, Windows games by Wine.
fn native_games_from(
    profile_names: &[String],
    library: &[crate::games::DetectedGame],
) -> HashMap<String, String> {
    use crate::games::Source;
    let mut games = HashMap::new();
    // Profile names first, so a library title wins.
    for name in profile_names {
        if !name.eq_ignore_ascii_case("proton")
            && !name.to_ascii_lowercase().ends_with(".exe")
            && !is_infrastructure(name)
        {
            games.insert(name.clone(), name.clone());
        }
    }
    let usable = |name: &str| {
        !name.to_ascii_lowercase().ends_with(".exe")
            && !is_infrastructure(name)
            && !crate::games::is_generic_program(name)
            && !GENERIC_NATIVE_NAMES.contains(&name.to_ascii_lowercase().as_str())
    };
    for game in library {
        let names: Vec<&String> = match game.source {
            Source::Native | Source::Flatpak => game.executables.iter().take(1).collect(),
            Source::Lutris | Source::Heroic | Source::Faugus if game.launch_command.is_some() => {
                game.executables.iter().filter(|e| usable(e)).collect()
            }
            _ => Vec::new(),
        };
        for name in names {
            games.insert(name.clone(), game.name.clone());
        }
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

/// How long a game's graphics path, once told, is taken as known: its
/// memory map is a large read, and the watcher asks every few seconds. A
/// game that loads its renderer late is read again after this.
const GRAPHICS_KEPT: std::time::Duration = std::time::Duration::from_secs(30);

/// [`graphics_from_maps`] of process `pid`, read again only after
/// [`GRAPHICS_KEPT`] once it is known.
fn graphics_of(pid: u32, executable: &str) -> Graphics {
    type Seen = HashMap<(u32, String), (std::time::Instant, Graphics)>;
    static SEEN: std::sync::Mutex<Option<Seen>> = std::sync::Mutex::new(None);
    let key = (pid, executable.to_owned());
    let now = std::time::Instant::now();
    let mut seen = SEEN
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let seen = seen.get_or_insert_with(HashMap::new);
    if let Some((at, graphics)) = seen.get(&key)
        && now.saturating_duration_since(*at) < GRAPHICS_KEPT
    {
        return *graphics;
    }
    let graphics = std::fs::read_to_string(format!("/proc/{pid}/maps"))
        .map_or(Graphics::Unknown, |maps| graphics_from_maps(&maps));
    // Only the game being watched is kept.
    seen.retain(|(p, _), _| *p == pid);
    if graphics == Graphics::Unknown {
        seen.remove(&key);
    } else {
        seen.insert(key, (now, graphics));
    }
    graphics
}

/// A GPU a process has open.
#[derive(Debug, Clone, PartialEq, Eq)]
struct OpenGpu {
    /// DRM card (`card0`).
    card: String,
    /// PCI address of the card (`0000:01:00.0`).
    pci_slot: String,
    /// Opened through the NVIDIA driver's own node (`/dev/nvidia0`).
    nvidia_node: bool,
    /// The firmware's boot display adapter.
    boot_vga: bool,
    /// In runtime suspend: nothing renders on it.
    asleep: bool,
    /// GPU time the process has submitted through this card's render nodes,
    /// from their DRM fdinfo; `None` where the driver does not publish it.
    work: Option<u64>,
}

/// GPU time past which the one card with work is taken as the game's even
/// beside an open card that publishes no fdinfo (nouveau): one second, in
/// nanoseconds (cycles on xe, the same order). Less is what enumerating
/// devices and a presentation path submit.
const DOMINANT_WORK: u64 = 1_000_000_000;

/// Which of the GPUs a process has open it renders on.
///
/// A game can hold more than one: on a hybrid laptop DXVK renders on the
/// NVIDIA card through `/dev/nvidia0` while the compositor path keeps the
/// iGPU's render node open; under `DRI_PRIME` both render nodes are open; and
/// any Vulkan program opens every GPU's render node just to enumerate them
/// (`vkcube` on the RX 9060 XT of the reference desktop holds the Vega iGPU's
/// node as well). So: the NVIDIA driver's own node first (the proprietary
/// driver renders only through it); then the card the process has actually
/// submitted work to, from DRM fdinfo (amdgpu, i915, xe publish it) — and
/// while fdinfo is readable but no work has been submitted yet, no answer, so
/// callers fall back to the GPU games are expected on. Beside an open card
/// that publishes no fdinfo (nouveau), the card with work is the answer only
/// once its work is clearly a game's ([`DOMINANT_WORK`]); the silent card may
/// be the one rendering. Where no driver says, the card that is not the boot
/// display adapter (a secondary card is opened on purpose, for offload);
/// then the only one. Among equals, the first.
fn choose_render_gpu(open: &[OpenGpu]) -> Option<&str> {
    if let Some(g) = open.iter().find(|g| g.nvidia_node) {
        return Some(g.card.as_str());
    }
    if open.iter().any(|g| g.work.is_some()) {
        // `max_by_key` keeps the last of equals; reversed, the first wins.
        let busiest = open
            .iter()
            .rev()
            .filter(|g| g.work.is_some_and(|w| w > 0))
            .max_by_key(|g| g.work)?;
        let silent = open.iter().any(|g| g.work.is_none());
        return (!silent || busiest.work.is_some_and(|w| w >= DOMINANT_WORK))
            .then_some(busiest.card.as_str());
    }
    open.iter()
        .find(|g| open.len() > 1 && !g.boot_vga)
        .or_else(|| open.first())
        .map(|g| g.card.as_str())
}

/// The GPUs a process has open, from its GPU descriptors as `(gpu, DRM
/// client id)`: one entry per card and kind of node, with the work of each
/// DRM client counted once. Descriptors dup'd from one DRM file share its
/// client id and its counters (Xwayland holds four of one client on a render
/// node), so summing them would count the same work again.
fn merge_open_gpus(fds: impl IntoIterator<Item = (OpenGpu, Option<u64>)>) -> Vec<OpenGpu> {
    let mut open: Vec<OpenGpu> = Vec::new();
    let mut clients: Vec<(String, u64)> = Vec::new();
    for (gpu, client) in fds {
        if let Some(id) = client {
            if clients.iter().any(|(c, i)| *c == gpu.card && *i == id) {
                continue;
            }
            clients.push((gpu.card.clone(), id));
        }
        if let Some(known) = open
            .iter_mut()
            .find(|g| g.card == gpu.card && g.nvidia_node == gpu.nvidia_node)
        {
            known.work = match (known.work, gpu.work) {
                (Some(a), Some(b)) => Some(a.saturating_add(b)),
                (a, b) => a.or(b),
            };
            continue;
        }
        open.push(gpu);
    }
    open
}

/// The `drm-client-id` of a DRM file descriptor's fdinfo.
fn fdinfo_client(fdinfo: &str) -> Option<u64> {
    fdinfo.lines().find_map(|l| {
        let (key, value) = l.split_once(':')?;
        (key.trim() == "drm-client-id")
            .then(|| value.trim().parse().ok())
            .flatten()
    })
}

/// GPU work submitted through one DRM file descriptor, from its fdinfo
/// (`drm-engine-<engine>: <ns> ns`, or `drm-cycles-<engine>: <n>` on xe),
/// summed over engines. A driver that publishes DRM fdinfo (it has a
/// `drm-client-id`) but lists no engine has had no work — amdgpu omits
/// engines it has not used, as for a game still in its launcher window — so
/// that is zero, not unknown. `None` when the driver publishes no fdinfo.
fn fdinfo_work(fdinfo: &str) -> Option<u64> {
    let values = |prefix: &str| -> Option<u64> {
        let mut found = false;
        let mut sum = 0u64;
        for line in fdinfo.lines() {
            let Some((key, value)) = line.split_once(':') else {
                continue;
            };
            if !key.starts_with(prefix) || key.starts_with("drm-engine-capacity") {
                continue;
            }
            if let Some(n) = value
                .split_whitespace()
                .next()
                .and_then(|v| v.parse::<u64>().ok())
            {
                found = true;
                sum = sum.saturating_add(n);
            }
        }
        found.then_some(sum)
    };
    values("drm-engine-")
        .or_else(|| values("drm-cycles-"))
        .or_else(|| fdinfo.contains("drm-client-id").then_some(0))
}

/// The DRM card under a PCI device directory.
fn card_of_device(device: &std::path::Path) -> Option<String> {
    std::fs::read_dir(device.join("drm"))
        .ok()?
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .find(|n| crate::hardware::is_card_node(n))
}

/// The PCI device of NVIDIA minor `minor`, from the driver's own records
/// (`/proc/driver/nvidia/gpus/<slot>/information`, `Device Minor: N`).
fn nvidia_device(minor: u32) -> Option<std::path::PathBuf> {
    std::fs::read_dir("/proc/driver/nvidia/gpus")
        .ok()?
        .flatten()
        .find_map(|e| {
            let info = std::fs::read_to_string(e.path().join("information")).ok()?;
            let m = info.lines().find_map(|l| {
                l.strip_prefix("Device Minor:")
                    .and_then(|v| v.trim().parse::<u32>().ok())
            })?;
            (m == minor).then(|| std::path::Path::new("/sys/bus/pci/devices").join(e.file_name()))
        })
}

/// The DRM card a process renders on, from the GPU device nodes it has open.
fn render_card(pid: u32) -> Option<String> {
    let fds = std::fs::read_dir(format!("/proc/{pid}/fd")).ok()?;
    let mut held: Vec<(OpenGpu, Option<u64>)> = Vec::new();
    for fd in fds.flatten() {
        let Ok(target) = std::fs::read_link(fd.path()) else {
            continue;
        };
        let Some(name) = target.file_name().map(|n| n.to_string_lossy().into_owned()) else {
            continue;
        };
        let (device, nvidia_node) = if name.starts_with("renderD") {
            // /sys/class/drm/renderD128/device → the PCI device.
            let Ok(d) = std::fs::canonicalize(format!("/sys/class/drm/{name}/device")) else {
                continue;
            };
            (d, false)
        } else if let Some(minor) = name
            .strip_prefix("nvidia")
            .and_then(|m| m.parse::<u32>().ok())
        {
            let Some(d) = nvidia_device(minor) else {
                continue;
            };
            (d, true)
        } else {
            continue;
        };
        let Some(card) = card_of_device(&device) else {
            continue;
        };
        let fdinfo = if nvidia_node {
            None
        } else {
            std::fs::read_to_string(format!(
                "/proc/{pid}/fdinfo/{}",
                fd.file_name().to_string_lossy()
            ))
            .ok()
        };
        let attr = |f: &str| std::fs::read_to_string(device.join(f)).unwrap_or_default();
        let gpu = OpenGpu {
            card,
            pci_slot: device
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            nvidia_node,
            boot_vga: attr("boot_vga").trim() == "1",
            asleep: attr("power/runtime_status").trim() == "suspended",
            work: fdinfo.as_deref().and_then(fdinfo_work),
        };
        held.push((gpu, fdinfo.as_deref().and_then(fdinfo_client)));
    }
    let open = drop_enumerated_only(
        merge_open_gpus(held),
        pid,
        crate::gpu_telemetry::nvidia_graphics_pids,
    );
    choose_render_gpu(&open).map(str::to_owned)
}

/// Remove the NVIDIA cards `pid` has open without rendering on them.
///
/// Enumerating Vulkan devices opens `/dev/nvidia0` and the card's render
/// node, so a game on the integrated GPU holds NVIDIA nodes too (checked with
/// `vkcube --gpu_number 0` on the lab laptop: seven `/dev/nvidia0` fds). The
/// driver's list of processes with a graphics context tells them apart; when
/// it cannot be read, nothing is removed. A card in runtime suspend renders
/// nothing and is not asked: asking NVML would wake it, every few seconds,
/// for as long as the game runs.
fn drop_enumerated_only(
    open: Vec<OpenGpu>,
    pid: u32,
    contexts: impl Fn(&str) -> Option<Vec<u32>>,
) -> Vec<OpenGpu> {
    let idle: Vec<String> = open
        .iter()
        .filter(|g| g.nvidia_node)
        .filter(|g| g.asleep || contexts(&g.pci_slot).is_some_and(|pids| !pids.contains(&pid)))
        .map(|g| g.card.clone())
        .collect();
    if idle.is_empty() {
        return open;
    }
    open.into_iter()
        .filter(|g| !idle.contains(&g.card))
        .collect()
}

/// Fill in what the launcher and the live process can say.
fn enrich(mut game: GameIdentity) -> GameIdentity {
    game.graphics = graphics_of(game.pid, &game.executable);
    game.render_card = render_card(game.pid);
    if let Some(id) = game.steam_app_id.clone()
        && let Some(home) = std::env::var_os("HOME").map(PathBuf::from)
    {
        for library in crate::games::steam_libraries(&home) {
            let steamapps = library.join("steamapps");
            let Ok(text) = std::fs::read_to_string(steamapps.join(format!("appmanifest_{id}.acf")))
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
    game
}

/// The running game, if there is one.
///
/// When more than one is found, the one drawing wins, then the busiest of
/// late ([`Activity`]): that is the one being played, and it is also the one
/// falcond will be holding a profile for.
#[must_use]
pub fn detect() -> Option<GameIdentity> {
    static BASELINES: std::sync::Mutex<Baselines> = std::sync::Mutex::new(Baselines {
        older: None,
        newer: None,
    });
    let procs = snapshot();
    let ticks: HashMap<u32, u64> = procs.iter().map(|p| (p.pid, p.cpu_ticks)).collect();
    let previous = BASELINES
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .take(std::time::Instant::now(), &ticks);
    let activity = Activity {
        previous,
        renders: Some(Box::new(|pid| submits_gpu_work(Path::new("/proc"), pid))),
    };
    let games = identify_ranked(&procs, &known_native_games(), &activity);
    let by_pid: HashMap<u32, &Proc> = procs.iter().map(|p| (p.pid, p)).collect();
    let chosen = if games.len() > 1 {
        games.into_iter().max_by_key(|g| {
            let p = by_pid.get(&g.pid);
            (
                submits_gpu_work(Path::new("/proc"), g.pid),
                p.map_or(0, |p| activity.recent(p)),
                p.map_or(0, |p| p.cpu_ticks),
            )
        })
    } else {
        games.into_iter().next()
    };
    chosen.map(enrich)
}

/// The shortest time CPU use is compared over: shorter, a tick or two of
/// noise decides.
const MIN_WINDOW: std::time::Duration = std::time::Duration::from_secs(2);

/// The CPU ticks of earlier looks, two kept so that a look soon after
/// another still compares over at least [`MIN_WINDOW`].
struct Baselines {
    older: Option<(std::time::Instant, HashMap<u32, u64>)>,
    newer: Option<(std::time::Instant, HashMap<u32, u64>)>,
}

impl Baselines {
    /// The ticks to compare `current` with — the newest look at least
    /// [`MIN_WINDOW`] old, else the oldest kept — and `current` remembered.
    fn take(&mut self, now: std::time::Instant, current: &HashMap<u32, u64>) -> HashMap<u32, u64> {
        let old_enough = |b: &Option<(std::time::Instant, HashMap<u32, u64>)>| {
            b.as_ref()
                .is_some_and(|(at, _)| now.saturating_duration_since(*at) >= MIN_WINDOW)
        };
        let pick = if old_enough(&self.newer) {
            &self.newer
        } else if self.older.is_some() {
            &self.older
        } else {
            &self.newer
        };
        let previous = pick.as_ref().map(|(_, t)| t.clone()).unwrap_or_default();
        if self.newer.is_none() || old_enough(&self.newer) {
            self.older = self.newer.take();
            self.newer = Some((now, current.clone()));
        }
        previous
    }
}

/// Whether process `pid` of the `/proc`-like tree at `root` submits GPU
/// work: it holds an NVIDIA device node (the proprietary driver publishes no
/// fdinfo, and only opens one to draw or compute), or a DRM render node
/// whose fdinfo shows engine time. A launcher's or a helper's process has
/// none, or none yet.
fn submits_gpu_work(root: &Path, pid: u32) -> bool {
    let dir = root.join(pid.to_string());
    let Ok(fds) = std::fs::read_dir(dir.join("fd")) else {
        return false;
    };
    // A game holds a few thousand descriptors at most.
    fds.flatten().take(16_384).any(|fd| {
        let Ok(target) = std::fs::read_link(fd.path()) else {
            return false;
        };
        let Some(name) = target.file_name().and_then(|n| n.to_str()) else {
            return false;
        };
        if name
            .strip_prefix("nvidia")
            .is_some_and(|m| !m.is_empty() && m.bytes().all(|b| b.is_ascii_digit()))
        {
            return true;
        }
        name.starts_with("renderD")
            && std::fs::read_to_string(dir.join("fdinfo").join(fd.file_name()))
                .ok()
                .as_deref()
                .and_then(fdinfo_work)
                .is_some_and(|work| work > 0)
    })
}

/// What is really in effect inside a running game, read from the game
/// process and the system while it runs — never from settings alone.
///
/// A saved setting says what was asked for; this says what the game got. A
/// layer counts only when it is mapped in the game process (`MangoHud`,
/// vkBasalt, lsfg-vk), frame generation only when lsfg-vk is mapped *and*
/// has an entry for the game, Gamescope only when it is in the game's
/// process tree, a scheduler only when the kernel reports one loaded.
// Independent facts read one by one; grouping them to please the lint would
// only add indirection.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InGame {
    /// `MangoHud`'s layer or preload library is in the game.
    pub mangohud: bool,
    /// vkBasalt's post-processing layer is in the game.
    pub vkbasalt: bool,
    /// vkBasalt is in the Gamescope around the game instead: Gamescope, a
    /// Vulkan program itself, takes `ENABLE_VKBASALT` from Steam's launch
    /// options and filters the image it composites (seen with Shadow of the
    /// Tomb Raider: the layer mapped in `gamescope`, not in the game).
    pub vkbasalt_in_gamescope: bool,
    /// lsfg-vk generates frames for the game: its multiplier.
    pub frame_generation: Option<u32>,
    /// lsfg-vk's file changed after the game started. lsfg-vk applies a new
    /// multiplier live, but neither starts nor stops generating for a game
    /// already running (checked on the reference desktop), so what the file
    /// says now may not be what the game does until its next start.
    pub frame_generation_changed: bool,
    /// The game runs inside Gamescope.
    pub gamescope: bool,
    /// The sched-ext scheduler the kernel has loaded, if any.
    pub scheduler: Option<String>,
    /// The frame cap in the game's own environment (a Turbo preset's
    /// `VKD3D_FRAME_RATE`, or DXVK's `maxFrameRate` in `DXVK_CONFIG`).
    pub frame_cap: Option<u32>,
}

/// The frame cap an environment asks DXVK or VKD3D-Proton for.
#[must_use]
pub fn frame_cap_in(environ: &[u8]) -> Option<u32> {
    environ
        .split(|b| *b == 0)
        .filter_map(|e| std::str::from_utf8(e).ok())
        .find_map(|e| {
            if let Some(v) = e.strip_prefix("VKD3D_FRAME_RATE=") {
                return v.trim().parse().ok();
            }
            let config = e.strip_prefix("DXVK_CONFIG=")?;
            config.split(';').find_map(|kv| {
                let (k, v) = kv.split_once('=')?;
                k.trim()
                    .ends_with("maxFrameRate")
                    .then(|| v.trim().parse().ok())
                    .flatten()
            })
        })
        .filter(|fps| *fps > 0)
}

/// The Gamescope process names: the X11 and Wayland builds, and the reaper
/// Gamescope starts the game under.
const GAMESCOPE_NAMES: &[&str] = &["gamescope", "gamescope-wl", "gamescopereaper"];

/// Whether the game runs inside Gamescope, from the process names around
/// it (`names`: its tree and its ancestors) and its environment.
///
/// Where Gamescope sits depends on who started the game: Big Game Mode's own
/// launch puts it above the game; Steam's launch options put it above
/// Steam's reaper, so it is an ancestor of the whole tree, never inside it
/// (seen on the reference desktop: `steam → gamescope-wl → gamescopereaper
/// → reaper → … → SOTTR.exe`). Gamescope also hands the game its own
/// display (`GAMESCOPE_WAYLAND_DISPLAY`), which settles it whatever the
/// tree looks like.
#[must_use]
pub fn wrapped_by_gamescope<'a>(names: impl IntoIterator<Item = &'a str>, environ: &[u8]) -> bool {
    names.into_iter().any(|n| GAMESCOPE_NAMES.contains(&n))
        || environ
            .split(|b| *b == 0)
            .any(|e| e.starts_with(b"GAMESCOPE_WAYLAND_DISPLAY="))
}

/// `pid`'s ancestors, nearest first, up to init: (pid, name).
fn ancestors(process: u32) -> Vec<(u32, String)> {
    let mut names = Vec::new();
    let mut current = process;
    for _ in 0..64 {
        let Some((_, _, parent)) = std::fs::read_to_string(format!("/proc/{current}/stat"))
            .ok()
            .and_then(|stat| parse_stat_name(&stat))
        else {
            break;
        };
        if parent <= 1 {
            break;
        }
        current = parent;
        if let Some((_, name, _)) = std::fs::read_to_string(format!("/proc/{current}/stat"))
            .ok()
            .and_then(|stat| parse_stat_name(&stat))
        {
            names.push((current, name));
        }
    }
    names
}

/// `(process, comm, parent)` from a `/proc/<pid>/stat` line; the name is in
/// parentheses and may hold spaces.
fn parse_stat_name(stat: &str) -> Option<(u32, String, u32)> {
    let open = stat.find('(')?;
    let close = stat.rfind(')')?;
    let process = stat[..open].trim().parse().ok()?;
    let name = stat[open + 1..close].to_owned();
    let parent = stat[close + 1..].split_whitespace().nth(1)?.parse().ok()?;
    Some((process, name, parent))
}

/// Read [`InGame`] for `game`.
#[must_use]
pub fn in_game(game: &GameIdentity) -> InGame {
    let maps = std::fs::read_to_string(format!("/proc/{}/maps", game.pid)).unwrap_or_default();
    let layers = layers_from_maps(&maps);
    let environ = std::fs::read(format!("/proc/{}/environ", game.pid)).unwrap_or_default();
    let ancestors = ancestors(game.pid);
    let gamescope = wrapped_by_gamescope(
        game.tree
            .iter()
            .map(|(_, n)| n.as_str())
            .chain(ancestors.iter().map(|(_, n)| n.as_str())),
        &environ,
    );
    // The Gamescope itself (not its reaper), in the tree or above it.
    let vkbasalt_in_gamescope = game
        .tree
        .iter()
        .chain(ancestors.iter())
        .filter(|(_, n)| n == "gamescope" || n == "gamescope-wl")
        .any(|(pid, _)| {
            std::fs::read_to_string(format!("/proc/{pid}/maps"))
                .is_ok_and(|m| layers_from_maps(&m).vkbasalt)
        });
    let frame_generation = layers
        .lsfg
        .then(|| crate::fg::read_profile(&game.process_name).0);
    InGame {
        mangohud: layers.mangohud,
        vkbasalt: layers.vkbasalt,
        vkbasalt_in_gamescope,
        frame_generation: frame_generation.filter(|m| *m > 1 && crate::fg::is_lossless_dll_ready()),
        frame_generation_changed: layers.lsfg
            && changed_since_start(&crate::fg::config_path(), game.pid),
        gamescope,
        scheduler: loaded_scheduler(),
        frame_cap: frame_cap_in(&environ),
    }
}

/// Whether `file` was modified after process `pid` started.
fn changed_since_start(file: &Path, pid: u32) -> bool {
    let (Some(age), Ok(modified)) = (
        running_for(pid),
        std::fs::metadata(file).and_then(|m| m.modified()),
    ) else {
        return false;
    };
    let started = std::time::SystemTime::now()
        .checked_sub(std::time::Duration::from_secs(age))
        .unwrap_or(std::time::UNIX_EPOCH);
    modified > started
}

/// Vulkan layers and preload libraries mapped in a process.
#[derive(Debug, Default, PartialEq, Eq)]
struct Layers {
    mangohud: bool,
    vkbasalt: bool,
    lsfg: bool,
}

fn layers_from_maps(maps: &str) -> Layers {
    let mut l = Layers::default();
    for name in maps
        .lines()
        .filter_map(|line| line.split_whitespace().nth(5))
        .filter_map(|path| path.rsplit('/').next())
    {
        let name = name.to_ascii_lowercase();
        l.mangohud |= name.starts_with("libmangohud");
        l.vkbasalt |= name.starts_with("libvkbasalt");
        l.lsfg |= name.starts_with("liblsfg-vk");
    }
    l
}

/// The sched-ext scheduler loaded now (`/sys/kernel/sched_ext`), without
/// the `_1.2.3` version suffix some schedulers add.
#[must_use]
pub fn loaded_scheduler() -> Option<String> {
    let state = std::fs::read_to_string("/sys/kernel/sched_ext/state").ok()?;
    if state.trim() != "enabled" {
        return None;
    }
    let ops = std::fs::read_to_string("/sys/kernel/sched_ext/root/ops").ok()?;
    let ops = ops.trim();
    (!ops.is_empty()).then(|| ops.split('_').next().unwrap_or(ops).to_owned())
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
    matching_profile_in(
        Path::new(crate::profiles::SYSTEM_PROFILES_DIR),
        process_name,
        profile_mode,
    )
}

/// [`matching_profile`] among the profiles under `base`.
///
/// falcond loads the mode's profiles, then lets each user profile override
/// the one of the same name, compared without regard to case, and matches a
/// process by exact name, then without regard to case; `Proton`, in any
/// case, is its fallback. So a user profile matching the process is the one
/// in effect — over a system one, or as its own.
fn matching_profile_in(
    base: &Path,
    process_name: &str,
    profile_mode: &str,
) -> Option<ProfileMatch> {
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
            if let Some(name) = profile_name_field(&content)
                && !name.eq_ignore_ascii_case("proton")
            {
                candidates.push(ProfileMatch { name, path, user });
            }
        }
    }
    let matching = |c: &&ProfileMatch| c.name.eq_ignore_ascii_case(process_name);
    candidates
        .iter()
        .filter(matching)
        .find(|c| c.user)
        .or_else(|| candidates.iter().find(|c| c.name == process_name))
        .or_else(|| candidates.iter().find(matching))
        .cloned()
}

#[cfg(test)]
// PIDs are copied verbatim from a real process list; pid and ppid are the
// /proc names.
#[allow(clippy::unreadable_literal, clippy::similar_names)]
mod tests {
    use super::*;

    fn open_gpu(card: &str, nvidia_node: bool, boot_vga: bool) -> OpenGpu {
        OpenGpu {
            card: card.into(),
            pci_slot: if card == "card0" {
                "0000:01:00.0"
            } else {
                "0000:00:02.0"
            }
            .into(),
            nvidia_node,
            boot_vga,
            asleep: false,
            work: None,
        }
    }

    #[test]
    fn layers_come_from_the_libraries_the_game_mapped() {
        // Excerpt of Shadow of the Tomb Raider's maps under Proton, with
        // MangoHud through the Steam runtime's /run/host.
        let maps = "\
7f00 r-xp 0 0:1 1 /run/host/usr/lib/mangohud/libMangoHud.so
7f01 r-xp 0 0:1 2 /usr/lib/libvkbasalt.so
7f02 r-xp 0 0:1 3 /usr/lib/liblsfg-vk.so
7f03 r-xp 0 0:1 4 /home/u/Proton/files/lib/vkd3d/x86_64-windows/d3d12.dll";
        assert_eq!(
            layers_from_maps(maps),
            Layers {
                mangohud: true,
                vkbasalt: true,
                lsfg: true
            }
        );
        assert_eq!(
            layers_from_maps("7f00 r-xp 0 0:1 1 /usr/lib/libc.so.6"),
            Layers::default()
        );
    }

    fn with_work(mut g: OpenGpu, work: u64) -> OpenGpu {
        g.work = Some(work);
        g
    }

    #[test]
    fn a_vulkan_game_on_an_amd_desktop_renders_where_it_submits_work() {
        // The reference desktop: card1 = RX 9060 XT (boot display, drives
        // the monitors), card0 = the 5700G's Vega. Enumerating Vulkan devices
        // opens the Vega's render node too; only the RX has GPU time.
        let open = [
            with_work(open_gpu("card1", false, true), 18_885_878),
            with_work(open_gpu("card0", false, false), 0),
        ];
        assert_eq!(choose_render_gpu(&open), Some("card1"));
    }

    #[test]
    fn work_decides_under_dri_prime_too() {
        let open = [
            with_work(open_gpu("card0", false, true), 1_000),
            with_work(open_gpu("card1", false, false), 9_000_000),
        ];
        assert_eq!(choose_render_gpu(&open), Some("card1"));
    }

    #[test]
    fn before_any_work_there_is_no_answer_yet() {
        // Just started: fdinfo readable, nothing submitted anywhere yet;
        // the fd order (Vega first, as with vkcube) must not decide.
        let open = [
            with_work(open_gpu("card0", false, false), 0),
            with_work(open_gpu("card1", false, true), 0),
        ];
        assert_eq!(choose_render_gpu(&open), None);
    }

    #[test]
    fn fdinfo_engines_are_summed_and_capacity_is_not_work() {
        let amdgpu = "drm-driver:\tamdgpu\ndrm-pdev:\t0000:03:00.0\ndrm-memory-vram:\t12192 KiB\n\
                      drm-engine-gfx:\t18885878 ns\ndrm-engine-compute:\t100 ns\n";
        assert_eq!(fdinfo_work(amdgpu), Some(18_885_978));
        let enumerated = "drm-driver:\tamdgpu\ndrm-client-id:\t2188\ndrm-pdev:\t0000:0a:00.0\n\
                          drm-memory-vram:\t12 KiB\n";
        assert_eq!(fdinfo_work(enumerated), Some(0));
        assert_eq!(fdinfo_work("pos:\t0\nflags:\t02100002\n"), None);
        let i915 = "drm-engine-render:\t500 ns\ndrm-engine-capacity-render:\t1\n";
        assert_eq!(fdinfo_work(i915), Some(500));
        let xe = "drm-cycles-rcs:\t42\ndrm-total-cycles-rcs:\t9000\n";
        assert_eq!(fdinfo_work(xe), Some(42));
    }

    #[test]
    fn nvidia_nodes_held_only_for_enumeration_do_not_make_it_the_render_gpu() {
        // vkcube --gpu_number 0 on the lab laptop: Intel renders, yet
        // /dev/nvidia0 and renderD129 (card0) are open beside renderD128.
        let held = || {
            vec![
                open_gpu("card0", true, false),
                open_gpu("card0", false, false),
                open_gpu("card1", false, true),
            ]
        };
        let no_context = drop_enumerated_only(held(), 42, |_| Some(vec![7, 9]));
        assert_eq!(choose_render_gpu(&no_context), Some("card1"));
        // The process does hold a context on the GeForce: it renders there.
        let context = drop_enumerated_only(held(), 42, |_| Some(vec![42]));
        assert_eq!(choose_render_gpu(&context), Some("card0"));
        // NVML unavailable: nothing is removed, as before.
        let unknown = drop_enumerated_only(held(), 42, |_| None);
        assert_eq!(choose_render_gpu(&unknown), Some("card0"));
    }

    #[test]
    fn a_suspended_geforce_is_idle_and_nvml_is_not_asked() {
        // A Turing laptop with RTD3: the game on the iGPU enumerated Vulkan
        // devices, so it holds /dev/nvidia0, and the GeForce went to sleep.
        let mut nv = open_gpu("card0", true, false);
        nv.asleep = true;
        let open = vec![nv, with_work(open_gpu("card1", false, true), 5_000)];
        let kept = drop_enumerated_only(open, 42, |_| -> Option<Vec<u32>> {
            panic!("NVML would wake the GPU")
        });
        assert_eq!(choose_render_gpu(&kept), Some("card1"));
    }

    #[test]
    fn dupd_descriptors_of_one_drm_client_count_its_work_once() {
        // Xwayland: four fds of client 77 on renderD128 (card1), each
        // showing the same counters, beside 3 ms of its own on card0.
        let fd = |card: &str, work: u64, client: u64| {
            (
                with_work(open_gpu(card, false, card == "card1"), work),
                Some(client),
            )
        };
        let mut fds: Vec<_> = (0..4).map(|_| fd("card1", 2_000_000, 77)).collect();
        fds.push(fd("card0", 3_000_000, 78));
        let open = merge_open_gpus(fds);
        assert_eq!(open.len(), 2);
        assert_eq!(open[0].work, Some(2_000_000));
        assert_eq!(choose_render_gpu(&open), Some("card0"));
        // Two clients on one card are two DRM files: both count.
        let open = merge_open_gpus([fd("card1", 5, 1), fd("card1", 7, 2)]);
        assert_eq!(open[0].work, Some(12));
        assert_eq!(
            fdinfo_client("drm-driver:\tamdgpu\ndrm-client-id:\t77\n"),
            Some(77)
        );
        assert_eq!(fdinfo_client("pos:\t0\n"), None);
    }

    #[test]
    fn a_little_igpu_work_does_not_outvote_a_card_that_publishes_no_fdinfo() {
        // Intel iGPU (fdinfo) + GeForce on nouveau (none): the game opened
        // both; a little work on the iGPU proves nothing about where it
        // renders.
        let nouveau = open_gpu("card0", false, false);
        let open = [
            nouveau.clone(),
            with_work(open_gpu("card1", false, true), 4_000_000),
        ];
        assert_eq!(choose_render_gpu(&open), None);
        // Seconds of GPU time on the iGPU are a game's.
        let open = [
            nouveau,
            with_work(open_gpu("card1", false, true), 6_000_000_000),
        ];
        assert_eq!(choose_render_gpu(&open), Some("card1"));
    }

    #[test]
    fn the_first_of_two_equally_busy_cards_is_the_answer() {
        let open = [
            with_work(open_gpu("card0", false, false), 500),
            with_work(open_gpu("card1", false, true), 500),
        ];
        assert_eq!(choose_render_gpu(&open), Some("card0"));
    }

    #[test]
    fn a_game_on_the_geforce_of_a_hybrid_laptop_renders_on_the_geforce() {
        // The lab laptop: card1 = i915 (boot display, drives eDP), card0 =
        // GTX 1050 Ti. DXVK holds /dev/nvidia0; the iGPU's render node is
        // open too, and comes first in the fd table.
        let open = [
            open_gpu("card1", false, true),
            open_gpu("card0", true, false),
        ];
        assert_eq!(choose_render_gpu(&open), Some("card0"));
    }

    #[test]
    fn under_dri_prime_the_secondary_card_is_the_one_rendering() {
        let open = [
            open_gpu("card0", false, true),
            open_gpu("card1", false, false),
        ];
        assert_eq!(choose_render_gpu(&open), Some("card1"));
    }

    #[test]
    fn one_open_render_node_is_the_answer_whatever_it_is() {
        assert_eq!(
            choose_render_gpu(&[open_gpu("card1", false, true)]),
            Some("card1")
        );
        assert_eq!(choose_render_gpu(&[]), None);
    }

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
    fn the_steam_runtime_starting_up_is_not_the_game() {
        // Shadow of the Tomb Raider on the lab laptop: Steam runs the install
        // script through `proton run`, so there is no `waitforexitandrun` to
        // name the Proton tool, and the runtime's probes are the busiest
        // processes left in the tree.
        let rt = "/usr/lib/pressure-vessel/from-host/libexec/steam-runtime-tools-0";
        let tree = vec![
            p(
                1,
                0,
                "/h/.local/share/Steam/ubuntu12_32/reaper|SteamLaunch AppId=750920 Install=1 --",
                1,
            ),
            p(
                2,
                1,
                "/s/steamapps/common/SteamLinuxRuntime_4/pressure-vessel/libexec/steam-runtime-tools-0/srt-bwrap|--args 26",
                2,
            ),
            p(3, 2, &format!("{rt}/pv-adverb|--generate-locales"), 3),
            p(
                4,
                3,
                &format!("{rt}/i386-linux-gnu-capsule-capture-libs|--dest=/tmp"),
                60,
            ),
            p(5, 3, &format!("{rt}/x86_64-linux-gnu-check-vulkan|"), 40),
            p(6, 3, &format!("{rt}/srt-logger|--sh-syntax"), 10),
            p(
                7,
                3,
                "/s/SteamLinuxRuntime_4/pressure-vessel/bin/steam-runtime-launcher-service|",
                5,
            ),
            p(
                8,
                3,
                "python3|/s/steamapps/common/Proton - Experimental/proton run /h/.local/share/Steam/legacycompat/iscriptevaluator.exe",
                30,
            ),
            p(9, 8, "C:\\windows\\system32\\wineboot.exe|--init", 200),
            p(
                10,
                8,
                "C:\\Program Files (x86)\\Steam\\legacycompat\\iscriptevaluator.exe|legacycompat\\evaluatorscript_750920.vdf",
                100,
            ),
        ];
        assert!(identify(&tree).is_empty(), "{:?}", identify(&tree));
        assert!(!is_infrastructure("SOTTR.exe"));
        assert!(!is_infrastructure("supertuxkart"));
    }

    #[test]
    fn the_containers_ldconfig_is_not_the_game() {
        // BLOODSTRIKE on SteamLinuxRuntime_4 (a user's log): pressure-vessel
        // rebuilds the container's library cache before Proton starts, and
        // its ldconfig, waiting on a busy lock, was announced as the game.
        let tree = vec![
            p(
                1,
                0,
                "/h/.local/share/Steam/ubuntu12_32/reaper|SteamLaunch AppId=3199170 --",
                1,
            ),
            p(
                2,
                1,
                "/s/steamapps/common/SteamLinuxRuntime_4/pressure-vessel/bin/pressure-vessel-wrap|--",
                5,
            ),
            p(
                3,
                2,
                "/sbin/ldconfig|-X -C /run/pressure-vessel/ldso/ld.so.cache",
                80,
            ),
            p(4, 2, "/usr/sbin/ldconfig.real|-p", 20),
        ];
        assert!(identify(&tree).is_empty(), "{:?}", identify(&tree));
    }

    #[test]
    fn a_process_without_a_command_line_is_not_the_game() {
        // A process on its way out has an empty cmdline. Left in the pool it
        // was once chosen, and Home announced a game with no name.
        let tree = vec![
            p(
                1,
                0,
                "/h/.local/share/Steam/ubuntu12_32/reaper|SteamLaunch AppId=750920 --",
                1,
            ),
            p(2, 1, "", 500),
            p(
                3,
                1,
                "/s/SteamLinuxRuntime_4/pressure-vessel/bin/srt-logger|",
                5,
            ),
        ];
        assert!(identify(&tree).is_empty(), "{:?}", identify(&tree));
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

    #[test]
    fn gamescope_is_seen_above_steams_reaper_and_in_the_games_environment() {
        // Steam's launch options: Gamescope above the reaper (an ancestor).
        let tree = ["reaper", "pv-adverb", "python3", "SOTTR.exe"];
        let ancestors = ["gamescopereaper", "gamescope-wl", "steam", "bash"];
        assert!(!wrapped_by_gamescope(tree, b""));
        assert!(wrapped_by_gamescope(tree.into_iter().chain(ancestors), b""));
        // Big Game Mode's own launch: Gamescope in the tree.
        assert!(wrapped_by_gamescope(["gamescope", "Game.exe"], b""));
        // Gamescope's own display in the game's environment settles it.
        assert!(wrapped_by_gamescope(
            ["Game.exe"],
            b"HOME=/h\0GAMESCOPE_WAYLAND_DISPLAY=/run/x\0"
        ));
        assert!(!wrapped_by_gamescope(["Game.exe"], b"DISPLAY=:0\0"));
    }

    #[test]
    fn a_stat_line_gives_the_name_and_parent_even_with_spaces_in_the_name() {
        assert_eq!(
            parse_stat_name("1355477 (SOTTR.exe) S 1355400 1 2 0 -1"),
            Some((1_355_477, "SOTTR.exe".into(), 1_355_400))
        );
        assert_eq!(
            parse_stat_name("7 (my game (x)) R 3 7 7 0 -1"),
            Some((7, "my game (x)".into(), 3))
        );
        assert_eq!(parse_stat_name("garbage"), None);
    }

    #[test]
    fn the_frame_cap_is_read_from_the_games_environment() {
        let env = b"HOME=/h\0VKD3D_FRAME_RATE=60\0X=1\0";
        assert_eq!(frame_cap_in(env), Some(60));
        let env = b"DXVK_CONFIG=dxgi.maxFrameRate = 60; d3d9.maxFrameRate = 60\0";
        assert_eq!(frame_cap_in(env), Some(60));
        assert_eq!(frame_cap_in(b"DXVK_CONFIG=dxgi.syncInterval = 0\0"), None);
        assert_eq!(frame_cap_in(b"VKD3D_FRAME_RATE=0\0"), None);
        assert_eq!(frame_cap_in(b""), None);
    }

    #[test]
    fn a_launchers_web_page_is_not_the_game() {
        // Cyberpunk 2077 on the reference desktop: REDlauncher, installed in
        // the prefix, draws its window with Qt's web engine, busier than
        // anything else before the game starts.
        let launcher = |pid, ppid, exe: &str, cpu| {
            p(
                pid,
                ppid,
                &format!(
                    "C:\\users\\steamuser\\AppData\\Local\\Programs\\CD Projekt Red\\REDlauncher\\{exe}|"
                ),
                cpu,
            )
        };
        let mut tree = vec![
            p(
                1,
                0,
                "/home/u/.local/share/Steam/ubuntu12_32/reaper|SteamLaunch AppId=1091500 --",
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
                "S:\\steamapps\\common\\Cyberpunk 2077\\REDprelauncher.exe|",
                50,
            ),
            launcher(4, 3, "REDlauncher.exe", 900),
            launcher(5, 4, "QtWebEngineProcess.exe", 4000),
            launcher(6, 4, "REDupdater.exe", 3000),
        ];
        assert!(identify(&tree).is_empty(), "{:?}", identify(&tree));
        // The game starts from the library: it is the game, however busy the
        // launcher's page still is.
        tree.push(p(
            7,
            4,
            "S:\\steamapps\\common\\Cyberpunk 2077\\bin\\x64\\Cyberpunk2077.exe|",
            200,
        ));
        tree.push(launcher(8, 4, "Other.exe", 9000));
        let games = identify(&tree);
        assert_eq!(games.len(), 1);
        assert_eq!(games[0].process_name, "Cyberpunk2077.exe");
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

    // ── Regression: store clients, wrappers, ranking ──────────────────────

    fn reaper(pid: u32, app: &str) -> Proc {
        p(
            pid,
            0,
            &format!("/h/.local/share/Steam/ubuntu12_32/reaper|SteamLaunch AppId={app} --"),
            1,
        )
    }

    #[test]
    fn a_tree_with_a_cycle_still_ends() {
        // A pid reused while /proc is read can make a process its own
        // ancestor; the walk once grew without end on such a list.
        let tree = vec![
            p(
                1,
                1,
                "/h/.local/share/Steam/ubuntu12_32/reaper|SteamLaunch AppId=570 --",
                1,
            ),
            p(
                2,
                1,
                "/g/steamapps/common/dota 2 beta/game/bin/linuxsteamrt64/dota2|",
                4_000,
            ),
        ];
        assert_eq!(identify(&tree)[0].process_name, "dota2");
    }

    #[test]
    fn store_clients_are_never_the_game() {
        // Unravel Two through the EA app on Steam: the client signs in and
        // updates for longer than a profile offer waits.
        let ea = "C:\\Program Files\\Electronic Arts\\EA Desktop\\EA Desktop";
        let mut tree = vec![
            reaper(1, "1225570"),
            p(
                2,
                1,
                "python3|/s/steamapps/common/Proton 9.0/proton waitforexitandrun x",
                5,
            ),
            p(3, 2, "c:\\windows\\system32\\steam.exe|S:\\x", 3),
            p(4, 3, &format!("{ea}\\EADesktop.exe|"), 9_000),
            p(5, 4, &format!("{ea}\\QtWebEngineProcess.exe|"), 8_000),
            p(6, 4, &format!("{ea}\\EABackgroundService.exe|"), 7_000),
            p(7, 4, &format!("{ea}\\EALocalHostSvc.exe|"), 6_000),
            p(
                8,
                2,
                "S:\\steamapps\\common\\Unravel Two\\Link2EA.exe|",
                900,
            ),
            p(
                9,
                2,
                "S:\\steamapps\\common\\Unravel Two\\__Installer\\Touchup.exe|",
                5_000,
            ),
        ];
        assert!(identify(&tree).is_empty(), "{:?}", identify(&tree));
        tree.push(p(
            10,
            4,
            "S:\\steamapps\\common\\Unravel Two\\UnravelTwo.exe|",
            300,
        ));
        assert_eq!(identify(&tree)[0].process_name, "UnravelTwo.exe");

        // Ubisoft Connect opened alone in Lutris.
        let upc = "C:\\Program Files (x86)\\Ubisoft\\Ubisoft Game Launcher";
        let lutris = vec![
            p(20, 1, "/usr/bin/lutris|", 500),
            p(21, 20, "/h/wine/bin/wine64-preloader|", 100),
            p(22, 21, &format!("{upc}\\upc.exe|"), 9_000),
            p(23, 22, &format!("{upc}\\UplayWebCore.exe|"), 8_000),
            p(24, 22, &format!("{upc}\\UbisoftConnect.exe|"), 800),
            p(
                25,
                21,
                "C:\\Program Files (x86)\\GOG Galaxy\\GalaxyClient Helper.exe|",
                800,
            ),
            p(26, 21, "C:\\Rockstar\\SocialClubHelper.exe|", 800),
            p(27, 21, "C:\\g\\DuneSandbox_BE.exe|", 800),
            p(28, 21, "C:\\g\\BEService_x64.exe|", 800),
            p(29, 21, "C:\\g\\EOSOverlayRenderer-Win64-Shipping.exe|", 800),
            p(30, 21, "C:\\g\\UnrealCEFSubProcess.exe|", 800),
        ];
        assert!(identify(&lutris).is_empty(), "{:?}", identify(&lutris));
        for name in [
            "EADesktop.exe",
            "Link2EA.exe",
            "upc.exe",
            "PlayGTAV.exe",
            "RockstarService.exe",
            "Battle.net Helper.exe",
        ] {
            assert!(is_infrastructure(name), "{name}");
        }
        assert!(
            !is_infrastructure("EarthDefenseForce.exe"),
            "starts like EA's, is a game"
        );
    }

    #[test]
    fn the_wrappers_around_a_native_game_are_not_the_game() {
        // Dota 2 started through Steam's launch options with Gamescope.
        let tree = vec![
            reaper(1, "570"),
            p(2, 1, "/usr/bin/gamescope|-W 2560 -- %command%", 9_000),
            p(3, 2, "/usr/bin/Xwayland|:1", 8_000),
            p(4, 2, "/usr/bin/mangoapp|", 700),
            p(
                5,
                2,
                "/g/steamapps/common/dota 2 beta/game/bin/linuxsteamrt64/dota2|",
                4_000,
            ),
        ];
        assert_eq!(identify(&tree)[0].process_name, "dota2");
        for name in [
            "heroic",
            "legendary",
            "gogdl",
            "umu-run",
            "gamescope-wl",
            "bwrap",
            "obs-gamecapture",
            "bigame-ui",
            "moonlight",
        ] {
            assert!(is_infrastructure(name), "{name}");
        }
    }

    #[test]
    fn the_game_is_the_one_busy_now_or_drawing_not_the_one_open_longest() {
        let tree = || {
            vec![
                reaper(1, "4242"),
                p(
                    2,
                    1,
                    "python3|/s/steamapps/common/Proton 9.0/proton waitforexitandrun x",
                    5,
                ),
                // Open for an hour, idle now.
                p(3, 2, "S:\\steamapps\\common\\G\\Companion.exe|", 4_000),
                // Started a minute ago.
                p(4, 2, "S:\\steamapps\\common\\G\\Game.exe|", 2_000),
            ]
        };
        assert_eq!(identify(&tree())[0].pid, 3, "by total CPU time alone");
        let activity = Activity {
            previous: HashMap::from([(3, 3_990), (4, 500)]),
            renders: None,
        };
        let found = identify_ranked(&tree(), &HashMap::<String, String>::new(), &activity);
        assert_eq!(found[0].pid, 4, "by CPU time since the last look");
        let drawing = Activity {
            previous: HashMap::new(),
            renders: Some(Box::new(|pid| pid == 4)),
        };
        let found = identify_ranked(&tree(), &HashMap::<String, String>::new(), &drawing);
        assert_eq!(found[0].pid, 4, "the one submitting GPU work");
        // A pid reused since the last look counts all its time.
        let reused = Activity {
            previous: HashMap::from([(4, 9_999)]),
            renders: None,
        };
        assert_eq!(reused.recent(&tree()[3]), 2_000);
    }

    #[test]
    fn cpu_time_is_compared_over_at_least_the_minimum_window() {
        let start = std::time::Instant::now();
        let ticks = |n: u64| HashMap::from([(1, n)]);
        let mut b = Baselines {
            older: None,
            newer: None,
        };
        assert!(b.take(start, &ticks(10)).is_empty());
        // Right after: still compared with the first look.
        assert_eq!(b.take(start + MIN_WINDOW / 4, &ticks(11))[&1], 10);
        let later = start + MIN_WINDOW * 3;
        assert_eq!(b.take(later, &ticks(50))[&1], 10);
        // A look just after a fresh one compares with the one before it.
        assert_eq!(b.take(later + MIN_WINDOW / 4, &ticks(51))[&1], 10);
        assert_eq!(b.take(later + MIN_WINDOW * 2, &ticks(90))[&1], 50);
    }

    #[test]
    fn gpu_work_is_read_from_the_processs_descriptors() {
        let root = std::env::temp_dir().join(format!("bigame_gpu_fds_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let fd = |pid: u32, n: u32, target: &str, fdinfo: &str| {
            let dir = root.join(pid.to_string());
            std::fs::create_dir_all(dir.join("fd")).unwrap();
            std::fs::create_dir_all(dir.join("fdinfo")).unwrap();
            std::os::unix::fs::symlink(target, dir.join("fd").join(n.to_string())).unwrap();
            std::fs::write(dir.join("fdinfo").join(n.to_string()), fdinfo).unwrap();
        };
        fd(
            10,
            3,
            "/dev/dri/renderD128",
            "drm-client-id:\t7\ndrm-engine-gfx:\t18885878 ns\n",
        );
        fd(11, 3, "/dev/dri/renderD128", "drm-client-id:\t8\n");
        fd(11, 4, "/dev/nvidiactl", "pos:\t0\n");
        fd(12, 5, "/dev/nvidia0", "pos:\t0\n");
        fd(13, 0, "/dev/null", "pos:\t0\n");
        assert!(submits_gpu_work(&root, 10));
        assert!(
            !submits_gpu_work(&root, 11),
            "a render node with no work, the control node"
        );
        assert!(submits_gpu_work(&root, 12));
        assert!(!submits_gpu_work(&root, 13));
        assert!(!submits_gpu_work(&root, 99));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_proc_tree_is_read_without_status_files() {
        let root = std::env::temp_dir().join(format!("bigame_fake_proc_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let dir = root.join("4242");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("stat"),
            "4242 (Game.exe) S 4000 4242 0 0 -1 0 0 0 0 0 1200 370 0 0 20 0 60 0 99",
        )
        .unwrap();
        std::fs::write(dir.join("cmdline"), b"C:\\g\\Game.exe\0-dx12\0").unwrap();
        std::fs::create_dir_all(root.join("self")).unwrap();
        let procs = snapshot_in(&root);
        assert_eq!(procs.len(), 1);
        assert_eq!(procs[0].pid, 4242);
        assert_eq!(procs[0].ppid, 4000);
        assert_eq!(procs[0].cpu_ticks, 1570);
        assert_eq!(procs[0].cmdline, "C:\\g\\Game.exe -dx12");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn native_games_of_every_launcher_are_known() {
        use crate::games::{DetectedGame, Source};
        let game = |name: &str, source: Source, exes: &[&str], native: bool| DetectedGame {
            name: name.into(),
            source,
            app_id: None,
            install_path: None,
            executables: exes.iter().map(|e| (*e).to_owned()).collect(),
            launch_file: None,
            cover: None,
            icon: None,
            launch_command: native.then(|| vec!["x".to_owned()]),
            launcher: None,
        };
        let library = [
            game("SuperTuxKart", Source::Lutris, &["supertuxkart"], true),
            game(
                "Altered Beast Remake Linux",
                Source::Lutris,
                &["Altered Beast Remake"],
                true,
            ),
            game(
                "Rock & Roll Racing",
                Source::Lutris,
                &["Rock N Roll Racing.exe"],
                false,
            ),
            game(
                "Linux Port",
                Source::Heroic,
                &["GameBinary", "run", "nw"],
                true,
            ),
            game("Windows Build", Source::Heroic, &["Some.bin"], false),
            game("KMines", Source::Native, &["kmines"], true),
            game("Hades", Source::Steam, &["hades"], false),
        ];
        let known = native_games_from(&["cs2".to_owned(), "proton".to_owned()], &library);
        let mut keys: Vec<&str> = known.keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            [
                "Altered Beast Remake",
                "GameBinary",
                "cs2",
                "kmines",
                "supertuxkart"
            ]
        );
        assert_eq!(known["supertuxkart"], "SuperTuxKart");
    }

    #[test]
    fn the_profile_in_effect_is_the_users_whatever_the_case() {
        let base = std::env::temp_dir().join(format!("bigame_profiles_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("user")).unwrap();
        std::fs::write(base.join("game.conf"), "name = \"Game.exe\"\n").unwrap();
        std::fs::write(base.join("user/mine.conf"), "name = \"game.exe\"\n").unwrap();
        std::fs::write(base.join("proton.conf"), "name = \"proton\"\n").unwrap();
        std::fs::write(base.join("other.conf"), "name = \"Other.exe\"\n").unwrap();
        let found = matching_profile_in(&base, "Game.exe", "default").unwrap();
        assert!(found.user, "the user's file overrides falcond's: {found:?}");
        assert_eq!(found.path, base.join("user/mine.conf"));
        assert_eq!(
            matching_profile_in(&base, "OTHER.EXE", "default")
                .unwrap()
                .name,
            "Other.exe"
        );
        assert_eq!(matching_profile_in(&base, "Proton", "default"), None);
        let _ = std::fs::remove_dir_all(&base);
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
