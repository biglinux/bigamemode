//! Library discovery: what is installed, what it is called, and what it looks
//! like.
//!
//! A launcher's record of a game is a claim, not proof: Steam keeps manifests
//! of titles it is still downloading, Lutris keeps the configuration of a game
//! whose files were deleted, Heroic caches the whole store library, and a
//! menu entry outlives the program it starts. Every claim is checked against
//! the disk — the install directory, or the file the launcher would run —
//! before it becomes a game here. A profile is never evidence of a game: the
//! ones falcond ships cover titles that may not be installed at all.
//!
//! Profiles are keyed on the process falcond sees, never on the title: falcond
//! matches `/proc/<pid>/comm`, and Steam's `installdir` is often nothing like
//! it:
//!
//! ```text
//! ARC Raiders        installdir "Arc Raiders"        real process PioneerGame.exe
//! Dead by Daylight   installdir "Dead by Daylight"   real process DeadByDaylight.exe
//! ```
//!
//! A profile keyed on `installdir` is loaded by falcond and never matches
//! anything, so detection looks inside the install directory for the binaries
//! that actually run, and ranks them.
//!
//! Artwork is discovered from what the launchers have already downloaded. No
//! API key, no network request, no third-party service — if Steam has a cover
//! on disk, it is used, and if it does not, the fallback chain degrades to an
//! icon rather than to an empty box.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

mod appinfo;
mod sqlite;
mod vdf;

/// Where a game came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Source {
    /// Steam.
    Steam,
    /// Lutris.
    Lutris,
    /// Heroic — Epic, GOG, Amazon or a sideloaded title.
    Heroic,
    /// Faugus Launcher: Windows games under UMU, and native ones.
    Faugus,
    /// A native game in the application menu (a `.desktop` entry in the
    /// `Game` category): from the distribution's repositories, or installed
    /// by hand.
    Native,
    /// A Flatpak in the application menu's `Game` category.
    Flatpak,
}

impl Source {
    /// Name for display.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Steam => "Steam",
            Self::Lutris => "Lutris",
            Self::Heroic => "Heroic",
            Self::Faugus => "Faugus",
            Self::Native => "Native",
            Self::Flatpak => "Flatpak",
        }
    }
}

/// An installed game.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetectedGame {
    /// Title as the launcher shows it.
    pub name: String,
    /// Launcher.
    pub source: Source,
    /// Steam `AppID` for a Steam title; the application id for a Flatpak.
    pub app_id: Option<String>,
    /// Install directory, when the launcher records one and it exists.
    pub install_path: Option<PathBuf>,
    /// Candidate process names, best first.
    ///
    /// falcond matches on process name, so this — not the title — is what a
    /// profile must be keyed on.
    pub executables: Vec<String>,
    /// The file the launcher runs, when it records one: Lutris's `exe`,
    /// Heroic's `executable`, a menu entry's program. Absolute, and checked
    /// to exist.
    pub launch_file: Option<PathBuf>,
    /// Portrait cover already on disk, if a launcher cached one.
    pub cover: Option<PathBuf>,
    /// Icon name from the application menu, for games with no cover art.
    pub icon: Option<String>,
    /// A command that starts this game directly, when one exists.
    ///
    /// `None` for anything that has to go through a launcher process — Steam
    /// titles in particular, where `steam -applaunch` returns immediately and
    /// the game runs in a separate tree — and for Flatpaks, whose sandbox
    /// does not see the environment a launch would set. That distinction
    /// matters: a benchmark needs a handle on the process it is measuring, so
    /// features that require one are offered only where this is `Some`.
    pub launch_command: Option<Vec<String>>,
    /// Where the game's own launcher keeps its settings for it, when that
    /// launcher has per-game settings Big Game Mode can write (`MangoHud`).
    pub launcher: Option<LauncherRef>,
}

/// A game's entry in its launcher's own configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LauncherRef {
    /// Heroic: `<config_dir>/GamesConfig/<app_name>.json`.
    Heroic {
        /// The game's id in Heroic (Epic's app name, GOG's id, …).
        app_name: String,
        /// Heroic's configuration directory (native or Flatpak).
        config_dir: PathBuf,
        /// The store backend that lists it (`legendary`, `gog`, `nile`,
        /// `sideload`): what a `heroic://launch` link names as its runner.
        /// `None` for a store Heroic's links cannot start.
        runner: Option<&'static str>,
    },
    /// Lutris: the game's YAML configuration.
    Lutris {
        /// `…/lutris/games/<slug>-<id>.yml`.
        config_file: PathBuf,
    },
}

impl LauncherRef {
    /// The launcher's name, as people know it.
    #[must_use]
    pub fn launcher_name(&self) -> &'static str {
        match self {
            Self::Heroic { .. } => "Heroic",
            Self::Lutris { .. } => "Lutris",
        }
    }

    /// The Flatpak application id when this launcher runs as a Flatpak
    /// (its configuration is under `~/.var/app/<id>/`).
    #[must_use]
    pub fn flatpak_id(&self) -> Option<&'static str> {
        let path = match self {
            Self::Heroic { config_dir, .. } => config_dir,
            Self::Lutris { config_file } => config_file,
        };
        let s = path.to_string_lossy();
        match self {
            Self::Heroic { .. } if s.contains("/.var/app/com.heroicgameslauncher.hgl/") => {
                Some("com.heroicgameslauncher.hgl")
            }
            Self::Lutris { .. } if s.contains("/.var/app/net.lutris.Lutris/") => {
                Some("net.lutris.Lutris")
            }
            _ => None,
        }
    }
}

impl DetectedGame {
    /// The process name a profile should be keyed on.
    ///
    /// Falls back to the title only when no executable could be found, and
    /// callers should treat that as "ask the user" rather than "good enough":
    /// a title-keyed profile never matches a process.
    #[must_use]
    pub fn profile_key(&self) -> &str {
        self.executables
            .first()
            .map_or(self.name.as_str(), String::as_str)
    }

    /// Whether a real executable was found, as opposed to guessing the title.
    #[must_use]
    pub fn has_real_executable(&self) -> bool {
        !self.executables.is_empty()
    }

    /// What makes this game the same game as another: its Steam or Flatpak
    /// id, its install directory, the file that starts it — and, failing all
    /// of those, its title within one launcher.
    fn identities(&self) -> Vec<Identity> {
        let mut ids = Vec::new();
        match (self.source, &self.app_id) {
            (Source::Steam, Some(id)) => ids.push(Identity::Steam(id.clone())),
            (Source::Flatpak, Some(id)) => ids.push(Identity::Flatpak(id.clone())),
            _ => {}
        }
        for path in self.install_path.iter().chain(&self.launch_file) {
            ids.push(Identity::Path(canonical(path)));
        }
        if ids.is_empty() {
            ids.push(Identity::Title(self.source, self.name.to_lowercase()));
        }
        ids
    }
}

/// One way of telling two detections apart — or not.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum Identity {
    Steam(String),
    Flatpak(String),
    Path(PathBuf),
    Title(Source, String),
}

fn canonical(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// Discover everything installed, sorted by title.
///
/// One entry per game, however many launchers know it: Steam's word wins over
/// Heroic's, Heroic's over Lutris's, Lutris's over Faugus's, and the
/// application menu comes last, so a menu entry for a Lutris game is folded
/// into the Lutris one.
///
/// The result is kept and handed out again while nothing it was read from
/// changed (see `Sources::watched`): a game starting asks for the library
/// several times, and each full scan walks every install folder, often on
/// the same disk the game is loading from.
#[must_use]
pub fn detect_all() -> Vec<DetectedGame> {
    static CACHE: std::sync::Mutex<Option<Cached>> = std::sync::Mutex::new(None);
    let sources = Sources::of_home(&crate::paths::home_dir());
    let mut cache = CACHE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    cached_scan(&mut cache, &sources, std::time::Instant::now()).0
}

/// Every game in `sources`, folded and sorted.
fn detect_in(sources: &Sources) -> Vec<DetectedGame> {
    let mut games = steam_games_in(&sources.steam_roots);
    games.extend(heroic_games(&sources.heroic));
    games.extend(lutris_games(&sources.lutris));
    games.extend(faugus_games(&sources.faugus));
    games.extend(
        menu_games_in(&sources.applications, &sources.path_dirs, &sources.flatpak)
            .into_iter()
            .map(DetectedGame::from),
    );
    let mut games = dedup(games);
    games.sort_by_key(|g| g.name.to_lowercase());
    games
}

/// The base directories of the XDG Base Directory specification, for one
/// home.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Xdg {
    config: PathBuf,
    data: PathBuf,
    cache: PathBuf,
    /// `XDG_DATA_DIRS`.
    data_dirs: Vec<PathBuf>,
}

impl Xdg {
    /// The session's directories when `home` is the session's home, where
    /// `XDG_*_HOME` describe it; the specification's defaults under `home`
    /// otherwise.
    fn of_home(home: &Path) -> Self {
        let session = home == crate::paths::home_dir();
        let var = |name: &str, default: &str| {
            std::env::var_os(name)
                .map(PathBuf::from)
                .filter(|p| session && p.is_absolute())
                .unwrap_or_else(|| home.join(default))
        };
        let data_dirs = std::env::var("XDG_DATA_DIRS")
            .ok()
            .filter(|d| !d.is_empty())
            .unwrap_or_else(|| "/usr/local/share:/usr/share".to_owned());
        Self {
            config: var("XDG_CONFIG_HOME", ".config"),
            data: var("XDG_DATA_HOME", ".local/share"),
            cache: var("XDG_CACHE_HOME", ".cache"),
            data_dirs: data_dirs
                .split(':')
                .map(PathBuf::from)
                .filter(|p| p.is_absolute())
                .collect(),
        }
    }
}

/// Where every launcher keeps what a scan reads.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Sources {
    /// Steam roots before `libraryfolders.vdf` adds the other libraries.
    steam_roots: Vec<PathBuf>,
    heroic: Vec<PathBuf>,
    lutris: Vec<LutrisRoot>,
    /// Faugus Launcher's `games.json` files.
    faugus: Vec<PathBuf>,
    applications: Vec<PathBuf>,
    path_dirs: Vec<PathBuf>,
    flatpak: Vec<PathBuf>,
}

impl Sources {
    fn of_home(home: &Path) -> Self {
        let xdg = Xdg::of_home(home);
        let applications = std::iter::once(xdg.data.clone())
            .chain(xdg.data_dirs.iter().cloned())
            .map(|d| d.join("applications"))
            .collect();
        Self {
            steam_roots: steam_roots(home, &xdg),
            heroic: heroic_config_dirs(home, &xdg),
            lutris: lutris_roots(home, &xdg),
            faugus: faugus_files(home, &xdg),
            applications,
            path_dirs: std::env::var_os("PATH")
                .map(|p| std::env::split_paths(&p).collect())
                .unwrap_or_default(),
            flatpak: vec![PathBuf::from("/var/lib/flatpak"), xdg.data.join("flatpak")],
        }
    }

    /// The files and directories whose modification marks a change in what
    /// is installed: Steam's manifests and their folders, its app cache and
    /// covers; Heroic's, Lutris's and Faugus's records; the menu's
    /// directories and the Flatpak installations.
    fn watched(&self) -> Vec<PathBuf> {
        let mut paths = Vec::new();
        for root in steam_libraries_in(&self.steam_roots) {
            let steamapps = root.join("steamapps");
            paths.push(root.join("steamapps/libraryfolders.vdf"));
            paths.push(root.join("appcache/appinfo.vdf"));
            paths.push(root.join("appcache/librarycache"));
            paths.push(steamapps.join("common"));
            if let Ok(entries) = std::fs::read_dir(&steamapps) {
                paths.extend(
                    entries
                        .flatten()
                        .map(|e| e.path())
                        .filter(|p| p.extension().is_some_and(|e| e == "acf")),
                );
            }
            paths.push(steamapps);
        }
        for base in &self.heroic {
            for (file, _) in HEROIC_LIBRARIES.iter().chain(HEROIC_INSTALLED) {
                paths.push(base.join(file));
            }
            paths.push(base.join("images-cache"));
        }
        for root in &self.lutris {
            if let Ok(entries) = std::fs::read_dir(&root.games) {
                paths.extend(entries.flatten().map(|e| e.path()));
            }
            paths.push(root.games.clone());
            paths.push(root.data.join("pga.db"));
            paths.push(root.data.join("coverart"));
            paths.push(root.cache.join("coverart"));
        }
        paths.extend(self.faugus.iter().cloned());
        paths.extend(self.applications.iter().cloned());
        paths.extend(self.flatpak.iter().map(|f| f.join("app")));
        paths
    }
}

/// How long a scan is handed out at most, whatever the files say: what no
/// watched file records (a game's folder emptied by hand) is seen after
/// this.
const LIBRARY_MAX_AGE: std::time::Duration = std::time::Duration::from_secs(300);

/// A scan and what it was read from.
struct Cached {
    sources: Sources,
    /// The watched files' modification times, taken before the scan.
    sources_stamp: Vec<Option<std::time::SystemTime>>,
    /// The games' own folders and files, after it.
    games_stamp: Vec<Option<std::time::SystemTime>>,
    at: std::time::Instant,
    games: Vec<DetectedGame>,
}

fn stamp(path: &Path) -> Option<std::time::SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// The modification times of the games' install folders and launch files:
/// a game deleted by hand leaves its launcher's record behind.
fn games_stamp(games: &[DetectedGame]) -> Vec<Option<std::time::SystemTime>> {
    games
        .iter()
        .flat_map(|g| g.install_path.iter().chain(&g.launch_file))
        .map(|p| stamp(p))
        .collect()
}

/// The library of `sources`: `cache`'s while it is current, a new scan
/// otherwise. `true` when it scanned.
fn cached_scan(
    cache: &mut Option<Cached>,
    sources: &Sources,
    now: std::time::Instant,
) -> (Vec<DetectedGame>, bool) {
    let sources_stamp: Vec<_> = sources.watched().iter().map(|p| stamp(p)).collect();
    if let Some(c) = cache.as_ref() {
        if c.sources == *sources
            && now.saturating_duration_since(c.at) < LIBRARY_MAX_AGE
            && c.sources_stamp == sources_stamp
            && c.games_stamp == games_stamp(&c.games)
        {
            return (c.games.clone(), false);
        }
    }
    // Stamped before reading: a change during the scan shows on the next
    // call as a difference, never as a stale result kept for good.
    let games = detect_in(sources);
    *cache = Some(Cached {
        sources: sources.clone(),
        sources_stamp,
        games_stamp: games_stamp(&games),
        at: now,
        games: games.clone(),
    });
    (games, true)
}

/// Keep the first of every game, in the order given.
fn dedup(games: Vec<DetectedGame>) -> Vec<DetectedGame> {
    let mut seen: HashSet<Identity> = HashSet::new();
    let mut roots: Vec<PathBuf> = Vec::new();
    let mut kept = Vec::with_capacity(games.len());
    for game in games {
        let ids = game.identities();
        // A launch file inside another game's install directory is that game
        // again (a menu entry or Lutris shortcut for it).
        let inside_known_root = game
            .launch_file
            .as_deref()
            .map(canonical)
            .is_some_and(|file| roots.iter().any(|root| file.starts_with(root)));
        if inside_known_root || ids.iter().any(|id| seen.contains(id)) {
            tracing::debug!(game = %game.name, source = game.source.label(), "already listed by another launcher");
            continue;
        }
        seen.extend(ids);
        if let Some(root) = &game.install_path {
            roots.push(canonical(root));
        }
        kept.push(game);
    }
    kept
}

/// Whether a directory exists and has something in it. An empty directory is
/// what a launcher leaves behind after removing a game, or creates before
/// downloading one.
fn populated_dir(path: &Path) -> bool {
    std::fs::read_dir(path).is_ok_and(|mut entries| entries.next().is_some())
}

/// Whether `path` is a file the user can execute.
fn executable_file(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

// ── The application menu ─────────────────────────────────────────────────────

/// Programs that run whatever they are given — a browser, an interpreter, a
/// runtime — and so name no game: a profile or a running-game match keyed
/// on `firefox` or `java` would take every page or program they run for the
/// game. A menu entry that starts a game through one is not listed.
const GENERIC_PROGRAMS: &[&str] = &[
    "java",
    "javaw",
    "mono",
    "node",
    "nodejs",
    "electron",
    "firefox",
    "firefox-esr",
    "librewolf",
    "chromium",
    "chromium-browser",
    "chrome",
    "brave",
    "brave-browser",
    "vivaldi",
    "opera",
    "microsoft-edge",
    "microsoft-edge-stable",
    "falkon",
    "epiphany",
];

/// Whether a process name is a browser's, an interpreter's or a runtime's
/// (`GENERIC_PROGRAMS`, `python*`, `google-chrome*`, `electron*`).
#[must_use]
pub fn is_generic_program(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    GENERIC_PROGRAMS.contains(&lower.as_str())
        || lower.starts_with("google-chrome")
        || lower.starts_with("electron")
        // python3, python3.13, perl5.40: versioned interpreters.
        || ["python", "pypy", "ruby", "perl", "lua", "luajit"].iter().any(|p| {
            lower
                .strip_prefix(p)
                .is_some_and(|v| v.chars().all(|c| c.is_ascii_digit() || c == '.'))
        })
}

/// Main categories an entry cannot carry and be a game, whatever else it
/// lists: a store (Heroic is `Game;PackageManager;`), a tool (ProtonUp-Qt is
/// `Game;Utility;`), a settings panel. The `Game` category alone says what an
/// entry is about, not what it is.
const NOT_GAME_CATEGORIES: &[&str] = &[
    "PackageManager",
    "Utility",
    "Settings",
    "System",
    "Development",
];

/// A game in the application menu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuGame {
    /// The entry's `Name`.
    pub name: String,
    /// The process name falcond will see (the basename of the program).
    ///
    /// For a Flatpak this is the application's `command`, which
    /// [`menu_game`] can only take from an explicit `--command=`; otherwise
    /// it is empty until [`menu_games_in`] reads the application's metadata.
    pub program: String,
    /// `Exec` as an argument vector, field codes (`%U`, `%f`, …) removed.
    pub argv: Vec<String>,
    /// The entry's `Icon`.
    pub icon: Option<String>,
    /// The application id, when the entry runs a Flatpak.
    pub flatpak: Option<String>,
    /// Where the program is, once [`menu_games_in`] has found it. Absolute.
    pub program_path: Option<PathBuf>,
}

impl From<MenuGame> for DetectedGame {
    fn from(game: MenuGame) -> Self {
        let flatpak = game.flatpak.is_some();
        Self {
            name: game.name,
            source: if flatpak {
                Source::Flatpak
            } else {
                Source::Native
            },
            app_id: game.flatpak,
            install_path: None,
            // A Flatpak whose metadata names no command has no known process.
            executables: Some(game.program)
                .filter(|p| !p.is_empty())
                .into_iter()
                .collect(),
            launch_file: game.program_path,
            cover: None,
            icon: game.icon,
            launch_command: (!flatpak).then_some(game.argv),
            launcher: None,
        }
    }
}

/// A menu entry, when the entry is a game.
///
/// Reads only the `[Desktop Entry]` group (actions such as `SuperTuxKart`'s
/// *Software Render* live in groups of their own), requires the `Game`
/// category, and takes the program from `Exec` past `env` and its
/// `VAR=value` assignments. Launchers and entries that start a game through
/// a launcher (`steam steam://rungameid/…`) are not games here: their process
/// is the launcher, and Steam's are found by their tree. A `flatpak run`
/// entry is a game whose process is the Flatpak's own command; whether the
/// Flatpak is installed is for [`menu_games_in`] to check.
#[must_use]
pub fn menu_game(content: &str) -> Option<MenuGame> {
    menu_game_at(content, None)
}

