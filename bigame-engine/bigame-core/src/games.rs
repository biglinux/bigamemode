//! Library discovery: what is installed, what it is called, and what it looks
//! like.
//!
//! The audit's GAME-01 was here. Detection stored Steam's `installdir` as the
//! "executable", and profiles were keyed on it — but falcond matches
//! `/proc/<pid>/comm`. On this bench the consequence was exact and checkable:
//!
//! ```text
//! ARC Raiders        installdir "Arc Raiders"        real process PioneerGame.exe
//! Dead by Daylight   installdir "Dead by Daylight"   real process DeadByDaylight.exe
//! ```
//!
//! Both profiles the old UI wrote were loaded by falcond and could never match
//! anything. So detection now looks inside the install directory for the
//! binaries that actually run, and ranks them.
//!
//! Artwork is discovered from what the launchers have already downloaded. No
//! API key, no network request, no third-party service — if Steam has a cover
//! on disk, it is used, and if it does not, the fallback chain degrades to an
//! icon rather than to an empty box.

use std::path::{Path, PathBuf};

/// Where a game came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
        }
    }
}

/// An installed game.
#[derive(Debug, Clone)]
pub struct DetectedGame {
    /// Title as the launcher shows it.
    pub name: String,
    /// Launcher.
    pub source: Source,
    /// Steam `AppID`, when the game came from Steam.
    pub app_id: Option<String>,
    /// Install directory, when known.
    pub install_path: Option<PathBuf>,
    /// Candidate process names, best first.
    ///
    /// falcond matches on process name, so this — not the title — is what a
    /// profile must be keyed on.
    pub executables: Vec<String>,
    /// Portrait cover already on disk, if a launcher cached one.
    pub cover: Option<PathBuf>,
    /// A command that starts this game directly, when one exists.
    ///
    /// `None` for anything that has to go through a launcher process — Steam
    /// titles in particular, where `steam -applaunch` returns immediately and
    /// the game runs in a separate tree. That distinction matters: a benchmark
    /// needs a handle on the process it is measuring, so features that require
    /// one are offered only where this is `Some`.
    pub launch_command: Option<Vec<String>>,
}

impl DetectedGame {
    /// The process name a profile should be keyed on.
    ///
    /// Falls back to the title only when no executable could be found, and
    /// callers should treat that as "ask the user" rather than "good enough" —
    /// a title-keyed profile is the bug this module exists to fix.
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
}

/// Discover everything installed, sorted by title.
#[must_use]
pub fn detect_all() -> Vec<DetectedGame> {
    let Ok(home) = std::env::var("HOME") else {
        return Vec::new();
    };
    let home = Path::new(&home);

    let mut games = Vec::new();
    detect_steam(home, &mut games);
    detect_lutris(home, &mut games);
    detect_heroic(home, &mut games);
    detect_native(&mut games);

    games.sort_by_key(|g| g.name.to_lowercase());
    games.dedup_by(|a, b| a.name == b.name && a.source == b.source);
    games
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

/// A game in the application menu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuGame {
    /// The entry's `Name`.
    pub name: String,
    /// The process name falcond will see (the basename of the program).
    pub program: String,
    /// `Exec` as an argument vector, field codes (`%U`, `%f`, …) removed.
    pub argv: Vec<String>,
}

