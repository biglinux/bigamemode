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

use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Where a game came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Source {
    /// Steam.
    Steam,
    /// Lutris.
    Lutris,
    /// Heroic — Epic, GOG, Amazon or a sideloaded title.
    Heroic,
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
/// Heroic's, Heroic's over Lutris's, and the application menu comes last, so
/// a menu entry for a Lutris game is folded into the Lutris one.
#[must_use]
pub fn detect_all() -> Vec<DetectedGame> {
    let home = crate::paths::home_dir();
    let mut games = steam_games(&home);
    games.extend(heroic_games(&heroic_config_dirs(&home)));
    games.extend(lutris_games(&lutris_roots(&home)));
    games.extend(menu_games().into_iter().map(DetectedGame::from));
    let mut games = dedup(games);
    games.sort_by_key(|g| g.name.to_lowercase());
    games
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

/// Programs listed as games that are not games here: launchers, stores,
/// tools, game streaming (the game runs elsewhere) — and BiGame-mode itself,
/// whose menu entry is in the Game category too.
const NOT_GAMES: &[&str] = &[
    "bigame-ui",
    "steam",
    "lutris",
    "heroic",
    "heroic-run",
    "legendary",
    "bottles",
    "itch",
    "minigalaxy",
    "gamehub",
    "playonlinux",
    "protonup-qt",
    "protonplus",
    "gamescope",
    "mangohud",
    "mangojuice",
    "goverlay",
    "flatpak",
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
            "Exec" => exec = Some(value.trim().to_owned()),
            "Name" => name = Some(value.trim().to_owned()),
            "Icon" => icon = Some(value.trim().to_owned()).filter(|v| !v.is_empty()),
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
    let argv: Vec<String> = exec_arguments(&exec?)
        .into_iter()
        .filter_map(|a| expand_field_codes(&a))
        .collect();
    let launched = program_of(&argv)?;
    let (program, flatpak) = if crate::running::falcond_name(launched) == "flatpak" {
        let run = flatpak_run(&argv)?;
        (run.command.unwrap_or_default(), Some(run.app_id))
    } else {
        (crate::running::falcond_name(launched).to_owned(), None)
    };
    if flatpak.is_none() && !is_game_program(&program) {
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

/// Whether a process name is a game's rather than a launcher's or a tool's.
fn is_game_program(program: &str) -> bool {
    !program.is_empty()
        && !NOT_GAMES.contains(&program.to_ascii_lowercase().as_str())
        && !crate::running::is_infrastructure(program)
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
];

/// The program an `Exec` line really runs, past [`WRAPPERS`], their options,
/// `VAR=value` assignments, and Gamescope's own arguments up to `--`.
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
        // Options, their values (env -u NAME, env -C DIR, nice -n 5, …) and
        // assignments come before the program.
        while let Some(next) = rest.peek() {
            if next.starts_with('-') {
                let takes_value = matches!(*next, "-u" | "-C" | "-n" | "-c" | "-p");
                rest.next();
                if takes_value {
                    rest.next();
                }
            } else if next.contains('=') && !next.starts_with('/') {
                rest.next();
            } else {
                break;
            }
        }
    }
    None
}

/// Apply the Desktop Entry field codes to one argument: `%f`/`%F`/`%u`/`%U`
/// (files and URLs, of which there are none) remove the argument when it is
/// all they are and vanish inside one; `%%` is a percent sign; the rest
/// (`%i`, `%c`, `%k`, deprecated ones) are dropped.
fn expand_field_codes(arg: &str) -> Option<String> {
    if matches!(arg, "%f" | "%F" | "%u" | "%U" | "%i" | "%c" | "%k") {
        return None;
    }
    let mut out = String::with_capacity(arg.len());
    let mut chars = arg.chars();
    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        if chars.next() == Some('%') {
            out.push('%');
        }
    }
    Some(out)
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
    let data_home = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| crate::paths::home_dir().join(".local/share"));
    let data_dirs = std::env::var("XDG_DATA_DIRS")
        .ok()
        .filter(|d| !d.is_empty())
        .unwrap_or_else(|| "/usr/local/share:/usr/share".to_owned());
    let applications: Vec<PathBuf> = std::iter::once(data_home.clone())
        .chain(data_dirs.split(':').map(PathBuf::from))
        .map(|d| d.join("applications"))
        .collect();
    let path_dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    let installations = [PathBuf::from("/var/lib/flatpak"), data_home.join("flatpak")];
    menu_games_in(&applications, &path_dirs, &installations)
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
                .as_deref()
                .and_then(menu_game)
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
                let Some(program_path) = resolve_program(&game.program, path_dirs) else {
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

/// Steam library roots, including extra library folders the user has added.
///
/// `~/.steam/steam` is normally a symlink to `~/.local/share/Steam`; roots are
/// canonicalised so one library is scanned once.
#[must_use]
pub fn steam_libraries(home: &Path) -> Vec<PathBuf> {
    let mut roots = vec![
        home.join(".local/share/Steam"),
        home.join(".steam/steam"),
        home.join(".var/app/com.valvesoftware.Steam/.local/share/Steam"),
    ];
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

/// Extract `"path" "..."` values from a Steam VDF file.
#[must_use]
pub fn parse_vdf_paths(content: &str) -> Vec<String> {
    content
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            let rest = line.strip_prefix("\"path\"")?;
            let mut parts = rest.split('"').filter(|p| !p.trim().is_empty());
            parts.next().map(str::to_owned)
        })
        .collect()
}

/// The installed Steam titles of every library under `home`.
#[must_use]
pub fn steam_games(home: &Path) -> Vec<DetectedGame> {
    let libraries = steam_libraries(home);
    let mut games = Vec::new();
    for root in &libraries {
        let steamapps = root.join("steamapps");
        let Ok(entries) = std::fs::read_dir(&steamapps) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if !name.starts_with("appmanifest_") || !name.ends_with(".acf") {
                continue;
            }
            if let Some(game) = parse_acf(&path, &steamapps, &libraries) {
                games.push(game);
            }
        }
    }
    games
}