/// [`menu_game`] for the entry at `location`, which `%k` stands for.
fn menu_game_at(content: &str, location: Option<&Path>) -> Option<MenuGame> {
    let mut in_entry = false;
    let (mut exec, mut name, mut categories, mut icon) = (None, None, None, None);
    let mut hidden = false;
    let mut application = false;
    for line in content.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_entry = line == "[Desktop Entry]";
            continue;
        }
        if !in_entry {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        match key.trim() {
            "Exec" => exec = Some(unescape_value(value.trim())),
            "Name" => name = Some(unescape_value(value.trim())),
            "Icon" => icon = Some(unescape_value(value.trim())).filter(|v| !v.is_empty()),
            "Categories" => categories = Some(value.to_owned()),
            "Type" => application = value.trim() == "Application",
            "Hidden" | "NoDisplay" => hidden |= value.trim() == "true",
            _ => {}
        }
    }
    if !application || hidden {
        return None;
    }
    let categories = categories?;
    let categories: Vec<&str> = categories.split(';').map(str::trim).collect();
    if !categories.contains(&"Game") || categories.iter().any(|c| NOT_GAME_CATEGORIES.contains(c)) {
        return None;
    }
    let location = location.map(|l| l.to_string_lossy().into_owned());
    let fields = FieldCodes {
        name: name.as_deref().unwrap_or_default(),
        icon: icon.as_deref(),
        location: location.as_deref(),
    };
    let argv: Vec<String> = exec_arguments(&exec?)
        .into_iter()
        .flat_map(|a| expand_field_codes(&a, &fields))
        .collect();
    let launched = launched_program(&argv)?;
    let (program, flatpak) = if crate::running::falcond_name(&launched) == "flatpak" {
        let run = flatpak_run(&argv)?;
        (run.command.unwrap_or_default(), Some(run.app_id))
    } else {
        (crate::running::falcond_name(&launched).to_owned(), None)
    };
    // A Flatpak's process is known here only from `--command=`; otherwise
    // [`menu_games_in`] checks it once the metadata names it.
    if (flatpak.is_none() || !program.is_empty()) && !is_game_program(&program) {
        return None;
    }
    let name = name.unwrap_or_else(|| program.clone());
    Some(MenuGame {
        name,
        program,
        argv,
        icon,
        flatpak,
        program_path: None,
    })
}

/// Whether a process name is a game's rather than a launcher's, a tool's
/// ([`crate::running::is_infrastructure`], the one list both the menu and
/// running-game detection use) or a generic program's.
fn is_game_program(program: &str) -> bool {
    !program.is_empty()
        && !crate::running::is_infrastructure(program)
        && !is_generic_program(program)
}

/// What a `flatpak run` line runs.
struct FlatpakRun {
    app_id: String,
    /// An explicit `--command=`.
    command: Option<String>,
}

/// The application id and explicit command of `flatpak run [options] APP_ID …`.
fn flatpak_run(argv: &[String]) -> Option<FlatpakRun> {
    let mut rest = argv.iter().map(String::as_str);
    rest.find(|a| crate::running::falcond_name(a) == "flatpak")?;
    if rest.next() != Some("run") {
        return None;
    }
    let mut command = None;
    for arg in rest {
        if let Some(c) = arg.strip_prefix("--command=") {
            command = Some(crate::running::falcond_name(c).to_owned());
        } else if !arg.starts_with('-') {
            return Some(FlatpakRun {
                app_id: arg.to_owned(),
                command,
            });
        }
    }
    None
}

/// Programs that start another one and then become, or wait for, it: the
/// game is what they run, and it is the game's process a profile must match.
const WRAPPERS: &[&str] = &[
    "env",
    "prime-run",
    "gamemoderun",
    "mangohud",
    "nice",
    "ionice",
    "gamescope",
    "taskset",
    "obs-gamecapture",
    "strangle",
    "pw-jack",
    "firejail",
    "flatpak-spawn",
    "switcherooctl",
    "systemd-inhibit",
];

/// Whether `option` of `wrapper` takes the next argument as its value.
fn option_takes_value(wrapper: &str, option: &str) -> bool {
    match wrapper {
        "env" => matches!(option, "-u" | "-C" | "-S" | "--unset" | "--chdir"),
        "nice" => matches!(option, "-n" | "--adjustment"),
        "ionice" => matches!(
            option,
            "-c" | "-n" | "-p" | "-P" | "-u" | "--class" | "--classdata"
        ),
        "pw-jack" => matches!(option, "-s" | "-p"),
        "switcherooctl" => matches!(option, "-g" | "--gpu"),
        _ => false,
    }
}

/// Whether the positional argument `arg` of `wrapper` is its own rather
/// than the program: `taskset`'s CPU mask or list, `switcherooctl`'s
/// `launch`, `strangle`'s frame limit.
fn wrapper_positional(wrapper: &str, arg: &str, taken: usize) -> bool {
    match wrapper {
        "taskset" => taken == 0,
        "switcherooctl" => taken == 0 && arg == "launch",
        "strangle" => taken == 0 && arg.starts_with(|c: char| c.is_ascii_digit()),
        _ => false,
    }
}

/// The program an `Exec` line really runs, past [`WRAPPERS`], their options
/// and arguments, `VAR=value` assignments, and Gamescope's own arguments up
/// to `--`.
fn program_of(argv: &[String]) -> Option<&str> {
    let mut rest = argv.iter().map(String::as_str).peekable();
    while let Some(arg) = rest.next() {
        let base = crate::running::falcond_name(arg);
        if !WRAPPERS.contains(&base) {
            return Some(arg);
        }
        if base == "gamescope" {
            rest.find(|a| *a == "--")?;
            continue;
        }
        let mut positionals = 0;
        while let Some(next) = rest.peek() {
            if next.starts_with('-') {
                let takes_value = !next.contains('=') && option_takes_value(base, next);
                rest.next();
                if takes_value {
                    rest.next();
                }
            } else if next.contains('=') && !next.starts_with('/') {
                rest.next();
            } else if wrapper_positional(base, next, positionals) {
                positionals += 1;
                rest.next();
            } else {
                break;
            }
        }
    }
    None
}

/// The shells an entry may start its game through.
const SHELLS: &[&str] = &["sh", "bash", "dash", "zsh"];

/// The program an `Exec` line starts ([`program_of`]), looking into a shell:
/// `sh -c "cd /opt/g && exec ./game"` runs `/opt/g/game`, `bash start.sh`
/// runs the script.
fn launched_program(argv: &[String]) -> Option<String> {
    let launched = program_of(argv)?;
    if !SHELLS.contains(&crate::running::falcond_name(launched)) {
        return Some(launched.to_owned());
    }
    let at = argv.iter().position(|a| a == launched)?;
    let mut rest = argv[at + 1..].iter();
    while let Some(arg) = rest.next() {
        if arg == "-c" {
            return shell_command_program(rest.next()?);
        }
        if !arg.starts_with('-') {
            // `bash script`: the script is the program.
            return Some(arg.clone());
        }
    }
    None
}

/// The program the last command of a `sh -c` string runs, resolved against
/// the directory an earlier `cd` moved to.
fn shell_command_program(command: &str) -> Option<String> {
    let mut dir: Option<PathBuf> = None;
    let mut last = None;
    for part in command
        .split(['\n', ';'])
        .flat_map(|p| p.split("&&"))
        .flat_map(|p| p.split("||"))
    {
        let words = exec_arguments(part.trim());
        let words: Vec<String> = words.into_iter().skip_while(|w| w == "exec").collect();
        match words.first().map(String::as_str) {
            None => {}
            Some("cd") => dir = words.get(1).map(PathBuf::from),
            Some(_) => last = Some(words),
        }
    }
    let program = program_of(&last?)?.to_owned();
    let path = Path::new(&program);
    Some(match &dir {
        Some(dir) if program.contains('/') && path.is_relative() => {
            dir.join(path).to_string_lossy().into_owned()
        }
        _ => program,
    })
}

/// What the field codes of one entry stand for.
struct FieldCodes<'a> {
    /// `%c`: the entry's name.
    name: &'a str,
    /// `%i`: `--icon <Icon>`, when the entry has an icon.
    icon: Option<&'a str>,
    /// `%k`: where the entry is.
    location: Option<&'a str>,
}

/// Apply the Desktop Entry field codes to one argument: `%f`/`%F`/`%u`/`%U`
/// (files and URLs, of which there are none) remove the argument when it is
/// all they are and vanish inside one; `%c` is the name, `%k` the entry's
/// location, `%i` the `--icon` option (or nothing); `%%` is a percent sign;
/// deprecated codes vanish.
fn expand_field_codes(arg: &str, fields: &FieldCodes<'_>) -> Vec<String> {
    match arg {
        "%f" | "%F" | "%u" | "%U" => return Vec::new(),
        "%i" => {
            return fields
                .icon
                .map(|icon| vec!["--icon".to_owned(), icon.to_owned()])
                .unwrap_or_default();
        }
        "%k" => return fields.location.map(str::to_owned).into_iter().collect(),
        _ => {}
    }
    let mut out = String::with_capacity(arg.len());
    let mut chars = arg.chars();
    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('%') => out.push('%'),
            Some('c') => out.push_str(fields.name),
            Some('k') => out.push_str(fields.location.unwrap_or_default()),
            _ => {}
        }
    }
    vec![out]
}

/// A Desktop Entry string value with its escapes decoded: `\s`, `\n`, `\t`,
/// `\r` and `\\`. Other backslashes stay, for `Exec`'s own quoting rules,
/// which apply after these.
fn unescape_value(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('s') => out.push(' '),
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('\\') | None => out.push('\\'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
        }
    }
    out
}

/// Whether `path` can be run here directly: an executable ELF binary or a
/// script with a `#!` line. A Windows `.exe` from a Wine runner cannot, and
/// starting one outside its prefix measures nothing.
fn runs_natively(path: &Path) -> bool {
    use std::io::Read;
    if !executable_file(path) {
        return false;
    }
    let mut magic = [0u8; 4];
    std::fs::File::open(path)
        .and_then(|mut f| f.read_exact(&mut magic))
        .is_ok_and(|()| &magic == b"\x7fELF" || magic.starts_with(b"#!"))
}

/// Where a menu entry's program is: the path itself when absolute, otherwise
/// the first executable of that name in `path_dirs`. A relative path with a
/// directory in it is resolved against the current directory by the menu,
/// which is nowhere in particular, so it is not resolved here.
fn resolve_program(program: &str, path_dirs: &[PathBuf]) -> Option<PathBuf> {
    let candidate = Path::new(program);
    if candidate.is_absolute() {
        return executable_file(candidate).then(|| candidate.to_path_buf());
    }
    if program.contains('/') {
        return None;
    }
    path_dirs
        .iter()
        .map(|dir| dir.join(program))
        .find(|p| executable_file(p))
}

/// The `command` of an installed Flatpak, from its metadata; `None` when it
/// is not installed in any of `installations`.
fn flatpak_command(app_id: &str, installations: &[PathBuf]) -> Option<String> {
    installations.iter().find_map(|root| {
        let metadata = root
            .join("app")
            .join(app_id)
            .join("current/active/metadata");
        let content = std::fs::read_to_string(metadata).ok()?;
        let command = content.lines().find_map(|line| {
            line.trim()
                .strip_prefix("command=")
                .map(|c| crate::running::falcond_name(c.trim()).to_owned())
        });
        // An application with no command line in its metadata is still
        // installed; its process is then whatever `--command=` said.
        Some(command.unwrap_or_default())
    })
}

/// Every game in the application menu: `XDG_DATA_HOME` and each of
/// `XDG_DATA_DIRS`, the first entry of a name winning, as the menu does;
/// each checked to exist (see [`menu_games_in`]).
#[must_use]
pub fn menu_games() -> Vec<MenuGame> {
    let sources = Sources::of_home(&crate::paths::home_dir());
    menu_games_in(&sources.applications, &sources.path_dirs, &sources.flatpak)
}

/// The games among the `.desktop` entries of `applications`, keeping only
/// those whose program is really there: an executable file, found in
/// `path_dirs` when the entry names it without a path, or a Flatpak present
/// in one of `installations` (whose `command` then names the process).
#[must_use]
pub fn menu_games_in(
    applications: &[PathBuf],
    path_dirs: &[PathBuf],
    installations: &[PathBuf],
) -> Vec<MenuGame> {
    let mut seen = HashSet::new();
    let mut games = Vec::new();
    for dir in applications {
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };
        let mut paths: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
        paths.sort();
        for path in paths {
            if path.extension().is_none_or(|e| e != "desktop") {
                continue;
            }
            let Some(id) = path.file_name().map(std::borrow::ToOwned::to_owned) else {
                continue;
            };
            if !seen.insert(id) {
                continue;
            }
            let Some(mut game) = std::fs::read_to_string(&path)
                .ok()
                .and_then(|content| menu_game_at(&content, Some(&path)))
            else {
                continue;
            };
            if let Some(app_id) = &game.flatpak {
                let Some(command) = flatpak_command(app_id, installations) else {
                    tracing::debug!(entry = %path.display(), app = %app_id, "ignoring menu entry: Flatpak not installed");
                    continue;
                };
                if game.program.is_empty() {
                    game.program = command;
                }
                if !is_game_program(&game.program) {
                    continue;
                }
            } else {
                let launched = launched_program(&game.argv).unwrap_or_default();
                // A script a shell is given needs no executable bit.
                let via_shell = program_of(&game.argv)
                    .is_some_and(|p| SHELLS.contains(&crate::running::falcond_name(p)));
                let script = Path::new(&launched);
                let found = if via_shell && script.is_absolute() && script.is_file() {
                    Some(script.to_path_buf())
                } else {
                    resolve_program(&launched, path_dirs)
                };
                let Some(program_path) = found else {
                    tracing::debug!(entry = %path.display(), program = %game.program, "ignoring menu entry: program not found");
                    continue;
                };
                game.program_path = Some(program_path);
            }
            games.push(game);
        }
    }
    games
}

/// The arguments of a desktop entry's `Exec`: split on spaces, except inside
/// double quotes, where `\"`, `\\`, `` \` `` and `\$` are escapes (Desktop
/// Entry Specification, *The Exec key*). A quoted path with spaces is one
/// argument.
fn exec_arguments(exec: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    let mut started = false;
    let mut chars = exec.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                quoted = !quoted;
                started = true;
            }
            '\\' if quoted => {
                if let Some(next) = chars.next() {
                    current.push(next);
                }
            }
            c if c.is_whitespace() && !quoted => {
                if started {
                    args.push(std::mem::take(&mut current));
                    started = false;
                }
            }
            c => {
                current.push(c);
                started = true;
            }
        }
    }
    if started {
        args.push(current);
    }
    args
}

// ── Steam ────────────────────────────────────────────────────────────────────

/// Where a Steam client may keep its library: the native client under
/// `XDG_DATA_HOME` (as `steam.sh` puts it) and its `~/.steam` links, the
/// Flatpak, and the Snap.
fn steam_roots(home: &Path, xdg: &Xdg) -> Vec<PathBuf> {
    let mut roots = vec![
        xdg.data.join("Steam"),
        home.join(".local/share/Steam"),
        home.join(".steam/steam"),
        home.join(".steam/root"),
        home.join(".var/app/com.valvesoftware.Steam/.local/share/Steam"),
        home.join("snap/steam/common/.local/share/Steam"),
    ];
    roots.dedup();
    roots
}

/// Steam library roots, including extra library folders the user has added.
///
/// `~/.steam/steam` is normally a symlink to `~/.local/share/Steam`; roots are
/// canonicalised so one library is scanned once.
#[must_use]
pub fn steam_libraries(home: &Path) -> Vec<PathBuf> {
    steam_libraries_in(&steam_roots(home, &Xdg::of_home(home)))
}

fn steam_libraries_in(roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut roots = roots.to_vec();
    // libraryfolders.vdf lists games kept on other disks.
    for root in roots.clone() {
        let vdf = root.join("steamapps/libraryfolders.vdf");
        let Ok(content) = std::fs::read_to_string(&vdf) else {
            continue;
        };
        for path in parse_vdf_paths(&content) {
            let p = PathBuf::from(path);
            if !roots.contains(&p) {
                roots.push(p);
            }
        }
    }
    let mut libraries: Vec<PathBuf> = Vec::new();
    for root in roots {
        let Ok(root) = std::fs::canonicalize(&root) else {
            continue;
        };
        if root.join("steamapps").is_dir() && !libraries.contains(&root) {
            libraries.push(root);
        }
    }
    libraries
}

/// Extract the `"path"` values from a Steam VDF file (`libraryfolders.vdf`),
/// escapes decoded.
#[must_use]
pub fn parse_vdf_paths(content: &str) -> Vec<String> {
    vdf::values(content, "path")
}

/// The installed Steam titles of every library under `home`.
#[must_use]
pub fn steam_games(home: &Path) -> Vec<DetectedGame> {
    steam_games_in(&steam_roots(home, &Xdg::of_home(home)))
}

fn steam_games_in(roots: &[PathBuf]) -> Vec<DetectedGame> {
    let libraries = steam_libraries_in(roots);
    let mut manifests = Vec::new();
    for root in &libraries {
        let steamapps = root.join("steamapps");
        let Ok(entries) = std::fs::read_dir(&steamapps) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with("appmanifest_") && name.ends_with(".acf") {
                manifests.push((entry.path(), steamapps.clone()));
            }
        }
    }
    let apps = steam_app_info(&libraries, &manifests);
    manifests
        .iter()
        .filter_map(|(manifest, steamapps)| parse_acf(manifest, steamapps, &libraries, &apps))
        .collect()
}

/// appinfo's records for the apps of `manifests`, from every client's
/// `appcache/appinfo.vdf` (the native client's and the Flatpak's each keep
/// one), the first that has an app winning.
fn steam_app_info(
    libraries: &[PathBuf],
    manifests: &[(PathBuf, PathBuf)],
) -> HashMap<u32, appinfo::AppInfo> {
    let mut wanted: HashSet<u32> = manifests
        .iter()
        .filter_map(|(m, _)| {
            m.file_stem()?
                .to_str()?
                .strip_prefix("appmanifest_")?
                .parse()
                .ok()
        })
        .collect();
    let mut apps = HashMap::new();
    for root in libraries {
        if wanted.is_empty() {
            break;
        }
        let cache = root.join("appcache/appinfo.vdf");
        if !cache.is_file() {
            continue;
        }
        for (id, info) in appinfo::read(&cache, &wanted) {
            wanted.remove(&id);
            apps.insert(id, info);
        }
    }
    apps
}

/// Apps that are runtimes, tooling or servers rather than games, by
/// `AppID`: Steam's own runtimes and redistributables, every Proton, the
/// anti-cheat runtimes, `SteamVR`. appinfo's type says the same for any app
/// it knows (`Tool`); this covers a client whose cache is missing.
const STEAM_TOOL_IDS: &[&str] = &[
    "228980",  // Steamworks Common Redistributables
    "250820",  // SteamVR
    "1070560", // Steam Linux Runtime 1.0 (scout)
    "1391110", // Steam Linux Runtime 2.0 (soldier)
    "1628350", // Steam Linux Runtime 3.0 (sniper)
    "4183110", // Steam Linux Runtime 4.0
    "1161040", // Proton BattlEye Runtime
    "1826330", // Proton EasyAntiCheat Runtime
    "1493710", // Proton Experimental
    "2180100", // Proton Hotfix
    "858280",  // Proton 3.7
    "930400",  // Proton 3.16
    "961940",  // Proton 4.2
    "1054830", // Proton 4.11
    "1113280", // Proton 5.0
    "1245040", // Proton 5.13
    "1420170", // Proton 6.3
    "1887720", // Proton 7.0
    "2348590", // Proton 8.0
    "2805730", // Proton 9.0
    "3658110", // Proton 10.0
];