/// A menu entry, when the entry is a game.
///
/// Reads only the `[Desktop Entry]` group (actions such as `SuperTuxKart`'s
/// *Software Render* live in groups of their own), requires the `Game`
/// category, and takes the program from `Exec` past `env` and its
/// `VAR=value` assignments. Launchers and entries that start a game through
/// a launcher (`steam steam://rungameid/…`, `flatpak run …`) are not games
/// here: their process is the launcher, and Steam's are found by their tree.
#[must_use]
pub fn menu_game(content: &str) -> Option<MenuGame> {
    let mut in_entry = false;
    let (mut exec, mut name, mut categories) = (None, None, None);
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
            "Categories" => categories = Some(value.to_owned()),
            "Type" => application = value.trim() == "Application",
            "Hidden" | "NoDisplay" => hidden |= value.trim() == "true",
            _ => {}
        }
    }
    if !application || hidden {
        return None;
    }
    categories?.split(';').find(|c| c.trim() == "Game")?;
    let argv: Vec<String> = exec_arguments(&exec?)
        .into_iter()
        .filter_map(|a| expand_field_codes(&a))
        .collect();
    let program = crate::running::falcond_name(program_of(&argv)?).to_owned();
    if program.is_empty()
        || NOT_GAMES.contains(&program.to_ascii_lowercase().as_str())
        || crate::running::is_infrastructure(&program)
    {
        return None;
    }
    let name = name.unwrap_or_else(|| program.clone());
    Some(MenuGame {
        name,
        program,
        argv,
    })
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
                let takes_value = matches!(*next, "-u" | "-C" | "-n" | "-c" | "-t" | "-p");
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
    use std::os::unix::fs::PermissionsExt;
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    if !meta.is_file() || meta.permissions().mode() & 0o111 == 0 {
        return false;
    }
    let mut magic = [0u8; 4];
    std::fs::File::open(path)
        .and_then(|mut f| f.read_exact(&mut magic))
        .is_ok_and(|()| &magic == b"\x7fELF" || magic.starts_with(b"#!"))
}

/// Every game in the application menu: `XDG_DATA_HOME` and each of
/// `XDG_DATA_DIRS`, the first entry of a name winning, as the menu does.
#[must_use]
pub fn menu_games() -> Vec<MenuGame> {
    let data_home = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| Some(crate::paths::home_dir().join(".local/share")));
    let data_dirs = std::env::var("XDG_DATA_DIRS")
        .ok()
        .filter(|d| !d.is_empty())
        .unwrap_or_else(|| "/usr/local/share:/usr/share".to_owned());
    let dirs = data_home
        .into_iter()
        .chain(data_dirs.split(':').map(PathBuf::from))
        .map(|d| d.join("applications"));
    let mut seen = std::collections::HashSet::new();
    let mut games = Vec::new();
    for dir in dirs {
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
            if let Some(game) = std::fs::read_to_string(&path)
                .ok()
                .as_deref()
                .and_then(menu_game)
            {
                games.push(game);
            }
        }
    }
    games
}

/// Native games from the menu, unless a launcher already listed the same
/// executable. They start directly, so they can be measured.
fn detect_native(games: &mut Vec<DetectedGame>) {
    for game in menu_games() {
        if games.iter().any(|g| g.executables.contains(&game.program)) {
            continue;
        }
        games.push(DetectedGame {
            name: game.name,
            source: Source::Native,
            app_id: None,
            install_path: None,
            executables: vec![game.program],
            cover: None,
            launch_command: Some(game.argv),
        });
    }
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
    roots.retain(|r| r.join("steamapps").is_dir());
    roots.dedup();
    roots
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

fn detect_steam(home: &Path, games: &mut Vec<DetectedGame>) {
    for root in steam_libraries(home) {
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
            if let Some(game) = parse_acf(&path, &steamapps, home) {
                games.push(game);
            }
        }
    }
}

/// Titles that are runtimes and tooling rather than games.
fn is_steam_runtime(name: &str) -> bool {
    name.starts_with("Proton")
        || name.starts_with("Steam Linux Runtime")
        || name.contains("Steamworks")
        || name.contains("EasyAntiCheat Runtime")
        || name.starts_with("Steam Deck")
}