/// Titles that are runtimes and tooling rather than games.
fn is_steam_runtime(name: &str) -> bool {
    name.starts_with("Proton")
        || name.starts_with("Steam Linux Runtime")
        || name.contains("Steamworks")
        || name.contains("EasyAntiCheat Runtime")
        || name.starts_with("Steam Deck")
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

fn parse_acf(manifest: &Path, steamapps: &Path, libraries: &[PathBuf]) -> Option<DetectedGame> {
    let content = std::fs::read_to_string(manifest).ok()?;
    let name = acf_value(&content, "name")?;
    if is_steam_runtime(&name) {
        return None;
    }
    let installdir = acf_value(&content, "installdir")?;
    let app_id = acf_value(&content, "appid").or_else(|| {
        manifest
            .file_stem()?
            .to_string_lossy()
            .strip_prefix("appmanifest_")
            .map(str::to_owned)
    })?;

    if !acf_installed(&content) {
        tracing::debug!(app = %app_id, title = %name, "ignoring Steam manifest: not fully installed");
        return None;
    }
    let install_path = steamapps.join("common").join(&installdir);
    if !populated_dir(&install_path) {
        tracing::debug!(app = %app_id, title = %name, dir = %install_path.display(), "ignoring Steam manifest: install directory missing");
        return None;
    }

    Some(DetectedGame {
        cover: steam_cover_in(libraries, &app_id),
        app_id: Some(app_id),
        executables: find_executables(&install_path),
        install_path: Some(install_path),
        launch_file: None,
        name,
        source: Source::Steam,
        icon: None,
        // Steam titles start through the client, which returns immediately and
        // runs the game in a separate process tree. There is no command here
        // that yields a handle on the game itself.
        launch_command: None,
    })
}

/// Read a `"key" "value"` pair out of Valve's ACF format.
#[must_use]
pub fn acf_value(content: &str, key: &str) -> Option<String> {
    let needle = format!("\"{key}\"");
    content.lines().find_map(|line| {
        let line = line.trim();
        let rest = line.strip_prefix(&needle)?;
        let value = rest.split('"').nth(1)?;
        (!value.is_empty()).then(|| value.to_owned())
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
    // Store overlays and helper browsers ship inside game directories and are
    // often larger than the game binary itself.
    "epicwebhelper",
    "webhelper",
    "cefprocess",
    "crashpad",
    "steamerrorreporter",
];

/// Whether a filename looks like a support tool rather than the game itself.
#[must_use]
pub fn is_support_binary(filename: &str) -> bool {
    let lower = filename.to_ascii_lowercase();
    NOT_THE_GAME.iter().any(|needle| lower.contains(needle))
}

/// Find the process names a game is likely to run under, best first.
///
/// Ranked by size, because the game's own binary is essentially always the
/// largest executable a title ships. Windows executables are searched deeper
/// than one level, since engines commonly bury the real binary under
/// `Binaries/Win64/`.
#[must_use]
pub fn find_executables(install_dir: &Path) -> Vec<String> {
    let mut found: Vec<(u64, String)> = Vec::new();
    collect_executables(install_dir, 4, &mut found);
    // Largest first: the game binary is essentially always the biggest.
    found.sort_by_key(|(size, _)| std::cmp::Reverse(*size));
    let mut names: Vec<String> = found.into_iter().map(|(_, n)| n).collect();
    names.dedup();
    names.truncate(8);
    names
}

fn collect_executables(dir: &Path, depth: u32, out: &mut Vec<(u64, String)>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        if meta.is_dir() {
            if depth > 0 {
                collect_executables(&path, depth - 1, out);
            }
            continue;
        }
        if !meta.is_file() {
            continue;
        }
        let Some(filename) = path.file_name().map(|n| n.to_string_lossy().into_owned()) else {
            continue;
        };
        if is_support_binary(&filename) {
            continue;
        }
        // A real game binary is never tiny; this also filters wrapper scripts.
        if meta.len() < 256 * 1024 {
            continue;
        }
        let is_windows = filename.to_ascii_lowercase().ends_with(".exe");
        if is_windows || is_native_executable(&path, &meta) {
            out.push((meta.len(), filename));
        }
    }
}

fn is_native_executable(path: &Path, meta: &std::fs::Metadata) -> bool {
    use std::io::Read;
    use std::os::unix::fs::PermissionsExt;

    if meta.permissions().mode() & 0o111 == 0 {
        return false;
    }
    // Confirm ELF rather than trusting the executable bit, which is set on all
    // sorts of data files inside game directories.
    let Ok(mut file) = std::fs::File::open(path) else {
        return false;
    };
    let mut magic = [0u8; 4];
    file.read_exact(&mut magic).is_ok() && magic == *b"\x7fELF"
}

/// The basename of a launcher's executable, and the ranked executables of
/// its install directory after it: the launcher's word first, since it is
/// exact, and the scan for the profiles of games with a separate launcher.
fn executables_of(executable: Option<&Path>, install_path: Option<&Path>) -> Vec<String> {
    let mut names: Vec<String> = executable
        .and_then(Path::file_name)
        .map(|n| n.to_string_lossy().into_owned())
        .into_iter()
        .collect();
    for name in install_path.map(find_executables).unwrap_or_default() {
        if !names.contains(&name) {
            names.push(name);
        }
    }
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

/// The native Lutris and the Flatpak one.
fn lutris_roots(home: &Path) -> Vec<LutrisRoot> {
    let flatpak = home.join(".var/app/net.lutris.Lutris");
    vec![
        LutrisRoot {
            games: home.join(".config/lutris/games"),
            data: home.join(".local/share/lutris"),
            cache: home.join(".cache/lutris"),
        },
        LutrisRoot {
            games: flatpak.join("config/lutris/games"),
            data: flatpak.join("data/lutris"),
            cache: flatpak.join("cache/lutris"),
        },
    ]
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
    /// `game.prefix` (Wine).
    pub prefix: Option<String>,
}

/// Pull the launch keys out of a Lutris game YAML.
///
/// Top-level `name` and `runner`, and the indented `exe`, `main_file`,
/// `working_dir` and `prefix` of the `game` section. A line scan rather than
/// a YAML parser: the files are flat, and these keys do not appear in the
/// other sections.
#[must_use]
pub fn parse_lutris_yml(content: &str) -> LutrisConfig {
    let mut cfg = LutrisConfig::default();
    for line in content.lines() {
        let indented = line.starts_with([' ', '\t']);
        let trimmed = line.trim();
        let Some((key, value)) = trimmed.split_once(':') else {
            continue;
        };
        let value = value.trim().trim_matches(['"', '\'']);
        if value.is_empty() {
            continue;
        }
        let slot = match (indented, key.trim()) {
            (false, "name") => &mut cfg.name,
            (false, "runner") => &mut cfg.runner,
            (true, "exe") => &mut cfg.exe,
            (true, "main_file") => &mut cfg.main_file,
            (true, "working_dir") => &mut cfg.working_dir,
            (true, "prefix") => &mut cfg.prefix,
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
        let file = self
            .exe
            .as_deref()
            .or(self.main_file.as_deref())
            .ok_or("no executable recorded")?;
        if file.contains('$') {
            // `$GAMEDIR` and friends are resolved by Lutris from its database.
            return Err("path uses a Lutris variable");
        }
        let file = Path::new(file);
        let path = if file.is_absolute() {
            file.to_path_buf()
        } else {
            let base = self
                .working_dir
                .as_deref()
                .or(self.prefix.as_deref())
                .ok_or("relative path with no directory")?;
            Path::new(base).join(file)
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
            let cfg = parse_lutris_yml(&content);
            let launch_file = match cfg.launch_file() {
                Ok(file) => file,
                Err(reason) => {
                    tracing::debug!(config = %path.display(), reason, "ignoring Lutris game");
                    continue;
                }
            };
            let name = cfg.name.unwrap_or_else(|| slug_to_title(slug));
            let exe = launch_file
                .file_name()
                .map(|f| f.to_string_lossy().into_owned());
            // Only what runs here directly is offered as launchable: a
            // Windows binary needs its Wine runner.
            let launch_command = runs_natively(&launch_file)
                .then(|| vec![launch_file.to_string_lossy().into_owned()]);
            games.push(DetectedGame {
                cover: lutris_cover(root, slug),
                executables: exe.into_iter().collect(),
                name,
                source: Source::Lutris,
                app_id: None,
                install_path: None,
                launch_file: Some(launch_file),
                icon: None,
                launch_command,
            });
        }
    }
    games
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

/// The native Heroic's configuration directory and the Flatpak one's.
fn heroic_config_dirs(home: &Path) -> Vec<PathBuf> {
    vec![
        home.join(".config/heroic"),
        home.join(".var/app/com.heroicgameslauncher.hgl/config/heroic"),
    ]
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
            })
        })
        .collect()
}

/// The games of a store backend's own `installed.json`: legendary's map of
/// app name to record, GOG's `{"installed": [...]}` and Amazon's list. Each
/// record names its `install_path` (Amazon: `path`) and, when it can, its
/// `title` and `executable`.
#[must_use]
pub fn heroic_installed_entries(json: &str) -> Vec<HeroicEntry> {
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
        .filter_map(|r| {
            let string = |key: &str| {
                r.get(key)
                    .and_then(serde_json::Value::as_str)
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
            };
            let install_path = string("install_path").or_else(|| string("path"))?;
            Some(HeroicEntry {
                // Not every backend records the title here; the directory
                // name is what the user would recognise otherwise.
                title: string("title")
                    .or_else(|| string("app_name"))
                    .or_else(|| string("appName"))
                    .or_else(|| string("id"))
                    .unwrap_or_else(|| install_path.clone()),
                executable: string("executable").map(PathBuf::from),
                install_path: Some(PathBuf::from(install_path)),
            })
        })
        .collect()
}

/// Files Heroic writes about installed games, relative to its configuration
/// directory. The store libraries carry titles and the installed flag; the
/// backends' own lists are the record of what was actually installed.
const HEROIC_LIBRARIES: &[&str] = &[
    "store_cache/legendary_library.json",
    "store_cache/gog_library.json",
    "store_cache/nile_library.json",
    "store_cache/zoom-library.json",
    "sideload_apps/library.json",
];
const HEROIC_INSTALLED: &[&str] = &[
    "legendaryConfig/legendary/installed.json",
    "gog_store/installed.json",
    "nile_config/nile/installed.json",
];

/// The installed games of every Heroic in `configs`.
#[must_use]
pub fn heroic_games(configs: &[PathBuf]) -> Vec<DetectedGame> {
    let mut games = Vec::new();
    for base in configs {
        let mut entries = Vec::new();
        for file in HEROIC_LIBRARIES {
            if let Ok(json) = std::fs::read_to_string(base.join(file)) {
                entries.extend(heroic_library_entries(&json));
            }
        }
        for file in HEROIC_INSTALLED {
            if let Ok(json) = std::fs::read_to_string(base.join(file)) {
                entries.extend(heroic_installed_entries(&json));
            }
        }
        for entry in entries {
            let Some(game) = heroic_game(base, entry) else {
                continue;
            };
            games.push(game);
        }
    }
    games
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
    let launch_command = launch_file
        .as_deref()
        .filter(|exe| runs_natively(exe))
        .map(|exe| vec![exe.to_string_lossy().into_owned()]);
    Some(DetectedGame {
        cover: heroic_cover(base, &entry.title),
        executables: executables_of(launch_file.as_deref(), install_path.as_deref()),
        name: entry.title,
        source: Source::Heroic,
        app_id: None,
        install_path,
        launch_file,
        icon: None,
        launch_command,
    })
}

fn heroic_cover(base: &Path, title: &str) -> Option<PathBuf> {
    let slug: String = title
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    let dir = base.join("images-cache");
    for ext in ["jpg", "png", "webp"] {
        let path = dir.join(format!("{slug}.{ext}"));
        if path.is_file() {
            return Some(path);
        }
    }
    None
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
        let bigame = "[Desktop Entry]\nType=Application\nName=BiGame-mode\nExec=bigame-ui\nCategories=Game;System;Settings;\n";
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
        assert_eq!(found[0].profile_key(), "run.sh");
        assert!(found[0].launch_command.is_some());
        assert_eq!(found[1].profile_key(), "Game.exe");
        // A Windows binary is not started from here.
        assert_eq!(found[1].launch_command, None);
        let _ = fs::remove_dir_all(&root);
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
        let legendary = r#"{"Fortnite": {"app_name": "Fortnite", "title": "Fortnite", "install_path": "/g/Fortnite", "executable": "FortniteLauncher.exe"}}"#;
        let gog = r#"{"installed": [{"appName": "1", "install_path": "/g/Cuphead", "platform": "windows"}]}"#;
        let nile = r#"[{"id": "amzn1", "path": "/g/Amazon", "version": "1"}]"#;
        let l = heroic_installed_entries(legendary);
        assert_eq!(l[0].title, "Fortnite");
        assert_eq!(
            l[0].executable.as_deref(),
            Some(Path::new("FortniteLauncher.exe"))
        );
        assert_eq!(
            heroic_installed_entries(gog)[0].install_path.as_deref(),
            Some(Path::new("/g/Cuphead"))
        );
        assert_eq!(
            heroic_installed_entries(nile)[0].install_path.as_deref(),
            Some(Path::new("/g/Amazon"))
        );
        assert!(heroic_installed_entries("{}").is_empty());
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

    #[test]
    fn detection_runs_on_this_machine_without_panicking() {
        let games = detect_all();
        for game in &games {
            assert!(!game.name.is_empty());
            assert!(!game.profile_key().is_empty());
        }
    }
}