/// appinfo types that are not something to play: Proton, runtimes, servers
/// and SDKs are `Tool`; a few are `Config`.
const STEAM_NOT_GAME_TYPES: &[&str] = &["tool", "config", "dlc", "music", "video"];

/// Titles that are runtimes and tooling rather than games.
fn is_steam_runtime(name: &str) -> bool {
    name.starts_with("Proton")
        || name.starts_with("Steam Linux Runtime")
        || name.contains("Steamworks")
        || name.contains("EasyAntiCheat Runtime")
        || name.starts_with("Steam Deck")
        || name.starts_with("SteamVR")
        || name.contains("Dedicated Server")
        || name.ends_with(" SDK")
}

/// Whether a Steam app is a tool rather than a game: by appinfo's type when
/// the cache has the app, by its id and title otherwise.
fn is_steam_tool(app_id: &str, name: &str, info: Option<&appinfo::AppInfo>) -> bool {
    match info.and_then(|i| i.kind.as_deref()) {
        Some(kind) => STEAM_NOT_GAME_TYPES.contains(&kind),
        None => STEAM_TOOL_IDS.contains(&app_id) || is_steam_runtime(name),
    }
}

/// Steam's `StateFlags` bit for a title whose files are all there.
const STATE_FULLY_INSTALLED: u32 = 4;

/// Whether a manifest describes a title Steam considers installed: no
/// `StateFlags` (older manifests) or the *`FullyInstalled`* bit set. A title
/// being downloaded has a manifest and a partial directory but not the bit.
#[must_use]
pub fn acf_installed(content: &str) -> bool {
    acf_value(content, "StateFlags")
        .and_then(|flags| flags.parse::<u32>().ok())
        .is_none_or(|flags| flags & STATE_FULLY_INSTALLED != 0)
}

fn parse_acf(
    manifest: &Path,
    steamapps: &Path,
    libraries: &[PathBuf],
    apps: &HashMap<u32, appinfo::AppInfo>,
) -> Option<DetectedGame> {
    let content = std::fs::read_to_string(manifest).ok()?;
    // Steam keeps the store's spelling, trailing space included ("Gauntlet™ ").
    let name = acf_value(&content, "name")?.trim().to_owned();
    let installdir = acf_value(&content, "installdir")?;
    let app_id = acf_value(&content, "appid").or_else(|| {
        manifest
            .file_stem()?
            .to_string_lossy()
            .strip_prefix("appmanifest_")
            .map(str::to_owned)
    })?;
    let info = app_id.parse().ok().and_then(|id: u32| apps.get(&id));
    if name.is_empty() || is_steam_tool(&app_id, &name, info) {
        return None;
    }

    if !acf_installed(&content) {
        tracing::debug!(app = %app_id, title = %name, "ignoring Steam manifest: not fully installed");
        return None;
    }
    let install_path = steamapps.join("common").join(&installdir);
    if !populated_dir(&install_path) {
        tracing::debug!(app = %app_id, title = %name, dir = %install_path.display(), "ignoring Steam manifest: install directory missing");
        return None;
    }
    let beta = acf_value(&content, "BetaKey");
    let launch = info.and_then(|i| steam_launch_file(&install_path, i, beta.as_deref()));

    Some(DetectedGame {
        cover: steam_cover_in(libraries, &app_id),
        app_id: Some(app_id),
        executables: ranked_executables(&install_path, launch.as_deref()),
        install_path: Some(install_path),
        launch_file: None,
        name,
        source: Source::Steam,
        icon: None,
        // Steam titles start through the client, which returns immediately and
        // runs the game in a separate process tree. There is no command here
        // that yields a handle on the game itself.
        launch_command: None,
        launcher: None,
    })
}

/// The file Steam starts for an installed app, from appinfo's launch
/// entries: those for the branch installed (`BetaKey`) or for every branch,
/// `default` before untyped, options and `none`, whose file is in the
/// install folder and is not a launcher, a helper or a script. `None` when
/// the app is started through another store (`link2ea://…`) or only through
/// its launcher (Cyberpunk 2077's `redprelauncher.exe`): the files decide
/// then.
fn steam_launch_file(
    install: &Path,
    info: &appinfo::AppInfo,
    beta: Option<&str>,
) -> Option<PathBuf> {
    let branch = |l: &appinfo::Launch| -> Option<u8> {
        match (&l.betakey, beta) {
            (None, _) => Some(1),
            (Some(keys), Some(beta)) => keys
                .split(',')
                .any(|k| k.trim().eq_ignore_ascii_case(beta))
                .then_some(0),
            (Some(_), None) => None,
        }
    };
    let kind = |l: &appinfo::Launch| -> Option<u8> {
        match l.kind.as_deref() {
            Some("default") => Some(0),
            None => Some(1),
            Some(k) if k.starts_with("option") => Some(2),
            Some("none") => Some(3),
            // vr, server, editor, manual, …
            Some(_) => None,
        }
    };
    let mut entries: Vec<(u8, u8, usize, &appinfo::Launch)> = info
        .launch
        .iter()
        .enumerate()
        .filter(|(_, l)| l.oslist.as_deref().is_none_or(|os| os.trim() != "macos"))
        .filter(|(_, l)| !l.executable.contains("://"))
        .filter_map(|(i, l)| Some((branch(l)?, kind(l)?, i, l)))
        .collect();
    entries.sort_by_key(|(branch, kind, i, _)| (*branch, *kind, *i));
    entries.into_iter().find_map(|(_, _, _, l)| {
        let file = find_case_insensitive(install, &l.executable)?;
        usable_launch_file(&file).then_some(file)
    })
}

/// `relative` under `root`, matching each component without regard to case
/// as Windows does: appinfo names `redprelauncher.exe` for the file on disk
/// called `REDprelauncher.exe`.
fn find_case_insensitive(root: &Path, relative: &str) -> Option<PathBuf> {
    let mut path = root.to_path_buf();
    for part in relative
        .split(['/', '\\'])
        .filter(|p| !p.is_empty() && *p != ".")
    {
        if part == ".." {
            return None;
        }
        let exact = path.join(part);
        if exact.exists() {
            path = exact;
            continue;
        }
        let found = std::fs::read_dir(&path).ok()?.flatten().find(|e| {
            e.file_name()
                .to_str()
                .is_some_and(|n| n.eq_ignore_ascii_case(part))
        })?;
        path = found.path();
    }
    path.is_file().then_some(path)
}

/// Read a `"key" "value"` pair out of Valve's ACF format: the first at any
/// depth, the key compared without regard to case, escapes decoded.
#[must_use]
pub fn acf_value(content: &str, key: &str) -> Option<String> {
    vdf::first_value(content, key).filter(|v| !v.is_empty())
}

/// The installed game a running process belongs to, from any launcher: the
/// game whose install folder holds the process, else the one game that
/// lists its executable. What names it and shows its cover when the process
/// alone says neither.
#[must_use]
pub fn installed_game_for_process(
    process_name: &str,
    install_path: Option<&Path>,
) -> Option<DetectedGame> {
    game_among(&detect_all(), process_name, install_path).cloned()
}

/// [`installed_game_for_process`] for a running game, also by where its
/// executable is: a Wine game's `Z:\…` or `C:\…` path is mapped to the
/// file in its prefix, so two games that both run as `Game.exe` (RPG Maker)
/// are told apart by folder rather than guessed by name.
#[must_use]
pub fn installed_game_for_running(game: &crate::running::GameIdentity) -> Option<DetectedGame> {
    let games = detect_all();
    let file = host_path(game.pid, &game.executable).map(|f| canonical(&f));
    let by_file = file.as_deref().and_then(|file| {
        games.iter().find(|g| {
            g.launch_file
                .as_deref()
                .is_some_and(|l| canonical(l) == file)
                || g.install_path
                    .as_deref()
                    .is_some_and(|root| file.starts_with(canonical(root)))
        })
    });
    by_file
        .or_else(|| game_among(&games, &game.process_name, game.install_path.as_deref()))
        .cloned()
}

/// Where a process's executable is on this machine: `argv[0]` itself when it
/// is a Unix path; for a Windows path, the drive's link in the process's
/// Wine prefix (`dosdevices/z:` is `/`, `c:` is `drive_c`).
fn host_path(pid: u32, executable: &str) -> Option<PathBuf> {
    let environ = std::fs::read(format!("/proc/{pid}/environ")).unwrap_or_default();
    let prefix = environ
        .split(|b| *b == 0)
        .find_map(|e| e.strip_prefix(b"WINEPREFIX="))
        .map_or_else(
            || crate::paths::home_dir().join(".wine"),
            |p| PathBuf::from(String::from_utf8_lossy(p).into_owned()),
        );
    host_path_in(executable, &prefix)
}

/// [`host_path`] with the Wine prefix known.
fn host_path_in(executable: &str, prefix: &Path) -> Option<PathBuf> {
    if executable.starts_with('/') {
        return Some(PathBuf::from(executable));
    }
    let mut chars = executable.chars();
    let drive = chars.next().filter(char::is_ascii_alphabetic)?;
    if chars.next() != Some(':') {
        return None;
    }
    let rest = executable.get(2..)?.replace('\\', "/");
    let device = prefix
        .join("dosdevices")
        .join(format!("{}:", drive.to_ascii_lowercase()));
    let root = std::fs::canonicalize(device).ok()?;
    Some(root.join(rest.trim_start_matches('/')))
}

fn game_among<'a>(
    games: &'a [DetectedGame],
    process_name: &str,
    install_path: Option<&Path>,
) -> Option<&'a DetectedGame> {
    let by_folder = install_path.and_then(|dir| {
        games.iter().find(|g| {
            g.install_path
                .as_deref()
                .is_some_and(|root| dir.starts_with(root) || root.starts_with(dir))
        })
    });
    by_folder.or_else(|| {
        let mut listing = games.iter().filter(|g| {
            g.executables
                .iter()
                .any(|e| e.eq_ignore_ascii_case(process_name))
        });
        let first = listing.next()?;
        // A name two games share (`Game.exe`, `nw.exe`) says neither.
        listing.next().is_none().then_some(first)
    })
}

/// Cover art Steam has already downloaded for `app_id`.
///
/// Steam keeps covers under a per-app directory in a further hash-named
/// subdirectory, so the search recurses rather than building a fixed path. The
/// filename preference degrades gracefully: not every title has
/// `library_600x900.jpg` (Dead by Daylight has only `library_capsule.jpg`).
#[must_use]
pub fn steam_cover(home: &Path, app_id: &str) -> Option<PathBuf> {
    steam_cover_in(&steam_libraries(home), app_id)
}

fn steam_cover_in(libraries: &[PathBuf], app_id: &str) -> Option<PathBuf> {
    // Portrait first: the card layout is a 2:3 poster.
    const PREFERRED: &[&str] = &[
        "library_600x900.jpg",
        "library_600x900_2x.jpg",
        "library_capsule.jpg",
        "library_header.jpg",
        "header.jpg",
    ];
    for root in libraries {
        let app_dir = root.join("appcache/librarycache").join(app_id);
        if !app_dir.is_dir() {
            continue;
        }
        for wanted in PREFERRED {
            if let Some(found) = find_file_named(&app_dir, wanted, 2) {
                return Some(found);
            }
        }
    }
    None
}

/// Depth-limited search for a file with an exact name.
fn find_file_named(dir: &Path, filename: &str, depth: u32) -> Option<PathBuf> {
    let direct = dir.join(filename);
    if direct.is_file() {
        return Some(direct);
    }
    if depth == 0 {
        return None;
    }
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if let Some(found) = find_file_named(&path, filename, depth - 1) {
                return Some(found);
            }
        }
    }
    None
}

// ── Executable discovery ─────────────────────────────────────────────────────

/// Substrings that mark a binary as tooling rather than the game.
const NOT_THE_GAME: &[&str] = &[
    "unitycrashhandler",
    "crashhandler",
    "crashreport",
    "anticheat",
    "easyanticheat",
    "battleye",
    "vcredist",
    "directx",
    "dxsetup",
    "dxwebsetup",
    "dotnetfx",
    "installer",
    "setup",
    "uninstall",
    "unins0",
    "redist",
    "launcher_installer",
    "ue4prereqsetup",
    "ueprereqsetup",
    "oalinst",
    "vc_redist",
    "physx",
    "errorreporter",
    // Store overlays and helper browsers ship inside game directories and are
    // often larger than the game binary itself.
    "epicwebhelper",
    "webhelper",
    "cefprocess",
    "cefsubprocess",
    "crashpad",
    "steamerrorreporter",
    "eosoverlayrenderer",
    "eosbootstrapper",
    "unrealcefsubprocess",
    // EA's installer helpers, which run under the game's reaper on its first
    // start for over a minute.
    "touchup",
    "cleanup",
    "activationui",
    // BattlEye's service and the `_BE` starters next to protected games.
    "beservice",
    "_be.exe",
    "start_protected_game",
];

/// Exact names of helpers too short to match as substrings.
const NOT_THE_GAME_NAMES: &[&str] = &["7z.exe", "7za.exe", "7zr.exe"];

/// Folders inside an install directory that hold redistributables,
/// installers and anti-cheat, never the game.
const SUPPORT_DIRS: &[&str] = &[
    "__installer",
    "_commonredist",
    "commonredist",
    "redist",
    "_redist",
    "redistributables",
    "redistributable",
    "easyanticheat",
    "battleye",
    "support",
    "directx",
    "vcredist",
    "dotnet",
    "prereqs",
    "prerequisites",
    "installers",
    // Engine/Binaries/ThirdParty: CEF, PhysX, Oodle.
    "thirdparty",
];

/// Whether a filename looks like a support tool rather than the game itself.
#[must_use]
pub fn is_support_binary(filename: &str) -> bool {
    let lower = filename.to_ascii_lowercase();
    NOT_THE_GAME.iter().any(|needle| lower.contains(needle))
        || NOT_THE_GAME_NAMES.contains(&lower.as_str())
        // .NET's installers: NDP472-KB4054530-x86-x64-AllOS-ENU.exe.
        || lower
            .strip_prefix("ndp")
            .is_some_and(|rest| rest.starts_with(|c: char| c.is_ascii_digit()))
}

/// Whether a folder of an install directory holds only support files.
fn is_support_dir(name: &str) -> bool {
    SUPPORT_DIRS.contains(&name.to_ascii_lowercase().as_str())
}

/// Find the process names a game is likely to run under, best first.
///
/// See [`ranked_executables`]; this is the ranking with nothing recorded
/// about the game.
#[must_use]
pub fn find_executables(install_dir: &Path) -> Vec<String> {
    ranked_executables(install_dir, None)
}

/// One binary found in an install directory.
#[derive(Debug, Clone)]
struct Found {
    name: String,
    size: u64,
}

/// The process names a game in `install_dir` is likely to run under, best
/// first, given the file its launcher records starting (`launch`).
///
/// Ranked, in order:
/// 1. an Unreal Engine `*-Shipping` binary: the game's own process, which
///    the small `<Game>.exe` at the top of the folder only starts (The
///    Callisto Protocol, The Outer Worlds' `IndianaEpicGameStore-…`);
/// 2. the variants of the recorded file it chooses between (Control's
///    `Control.exe` starts `Control_DX12.exe` or `Control_DX11.exe`);
/// 3. the recorded file;
/// 4. the rest, largest first, since the game's binary is nearly always the
///    largest a title ships;
/// 5. trials, demos, tests, servers, editors and launchers, which can be the
///    largest (Unravel Two's trial is bigger than the game).
///
/// Windows executables are searched up to four folders deep, since engines
/// bury the real binary under `Binaries/Win64/`.
#[must_use]
pub fn ranked_executables(install_dir: &Path, launch: Option<&Path>) -> Vec<String> {
    let mut found = Vec::new();
    collect_executables(install_dir, 4, &mut found);
    rank(found, launch.filter(|l| usable_launch_file(l)))
}

fn rank(mut found: Vec<Found>, launch: Option<&Path>) -> Vec<String> {
    let launch_name = launch
        .and_then(Path::file_name)
        .map(|n| n.to_string_lossy().into_owned());
    if let (Some(path), Some(name)) = (launch, &launch_name) {
        // The recorded file may be too small to have been found (Control's
        // selector) or deeper than the search.
        if !found.iter().any(|f| f.name == *name) {
            let size = std::fs::metadata(path).map_or(0, |m| m.len());
            found.push(Found {
                name: name.clone(),
                size,
            });
        }
    }
    let launch_stem = launch_name
        .as_deref()
        .map(|n| stem_of(n).to_ascii_lowercase());
    let tier = |f: &Found| -> u8 {
        let lower = f.name.to_ascii_lowercase();
        if is_secondary_build(&f.name) {
            4
        } else if lower.contains("-shipping") {
            0
        } else if let Some(stem) = &launch_stem {
            let variant = stem_of(&lower)
                .strip_prefix(stem.as_str())
                .is_some_and(|rest| rest.starts_with(['_', '-']));
            if variant {
                1
            } else if launch_name.as_deref() == Some(f.name.as_str()) {
                2
            } else {
                3
            }
        } else {
            3
        }
    };
    found.sort_by(|a, b| {
        tier(a)
            .cmp(&tier(b))
            .then(b.size.cmp(&a.size))
            .then_with(|| a.name.cmp(&b.name))
    });
    let mut seen = HashSet::new();
    let mut names: Vec<String> = found
        .into_iter()
        .map(|f| f.name)
        .filter(|n| seen.insert(n.clone()))
        .collect();
    names.truncate(8);
    names
}

/// A file name without its `.exe`.
fn stem_of(name: &str) -> &str {
    let len = name.len();
    if len > 4 && name[len - 4..].eq_ignore_ascii_case(".exe") {
        &name[..len - 4]
    } else {
        name
    }
}

/// Words that mark a binary as another build than the one played: a trial,
/// a demo, a test or development build, a server, an editor, a benchmark.
const SECONDARY_WORDS: &[&str] = &[
    "trial",
    "demo",
    "test",
    "server",
    "dedicated",
    "editor",
    "benchmark",
    "debug",
    "sdk",
    "config",
    "configurator",
    "settings",
];

/// Whether a binary's name says it is a secondary build or a launcher. Words
/// are split at `_`, `-`, `.`, spaces and case changes, so `UnravelTwo_trial`
/// and `Phoenix-Win64-Test` count and `Observer` does not.
fn is_secondary_build(name: &str) -> bool {
    let stem = stem_of(name);
    stem.to_ascii_lowercase().contains("launcher")
        || words(stem)
            .iter()
            .any(|w| SECONDARY_WORDS.contains(&w.as_str()))
}

/// The lowercase words of a name.
fn words(name: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut previous: Option<char> = None;
    for c in name.chars() {
        let boundary = !c.is_ascii_alphanumeric()
            || previous.is_some_and(|p| {
                (p.is_ascii_lowercase() && c.is_ascii_uppercase())
                    || (p.is_ascii_alphabetic() != c.is_ascii_alphabetic())
            });
        if boundary && !current.is_empty() {
            out.push(std::mem::take(&mut current).to_ascii_lowercase());
        }
        if c.is_ascii_alphanumeric() {
            current.push(c);
        }
        previous = Some(c);
    }
    if !current.is_empty() {
        out.push(current.to_ascii_lowercase());
    }
    out
}

/// Whether a file a launcher records can name the game's process: there, not
/// a script (the shell is the process, the game what it starts), not a
/// launcher or helper.
fn usable_launch_file(path: &Path) -> bool {
    let Some(name) = path.file_name().map(|n| n.to_string_lossy().into_owned()) else {
        return false;
    };
    path.is_file()
        && !is_script(path)
        && !is_support_binary(&name)
        && !crate::running::is_infrastructure(&name)
}