fn parse_acf(manifest: &Path, steamapps: &Path, home: &Path) -> Option<DetectedGame> {
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

    let install_path = steamapps.join("common").join(&installdir);
    let executables = if install_path.is_dir() {
        find_executables(&install_path)
    } else {
        Vec::new()
    };

    Some(DetectedGame {
        cover: steam_cover(home, &app_id),
        app_id: Some(app_id),
        install_path: install_path.is_dir().then_some(install_path),
        executables,
        name,
        source: Source::Steam,
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
/// Steam's cache layout changed: covers now live under a per-app directory in a
/// further hash-named subdirectory, so the search has to recurse rather than
/// build a fixed path. The filename preference degrades gracefully, which
/// matters — on this bench ARC Raiders has `library_600x900.jpg` and Dead by
/// Daylight does not, only `library_capsule.jpg`.
#[must_use]
pub fn steam_cover(home: &Path, app_id: &str) -> Option<PathBuf> {
    // Portrait first: the card layout is a 2:3 poster.
    const PREFERRED: &[&str] = &[
        "library_600x900.jpg",
        "library_600x900_2x.jpg",
        "library_capsule.jpg",
        "library_header.jpg",
        "header.jpg",
    ];
    for root in steam_libraries(home) {
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

// ── Lutris ───────────────────────────────────────────────────────────────────

fn detect_lutris(home: &Path, games: &mut Vec<DetectedGame>) {
    let dir = home.join(".config/lutris/games");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return;
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
        let (name, exe, exe_path) = parse_lutris_yml(&content);
        let name = name.unwrap_or_else(|| slug_to_title(slug));
        // Only a path that still exists is offered as launchable; a stale
        // Lutris entry pointing at a deleted directory is worse than none.
        let launch_command = exe_path
            .filter(|p| runs_natively(p))
            .map(|p| vec![p.to_string_lossy().into_owned()]);
        games.push(DetectedGame {
            cover: lutris_cover(home, slug),
            executables: exe.into_iter().collect(),
            name,
            source: Source::Lutris,
            app_id: None,
            install_path: None,
            launch_command,
        });
    }
}

/// Pull `name:` and `exe:` out of a Lutris game YAML.
///
/// Returns the declared name, the executable's basename (what a profile is
/// keyed on) and its full path (what can actually be launched).
#[must_use]
pub fn parse_lutris_yml(content: &str) -> (Option<String>, Option<String>, Option<PathBuf>) {
    let mut name = None;
    let mut exe = None;
    let mut exe_path = None;
    for line in content.lines() {
        let trimmed = line.trim();
        if let Some(v) = trimmed.strip_prefix("name:") {
            let v = v.trim().trim_matches(['"', '\'']);
            if !v.is_empty() {
                name = Some(v.to_owned());
            }
        }
        if exe.is_none() {
            if let Some(v) = trimmed.strip_prefix("exe:") {
                let v = v.trim().trim_matches(['"', '\'']);
                exe = Path::new(v)
                    .file_name()
                    .map(|f| f.to_string_lossy().into_owned());
                let path = PathBuf::from(v);
                if path.is_absolute() {
                    exe_path = Some(path);
                }
            }
        }
    }
    (name, exe, exe_path)
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

fn lutris_cover(home: &Path, slug: &str) -> Option<PathBuf> {
    for base in [
        home.join(".local/share/lutris/coverart"),
        home.join(".cache/lutris/coverart"),
        home.join(".local/share/lutris/banners"),
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

fn detect_heroic(home: &Path, games: &mut Vec<DetectedGame>) {
    let base = home.join(".config/heroic");
    for file in [
        "store_cache/gog_library.json",
        "store_cache/legendary_library.json",
        "store_cache/nile_library.json",
        "store_cache/zoom-library.json",
    ] {
        let Ok(content) = std::fs::read_to_string(base.join(file)) else {
            continue;
        };
        for title in json_string_values(&content, "title") {
            games.push(DetectedGame {
                cover: heroic_cover(&base, &title),
                name: title,
                source: Source::Heroic,
                app_id: None,
                install_path: None,
                executables: Vec::new(),
                launch_command: None,
            });
        }
    }

    // Sideloaded titles are one directory each.
    if let Ok(entries) = std::fs::read_dir(base.join("sideload_apps")) {
        for entry in entries.flatten() {
            if !entry.path().is_dir() {
                continue;
            }
            let dirname = entry.file_name().to_string_lossy().into_owned();
            let title = std::fs::read_to_string(entry.path().join("info.json"))
                .ok()
                .and_then(|c| json_string_values(&c, "title").into_iter().next())
                .unwrap_or_else(|| dirname.clone());
            let executables = find_executables(&entry.path());
            let launch_command = executables
                .first()
                .map(|exe| vec![entry.path().join(exe).to_string_lossy().into_owned()])
                .filter(|cmd| runs_natively(Path::new(&cmd[0])));
            games.push(DetectedGame {
                cover: heroic_cover(&base, &title),
                executables,
                install_path: Some(entry.path()),
                name: title,
                source: Source::Heroic,
                app_id: None,
                launch_command,
            });
        }
    }
}

/// Collect every `"key": "value"` string value from JSON text.
///
/// A line scan rather than a parser: Heroic's caches are large, change shape
/// between versions, and all that is needed here is titles. The key is matched
/// anywhere in the line rather than only at the start, because Heroic writes
/// both pretty-printed and compact objects depending on the store backend.
#[must_use]
pub fn json_string_values(content: &str, key: &str) -> Vec<String> {
    let needle = format!("\"{key}\":");
    let mut out = Vec::new();
    for line in content.lines() {
        let mut rest = line;
        while let Some(at) = rest.find(needle.as_str()) {
            rest = &rest[at + needle.len()..];
            let trimmed = rest.trim_start();
            let Some(value) = trimmed.strip_prefix('"') else {
                continue;
            };
            let Some(end) = value.find('"') else {
                break;
            };
            let found = &value[..end];
            if !found.is_empty() && found != "null" {
                out.push(found.to_owned());
            }
            rest = &value[end..];
        }
    }
    out
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

    const STK_DESKTOP: &str = "[Desktop Entry]\nName=SuperTuxKart\nName[pt_BR]=SuperTuxKart\nExec=supertuxkart\nType=Application\nCategories=Game;ArcadeGame;\nActions=SoftwareRender;\n\n[Desktop Action SoftwareRender]\nName=Software Render\nExec=SoftwareRender supertuxkart\n";

    #[test]
    fn a_menu_entry_in_the_game_category_is_a_game() {
        let stk = menu_game(STK_DESKTOP).unwrap();
        assert_eq!(stk.program, "supertuxkart");
        assert_eq!(stk.name, "SuperTuxKart");
        assert_eq!(stk.argv, ["supertuxkart"]);
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
        ] {
            assert_eq!(menu_game(&entry(exec)).unwrap().program, program, "{exec}");
        }
        let game = menu_game(&entry("game --file=%f --level=100%% %U")).unwrap();
        assert_eq!(game.argv, ["game", "--file=", "--level=100%"]);
        assert!(menu_game(&entry("prime-run")).is_none());
    }

    #[test]
    fn only_a_native_executable_is_launchable() {
        use std::os::unix::fs::PermissionsExt;
        let dir = crate::tests::tempdir("runs-natively");
        let write = |name: &str, bytes: &[u8], mode: u32| {
            let p = dir.join(name);
            std::fs::write(&p, bytes).unwrap();
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(mode)).unwrap();
            p
        };
        assert!(runs_natively(&write(
            "run.sh",
            b"#!/bin/sh\nexec game\n",
            0o755
        )));
        assert!(runs_natively(&write("game", b"\x7fELF\x02\x01", 0o755)));
        assert!(!runs_natively(&write("Game.exe", b"MZ\x90\x00", 0o755)));
        assert!(!runs_natively(&write("noexec", b"\x7fELF\x02\x01", 0o644)));
        assert!(!runs_natively(&dir.join("missing")));
    }

    #[test]
    fn launchers_tools_and_hidden_entries_are_not_games() {
        let steam_shortcut = "[Desktop Entry]\nType=Application\nName=Hades\nExec=steam steam://rungameid/1145360\nCategories=Game;\n";
        let bigame = "[Desktop Entry]\nType=Application\nName=BiGame-mode\nExec=bigame-ui\nCategories=Game;System;Settings;\n";
        let flatpak = "[Desktop Entry]\nType=Application\nName=0 A.D.\nExec=/usr/bin/flatpak run com.play0ad.zeroad\nCategories=Game;\n";
        let hidden =
            "[Desktop Entry]\nType=Application\nName=X\nExec=x\nNoDisplay=true\nCategories=Game;\n";
        let editor = "[Desktop Entry]\nType=Application\nName=Kate\nExec=kate %U\nCategories=Qt;KDE;Utility;TextEditor;\n";
        let gamepad_tool =
            "[Desktop Entry]\nType=Application\nName=Pad\nExec=pad\nCategories=GamepadTool;\n";
        for entry in [
            steam_shortcut,
            bigame,
            flatpak,
            hidden,
            editor,
            gamepad_tool,
        ] {
            assert_eq!(menu_game(entry), None, "{entry}");
        }
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

    use std::fs;

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

    #[test]
    fn acf_values_are_read() {
        let acf = "\"AppState\"\n{\n\t\"appid\"\t\"1808500\"\n\t\"name\"\t\"ARC Raiders\"\n\t\"installdir\"\t\"Arc Raiders\"\n}\n";
        assert_eq!(acf_value(acf, "appid").as_deref(), Some("1808500"));
        assert_eq!(acf_value(acf, "name").as_deref(), Some("ARC Raiders"));
        assert_eq!(acf_value(acf, "installdir").as_deref(), Some("Arc Raiders"));
        assert_eq!(acf_value(acf, "missing"), None);
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
        // EpicWebHelper.exe sits inside both Steam titles on this bench and is
        // larger than some game binaries, so size ranking alone is not enough.
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
        // This is GAME-01 in one assertion. Keyed on the title, falcond can
        // never match the process, and the profile does nothing.
        let game = DetectedGame {
            name: "ARC Raiders".into(),
            source: Source::Steam,
            app_id: Some("1808500".into()),
            install_path: None,
            executables: vec!["PioneerGame.exe".into()],
            cover: None,
            launch_command: None,
        };
        assert_eq!(game.profile_key(), "PioneerGame.exe");
        assert_ne!(game.profile_key(), "ARC Raiders");
        assert!(game.has_real_executable());
    }

    #[test]
    fn a_game_with_no_executable_is_flagged_rather_than_guessed() {
        let game = DetectedGame {
            name: "Some Game".into(),
            source: Source::Heroic,
            app_id: None,
            install_path: None,
            executables: Vec::new(),
            cover: None,
            launch_command: None,
        };
        // It still yields something usable, but callers can tell it is a guess.
        assert_eq!(game.profile_key(), "Some Game");
        assert!(!game.has_real_executable());
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
        // Steam moved covers under librarycache/<appid>/<hash>/, so a fixed
        // path finds nothing on a current install.
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
        // Dead by Daylight on this bench has no library_600x900.jpg.
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

    #[test]
    fn lutris_yaml_prefers_the_declared_name_and_exe_basename() {
        let (name, exe, path) = parse_lutris_yml(
            "name: Celeste\ngame:\n  exe: /home/u/Games/celeste/Celeste.bin.x86_64\n",
        );
        assert_eq!(name.as_deref(), Some("Celeste"));
        // The basename is what a profile is keyed on…
        assert_eq!(exe.as_deref(), Some("Celeste.bin.x86_64"));
        // …and the full path is what can be launched.
        assert_eq!(
            path.as_deref(),
            Some(Path::new("/home/u/Games/celeste/Celeste.bin.x86_64"))
        );
    }

    #[test]
    fn a_relative_lutris_exe_is_not_treated_as_launchable() {
        // Without an absolute path there is nothing to run from here.
        let (_, exe, path) = parse_lutris_yml("game:\n  exe: game.sh\n");
        assert_eq!(exe.as_deref(), Some("game.sh"));
        assert_eq!(path, None);
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

    #[test]
    fn heroic_titles_are_extracted_and_blanks_skipped() {
        let json = "{\n  \"library\": [\n    { \"title\": \"Hades\" },\n    { \"title\": \"\" },\n    { \"title\": \"null\" },\n    { \"title\": \"Celeste\" }\n  ]\n}\n";
        assert_eq!(
            json_string_values(json, "title"),
            vec!["Hades".to_owned(), "Celeste".to_owned()]
        );
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
