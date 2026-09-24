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
}

impl Source {
    /// Name for display.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Steam => "Steam",
            Self::Lutris => "Lutris",
            Self::Heroic => "Heroic",
        }
    }

    /// Symbolic icon to badge the cover with.
    #[must_use]
    pub fn icon(self) -> &'static str {
        match self {
            Self::Steam => "applications-games-symbolic",
            Self::Lutris | Self::Heroic => "application-x-executable-symbolic",
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

    /// Whether this game can be started, and measured, without a launcher.
    #[must_use]
    pub fn is_directly_launchable(&self) -> bool {
        self.launch_command.is_some()
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

    games.sort_by_key(|g| g.name.to_lowercase());
    games.dedup_by(|a, b| a.name == b.name && a.source == b.source);
    games
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
            .filter(|p| p.is_file())
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
                .filter(|cmd| std::path::Path::new(&cmd[0]).is_file());
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
    fn steam_titles_are_not_directly_launchable() {
        // `steam -applaunch` returns immediately and the game runs elsewhere,
        // so nothing that needs a handle on the game can be offered for them.
        let game = DetectedGame {
            name: "ARC Raiders".into(),
            source: Source::Steam,
            app_id: Some("1808500".into()),
            install_path: None,
            executables: vec!["PioneerGame.exe".into()],
            cover: None,
            launch_command: None,
        };
        assert!(!game.is_directly_launchable());
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