/// Whether a file starts with `#!`.
fn is_script(path: &Path) -> bool {
    use std::io::Read;
    let mut magic = [0u8; 2];
    std::fs::File::open(path)
        .and_then(|mut f| f.read_exact(&mut magic))
        .is_ok_and(|()| &magic == b"#!")
}

fn collect_executables(dir: &Path, depth: u32, out: &mut Vec<Found>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        let Some(filename) = path.file_name().map(|n| n.to_string_lossy().into_owned()) else {
            continue;
        };
        if meta.is_dir() {
            if depth > 0 && !is_support_dir(&filename) {
                collect_executables(&path, depth - 1, out);
            }
            continue;
        }
        if !meta.is_file() || is_support_binary(&filename) {
            continue;
        }
        // A real game binary is never tiny; this also filters wrapper scripts.
        if meta.len() < 256 * 1024 {
            continue;
        }
        let is_windows = filename.to_ascii_lowercase().ends_with(".exe");
        if is_windows || (!is_shared_library_name(&filename) && is_native_executable(&path, &meta))
        {
            out.push(Found {
                name: filename,
                size: meta.len(),
            });
        }
    }
}

/// `libcef.so`, `libvulkan.so.1`: a library, whatever its permissions.
fn is_shared_library_name(name: &str) -> bool {
    let mut rest = name;
    // Strip trailing `.<digits>` version parts, then look for `.so`.
    while let Some((head, tail)) = rest.rsplit_once('.') {
        if !tail.is_empty() && tail.chars().all(|c| c.is_ascii_digit()) {
            rest = head;
        } else {
            break;
        }
    }
    Path::new(rest)
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("so"))
}

/// The ELF machine this program runs on: a binary for another architecture
/// (Altered Beast ships `arm64/` beside `x64/`) is not what runs here.
const fn host_elf_machine() -> &'static [u16] {
    if cfg!(target_arch = "x86_64") {
        // x86-64, and i386 binaries run here too.
        &[62, 3]
    } else if cfg!(target_arch = "aarch64") {
        &[183]
    } else if cfg!(target_arch = "x86") {
        &[3]
    } else {
        &[]
    }
}

/// Whether `path` is an executable ELF program for this machine: the
/// executable bit, an `ET_EXEC` image or an `ET_DYN` one with an interpreter
/// (a position-independent executable). A shared library is `ET_DYN` with
/// none, so `UnityPlayer.so` or `libcef.so` with the executable bit set is
/// not taken for the game.
fn is_native_executable(path: &Path, meta: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    if meta.permissions().mode() & 0o111 == 0 {
        return false;
    }
    elf_program(path).unwrap_or(false)
}

fn elf_program(path: &Path) -> std::io::Result<bool> {
    use std::io::{Read, Seek, SeekFrom};
    const ET_EXEC: u16 = 2;
    const ET_DYN: u16 = 3;
    const PT_INTERP: u32 = 3;
    let mut file = std::fs::File::open(path)?;
    let mut header = [0u8; 64];
    file.read_exact(&mut header[..52])?;
    if &header[..4] != b"\x7fELF" || header[5] != 1 {
        // Not ELF, or big-endian.
        return Ok(false);
    }
    let wide = match header[4] {
        1 => false,
        2 => true,
        _ => return Ok(false),
    };
    if wide {
        file.read_exact(&mut header[52..64])?;
    }
    let u16_at = |at: usize| u16::from_le_bytes([header[at], header[at + 1]]);
    let kind = u16_at(16);
    if !host_elf_machine().contains(&u16_at(18)) {
        return Ok(false);
    }
    if kind == ET_EXEC {
        return Ok(true);
    }
    if kind != ET_DYN {
        return Ok(false);
    }
    let (phoff, phentsize, phnum) = if wide {
        let mut off = [0u8; 8];
        off.copy_from_slice(&header[32..40]);
        (u64::from_le_bytes(off), u16_at(54), u16_at(56))
    } else {
        let off = u32::from_le_bytes([header[28], header[29], header[30], header[31]]);
        (u64::from(off), u16_at(42), u16_at(44))
    };
    if phentsize < 4 || phnum == 0 || phnum > 128 {
        return Ok(false);
    }
    let mut table = vec![0u8; usize::from(phentsize) * usize::from(phnum)];
    file.seek(SeekFrom::Start(phoff))?;
    file.read_exact(&mut table)?;
    Ok(table
        .chunks_exact(usize::from(phentsize))
        .any(|entry| u32::from_le_bytes([entry[0], entry[1], entry[2], entry[3]]) == PT_INTERP))
}

/// The process names of a game known only by the file its launcher runs
/// (Lutris, Faugus): that file's name, after an Unreal `*-Shipping` binary
/// beside it; for a script, the programs of its folder, since the shell is
/// the process and the game what it starts.
fn executables_for_file(launch_file: &Path) -> Vec<String> {
    let Some(dir) = launch_file.parent() else {
        return Vec::new();
    };
    if is_script(launch_file) {
        // A script on PATH (`/usr/bin`) starts a program from anywhere; its
        // folder says nothing about the game.
        let system =
            std::env::var_os("PATH").is_some_and(|p| std::env::split_paths(&p).any(|d| d == dir));
        return if system {
            Vec::new()
        } else {
            find_executables(dir)
        };
    }
    let mut names = unreal_shipping_beside(dir);
    if let Some(name) = launch_file
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
    {
        if !names.contains(&name) {
            names.push(name);
        }
    }
    names
}

/// The `*-Shipping` binaries of an Unreal game whose bootstrap is in `dir`:
/// `<dir>/<Project>/Binaries/{Win64,Linux}/<Project>-Win64-Shipping.exe`.
/// Looked up at that one place rather than searched for, since a launch
/// file's folder may be a downloads folder full of other things.
fn unreal_shipping_beside(dir: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut names = Vec::new();
    for entry in entries.flatten().take(256) {
        for platform in ["Binaries/Win64", "Binaries/Linux"] {
            let Ok(binaries) = std::fs::read_dir(entry.path().join(platform)) else {
                continue;
            };
            names.extend(
                binaries
                    .flatten()
                    .filter_map(|b| b.file_name().into_string().ok())
                    .filter(|n| {
                        n.to_ascii_lowercase().contains("-shipping")
                            && !is_support_binary(n)
                            && !is_secondary_build(n)
                    }),
            );
        }
    }
    names.sort();
    names
}

// ── Lutris ───────────────────────────────────────────────────────────────────

/// One Lutris installation: where it keeps game configurations and its data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LutrisRoot {
    /// The `games/` directory of `.yml` configurations.
    pub games: PathBuf,
    /// The data directory holding `coverart/` and `banners/`.
    pub data: PathBuf,
    /// The cache directory holding a second `coverart/`.
    pub cache: PathBuf,
}

/// The native Lutris and the Flatpak one. Lutris keeps its game files under
/// `XDG_CONFIG_HOME` and, from 0.5.20 on a new install, under
/// `XDG_DATA_HOME`.
fn lutris_roots(home: &Path, xdg: &Xdg) -> Vec<LutrisRoot> {
    let flatpak = home.join(".var/app/net.lutris.Lutris");
    let data = xdg.data.join("lutris");
    let cache = xdg.cache.join("lutris");
    let mut roots = vec![
        LutrisRoot {
            games: xdg.config.join("lutris/games"),
            data: data.clone(),
            cache: cache.clone(),
        },
        LutrisRoot {
            games: data.join("games"),
            data,
            cache,
        },
        LutrisRoot {
            games: flatpak.join("config/lutris/games"),
            data: flatpak.join("data/lutris"),
            cache: flatpak.join("cache/lutris"),
        },
        LutrisRoot {
            games: flatpak.join("data/lutris/games"),
            data: flatpak.join("data/lutris"),
            cache: flatpak.join("cache/lutris"),
        },
    ];
    roots.dedup();
    roots
}

/// What a Lutris game configuration says about starting the game.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LutrisConfig {
    /// `name`, when the file declares one.
    pub name: Option<String>,
    /// `runner` (`wine`, `linux`, `steam`, …).
    pub runner: Option<String>,
    /// `game.exe`.
    pub exe: Option<String>,
    /// `game.main_file` (emulators and engines).
    pub main_file: Option<String>,
    /// `game.working_dir`.
    pub working_dir: Option<String>,
    /// `game.args`: the game's arguments, as a shell would split them.
    pub args: Option<String>,
    /// `game.prefix` (Wine).
    pub prefix: Option<String>,
    /// The game's folder in Lutris's database (`games.directory`), which
    /// `$GAMEDIR` stands for; the YAML does not record it.
    pub directory: Option<String>,
}

/// Pull the launch keys out of a Lutris game YAML.
///
/// Top-level `name` and `runner`, and the `exe`, `main_file`, `working_dir`
/// and `prefix` directly in the top-level `game` section. A line scan rather
/// than a YAML parser: the files are flat. The same keys inside other
/// sections — an installer's `script: game: exe:`, a `wine: prefix:` — are
/// not the game's and are skipped.
#[must_use]
pub fn parse_lutris_yml(content: &str) -> LutrisConfig {
    let mut cfg = LutrisConfig::default();
    let mut section = String::new();
    // A mapping nested in the current section (`game: { foo: { exe: … } }`),
    // by the indentation of its key: what is inside it is not the section's.
    let mut nested: Option<usize> = None;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let indent = line.len() - line.trim_start().len();
        let Some((key, value)) = trimmed.split_once(':') else {
            continue;
        };
        let key = key.trim();
        if indent == 0 {
            key.clone_into(&mut section);
            nested = None;
        } else if nested.is_some_and(|n| indent <= n) {
            nested = None;
        }
        if nested.is_some() {
            continue;
        }
        // The arguments keep their own quotes: only YAML's around the whole
        // value go.
        if indent > 0 && section == "game" && key == "args" && cfg.args.is_none() {
            let raw = value.trim();
            let unquoted = ['"', '\'']
                .iter()
                .find_map(|q| raw.strip_prefix(*q)?.strip_suffix(*q))
                .unwrap_or(raw);
            cfg.args = Some(unquoted.to_owned()).filter(|a| !a.is_empty());
            continue;
        }
        let value = value.trim().trim_matches(['"', '\'']);
        if value.is_empty() {
            if indent > 0 {
                nested = Some(indent);
            }
            continue;
        }
        let slot = match (indent, section.as_str(), key) {
            (0, _, "name") => &mut cfg.name,
            (0, _, "runner") => &mut cfg.runner,
            (1.., "game", "exe") => &mut cfg.exe,
            (1.., "game", "main_file") => &mut cfg.main_file,
            (1.., "game", "working_dir") => &mut cfg.working_dir,
            (1.., "game", "prefix") => &mut cfg.prefix,
            _ => continue,
        };
        if slot.is_none() {
            *slot = Some(value.to_owned());
        }
    }
    cfg
}

/// Runners whose games are not started from a file of their own: Steam's
/// belong to Steam (and are listed there when installed), Flatpaks to the
/// menu, and a web game to a browser.
const LUTRIS_INDIRECT_RUNNERS: &[&str] = &["steam", "flatpak", "web", "browser"];

impl LutrisConfig {
    /// The file Lutris would run, if the configuration names one and it is
    /// still there; otherwise why the game does not count as installed.
    ///
    /// # Errors
    /// The reason, for the debug log.
    pub fn launch_file(&self) -> Result<PathBuf, &'static str> {
        if self
            .runner
            .as_deref()
            .is_some_and(|r| LUTRIS_INDIRECT_RUNNERS.contains(&r))
        {
            return Err("started through another launcher");
        }
        let expand = |value: &str| -> Option<String> {
            let value = match &self.directory {
                Some(dir) => value.replace("$GAMEDIR", dir),
                None => value.to_owned(),
            };
            // Other variables are Lutris's own business.
            (!value.contains('$')).then_some(value)
        };
        let file = self
            .exe
            .as_deref()
            .or(self.main_file.as_deref())
            .ok_or("no executable recorded")?;
        let file = expand(file).ok_or("path uses a Lutris variable")?;
        let file = Path::new(&file);
        let path = if file.is_absolute() {
            file.to_path_buf()
        } else {
            // Lutris runs a relative file from the working directory, else
            // the game's folder, else (Wine) the prefix.
            let base = self
                .working_dir
                .as_deref()
                .or(self.directory.as_deref())
                .or(self.prefix.as_deref())
                .and_then(expand)
                .ok_or("relative path with no directory")?;
            Path::new(&base).join(file)
        };
        if path.is_file() {
            Ok(path)
        } else {
            Err("executable missing")
        }
    }
}

/// The installed games of every Lutris in `roots`.
#[must_use]
pub fn lutris_games(roots: &[LutrisRoot]) -> Vec<DetectedGame> {
    let mut games = Vec::new();
    for root in roots {
        let Ok(entries) = std::fs::read_dir(&root.games) else {
            continue;
        };
        let database = lutris_database(&root.data);
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_none_or(|e| e != "yml") {
                continue;
            }
            let Some(stem) = path.file_stem().map(|s| s.to_string_lossy().into_owned()) else {
                continue;
            };
            let slug = strip_numeric_suffix(&stem);
            let Ok(content) = std::fs::read_to_string(&path) else {
                continue;
            };
            let mut cfg = parse_lutris_yml(&content);
            // The database's title is the one Lutris shows; the file's
            // name, if any, may be an installer's.
            if let Some(record) = database.get(&stem) {
                cfg.name = record.name.clone().or(cfg.name);
                cfg.runner = cfg.runner.or_else(|| record.runner.clone());
                cfg.directory.clone_from(&record.directory);
            }
            let launch_file = match cfg.launch_file() {
                Ok(file) => file,
                Err(reason) => {
                    tracing::debug!(config = %path.display(), reason, "ignoring Lutris game");
                    continue;
                }
            };
            let name = cfg.name.unwrap_or_else(|| slug_to_title(slug));
            // Only what runs here directly is offered as launchable: a
            // Windows binary needs its Wine runner. It runs as Lutris runs
            // it, in its working directory and with its arguments.
            let launch_command = runs_natively(&launch_file).then(|| {
                lutris_command(
                    &launch_file,
                    cfg.working_dir.as_deref(),
                    cfg.args.as_deref(),
                )
            });
            games.push(DetectedGame {
                cover: lutris_cover(root, slug),
                executables: executables_for_file(&launch_file),
                name,
                source: Source::Lutris,
                app_id: None,
                install_path: None,
                launch_file: Some(launch_file),
                icon: None,
                launch_command,
                launcher: Some(LauncherRef::Lutris {
                    config_file: path.clone(),
                }),
            });
        }
    }
    games
}

/// What Lutris's database says about a game.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct LutrisRecord {
    name: Option<String>,
    runner: Option<String>,
    directory: Option<String>,
}

/// The games of Lutris's database (`<data>/pga.db`), by the name of their
/// configuration file without `.yml` (`configpath`).
fn lutris_database(data: &Path) -> HashMap<String, LutrisRecord> {
    let text = |row: &sqlite::Row, column: &str| {
        row.get(column)
            .and_then(sqlite::Value::text)
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(str::to_owned)
    };
    sqlite::read_table(&data.join("pga.db"), "games")
        .iter()
        .filter_map(|row| {
            Some((
                text(row, "configpath")?,
                LutrisRecord {
                    name: text(row, "name"),
                    runner: text(row, "runner"),
                    directory: text(row, "directory"),
                },
            ))
        })
        .collect()
}

/// Strip Lutris's trailing `-<digits>` install id from a slug.
#[must_use]
pub fn strip_numeric_suffix(slug: &str) -> &str {
    slug.rfind('-')
        .filter(|i| {
            let suffix = &slug[i + 1..];
            !suffix.is_empty() && suffix.chars().all(|c| c.is_ascii_digit())
        })
        .map_or(slug, |i| &slug[..i])
}

/// Turn a slug into a readable title.
#[must_use]
pub fn slug_to_title(slug: &str) -> String {
    slug.split(['-', '_'])
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut c = w.chars();
            c.next()
                .map_or_else(String::new, |f| f.to_uppercase().to_string() + c.as_str())
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn lutris_cover(root: &LutrisRoot, slug: &str) -> Option<PathBuf> {
    for base in [
        root.data.join("coverart"),
        root.cache.join("coverart"),
        root.data.join("banners"),
    ] {
        for ext in ["jpg", "png", "webp"] {
            let path = base.join(format!("{slug}.{ext}"));
            if path.is_file() {
                return Some(path);
            }
        }
    }
    None
}

// ── Heroic ───────────────────────────────────────────────────────────────────

/// The native Heroic's configuration directory (Electron's, under
/// `XDG_CONFIG_HOME`) and the Flatpak one's.
fn heroic_config_dirs(home: &Path, xdg: &Xdg) -> Vec<PathBuf> {
    vec![
        xdg.config.join("heroic"),
        home.join(".var/app/com.heroicgameslauncher.hgl/config/heroic"),
    ]
}

/// Whether a Heroic record is a DLC's: its `is_dlc`, at the top or in its
/// `install` block. A DLC shares its game's folder and would otherwise give
/// the game's card its own title.
fn heroic_is_dlc(record: &serde_json::Value) -> bool {
    let flag =
        |v: &serde_json::Value| v.get("is_dlc").and_then(serde_json::Value::as_bool) == Some(true);
    flag(record) || record.get("install").is_some_and(flag)
}

/// A game Heroic's records say is installed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HeroicEntry {
    /// Title.
    pub title: String,
    /// Install directory, as recorded.
    pub install_path: Option<PathBuf>,
    /// The executable, as recorded: absolute, or relative to the install
    /// directory.
    pub executable: Option<PathBuf>,
    /// The game's id in Heroic (`app_name`), which names its settings file.
    pub app_name: Option<String>,
    /// Its artwork's addresses (`art_square`, then `art_cover`): Heroic keeps
    /// what it downloaded under `images-cache/`, named by their SHA-256.
    pub art: Vec<String>,
    /// The store backend whose file listed it ([`LauncherRef::Heroic`]).
    pub runner: Option<&'static str>,
}

/// The installed games among a Heroic store library (`store_cache/*_library.json`,
/// `sideload_apps/library.json`): entries with `is_installed` and their
/// `install` block. Missing or unexpected fields skip the entry, not the file.
#[must_use]
pub fn heroic_library_entries(json: &str) -> Vec<HeroicEntry> {
    let Ok(root) = serde_json::from_str::<serde_json::Value>(json) else {
        return Vec::new();
    };
    let list = root
        .get("library")
        .or_else(|| root.get("games"))
        .and_then(serde_json::Value::as_array);
    list.map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .filter(|g| g.get("is_installed").and_then(serde_json::Value::as_bool) == Some(true))
        .filter(|g| !heroic_is_dlc(g))
        .filter_map(|g| {
            let install = g.get("install")?;
            let string = |v: &serde_json::Value, key: &str| {
                v.get(key)
                    .and_then(serde_json::Value::as_str)
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
            };
            Some(HeroicEntry {
                title: string(g, "title")?,
                install_path: string(install, "install_path").map(PathBuf::from),
                executable: string(install, "executable").map(PathBuf::from),
                app_name: string(g, "app_name"),
                art: heroic_art(g),
                runner: None,
            })
        })
        .collect()
}

/// The games of a store backend's own `installed.json`: legendary's map of
/// app name to record, GOG's `{"installed": [...]}` and Amazon's list. Each
/// record names its `install_path` (Amazon: `path`) and, when it can, its
/// `title` and `executable`. A record without a title takes the one the store
/// library has for its app name (`titles`), whether or not that library still
/// marks it installed, else its install folder's name: GOG's records carry
/// only a numeric id.
#[must_use]
pub(crate) fn heroic_installed_entries(
    json: &str,
    titles: &HashMap<String, String>,
) -> Vec<HeroicEntry> {
    let Ok(root) = serde_json::from_str::<serde_json::Value>(json) else {
        return Vec::new();
    };
    let records: Vec<&serde_json::Value> = match &root {
        serde_json::Value::Array(list) => list.iter().collect(),
        serde_json::Value::Object(map) => match map.get("installed") {
            Some(serde_json::Value::Array(list)) => list.iter().collect(),
            _ => map.values().collect(),
        },
        _ => Vec::new(),
    };
    records
        .into_iter()
        .filter(|r| !heroic_is_dlc(r))
        .filter_map(|r| {
            let string = |key: &str| {
                r.get(key)
                    .and_then(serde_json::Value::as_str)
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
            };
            let install_path = string("install_path").or_else(|| string("path"))?;
            let app_name = string("app_name")
                .or_else(|| string("appName"))
                .or_else(|| string("id"));
            let folder = Path::new(&install_path)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .filter(|n| !n.is_empty());
            Some(HeroicEntry {
                title: string("title")
                    .or_else(|| app_name.as_ref().and_then(|a| titles.get(a)).cloned())
                    .or(folder)
                    .or_else(|| app_name.clone())
                    .unwrap_or_else(|| install_path.clone()),
                executable: string("executable").map(PathBuf::from),
                install_path: Some(PathBuf::from(install_path)),
                app_name,
                art: heroic_art(r),
                runner: None,
            })
        })
        .collect()
}

/// Every title a store library lists, by app name, installed or not.
#[must_use]
pub fn heroic_library_titles(json: &str) -> HashMap<String, String> {
    let Ok(root) = serde_json::from_str::<serde_json::Value>(json) else {
        return HashMap::new();
    };
    root.get("library")
        .or_else(|| root.get("games"))
        .and_then(serde_json::Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .filter_map(|g| {
            let name = g.get("app_name")?.as_str()?;
            let title = g.get("title")?.as_str().filter(|t| !t.is_empty())?;
            Some((name.to_owned(), title.to_owned()))
        })
        .collect()
}

/// A Heroic record's artwork addresses, portrait first.
fn heroic_art(record: &serde_json::Value) -> Vec<String> {
    ["art_square", "art_cover"]
        .iter()
        .filter_map(|k| record.get(*k).and_then(serde_json::Value::as_str))
        .filter(|u| u.starts_with("https://") || u.starts_with("http://"))
        .map(str::to_owned)
        .collect()
}

/// The artwork of every game a Heroic store library lists, installed or
/// not, by `app_name`: a backend's `installed.json` names no artwork.
#[must_use]
pub fn heroic_library_art(json: &str) -> HashMap<String, Vec<String>> {
    let Ok(root) = serde_json::from_str::<serde_json::Value>(json) else {
        return HashMap::new();
    };
    root.get("library")
        .or_else(|| root.get("games"))
        .and_then(serde_json::Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .filter_map(|g| {
            let name = g.get("app_name")?.as_str()?.to_owned();
            let art = heroic_art(g);
            (!art.is_empty()).then_some((name, art))
        })
        .collect()
}

/// Files Heroic writes about installed games, relative to its configuration
/// directory. The store libraries carry titles and the installed flag; the
/// backends' own lists are the record of what was actually installed.
///
/// Each with the runner a `heroic://launch` link names for its games; Heroic
/// 2.22 knows `legendary`, `gog`, `nile` and `sideload`, not Zoom.
const HEROIC_LIBRARIES: &[(&str, Option<&str>)] = &[
    ("store_cache/legendary_library.json", Some("legendary")),
    ("store_cache/gog_library.json", Some("gog")),
    ("store_cache/nile_library.json", Some("nile")),
    ("store_cache/zoom-library.json", None),
    ("sideload_apps/library.json", Some("sideload")),
];
const HEROIC_INSTALLED: &[(&str, Option<&str>)] = &[
    (
        "legendaryConfig/legendary/installed.json",
        Some("legendary"),
    ),
    ("gog_store/installed.json", Some("gog")),
    ("nile_config/nile/installed.json", Some("nile")),
];

/// The installed games of every Heroic in `configs`.
#[must_use]
pub fn heroic_games(configs: &[PathBuf]) -> Vec<DetectedGame> {
    let mut games = Vec::new();
    for base in configs {
        let mut entries = Vec::new();
        let mut art = HashMap::new();
        let mut titles = HashMap::new();
        let with_runner = |list: Vec<HeroicEntry>, runner: Option<&'static str>| {
            list.into_iter().map(move |e| HeroicEntry { runner, ..e })
        };
        for (file, runner) in HEROIC_LIBRARIES {
            if let Ok(json) = std::fs::read_to_string(base.join(file)) {
                entries.extend(with_runner(heroic_library_entries(&json), *runner));
                art.extend(heroic_library_art(&json));
                titles.extend(heroic_library_titles(&json));
            }
        }
        for (file, runner) in HEROIC_INSTALLED {
            if let Ok(json) = std::fs::read_to_string(base.join(file)) {
                entries.extend(with_runner(
                    heroic_installed_entries(&json, &titles),
                    *runner,
                ));
            }
        }
        for mut entry in entries {
            if entry.art.is_empty() {
                if let Some(found) = entry.app_name.as_ref().and_then(|a| art.get(a)) {
                    entry.art.clone_from(found);
                }
            }
            let Some(game) = heroic_game(base, entry) else {
                continue;
            };
            games.push(game);
        }
    }
    games
}

/// `argv` run in `dir` (`env -C`), so a game Big Game Mode starts finds what
/// it opens relative to its working directory; `argv` alone without one.
fn in_directory(dir: Option<&Path>, argv: Vec<String>) -> Vec<String> {
    match dir.filter(|d| d.is_absolute()) {
        Some(dir) => [
            "env".to_owned(),
            "-C".to_owned(),
            dir.to_string_lossy().into_owned(),
        ]
        .into_iter()
        .chain(argv)
        .collect(),
        None => argv,
    }
}

/// How Lutris runs a native game: `file` with its arguments (`args`, split
/// as a shell splits them), in its working directory, or the file's own
/// folder when it names none (relative to the file's folder when relative).
fn lutris_command(file: &Path, working_dir: Option<&str>, args: Option<&str>) -> Vec<String> {
    let folder = file.parent();
    let dir = match (working_dir.map(Path::new), folder) {
        (Some(d), _) if d.is_absolute() => Some(d.to_path_buf()),
        (Some(d), Some(f)) => Some(f.join(d)),
        (None, f) => f.map(Path::to_path_buf),
        (Some(_), None) => None,
    };
    let mut command = vec![file.to_string_lossy().into_owned()];
    command.extend(shell_words(args.unwrap_or_default()));
    in_directory(dir.as_deref(), command)
}

/// `text` split into words as a POSIX shell splits it: blanks separate,
/// single quotes keep everything, double quotes and a backslash keep the
/// next character.
fn shell_words(text: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut started = false;
    let mut quote: Option<char> = None;
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some('\''), c) => word.push(c),
            (_, '\\') => {
                if let Some(next) = chars.next() {
                    word.push(next);
                }
                started = true;
            }
            (Some(_), c) => word.push(c),
            (None, '"' | '\'') => {
                quote = Some(c);
                started = true;
            }
            (None, c) if c.is_whitespace() => {
                if started {
                    words.push(std::mem::take(&mut word));
                    started = false;
                }
            }
            (None, c) => {
                word.push(c);
                started = true;
            }
        }
    }
    if started {
        words.push(word);
    }
    words
}

/// A Heroic record as a game, when its files are there.
fn heroic_game(base: &Path, entry: HeroicEntry) -> Option<DetectedGame> {
    let install_path = entry.install_path.filter(|p| populated_dir(p));
    let executable = entry.executable.map(|exe| match &install_path {
        Some(root) if exe.is_relative() => root.join(exe),
        _ => exe,
    });
    let launch_file = executable.filter(|exe| exe.is_file());
    if install_path.is_none() && launch_file.is_none() {
        tracing::debug!(title = %entry.title, "ignoring Heroic game: install path missing");
        return None;
    }
    // Started in its own folder: many native ports open their data
    // relative to the working directory.
    let launch_command = launch_file
        .as_deref()
        .filter(|exe| runs_natively(exe))
        .map(|exe| in_directory(exe.parent(), vec![exe.to_string_lossy().into_owned()]));
    Some(DetectedGame {
        cover: heroic_cover(base, &entry.art),
        executables: match (&install_path, &launch_file) {
            (Some(root), launch) => ranked_executables(root, launch.as_deref()),
            (None, Some(file)) => executables_for_file(file),
            (None, None) => Vec::new(),
        },
        name: entry.title,
        source: Source::Heroic,
        app_id: None,
        install_path,
        launch_file,
        icon: None,
        launch_command,
        launcher: entry.app_name.map(|app_name| LauncherRef::Heroic {
            app_name,
            config_dir: base.to_path_buf(),
            runner: entry.runner,
        }),
    })
}

/// The first of `art` Heroic has downloaded: `images-cache/<SHA-256 of the
/// address>`, without an extension.
fn heroic_cover(base: &Path, art: &[String]) -> Option<PathBuf> {
    let dir = base.join("images-cache");
    art.iter().find_map(|url| {
        let path = dir.join(crate::graphics::manifest::sha256_bytes(url.as_bytes()));
        path.is_file().then_some(path)
    })
}

// ── Faugus Launcher ──────────────────────────────────────────────────────────

/// Faugus Launcher's list of games: `faugus-launcher/games.json` under
/// `XDG_DATA_HOME` (under `XDG_CONFIG_HOME` before it moved), for the
/// native application and the Flatpak.
fn faugus_files(home: &Path, xdg: &Xdg) -> Vec<PathBuf> {
    let flatpak = home.join(".var/app/io.github.Faugus.faugus-launcher");
    vec![
        xdg.data.join("faugus-launcher/games.json"),
        xdg.config.join("faugus-launcher/games.json"),
        flatpak.join("data/faugus-launcher/games.json"),
        flatpak.join("config/faugus-launcher/games.json"),
    ]
}

/// A game in Faugus Launcher's `games.json`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FaugusEntry {
    /// `title`.
    pub title: String,
    /// `path`: the file Faugus runs, `~` and `$HOME` expanded.
    pub path: PathBuf,
    /// `runner`: `Linux-Native`, a Proton's name, or `Steam`.
    pub runner: Option<String>,
    /// `cover`: the artwork Faugus downloaded.
    pub cover: Option<PathBuf>,
}

/// The games of a Faugus `games.json`, a list of records. A record whose
/// `runner` is `Steam` names a Steam `AppID`, not a file: the game is
/// Steam's. A path with variables other than `$HOME` is skipped, since
/// Faugus expands them from its own environment.
#[must_use]
pub fn faugus_entries(json: &str, home: &Path) -> Vec<FaugusEntry> {
    let Ok(serde_json::Value::Array(records)) = serde_json::from_str::<serde_json::Value>(json)
    else {
        return Vec::new();
    };
    let expand = |value: &str| -> Option<PathBuf> {
        let home = home.to_string_lossy();
        let value = if let Some(rest) = value.strip_prefix('~') {
            format!("{home}{rest}")
        } else {
            value.replace("${HOME}", &home).replace("$HOME", &home)
        };
        (!value.contains('$') && value.starts_with('/')).then(|| PathBuf::from(value))
    };
    records
        .iter()
        .filter_map(|r| {
            let string = |key: &str| {
                r.get(key)
                    .and_then(serde_json::Value::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
            };
            let runner = string("runner");
            if runner.as_deref() == Some("Steam") {
                return None;
            }
            Some(FaugusEntry {
                title: string("title")?,
                path: expand(&string("path")?)?,
                runner,
                cover: string("cover").and_then(|c| expand(&c)),
            })
        })
        .collect()
}

/// The installed games of every Faugus `games.json` in `files`.
fn faugus_games(files: &[PathBuf]) -> Vec<DetectedGame> {
    let home = crate::paths::home_dir();
    let mut games = Vec::new();
    for file in files {
        let Ok(json) = std::fs::read_to_string(file) else {
            continue;
        };
        for entry in faugus_entries(&json, &home) {
            if !entry.path.is_file() {
                tracing::debug!(title = %entry.title, file = %entry.path.display(), "ignoring Faugus game: file missing");
                continue;
            }
            // Faugus lists the store clients it installs (the EA app) as
            // games.
            let program = entry
                .path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            if crate::running::is_infrastructure(&program) {
                continue;
            }
            let native = entry.runner.as_deref() == Some("Linux-Native");
            games.push(DetectedGame {
                name: entry.title,
                source: Source::Faugus,
                app_id: None,
                install_path: None,
                executables: executables_for_file(&entry.path),
                launch_command: (native && runs_natively(&entry.path))
                    .then(|| vec![entry.path.to_string_lossy().into_owned()]),
                launch_file: Some(entry.path),
                cover: entry.cover.filter(|c| c.is_file()),
                icon: None,
                launcher: None,
            });
        }
    }
    games
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    const STK_DESKTOP: &str = "[Desktop Entry]\nName=SuperTuxKart\nName[pt_BR]=SuperTuxKart\nExec=supertuxkart\nIcon=supertuxkart\nType=Application\nCategories=Game;ArcadeGame;\nActions=SoftwareRender;\n\n[Desktop Action SoftwareRender]\nName=Software Render\nExec=SoftwareRender supertuxkart\n";

    fn tempdir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "bigame_games_{name}_{}_{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write(path: &Path, bytes: &[u8]) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }

    fn write_executable(path: &Path, bytes: &[u8]) {
        write(path, bytes);
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }

    fn game(source: Source, name: &str) -> DetectedGame {
        DetectedGame {
            name: name.into(),
            source,
            app_id: None,
            install_path: None,
            executables: Vec::new(),
            launch_file: None,
            cover: None,
            icon: None,
            launch_command: None,
            launcher: None,
        }
    }

    // ── Menu ─────────────────────────────────────────────────────────────

    #[test]
    fn a_menu_entry_in_the_game_category_is_a_game() {
        let stk = menu_game(STK_DESKTOP).unwrap();
        assert_eq!(stk.program, "supertuxkart");
        assert_eq!(stk.name, "SuperTuxKart");
        assert_eq!(stk.argv, ["supertuxkart"]);
        assert_eq!(stk.icon.as_deref(), Some("supertuxkart"));
        let env = "[Desktop Entry]\nType=Application\nName=Xonotic\nExec=env SDL_VIDEODRIVER=wayland /usr/bin/xonotic-sdl %U\nCategories=Game;ActionGame;\n";
        let xonotic = menu_game(env).unwrap();
        assert_eq!(xonotic.program, "xonotic-sdl");
        assert_eq!(xonotic.name, "Xonotic");
        // The field code goes; env and its assignment stay, so the command
        // still runs the game with them.
        assert_eq!(
            xonotic.argv,
            ["env", "SDL_VIDEODRIVER=wayland", "/usr/bin/xonotic-sdl"]
        );
    }

    #[test]
    fn wrappers_and_field_codes_are_seen_through() {
        let entry = |exec: &str| {
            format!("[Desktop Entry]\nType=Application\nName=G\nExec={exec}\nCategories=Game;\n")
        };
        for (exec, program) in [
            ("/usr/bin/env FOO=1 game", "game"),
            ("env -u DISPLAY game --x", "game"),
            ("prime-run game", "game"),
            ("gamemoderun mangohud game", "game"),
            ("gamescope -w 1920 -h 1080 -- game", "game"),
            ("nice -n 5 /opt/g/game", "game"),
            ("ionice -t -c 3 game", "game"),
        ] {
            assert_eq!(menu_game(&entry(exec)).unwrap().program, program, "{exec}");
        }
        let game = menu_game(&entry("game --file=%f --level=100%% %U")).unwrap();
        assert_eq!(game.argv, ["game", "--file=", "--level=100%"]);
        assert!(menu_game(&entry("prime-run")).is_none());
    }

    #[test]
    fn only_a_native_executable_is_launchable() {
        let dir = tempdir("runs-natively");
        let file = |name: &str, bytes: &[u8], mode: u32| {
            let p = dir.join(name);
            fs::write(&p, bytes).unwrap();
            fs::set_permissions(&p, fs::Permissions::from_mode(mode)).unwrap();
            p
        };
        assert!(runs_natively(&file(
            "run.sh",
            b"#!/bin/sh\nexec game\n",
            0o755
        )));
        assert!(runs_natively(&file("game", b"\x7fELF\x02\x01", 0o755)));
        assert!(!runs_natively(&file("Game.exe", b"MZ\x90\x00", 0o755)));
        assert!(!runs_natively(&file("noexec", b"\x7fELF\x02\x01", 0o644)));
        assert!(!runs_natively(&dir.join("missing")));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn launchers_tools_and_hidden_entries_are_not_games() {
        let steam_shortcut = "[Desktop Entry]\nType=Application\nName=Hades\nExec=steam steam://rungameid/1145360\nCategories=Game;\n";
        let bigame = "[Desktop Entry]\nType=Application\nName=Big Game Mode\nExec=bigame-ui\nCategories=Game;System;Settings;\n";
        let hidden =
            "[Desktop Entry]\nType=Application\nName=X\nExec=x\nNoDisplay=true\nCategories=Game;\n";
        let editor = "[Desktop Entry]\nType=Application\nName=Kate\nExec=kate %U\nCategories=Qt;KDE;Utility;TextEditor;\n";
        let gamepad_tool =
            "[Desktop Entry]\nType=Application\nName=Pad\nExec=pad\nCategories=GamepadTool;\n";
        // Stores and tools file themselves under Game as well.
        let store = "[Desktop Entry]\nType=Application\nName=Heroic Games Launcher\nExec=/usr/bin/flatpak run com.heroicgameslauncher.hgl\nCategories=Game;PackageManager;\n";
        let tool = "[Desktop Entry]\nType=Application\nName=ProtonUp-Qt\nExec=/usr/bin/flatpak run net.davidotek.pupgui2\nCategories=Game;Utility;\n";
        for entry in [
            steam_shortcut,
            bigame,
            hidden,
            editor,
            gamepad_tool,
            store,
            tool,
        ] {
            assert_eq!(menu_game(entry), None, "{entry}");
        }
        // An emulator or an educational game is still a game.
        for categories in ["Game;Emulator;", "Education;Game;KidsGame;"] {
            let entry = format!(
                "[Desktop Entry]\nType=Application\nName=G\nExec=g\nCategories={categories}\n"
            );
            assert!(menu_game(&entry).is_some(), "{categories}");
        }
    }

    #[test]
    fn a_flatpak_entry_is_a_game_of_its_application() {
        let plain = "[Desktop Entry]\nType=Application\nName=0 A.D.\nExec=/usr/bin/flatpak run --branch=stable --arch=x86_64 com.play0ad.zeroad\nCategories=Game;\n";
        let game = menu_game(plain).unwrap();
        assert_eq!(game.flatpak.as_deref(), Some("com.play0ad.zeroad"));
        // Without --command= the process is only known from the metadata.
        assert_eq!(game.program, "");

        let with_command = "[Desktop Entry]\nType=Application\nName=Sober\nExec=/usr/bin/flatpak run --branch=stable --arch=x86_64 --command=sober --file-forwarding org.vinegarhq.Sober @@u %u @@\nCategories=GNOME;GTK;Game;\n";
        let sober = menu_game(with_command).unwrap();
        assert_eq!(sober.flatpak.as_deref(), Some("org.vinegarhq.Sober"));
        assert_eq!(sober.program, "sober");
    }

    #[test]
    fn a_menu_entry_whose_program_is_missing_is_not_installed() {
        let root = tempdir("menu-missing");
        let apps = root.join("applications");
        let bin = root.join("bin");
        write(
            &apps.join("gone.desktop"),
            b"[Desktop Entry]\nType=Application\nName=Gone\nExec=gone-game\nCategories=Game;\n",
        );
        write(
            &apps.join("gone-abs.desktop"),
            b"[Desktop Entry]\nType=Application\nName=Gone\nExec=/opt/nowhere/game\nCategories=Game;\n",
        );
        write(
            &apps.join("here.desktop"),
            b"[Desktop Entry]\nType=Application\nName=Here\nExec=here-game %U\nCategories=Game;\n",
        );
        write_executable(&bin.join("here-game"), b"#!/bin/sh\n");
        // Present but not executable: the menu could not start it either.
        write(
            &apps.join("data.desktop"),
            b"[Desktop Entry]\nType=Application\nName=Data\nExec=data-game\nCategories=Game;\n",
        );
        write(&bin.join("data-game"), b"#!/bin/sh\n");

        let games = menu_games_in(&[apps], std::slice::from_ref(&bin), &[]);
        assert_eq!(games.len(), 1, "{games:?}");
        assert_eq!(games[0].name, "Here");
        assert_eq!(
            games[0].program_path.as_deref(),
            Some(bin.join("here-game").as_path())
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_flatpak_entry_counts_only_when_the_flatpak_is_installed() {
        let root = tempdir("menu-flatpak");
        let apps = root.join("applications");
        let installation = root.join("flatpak");
        let entry = |id: &str| {
            format!(
                "[Desktop Entry]\nType=Application\nName={id}\nExec=/usr/bin/flatpak run --branch=stable {id}\nCategories=Game;\n"
            )
        };
        write(
            &apps.join("a.desktop"),
            entry("org.example.Installed").as_bytes(),
        );
        write(
            &apps.join("b.desktop"),
            entry("org.example.Removed").as_bytes(),
        );
        write(
            &installation.join("app/org.example.Installed/current/active/metadata"),
            b"[Application]\nname=org.example.Installed\nruntime=org.freedesktop.Platform/x86_64/24.08\ncommand=the-game\n",
        );

        let games = menu_games_in(
            std::slice::from_ref(&apps),
            &[],
            std::slice::from_ref(&installation),
        );
        assert_eq!(games.len(), 1, "{games:?}");
        assert_eq!(games[0].program, "the-game");
        assert_eq!(games[0].flatpak.as_deref(), Some("org.example.Installed"));
        let detected = DetectedGame::from(games[0].clone());
        assert_eq!(detected.source, Source::Flatpak);
        assert_eq!(detected.profile_key(), "the-game");
        assert_eq!(detected.launch_command, None);

        // An installed Flatpak whose command is a launcher is still not a game.
        write(
            &apps.join("c.desktop"),
            entry("com.usebottles.bottles").as_bytes(),
        );
        write(
            &installation.join("app/com.usebottles.bottles/current/active/metadata"),
            b"[Application]\ncommand=bottles\n",
        );
        assert_eq!(menu_games_in(&[apps], &[], &[installation]).len(), 1);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_quoted_exec_path_with_spaces_is_one_program() {
        assert_eq!(
            exec_arguments(r#""/home/g/Games/My Game/run game" --fullscreen %U"#),
            ["/home/g/Games/My Game/run game", "--fullscreen", "%U"]
        );
        let entry = "[Desktop Entry]\nType=Application\nName=My Game\nExec=\"/home/g/Games/My Game/run game\" --fullscreen\nCategories=Game;\n";
        let game = menu_game(entry).unwrap();
        assert_eq!(game.program, "run game");
        assert_eq!(game.name, "My Game");
        let streaming = "[Desktop Entry]\nType=Application\nName=NVIDIA GeForce NOW\nExec=\"/home/g/.local/share/applications/NVIDIA GeForce NOW\"\nCategories=Game;\n";
        assert_eq!(menu_game(streaming), None);
    }

    // ── Steam ────────────────────────────────────────────────────────────

    #[test]
    fn acf_values_are_read() {
        let acf = "\"AppState\"\n{\n\t\"appid\"\t\"1808500\"\n\t\"name\"\t\"ARC Raiders\"\n\t\"installdir\"\t\"Arc Raiders\"\n}\n";
        assert_eq!(acf_value(acf, "appid").as_deref(), Some("1808500"));
        assert_eq!(acf_value(acf, "name").as_deref(), Some("ARC Raiders"));
        assert_eq!(acf_value(acf, "installdir").as_deref(), Some("Arc Raiders"));
        assert_eq!(acf_value(acf, "missing"), None);
    }

    #[test]
    fn steam_state_flags_tell_a_download_from_an_install() {
        assert!(acf_installed("\"StateFlags\"\t\"4\"\n"));
        // Installed, update pending.
        assert!(acf_installed("\"StateFlags\"\t\"6\"\n"));
        // Downloading.
        assert!(!acf_installed("\"StateFlags\"\t\"1026\"\n"));
        // Older manifests have no flags at all.
        assert!(acf_installed("\"appid\"\t\"1\"\n"));
    }

    fn manifest(app_id: &str, name: &str, installdir: &str, flags: &str) -> String {
        format!(
            "\"AppState\"\n{{\n\t\"appid\"\t\"{app_id}\"\n\t\"name\"\t\"{name}\"\n\t\"StateFlags\"\t\"{flags}\"\n\t\"installdir\"\t\"{installdir}\"\n}}\n"
        )
    }

    #[test]
    fn a_steam_title_is_installed_only_with_its_directory() {
        let home = tempdir("steam-installed");
        let steamapps = home.join(".local/share/Steam/steamapps");
        write(
            &steamapps.join("appmanifest_1.acf"),
            manifest("1", "Here", "Here", "4").as_bytes(),
        );
        write(
            &steamapps.join("common/Here/Here.exe"),
            &vec![0u8; 300 * 1024],
        );
        // Manifest left behind, directory gone.
        write(
            &steamapps.join("appmanifest_2.acf"),
            manifest("2", "Gone", "Gone", "4").as_bytes(),
        );
        // Directory created, nothing in it yet.
        write(
            &steamapps.join("appmanifest_3.acf"),
            manifest("3", "Empty", "Empty", "4").as_bytes(),
        );
        fs::create_dir_all(steamapps.join("common/Empty")).unwrap();
        // Still downloading.
        write(
            &steamapps.join("appmanifest_4.acf"),
            manifest("4", "Partial", "Partial", "1026").as_bytes(),
        );
        write(&steamapps.join("common/Partial/part.bin"), b"...");
        // Tooling.
        write(
            &steamapps.join("appmanifest_5.acf"),
            manifest("5", "Proton 9.0", "Proton 9.0", "4").as_bytes(),
        );
        write(&steamapps.join("common/Proton 9.0/proton"), b"...");

        let games = steam_games(&home);
        assert_eq!(games.len(), 1, "{games:?}");
        assert_eq!(games[0].name, "Here");
        assert_eq!(games[0].app_id.as_deref(), Some("1"));
        assert_eq!(games[0].profile_key(), "Here.exe");
        assert!(games[0].install_path.is_some());
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn a_symlinked_steam_root_is_one_library() {
        let home = tempdir("steam-symlink");
        let real = home.join(".local/share/Steam");
        fs::create_dir_all(real.join("steamapps")).unwrap();
        fs::create_dir_all(home.join(".steam")).unwrap();
        std::os::unix::fs::symlink(&real, home.join(".steam/steam")).unwrap();
        assert_eq!(steam_libraries(&home).len(), 1);
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn steam_runtimes_are_not_games() {
        for name in [
            "Proton 9.0",
            "Proton Experimental",
            "Proton Hotfix",
            "Steam Linux Runtime 3.0 (sniper)",
            "Steamworks Common Redistributables",
            "Proton EasyAntiCheat Runtime",
        ] {
            assert!(is_steam_runtime(name), "{name} should be filtered");
        }
        assert!(!is_steam_runtime("ARC Raiders"));
        assert!(!is_steam_runtime("Dead by Daylight"));
    }

    #[test]
    fn steam_library_folders_are_parsed() {
        let vdf = "\"libraryfolders\"\n{\n\t\"0\"\n\t{\n\t\t\"path\"\t\t\"/home/u/.local/share/Steam\"\n\t}\n\t\"1\"\n\t{\n\t\t\"path\"\t\t\"/mnt/games/SteamLibrary\"\n\t}\n}\n";
        let paths = parse_vdf_paths(vdf);
        assert_eq!(
            paths,
            vec![
                "/home/u/.local/share/Steam".to_owned(),
                "/mnt/games/SteamLibrary".to_owned()
            ]
        );
    }

    #[test]
    fn cover_search_recurses_into_steams_hashed_subdirectories() {
        // Steam keeps covers under librarycache/<appid>/<hash>/, so a fixed
        // path finds nothing.
        let home = tempdir("cover");
        let cache = home.join(".local/share/Steam/appcache/librarycache/1808500/abc123hash");
        fs::create_dir_all(&cache).unwrap();
        fs::create_dir_all(home.join(".local/share/Steam/steamapps")).unwrap();
        fs::write(cache.join("library_600x900.jpg"), b"jpeg").unwrap();

        let found = steam_cover(&home, "1808500").expect("cover should be found");
        assert!(found.ends_with("library_600x900.jpg"));

        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn cover_search_falls_back_when_the_portrait_is_missing() {
        // Some titles (Dead by Daylight) have no library_600x900.jpg.
        let home = tempdir("cover_fallback");
        let cache = home.join(".local/share/Steam/appcache/librarycache/381210/hash");
        fs::create_dir_all(&cache).unwrap();
        fs::create_dir_all(home.join(".local/share/Steam/steamapps")).unwrap();
        fs::write(cache.join("library_capsule.jpg"), b"jpeg").unwrap();

        let found = steam_cover(&home, "381210").expect("should fall back");
        assert!(found.ends_with("library_capsule.jpg"));

        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn missing_cover_is_none_not_a_broken_path() {
        let home = tempdir("cover_missing");
        fs::create_dir_all(home.join(".local/share/Steam/steamapps")).unwrap();
        assert_eq!(steam_cover(&home, "999999"), None);
        let _ = fs::remove_dir_all(&home);
    }

    // ── Executables ──────────────────────────────────────────────────────

    #[test]
    fn support_binaries_are_not_the_game() {
        for name in [
            "UnityCrashHandler64.exe",
            "EasyAntiCheat_Setup.exe",
            "AntiCheatInstaller.exe",
            "vc_redist.x64.exe",
            "UEPrereqSetup_x64.exe",
            "unins000.exe",
        ] {
            assert!(is_support_binary(name), "{name} should be filtered");
        }
        assert!(!is_support_binary("PioneerGame.exe"));
        assert!(!is_support_binary("DeadByDaylight.exe"));
        assert!(!is_support_binary("DeadByDaylight-Win64-Shipping.exe"));
    }

    #[test]
    fn store_helpers_shipped_inside_games_are_filtered() {
        // EpicWebHelper.exe ships inside Steam titles and can be larger than the
        // game binary, so size ranking alone is not enough.
        for name in [
            "EpicWebHelper.exe",
            "steamerrorreporter64.exe",
            "crashpad_handler.exe",
        ] {
            assert!(is_support_binary(name), "{name} should be filtered");
        }
    }

    #[test]
    fn executables_are_ranked_and_support_tools_dropped() {
        // Mirrors the real ARC Raiders layout: the game binary at the top
        // level, an anti-cheat installer in a subdirectory.
        let dir = tempdir("exe_rank");
        fs::write(dir.join("PioneerGame.exe"), vec![0u8; 2 * 1024 * 1024]).unwrap();
        fs::create_dir_all(dir.join("Installers")).unwrap();
        fs::write(
            dir.join("Installers/AntiCheatInstaller.exe"),
            vec![0u8; 4 * 1024 * 1024],
        )
        .unwrap();
        // Too small to be a game binary.
        fs::write(dir.join("tiny.exe"), b"nope").unwrap();

        let exes = find_executables(&dir);
        assert_eq!(exes.first().map(String::as_str), Some("PioneerGame.exe"));
        assert!(!exes.iter().any(|e| e.contains("AntiCheat")));
        assert!(!exes.iter().any(|e| e == "tiny.exe"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn profile_key_is_the_process_name_not_the_title() {
        // Keyed on the title, falcond can never match the process, and the
        // profile does nothing.
        let mut arc = game(Source::Steam, "ARC Raiders");
        arc.app_id = Some("1808500".into());
        arc.executables = vec!["PioneerGame.exe".into()];
        assert_eq!(arc.profile_key(), "PioneerGame.exe");
        assert_ne!(arc.profile_key(), "ARC Raiders");
        assert!(arc.has_real_executable());
    }

    #[test]
    fn a_game_with_no_executable_is_flagged_rather_than_guessed() {
        let some = game(Source::Heroic, "Some Game");
        // It still yields something usable, but callers can tell it is a guess.
        assert_eq!(some.profile_key(), "Some Game");
        assert!(!some.has_real_executable());
    }

    // ── Lutris ───────────────────────────────────────────────────────────

    #[test]
    fn lutris_yaml_keys_are_read_from_their_sections() {
        let cfg = parse_lutris_yml(
            "name: Celeste\nrunner: linux\ngame:\n  exe: /home/u/Games/celeste/Celeste.bin.x86_64\n  working_dir: /home/u/Games/celeste\nsystem:\n  prefix_command: x\n",
        );
        assert_eq!(cfg.name.as_deref(), Some("Celeste"));
        assert_eq!(cfg.runner.as_deref(), Some("linux"));
        assert_eq!(
            cfg.exe.as_deref(),
            Some("/home/u/Games/celeste/Celeste.bin.x86_64")
        );
        assert_eq!(cfg.working_dir.as_deref(), Some("/home/u/Games/celeste"));
        let quoted = parse_lutris_yml(
            "slug: \"x\"\nrunner: wine\ngame:\n    exe: \"$GAMEDIR/drive_c/Game/game.exe\"\n  prefix: \"$GAMEDIR\"\n",
        );
        assert_eq!(
            quoted.exe.as_deref(),
            Some("$GAMEDIR/drive_c/Game/game.exe")
        );
        assert_eq!(quoted.prefix.as_deref(), Some("$GAMEDIR"));
    }

    #[test]
    fn a_lutris_game_is_installed_only_when_its_file_exists() {
        let root = tempdir("lutris");
        let games = root.join("config/lutris/games");
        let game_dir = root.join("Games/here");
        write_executable(&game_dir.join("run.sh"), b"#!/bin/sh\n");
        write(&game_dir.join("Game.exe"), b"MZ");
        write(
            &games.join("here-1.yml"),
            format!(
                "name: Here\nrunner: linux\ngame:\n  exe: {}/run.sh\n",
                game_dir.display()
            )
            .as_bytes(),
        );
        write(
            &games.join("relative-2.yml"),
            format!(
                "name: Relative\nrunner: wine\ngame:\n  exe: Game.exe\n  working_dir: {}\n",
                game_dir.display()
            )
            .as_bytes(),
        );
        write(
            &games.join("gone-3.yml"),
            format!(
                "name: Gone\nrunner: wine\ngame:\n  exe: {}/drive_c/Gone/gone.exe\n",
                root.display()
            )
            .as_bytes(),
        );
        write(
            &games.join("variable-4.yml"),
            b"name: Variable\nrunner: wine\ngame:\n  exe: $GAMEDIR/drive_c/game.exe\n  prefix: $GAMEDIR\n",
        );
        write(
            &games.join("steam-5.yml"),
            b"name: Via Steam\nrunner: steam\ngame:\n  appid: 750920\n",
        );
        write(
            &games.join("empty-6.yml"),
            b"name: Empty\nrunner: wine\ngame: {}\n",
        );

        let lutris = LutrisRoot {
            games,
            data: root.join("data"),
            cache: root.join("cache"),
        };
        let mut found = lutris_games(&[lutris]);
        found.sort_by(|a, b| a.name.cmp(&b.name));
        let names: Vec<&str> = found.iter().map(|g| g.name.as_str()).collect();
        assert_eq!(names, ["Here", "Relative"]);
        // A script is never the key: the shell is its process. With no
        // program beside it, the game is keyed on nothing it could guess.
        assert!(
            !found[0].has_real_executable(),
            "{:?}",
            found[0].executables
        );
        assert!(found[0].launch_command.is_some());
        assert_eq!(found[1].profile_key(), "Game.exe");
        // A Windows binary is not started from here.
        assert_eq!(found[1].launch_command, None);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_native_lutris_game_runs_in_its_working_directory_with_its_arguments() {
        let yml = "game:\n  exe: /games/port/game.x86_64\n  args: \"--fullscreen -name 'A  B'\"\n  working_dir: /games/port/data\nname: Port\nrunner: linux\nsystem:\n  args: not-the-games\n";
        let cfg = parse_lutris_yml(yml);
        assert_eq!(cfg.args.as_deref(), Some("--fullscreen -name 'A  B'"));
        assert_eq!(
            lutris_command(
                Path::new("/games/port/game.x86_64"),
                cfg.working_dir.as_deref(),
                cfg.args.as_deref()
            ),
            [
                "env",
                "-C",
                "/games/port/data",
                "/games/port/game.x86_64",
                "--fullscreen",
                "-name",
                "A  B"
            ]
        );
        // No working directory: the file's own folder, as Lutris has it.
        assert_eq!(
            lutris_command(Path::new("/games/port/run.sh"), None, None),
            ["env", "-C", "/games/port", "/games/port/run.sh"]
        );
        // A relative one is the file's folder's.
        assert_eq!(
            lutris_command(
                Path::new("/g/run.sh"),
                Some("bin"),
                Some("-x \"a b\" c\\ d")
            ),
            ["env", "-C", "/g/bin", "/g/run.sh", "-x", "a b", "c d"]
        );
        // The program a profile matches is still the game's.
        let argv = lutris_command(Path::new("/games/port/game.x86_64"), None, None);
        assert_eq!(program_of(&argv), Some("/games/port/game.x86_64"));
    }

    #[test]
    fn lutris_slugs_become_titles() {
        assert_eq!(
            strip_numeric_suffix("altered-beast-remake-linux-1771620880"),
            "altered-beast-remake-linux"
        );
        assert_eq!(strip_numeric_suffix("no-number-here"), "no-number-here");
        assert_eq!(
            slug_to_title("altered-beast-remake-linux"),
            "Altered Beast Remake Linux"
        );
    }

    // ── Heroic ───────────────────────────────────────────────────────────

    #[test]
    fn heroic_library_lists_only_installed_titles() {
        let json = r#"{"library": [
            {"app_name": "a", "title": "Hades", "is_installed": true,
             "install": {"install_path": "/games/Hades", "executable": "Hades.exe", "platform": "Windows"}},
            {"app_name": "b", "title": "Owned Only", "is_installed": false},
            {"app_name": "c", "title": "No Install Block", "is_installed": true},
            {"app_name": "d", "title": "", "is_installed": true, "install": {"install_path": "/x"}}
        ]}"#;
        let entries = heroic_library_entries(json);
        assert_eq!(entries.len(), 1, "{entries:?}");
        assert_eq!(entries[0].title, "Hades");
        assert_eq!(
            entries[0].install_path.as_deref(),
            Some(Path::new("/games/Hades"))
        );
        assert_eq!(
            entries[0].executable.as_deref(),
            Some(Path::new("Hades.exe"))
        );
        // GOG's cache uses "games"; sideload uses "games" too.
        let gog = r#"{"games": [{"title": "Cuphead", "is_installed": true, "install": {"install_path": "/g/Cuphead"}}]}"#;
        assert_eq!(heroic_library_entries(gog).len(), 1);
        assert!(heroic_library_entries("{}").is_empty());
        assert!(heroic_library_entries("not json").is_empty());
    }

    #[test]
    fn heroic_backend_records_are_read_in_their_three_shapes() {
        let none = HashMap::new();
        let legendary = r#"{"Fortnite": {"app_name": "Fortnite", "title": "Fortnite", "install_path": "/g/Fortnite", "executable": "FortniteLauncher.exe"}}"#;
        let gog = r#"{"installed": [{"appName": "1", "install_path": "/g/Cuphead", "platform": "windows"}]}"#;
        let nile = r#"[{"id": "amzn1", "path": "/g/Amazon", "version": "1"}]"#;
        let l = heroic_installed_entries(legendary, &none);
        assert_eq!(l[0].title, "Fortnite");
        assert_eq!(
            l[0].executable.as_deref(),
            Some(Path::new("FortniteLauncher.exe"))
        );
        assert_eq!(
            heroic_installed_entries(gog, &none)[0]
                .install_path
                .as_deref(),
            Some(Path::new("/g/Cuphead"))
        );
        assert_eq!(
            heroic_installed_entries(nile, &none)[0]
                .install_path
                .as_deref(),
            Some(Path::new("/g/Amazon"))
        );
        assert!(heroic_installed_entries("{}", &none).is_empty());
    }

    #[test]
    fn a_gog_record_without_a_title_is_named_by_the_library_else_its_folder() {
        // GOG's installed.json has only the numeric id; its library cache can
        // still list the game as not installed.
        let installed = r#"{"installed": [{"appName": "1207658845", "install_path": "/g/Another World 20th Anniversary Edition"}]}"#;
        let library = r#"{"games": [{"app_name": "1207658845", "title": "Another World: 20th Anniversary Edition", "is_installed": false}]}"#;
        let titles = heroic_library_titles(library);
        assert_eq!(
            heroic_installed_entries(installed, &titles)[0].title,
            "Another World: 20th Anniversary Edition"
        );
        assert_eq!(
            heroic_installed_entries(installed, &HashMap::new())[0].title,
            "Another World 20th Anniversary Edition"
        );
    }

    #[test]
    fn a_running_game_is_found_in_the_library_by_folder_or_executable() {
        let mut heroic = game(Source::Heroic, "Worlds");
        heroic.install_path = Some(PathBuf::from("/g/Worlds"));
        heroic.executables = vec!["Worlds.exe".into(), "Indiana-Win64-Shipping.exe".into()];
        heroic.cover = Some(PathBuf::from("/c/worlds"));
        let mut other = game(Source::Lutris, "Kart");
        other.executables = vec!["kart".into()];
        other.cover = Some(PathBuf::from("/c/kart"));
        let games = vec![heroic, other];
        let name = |p: &str, dir: Option<&str>| {
            game_among(&games, p, dir.map(Path::new)).map(|g| g.name.clone())
        };
        // The real game is deeper in the install folder than the launcher.
        assert_eq!(
            name("x.exe", Some("/g/Worlds/Indiana/Binaries/Win64")).as_deref(),
            Some("Worlds")
        );
        assert_eq!(
            name("indiana-win64-shipping.exe", None).as_deref(),
            Some("Worlds")
        );
        assert_eq!(name("kart", None).as_deref(), Some("Kart"));
        assert_eq!(name("nothing", None), None);
    }

    #[test]
    fn a_heroic_cover_is_found_by_the_hash_of_its_address() {
        let root = tempdir("heroic_cover");
        let config = root.join("config/heroic");
        let game = root.join("Games/Worlds");
        write(&game.join("Worlds.exe"), &vec![0u8; 300 * 1024]);
        let url = "https://cdn1.epicgames.com/item/x/Worlds_1200x1600-abc";
        // Only the library knows the artwork; installed.json names none.
        write(
            &config.join("store_cache/legendary_library.json"),
            format!(r#"{{"library": [{{"app_name": "w", "title": "Worlds", "is_installed": false, "art_square": "{url}"}}]}}"#).as_bytes(),
        );
        write(
            &config.join("legendaryConfig/legendary/installed.json"),
            format!(
                r#"{{"w": {{"app_name": "w", "title": "Worlds", "install_path": "{}", "executable": "Worlds.exe"}}}}"#,
                game.display()
            )
            .as_bytes(),
        );
        // Named by the SHA-256 of the address, with no extension.
        let expected = config
            .join("images-cache")
            .join(crate::graphics::manifest::sha256_bytes(url.as_bytes()));
        write(&expected, b"\xff\xd8\xff");
        let games = dedup(heroic_games(&[config]));
        assert_eq!(games.len(), 1, "{games:?}");
        assert_eq!(games[0].cover.as_deref(), Some(expected.as_path()));
        // legendary's file lists it, so a `heroic://launch` link names that runner.
        assert!(matches!(
            games[0].launcher,
            Some(LauncherRef::Heroic {
                runner: Some("legendary"),
                ..
            })
        ));
    }

    #[test]
    fn a_heroic_game_is_installed_only_with_its_files() {
        let root = tempdir("heroic");
        let config = root.join("config/heroic");
        let here = root.join("Games/Here");
        write(&here.join("Here.exe"), &vec![0u8; 300 * 1024]);
        let library = format!(
            r#"{{"library": [
                {{"title": "Here", "is_installed": true, "install": {{"install_path": "{here}", "executable": "Here.exe"}}}},
                {{"title": "Gone", "is_installed": true, "install": {{"install_path": "{root}/Games/Gone", "executable": "Gone.exe"}}}}
            ]}}"#,
            here = here.display(),
            root = root.display()
        );
        write(
            &config.join("store_cache/legendary_library.json"),
            library.as_bytes(),
        );
        // The backend's own record of the same game: one card, not two.
        let installed = format!(
            r#"{{"here": {{"app_name": "here", "title": "Here", "install_path": "{}", "executable": "Here.exe"}}}}"#,
            here.display()
        );
        write(
            &config.join("legendaryConfig/legendary/installed.json"),
            installed.as_bytes(),
        );

        let games = dedup(heroic_games(&[config]));
        assert_eq!(games.len(), 1, "{games:?}");
        assert_eq!(games[0].name, "Here");
        assert_eq!(games[0].profile_key(), "Here.exe");
        assert_eq!(games[0].install_path.as_deref(), Some(here.as_path()));
        let _ = fs::remove_dir_all(&root);
    }

    // ── Identity ─────────────────────────────────────────────────────────

    #[test]
    fn the_same_game_from_two_launchers_is_one_game() {
        let root = tempdir("dedup");
        let install = root.join("Games/Celeste");
        write_executable(&install.join("Celeste"), b"\x7fELF");

        let mut steam_a = game(Source::Steam, "Celeste");
        steam_a.app_id = Some("504230".into());
        let mut steam_b = steam_a.clone();
        steam_b.name = "Celeste (other library)".into();

        let mut heroic = game(Source::Heroic, "Celeste");
        heroic.install_path = Some(install.clone());
        let mut lutris = game(Source::Lutris, "celeste");
        lutris.launch_file = Some(install.join("Celeste"));
        let mut menu = game(Source::Native, "Celeste");
        menu.launch_file = Some(install.join("Celeste"));

        // Different launchers, different installs, same title: two games.
        let mut other = game(Source::Lutris, "Celeste");
        other.launch_file = Some(root.join("elsewhere/Celeste"));
        write_executable(&root.join("elsewhere/Celeste"), b"\x7fELF");

        let kept = dedup(vec![steam_a, steam_b, heroic, lutris, menu, other]);
        let sources: Vec<Source> = kept.iter().map(|g| g.source).collect();
        assert_eq!(
            sources,
            [Source::Steam, Source::Heroic, Source::Lutris],
            "{kept:?}"
        );
        let _ = fs::remove_dir_all(&root);
    }

    // ── Regression: executables, launch configuration, sources ───────────

    /// A little-endian ELF64 image: `kind` 2 (`ET_EXEC`) or 3 (`ET_DYN`), for
    /// `machine` (62 x86-64, 183 aarch64), with a `PT_INTERP` program header
    /// when `interp`, padded to `size` bytes.
    fn elf(kind: u16, machine: u16, interp: bool, size: usize) -> Vec<u8> {
        let mut b = vec![0u8; size.max(64 + 56)];
        b[..4].copy_from_slice(b"\x7fELF");
        b[4] = 2; // 64-bit
        b[5] = 1; // little-endian
        b[6] = 1;
        b[16..18].copy_from_slice(&kind.to_le_bytes());
        b[18..20].copy_from_slice(&machine.to_le_bytes());
        b[32..40].copy_from_slice(&64u64.to_le_bytes()); // e_phoff
        b[54..56].copy_from_slice(&56u16.to_le_bytes()); // e_phentsize
        b[56..58].copy_from_slice(&1u16.to_le_bytes()); // e_phnum
        let p_type: u32 = if interp { 3 } else { 1 };
        b[64..68].copy_from_slice(&p_type.to_le_bytes());
        b
    }

    const BIG: usize = 300 * 1024;

    #[test]
    fn shared_libraries_and_foreign_builds_are_not_the_game() {
        // Altered Beast Remake's Linux build: the game beside CEF's and
        // SwiftShader's libraries (larger, executable bit set), and an
        // arm64 copy of everything.
        let dir = tempdir("elf-kinds");
        write_executable(
            &dir.join("x64/Altered Beast Remake"),
            &elf(3, 62, true, BIG),
        );
        write_executable(&dir.join("x64/libcef.so"), &elf(3, 62, false, 4 * BIG));
        write_executable(&dir.join("x64/libvulkan.so.1"), &elf(3, 62, false, 2 * BIG));
        write_executable(&dir.join("x64/UnityPlayer.so"), &elf(3, 62, false, 3 * BIG));
        // A library without the .so name is told by its missing interpreter.
        write_executable(&dir.join("x64/libplugin"), &elf(3, 62, false, 3 * BIG));
        write_executable(
            &dir.join("arm64/Altered Beast Remake"),
            &elf(3, 183, true, 2 * BIG),
        );
        write_executable(&dir.join("tool_static"), &elf(2, 62, false, BIG / 2 + BIG));
        assert_eq!(
            find_executables(&dir),
            ["tool_static", "Altered Beast Remake"],
            "an ET_EXEC program and a PIE count, libraries and arm64 do not"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn helpers_and_their_folders_are_not_candidates_and_names_appear_once() {
        // Dune: Awakening and an EA title, as installed on the reference
        // desktop.
        let dir = tempdir("helpers");
        write(
            &dir.join("DuneSandbox/Binaries/Win64/DuneSandbox-Win64-Shipping.exe"),
            &vec![0; 4 * BIG],
        );
        write(
            &dir.join("DuneSandbox/Binaries/Win64/DuneSandbox_BE.exe"),
            &vec![0; 2 * BIG],
        );
        write(&dir.join("BEService_x64.exe"), &vec![0; 3 * BIG]);
        write(&dir.join("__Installer/Touchup.exe"), &vec![0; 5 * BIG]);
        write(&dir.join("__Installer/Cleanup.exe"), &vec![0; 5 * BIG]);
        write(
            &dir.join("Support/EA Help/ActivationUI.exe"),
            &vec![0; 5 * BIG],
        );
        write(
            &dir.join("_CommonRedist/vcredist/2019/VC_redist.x64.exe"),
            &vec![0; 5 * BIG],
        );
        write(
            &dir.join("Engine/Binaries/ThirdParty/CEF3/UnrealCEFSubProcess.exe"),
            &vec![0; 5 * BIG],
        );
        write(&dir.join("bin/7za.exe"), &vec![0; 5 * BIG]);
        write(&dir.join("DuneSandbox.exe"), &vec![0; BIG]);
        // The same name in two folders, not adjacent once sorted by size.
        write(&dir.join("a/Tool.exe"), &vec![0; BIG + 3]);
        write(&dir.join("b/Other.exe"), &vec![0; BIG + 2]);
        write(&dir.join("c/Tool.exe"), &vec![0; BIG + 1]);
        let exes = find_executables(&dir);
        assert_eq!(
            exes,
            [
                "DuneSandbox-Win64-Shipping.exe",
                "Tool.exe",
                "Other.exe",
                "DuneSandbox.exe"
            ]
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn trials_and_tests_rank_below_the_game_whatever_their_size() {
        let dir = tempdir("trial");
        // Unravel Two: the trial is larger than the game.
        write(&dir.join("UnravelTwo_trial.exe"), &vec![0; 3 * BIG]);
        write(&dir.join("UnravelTwo.exe"), &vec![0; 2 * BIG]);
        // It Takes Two: the same size, so only the name can tell.
        write(
            &dir.join("Nuts/Binaries/Win64/ItTakesTwo_Trial.exe"),
            &vec![0; BIG],
        );
        write(
            &dir.join("Nuts/Binaries/Win64/ItTakesTwo.exe"),
            &vec![0; BIG],
        );
        write(&dir.join("Phoenix-Win64-Test.exe"), &vec![0; 4 * BIG]);
        write(&dir.join("Observer.exe"), &vec![0; BIG / 2 + BIG]);
        assert_eq!(
            find_executables(&dir),
            [
                "UnravelTwo.exe",
                "Observer.exe",
                "ItTakesTwo.exe",
                "Phoenix-Win64-Test.exe",
                "UnravelTwo_trial.exe",
                "ItTakesTwo_Trial.exe",
            ]
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_recorded_bootstrap_or_selector_yields_to_the_game_it_starts() {
        let root = tempdir("heroic-ranking");
        let config = root.join("config/heroic");
        // The Callisto Protocol: Heroic records the Unreal bootstrap.
        let callisto = root.join("Games/TheCallistoProtocol");
        write(&callisto.join("TheCallistoProtocol.exe"), &vec![0; BIG]);
        write(
            &callisto
                .join("TheCallistoProtocol/Binaries/Win64/TheCallistoProtocol-Win64-Shipping.exe"),
            &vec![0; 8 * BIG],
        );
        write(
            &callisto.join("Engine/Binaries/Win64/EOSOverlayRenderer-Win64-Shipping.exe"),
            &vec![0; 9 * BIG],
        );
        // The Outer Worlds: the shipping binary has another project name.
        let worlds = root.join("Games/TOWSpacersChoice");
        write(
            &worlds.join("TheOuterWorldsSpacersChoiceEdition.exe"),
            &vec![0; BIG],
        );
        write(
            &worlds.join("Indiana/Binaries/Win64/IndianaEpicGameStore-Win64-Shipping.exe"),
            &vec![0; 6 * BIG],
        );
        // Control: Heroic records the DX selector, which is tiny.
        let control = root.join("Games/Control");
        write(&control.join("Control.exe"), &vec![0; 4096]);
        write(&control.join("Control_DX11.exe"), &vec![0; 2 * BIG]);
        write(&control.join("Control_DX12.exe"), &vec![0; 2 * BIG + 512]);
        write(&control.join("VC_redist.x64.exe"), &vec![0; 3 * BIG]);
        // A record whose file is right stays first.
        let tmnt = root.join("Games/TMNT");
        write(&tmnt.join("TMNT.exe"), &vec![0; BIG]);
        write(&tmnt.join("Big.exe"), &vec![0; 3 * BIG]);
        let record = |id: &str, title: &str, dir: &Path, exe: &str| {
            format!(
                r#""{id}": {{"app_name": "{id}", "title": "{title}", "install_path": "{}", "executable": "{exe}", "is_dlc": false}}"#,
                dir.display()
            )
        };
        write(
            &config.join("legendaryConfig/legendary/installed.json"),
            format!(
                "{{{}, {}, {}, {}}}",
                record(
                    "a",
                    "The Callisto Protocol",
                    &callisto,
                    "TheCallistoProtocol.exe"
                ),
                record(
                    "b",
                    "The Outer Worlds",
                    &worlds,
                    "TheOuterWorldsSpacersChoiceEdition.exe"
                ),
                record("c", "Control", &control, "Control.exe"),
                record("d", "TMNT", &tmnt, "TMNT.exe"),
            )
            .as_bytes(),
        );
        let games = heroic_games(&[config]);
        let key = |title: &str| {
            games
                .iter()
                .find(|g| g.name == title)
                .map(|g| g.executables.clone())
                .unwrap()
        };
        assert_eq!(
            key("The Callisto Protocol"),
            [
                "TheCallistoProtocol-Win64-Shipping.exe",
                "TheCallistoProtocol.exe"
            ]
        );
        assert_eq!(
            key("The Outer Worlds")[0],
            "IndianaEpicGameStore-Win64-Shipping.exe"
        );
        assert_eq!(
            key("Control"),
            ["Control_DX12.exe", "Control_DX11.exe", "Control.exe"]
        );
        assert_eq!(key("TMNT"), ["TMNT.exe", "Big.exe"]);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn heroic_dlc_records_never_name_the_game() {
        // legendary's installed.json on the reference desktop: the DLC ids
        // sort before their games', and share their folders.
        let root = tempdir("heroic-dlc");
        let config = root.join("config/heroic");
        let control = root.join("Games/Control");
        write(&control.join("Control_DX12.exe"), &vec![0; BIG]);
        let installed = format!(
            r#"{{
                "3da74e4fa1d94eceb950774f76dbfcea": {{"app_name": "3da74e4fa1d94eceb950774f76dbfcea", "title": "Control Expansion 2 AWE", "install_path": "{dir}", "executable": "", "is_dlc": true}},
                "Calluna": {{"app_name": "Calluna", "title": "Control", "install_path": "{dir}", "executable": "Control.exe", "is_dlc": false}}
            }}"#,
            dir = control.display()
        );
        write(
            &config.join("legendaryConfig/legendary/installed.json"),
            installed.as_bytes(),
        );
        let games = dedup(heroic_games(std::slice::from_ref(&config)));
        assert_eq!(games.len(), 1, "{games:?}");
        assert_eq!(games[0].name, "Control");
        // The store cache marks DLC inside the install block.
        let library = format!(
            r#"{{"library": [{{"app_name": "x", "title": "Classic Obi-Wan", "is_installed": true, "install": {{"install_path": "{}", "is_dlc": true}}}}]}}"#,
            control.display()
        );
        assert!(heroic_library_entries(&library).is_empty());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    // One library of real layouts, read in one scan; split up, each half
    // would test less.
    #[allow(clippy::too_many_lines)]
    fn steams_launch_configuration_names_the_game_and_its_type_filters_tools() {
        use appinfo::tests::{Kv, Writer};
        let home = tempdir("steam-appinfo");
        let root = home.join(".local/share/Steam");
        let steamapps = root.join("steamapps");
        let app = |id: u32, kind: &'static str, launch: Vec<(&'static str, Kv)>| {
            vec![(
                "appinfo",
                Kv::Section(vec![
                    ("appid", Kv::Int(id)),
                    ("common", Kv::Section(vec![("type", Kv::Text(kind))])),
                    ("config", Kv::Section(vec![("launch", Kv::Section(launch))])),
                ]),
            )]
        };
        let entry = |exe: &'static str| {
            Kv::Section(vec![
                ("executable", Kv::Text(exe)),
                ("type", Kv::Text("default")),
            ])
        };
        let mut w = Writer::new(29);
        // It Takes Two: the launch entry names the game under Binaries.
        w.app(
            1_426_210,
            &app(
                1_426_210,
                "Game",
                vec![("0", entry("Nuts/Binaries/Win64/ItTakesTwo.exe"))],
            ),
        );
        // Cyberpunk 2077: the default entry is the launcher, which is not
        // taken; the size ranking decides.
        w.app(1_091_500, &appinfo::tests::cyberpunk());
        // Dead by Daylight: the entry is the Unreal bootstrap.
        w.app(
            381_210,
            &app(381_210, "Game", vec![("0", entry("DeadByDaylight.exe"))]),
        );
        // A store link names no file.
        w.app(
            1_225_570,
            &app(
                1_225_570,
                "Game",
                vec![("0", entry("steam2ea://launchgame/1225570"))],
            ),
        );
        // A dedicated server is a tool.
        w.app(
            2_000_000,
            &app(2_000_000, "Tool", vec![("0", entry("Server.exe"))]),
        );
        write(&root.join("appcache/appinfo.vdf"), &w.finish());
        let install = |id: &str, title: &str, dir: &str, files: &[(&str, usize)]| {
            write(
                &steamapps.join(format!("appmanifest_{id}.acf")),
                manifest(id, title, dir, "4").as_bytes(),
            );
            for (file, size) in files {
                write(
                    &steamapps.join("common").join(dir).join(file),
                    &vec![0; *size],
                );
            }
        };
        install(
            "1426210",
            "It Takes Two",
            "ItTakesTwo",
            &[
                ("Nuts/Binaries/Win64/ItTakesTwo.exe", BIG),
                ("Nuts/Binaries/Win64/ItTakesTwo_Trial.exe", BIG),
                ("Big.exe", 4 * BIG),
            ],
        );
        install(
            "1091500",
            "Cyberpunk 2077",
            "Cyberpunk 2077",
            &[
                ("REDprelauncher.exe", 2 * BIG),
                ("bin/x64/Cyberpunk2077.exe", 9 * BIG),
            ],
        );
        install(
            "381210",
            "Dead by Daylight",
            "Dead by Daylight",
            &[
                ("DeadByDaylight.exe", BIG),
                (
                    "DeadByDaylight/Binaries/Win64/DeadByDaylight-Win64-Shipping.exe",
                    5 * BIG,
                ),
            ],
        );
        install(
            "1225570",
            "Unravel Two",
            "Unravel Two",
            &[
                ("UnravelTwo_trial.exe", 3 * BIG),
                ("UnravelTwo.exe", 2 * BIG),
            ],
        );
        install(
            "2000000",
            "Some Game Server",
            "Server",
            &[("Server.exe", BIG)],
        );
        // No appinfo record: SteamVR by its id, and a title with Steam's
        // trailing space and an escaped quote.
        install(
            "250820",
            "SteamVR",
            "SteamVR",
            &[("bin/vrstartup.exe", BIG)],
        );
        write(
            &steamapps.join("appmanifest_258970.acf"),
            "\"AppState\"\n{\n\t\"appid\"\t\"258970\"\n\t\"Name\"\t\"Gauntlet \\\"Slayer\\\"™ \"\n\t\"StateFlags\"\t\"4\"\n\t\"installdir\"\t\"Gauntlet\"\n}\n".as_bytes(),
        );
        write(
            &steamapps.join("common/Gauntlet/binaries/gauntlet.exe"),
            &vec![0; BIG],
        );

        let games = steam_games(&home);
        let keys: HashMap<&str, &str> = games
            .iter()
            .map(|g| (g.name.as_str(), g.profile_key()))
            .collect();
        assert_eq!(keys.len(), 5, "{keys:?}");
        assert_eq!(keys["It Takes Two"], "ItTakesTwo.exe");
        assert_eq!(keys["Cyberpunk 2077"], "Cyberpunk2077.exe");
        assert_eq!(
            keys["Dead by Daylight"],
            "DeadByDaylight-Win64-Shipping.exe"
        );
        assert_eq!(keys["Unravel Two"], "UnravelTwo.exe");
        assert_eq!(keys["Gauntlet \"Slayer\"™"], "gauntlet.exe");
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn a_launch_file_is_found_whatever_its_case() {
        let dir = tempdir("case");
        write(&dir.join("Bin/X64/Game.exe"), b"MZ");
        assert_eq!(
            find_case_insensitive(&dir, "bin\\x64\\game.EXE"),
            Some(dir.join("Bin/X64/Game.exe"))
        );
        assert_eq!(find_case_insensitive(&dir, "../etc/passwd"), None);
        assert_eq!(find_case_insensitive(&dir, "missing.exe"), None);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn vdf_values_decode_escapes_and_ignore_key_case() {
        let vdf = "\"libraryfolders\"\n{\n\t\"0\"\n\t{\n\t\t\"path\"\t\t\"/mnt/My \\\"Games\\\"\"\n\t}\n}\n";
        assert_eq!(parse_vdf_paths(vdf), ["/mnt/My \"Games\""]);
        let acf = "\"AppState\"\n{\n\t\"Name\"\t\"Say \\\"Hi\\\"\"\n}\n";
        assert_eq!(acf_value(acf, "name").as_deref(), Some("Say \"Hi\""));
    }

    #[test]
    fn a_lutris_script_is_keyed_on_the_program_it_starts() {
        // SuperTuxKart's portable build, as Lutris records it.
        let root = tempdir("lutris-script");
        let dir = root.join("SuperTuxKart-1.5-linux-x86_64");
        write_executable(
            &dir.join("run_game.sh"),
            b"#!/bin/sh\ncd \"$DIRNAME\"\n\"$DIRNAME/bin/supertuxkart\" \"$@\"\n",
        );
        write_executable(&dir.join("bin/supertuxkart"), &elf(3, 62, true, BIG));
        write_executable(&dir.join("lib/libopenal.so.1"), &elf(3, 62, false, 2 * BIG));
        let games = root.join("lutris/games");
        write(
            &games.join("supertuxkart-1771620561.yml"),
            format!("game:\n  exe: {}/run_game.sh\n", dir.display()).as_bytes(),
        );
        let found = lutris_games(&[LutrisRoot {
            games,
            data: root.join("data"),
            cache: root.join("cache"),
        }]);
        assert_eq!(found[0].executables, ["supertuxkart"]);
        assert_eq!(found[0].name, "Supertuxkart");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn only_the_top_level_game_section_names_the_file() {
        // The Ultima Online installer kept in Lutris's games folder.
        let cfg = parse_lutris_yml(
            "name: \"Ultima Online Imperial Shard\"\nrunner: wine\nscript:\n  game:\n    exe: \"/g/drive_c/UO/ImperialUOLauncher.exe\"\n  prefix: \"ultima-online\"\n  wine:\n    arch: win32\ninstaller:\n  - move:\n      dst: /x\n",
        );
        assert_eq!(cfg.exe, None);
        assert_eq!(cfg.prefix, None);
        assert_eq!(cfg.runner.as_deref(), Some("wine"));
        let nested = parse_lutris_yml(
            "game:\n  launch_configs:\n    exe: other.exe\n  exe: game.exe\nwine:\n  prefix: /p\n",
        );
        assert_eq!(nested.exe.as_deref(), Some("game.exe"));
        assert_eq!(nested.prefix, None);
    }

    #[test]
    fn lutris_titles_runners_and_folders_come_from_its_database() {
        // Lutris 0.5.22's files name neither title nor runner; pga.db does.
        let Ok(sqlite3) = which_sqlite3() else {
            return;
        };
        let root = tempdir("lutris-pga");
        let data = root.join("data");
        let games = root.join("games");
        let dir = root.join("Games/rnr");
        write(&dir.join("Rock N Roll Racing.exe"), b"MZ");
        write(
            &games.join("rock-and-roll-racing-2-1771620641.yml"),
            b"game:\n  exe: Rock N Roll Racing.exe\n",
        );
        write(
            &games.join("gamedir-2.yml"),
            b"game:\n  exe: $GAMEDIR/Rock N Roll Racing.exe\n",
        );
        fs::create_dir_all(&data).unwrap();
        // A long title, so the record spills to an overflow page.
        let long = "L".repeat(6000);
        let sql = format!(
            "CREATE TABLE games (id INTEGER PRIMARY KEY, name TEXT, sortname TEXT, slug TEXT, installer_slug TEXT, parent_slug TEXT, platform TEXT, runner TEXT, executable TEXT, directory TEXT, updated DATETIME, lastplayed INTEGER, installed INTEGER, installed_at INTEGER, year INTEGER, configpath TEXT, has_custom_banner INTEGER, has_custom_icon INTEGER, has_custom_coverart_big INTEGER, playtime REAL, service TEXT, service_id TEXT, discord_id TEXT);\
             INSERT INTO games (name, slug, runner, directory, installed, configpath) VALUES ('Rock & Roll Racing', 'rock-and-roll-racing-2', 'wine', '{d}', 1, 'rock-and-roll-racing-2-1771620641');\
             INSERT INTO games (name, slug, runner, directory, installed, configpath) VALUES ('{long}', 'x', 'wine', '{d}', 1, 'gamedir-2');",
            d = dir.display()
        );
        let status = std::process::Command::new(sqlite3)
            .arg(data.join("pga.db"))
            .arg(&sql)
            .status()
            .unwrap();
        assert!(status.success());
        let mut found = lutris_games(&[LutrisRoot {
            games,
            data,
            cache: root.join("cache"),
        }]);
        found.sort_by_key(|g| g.name.len());
        assert_eq!(found.len(), 2, "{found:?}");
        assert_eq!(found[0].name, "Rock & Roll Racing");
        assert_eq!(
            found[0].launch_file.as_deref(),
            Some(dir.join("Rock N Roll Racing.exe").as_path())
        );
        assert_eq!(found[1].name, long);
        let _ = fs::remove_dir_all(&root);
    }

    /// The `sqlite3` program, to write real databases for the tests.
    fn which_sqlite3() -> Result<PathBuf, ()> {
        std::env::var_os("PATH")
            .and_then(|p| {
                std::env::split_paths(&p)
                    .map(|d| d.join("sqlite3"))
                    .find(|p| p.is_file())
            })
            .ok_or(())
    }

    #[test]
    fn faugus_games_are_read_from_its_list() {
        // The records Faugus Launcher 2.4 writes (utils.game_to_save_dict).
        let root = tempdir("faugus");
        let game = root.join("Faugus/game/drive_c/Game/Game.exe");
        write(&game, &vec![0; BIG]);
        let native = root.join("Games/Native/native-game");
        write_executable(&native, &elf(3, 62, true, BIG));
        let ea = root.join(
            "Faugus/ea/drive_c/Program Files/Electronic Arts/EA Desktop/EA Desktop/EADesktop.exe",
        );
        write(&ea, &vec![0; BIG]);
        let cover = root.join("covers/g1.png");
        write(&cover, b"\x89PNG");
        let json = format!(
            r#"[
                {{"gameid": "g1", "title": "My Game", "path": "{game}", "prefix": "{root}/Faugus/game", "runner": "GE-Proton", "cover": "{cover}", "hidden": false}},
                {{"gameid": "g2", "title": "Native", "path": "{native}", "runner": "Linux-Native"}},
                {{"gameid": "g3", "title": "On Steam", "path": "1091500", "runner": "Steam"}},
                {{"gameid": "ea-app", "title": "EA App", "path": "{ea}", "runner": "GE-Proton"}},
                {{"gameid": "g4", "title": "Gone", "path": "{root}/nowhere.exe", "runner": "GE-Proton"}},
                {{"gameid": "g5", "title": "Var", "path": "$XDG_DATA_HOME/x.exe"}}
            ]"#,
            game = game.display(),
            native = native.display(),
            ea = ea.display(),
            cover = cover.display(),
            root = root.display(),
        );
        let file = root.join("data/faugus-launcher/games.json");
        write(&file, json.as_bytes());
        // Steam's record is Steam's; a variable Faugus expands from its own
        // environment cannot be.
        assert_eq!(faugus_entries(&json, &root).len(), 4);
        let games = faugus_games(&[file]);
        let names: Vec<&str> = games.iter().map(|g| g.name.as_str()).collect();
        assert_eq!(names, ["My Game", "Native"]);
        assert_eq!(games[0].source, Source::Faugus);
        assert_eq!(games[0].profile_key(), "Game.exe");
        assert_eq!(games[0].cover.as_deref(), Some(cover.as_path()));
        assert_eq!(games[0].launch_command, None, "a Windows game needs UMU");
        assert_eq!(games[1].profile_key(), "native-game");
        assert!(games[1].launch_command.is_some());
        let home = faugus_entries(
            r#"[{"title": "T", "path": "~/g/x.exe"}]"#,
            Path::new("/home/u"),
        );
        assert_eq!(home[0].path, Path::new("/home/u/g/x.exe"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn an_unchanged_library_is_not_scanned_again() {
        let home = tempdir("cache");
        let steamapps = home.join(".local/share/Steam/steamapps");
        write(
            &steamapps.join("appmanifest_1.acf"),
            manifest("1", "One", "One", "4").as_bytes(),
        );
        write(&steamapps.join("common/One/One.exe"), &vec![0; BIG]);
        // This home's Steam only, not the machine's menu.
        let sources = Sources {
            applications: Vec::new(),
            path_dirs: Vec::new(),
            flatpak: Vec::new(),
            ..Sources::of_home(&home)
        };
        let mut cache = None;
        let now = std::time::Instant::now();
        let (games, scanned) = cached_scan(&mut cache, &sources, now);
        assert!(scanned);
        assert_eq!(games.len(), 1);
        assert!(!cached_scan(&mut cache, &sources, now).1, "nothing changed");
        // A new manifest is a new game.
        write(
            &steamapps.join("appmanifest_2.acf"),
            manifest("2", "Two", "Two", "4").as_bytes(),
        );
        write(&steamapps.join("common/Two/Two.exe"), &vec![0; BIG]);
        let (games, scanned) = cached_scan(&mut cache, &sources, now);
        assert!(scanned);
        assert_eq!(games.len(), 2);
        // A game's folder deleted by hand, its manifest left behind.
        fs::remove_dir_all(steamapps.join("common/Two")).unwrap();
        let (games, scanned) = cached_scan(&mut cache, &sources, now);
        assert!(scanned);
        assert_eq!(games.len(), 1);
        // And whatever the files say, an old scan is not handed out.
        assert!(!cached_scan(&mut cache, &sources, now).1);
        assert!(cached_scan(&mut cache, &sources, now + LIBRARY_MAX_AGE).1);
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn another_homes_launchers_are_found_at_their_default_places() {
        let home = tempdir("xdg");
        let xdg = Xdg::of_home(&home);
        assert_eq!(xdg.config, home.join(".config"));
        assert_eq!(xdg.data, home.join(".local/share"));
        // The Snap's Steam.
        let snap = home.join("snap/steam/common/.local/share/Steam/steamapps");
        write(
            &snap.join("appmanifest_7.acf"),
            manifest("7", "Snapped", "Snapped", "4").as_bytes(),
        );
        write(&snap.join("common/Snapped/Snapped.exe"), &vec![0; BIG]);
        assert_eq!(steam_games(&home)[0].name, "Snapped");
        // A newer Lutris keeps its games under the data directory.
        let roots = lutris_roots(&home, &xdg);
        assert!(
            roots
                .iter()
                .any(|r| r.games == home.join(".local/share/lutris/games"))
        );
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn menu_field_codes_escapes_and_shells_are_understood() {
        let entry = |exec: &str| {
            format!(
                "[Desktop Entry]\nType=Application\nName=KMines\nIcon=kmines\nExec={exec}\nCategories=Game;\n"
            )
        };
        let kmines = menu_game(&entry("kmines -qwindowtitle %c %i")).unwrap();
        assert_eq!(
            kmines.argv,
            ["kmines", "-qwindowtitle", "KMines", "--icon", "kmines"]
        );
        let at =
            menu_game_at(&entry("game --desktop=%k"), Some(Path::new("/a/k.desktop"))).unwrap();
        assert_eq!(at.argv, ["game", "--desktop=/a/k.desktop"]);
        // `\s` is a space in any string value, before Exec's own quoting.
        let spaced = menu_game(&entry("\"/opt/My\\sGame/run\" --x")).unwrap();
        assert_eq!(spaced.program, "run");
        assert_eq!(spaced.argv[0], "/opt/My Game/run");
        // A game started through a shell is still a game.
        let shell = menu_game(&entry("sh -c \"cd /opt/g && exec ./game --fs\"")).unwrap();
        assert_eq!(shell.program, "game");
        assert_eq!(
            launched_program(&shell.argv).as_deref(),
            Some("/opt/g/./game")
        );
        let script = menu_game(&entry("bash /opt/g/start.sh")).unwrap();
        assert_eq!(script.program, "start.sh");
    }

    #[test]
    fn more_wrappers_are_seen_through_and_generic_programs_are_not_games() {
        let entry = |exec: &str| {
            format!("[Desktop Entry]\nType=Application\nName=G\nExec={exec}\nCategories=Game;\n")
        };
        for (exec, program) in [
            ("taskset -c 0-3 game", "game"),
            ("taskset 0x3 game", "game"),
            ("obs-gamecapture game", "game"),
            ("strangle 60 game", "game"),
            ("pw-jack -s 48000 game", "game"),
            ("firejail --noprofile game", "game"),
            ("flatpak-spawn --host game", "game"),
            ("switcherooctl launch -g 1 game", "game"),
            ("env -u X nice -n 5 game", "game"),
        ] {
            assert_eq!(
                menu_game(&entry(exec)).map(|g| g.program).as_deref(),
                Some(program),
                "{exec}"
            );
        }
        for exec in [
            "firefox https://play.example/game",
            "java -jar /opt/g/game.jar",
            "python3 /opt/g/main.py",
            "google-chrome-stable --app=https://x",
            "flatpak run --command=bottles-cli com.usebottles.bottles run -b G",
            "faugus-launcher --game g1",
            "umu-run game.exe",
        ] {
            assert_eq!(menu_game(&entry(exec)), None, "{exec}");
        }
    }

    #[test]
    fn a_name_two_games_share_does_not_say_which_runs() {
        let mut a = game(Source::Lutris, "RPG One");
        a.executables = vec!["Game.exe".into()];
        a.launch_file = Some(PathBuf::from("/g/one/Game.exe"));
        let mut b = game(Source::Lutris, "RPG Two");
        b.executables = vec!["Game.exe".into()];
        b.launch_file = Some(PathBuf::from("/g/two/Game.exe"));
        let games = vec![a, b];
        assert_eq!(game_among(&games, "Game.exe", None), None);
        assert_eq!(
            game_among(&games[..1], "game.exe", None).map(|g| g.name.as_str()),
            Some("RPG One")
        );
    }

    #[test]
    fn a_windows_path_is_found_in_the_processs_prefix() {
        let root = tempdir("dosdevices");
        let prefix = root.join("pfx");
        fs::create_dir_all(prefix.join("dosdevices")).unwrap();
        fs::create_dir_all(prefix.join("drive_c/Games")).unwrap();
        std::os::unix::fs::symlink("../drive_c", prefix.join("dosdevices/c:")).unwrap();
        std::os::unix::fs::symlink("/", prefix.join("dosdevices/z:")).unwrap();
        let canonical_prefix = fs::canonicalize(&prefix).unwrap();
        assert_eq!(
            host_path_in("C:\\Games\\RPG\\Game.exe", &prefix),
            Some(canonical_prefix.join("drive_c/Games/RPG/Game.exe"))
        );
        assert_eq!(
            host_path_in("Z:\\home\\u\\Game.exe", &prefix),
            Some(PathBuf::from("/home/u/Game.exe"))
        );
        assert_eq!(
            host_path_in("/opt/g/game", &prefix),
            Some(PathBuf::from("/opt/g/game"))
        );
        assert_eq!(host_path_in("game.exe", &prefix), None);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn detection_runs_on_this_machine_without_panicking() {
        let games = detect_all();
        for game in &games {
            assert!(!game.name.is_empty());
            assert!(!game.profile_key().is_empty());
        }
    }
}
