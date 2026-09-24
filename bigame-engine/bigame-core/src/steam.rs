//! Steam per-game launch options.
//!
//! The launch pipeline cannot wrap `steam -applaunch` (the client starts the
//! game in its own process tree), so without this nothing it builds —
//! Gamescope, `MangoHud`, the upscaling and frame-generation variables — would
//! reach the way most people start games. `environment.d` covers only
//! environment variables, never Gamescope, and only after a re-login.
//!
//! The mechanism Steam itself provides is the per-game **launch options**
//! string, where `%command%` stands for the game's own command line. Writing
//! `gamescope … -- %command%` there is what makes a wrapper apply to a Steam
//! launch.
//!
//! Editing Steam's configuration is delicate and this module is built around
//! that:
//!
//! * **Steam must not be running.** It holds `localconfig.vdf` in memory and
//!   rewrites it on exit, so an edit made underneath a running client is simply
//!   discarded — silently, which is worse than failing.
//! * **A backup is written first**, beside the original.
//! * **Only the one key is touched**, located by its full path rather than by
//!   name. `LaunchOptions` appears at several nesting depths in a real file —
//!   including inside `cloud` blocks — and editing the wrong one does nothing.
//! * **The result is read back** and compared.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// Path from the root of `localconfig.vdf` to the per-app settings.
const APPS_PATH: &[&str] = &["UserLocalConfigStore", "Software", "Valve", "Steam", "apps"];

/// A Steam account with a local configuration file.
#[derive(Debug, Clone)]
pub struct SteamUser {
    /// Numeric account id, the `userdata/<id>` directory name.
    pub id: String,
    /// That account's `localconfig.vdf`.
    pub config: PathBuf,
}

/// Every local Steam account that has a `localconfig.vdf`.
#[must_use]
pub fn users(home: &Path) -> Vec<SteamUser> {
    let mut out = Vec::new();
    for root in crate::games::steam_libraries(home) {
        let Ok(entries) = std::fs::read_dir(root.join("userdata")) else {
            continue;
        };
        for entry in entries.flatten() {
            let id = entry.file_name().to_string_lossy().into_owned();
            // `userdata/0` is the anonymous placeholder, not an account.
            if id == "0" || !id.chars().all(|c| c.is_ascii_digit()) {
                continue;
            }
            let config = entry.path().join("config/localconfig.vdf");
            if config.is_file() {
                out.push(SteamUser { id, config });
            }
        }
    }
    out.sort_by(|a, b| a.id.cmp(&b.id));
    out.dedup_by(|a, b| a.id == b.id);
    out
}

/// Whether a Steam client is currently running.
///
/// Editing the configuration while it is would be discarded on exit.
#[must_use]
pub fn is_running() -> bool {
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return false;
    };
    entries.flatten().any(|entry| {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !name.chars().all(|c| c.is_ascii_digit()) {
            return false;
        }
        std::fs::read_to_string(entry.path().join("comm")).is_ok_and(|comm| comm.trim() == "steam")
    })
}

// ── VDF navigation ───────────────────────────────────────────────────────────

/// Indentation depth of a line, in leading tab characters.
fn depth(line: &str) -> usize {
    line.bytes().take_while(|b| *b == b'\t').count()
}

/// The quoted key a line opens, if it is a bare `"key"` line.
fn block_key(line: &str) -> Option<&str> {
    let trimmed = line.trim();
    let inner = trimmed.strip_prefix('"')?.strip_suffix('"')?;
    (!inner.contains('"')).then_some(inner)
}

/// The key of a `"key"  "value"` pair line.
fn pair_key(line: &str) -> Option<&str> {
    let trimmed = line.trim();
    let rest = trimmed.strip_prefix('"')?;
    let end = rest.find('"')?;
    let key = &rest[..end];
    // A pair has a second quoted token after the key.
    rest[end + 1..].trim_start().starts_with('"').then_some(key)
}

/// The value of a `"key"  "value"` pair line.
fn pair_value(line: &str) -> Option<&str> {
    let trimmed = line.trim();
    let rest = trimmed.strip_prefix('"')?;
    let end = rest.find('"')?;
    let after = rest[end + 1..].trim_start();
    let v = after.strip_prefix('"')?;
    let vend = v.rfind('"')?;
    Some(&v[..vend])
}

/// Line range `[open_brace+1, close_brace)` of the block reached by `path`.
///
/// Matching is by nesting depth as well as by name, so a `LaunchOptions` inside
/// a `cloud` sub-block is never mistaken for the app's own.
fn find_block(lines: &[&str], path: &[&str]) -> Option<(usize, usize)> {
    let mut search_from = 0usize;
    let mut search_to = lines.len();
    // The first segment must be the document root. Without this, a path of
    // ["Steam"] would match the nested `Steam` block, and a caller could reach
    // a key it did not mean to.
    let mut expect_depth: Option<usize> = Some(0);

    for segment in path {
        let mut found = None;
        for i in search_from..search_to {
            if block_key(lines[i]) != Some(*segment) {
                continue;
            }
            if expect_depth.is_some_and(|d| depth(lines[i]) != d) {
                continue;
            }
            // The next non-empty line must open a block.
            let mut j = i + 1;
            while j < search_to && lines[j].trim().is_empty() {
                j += 1;
            }
            if j < search_to && lines[j].trim() == "{" {
                found = Some((i, j));
                break;
            }
        }
        let (key_line, open) = found?;
        let block_depth = depth(lines[key_line]);
        // Walk to the matching close brace at the same depth.
        let mut close = None;
        for (i, line) in lines.iter().enumerate().take(search_to).skip(open + 1) {
            if line.trim() == "}" && depth(line) == block_depth {
                close = Some(i);
                break;
            }
        }
        let close = close?;
        search_from = open + 1;
        search_to = close;
        expect_depth = Some(block_depth + 1);
    }
    Some((search_from, search_to))
}

/// Read the launch options Steam has stored for `app_id`.
///
/// Returns `None` when the app has no entry; `Some("")` when it has an empty
/// one — a distinction that matters, because the second means Steam knows the
/// app and the first does not.
#[must_use]
pub fn launch_options(config: &Path, app_id: &str) -> Option<String> {
    let content = std::fs::read_to_string(config).ok()?;
    let lines: Vec<&str> = content.lines().collect();
    let mut path: Vec<&str> = APPS_PATH.to_vec();
    path.push(app_id);
    let (from, to) = find_block(&lines, &path)?;
    let app_depth = depth(lines[from]);
    lines[from..to].iter().find_map(|line| {
        (pair_key(line) == Some("LaunchOptions") && depth(line) == app_depth)
            .then(|| pair_value(line).unwrap_or_default().to_owned())
    })
}

/// Set the launch options Steam stores for `app_id`.
///
/// # Errors
/// Returns an error if Steam is running, if the app has no entry in this
/// account's configuration, or if the file cannot be written or verified.
pub fn set_launch_options(config: &Path, app_id: &str, value: &str) -> Result<()> {
    anyhow::ensure!(
        !is_running(),
        "Steam is running. It keeps localconfig.vdf in memory and rewrites it on \
         exit, so this edit would be discarded. Close Steam and try again."
    );
    write_launch_options(config, app_id, value)
}

/// Write the launch options without checking whether Steam is running.
///
/// Split out from [`set_launch_options`] so the file-editing logic can be
/// tested against a fixture regardless of what is running on the machine. The
/// public entry point keeps the guard: a caller that skipped it would have its
/// edit silently discarded when Steam next exits, which is worse than an error.
fn write_launch_options(config: &Path, app_id: &str, value: &str) -> Result<()> {
    // Steam's own format has no escaping for these, and a stray quote or
    // newline would corrupt the file for every game, not just this one.
    anyhow::ensure!(
        !value.contains('"') && !value.contains('\n') && !value.contains('\\'),
        "launch options may not contain quotes, backslashes or newlines"
    );

    let content =
        std::fs::read_to_string(config).with_context(|| format!("read {}", config.display()))?;
    let mut lines: Vec<String> = content.lines().map(str::to_owned).collect();

    let updated = {
        let borrowed: Vec<&str> = lines.iter().map(String::as_str).collect();
        let mut path: Vec<&str> = APPS_PATH.to_vec();
        path.push(app_id);
        let (from, to) = find_block(&borrowed, &path).with_context(|| {
            format!(
                "Steam has no entry for app {app_id} in {}",
                config.display()
            )
        })?;
        let app_depth = depth(borrowed[from]);
        let existing = (from..to).find(|i| {
            pair_key(borrowed[*i]) == Some("LaunchOptions") && depth(borrowed[*i]) == app_depth
        });
        let indent = "\t".repeat(app_depth);
        let line = format!("{indent}\"LaunchOptions\"\t\t\"{value}\"");
        if let Some(i) = existing {
            lines[i] = line;
            i
        } else {
            lines.insert(from, line);
            from
        }
    };
    tracing::debug!(target: "launch", app_id, line = updated, "rewrote LaunchOptions");

    // Keep a copy before touching the user's Steam configuration.
    let backup = config.with_extension("vdf.bigame-backup");
    std::fs::copy(config, &backup).with_context(|| format!("back up to {}", backup.display()))?;

    let mut out = lines.join("\n");
    if content.ends_with('\n') {
        out.push('\n');
    }
    write_atomic(config, out.as_bytes())?;

    // Read back rather than trusting the write.
    let readback = launch_options(config, app_id);
    anyhow::ensure!(
        readback.as_deref() == Some(value),
        "wrote launch options but the file reads back {readback:?}"
    );
    Ok(())
}

fn write_atomic(path: &Path, content: &[u8]) -> Result<()> {
    let dir = path.parent().context("path has no parent")?;
    let tmp = dir.join(format!(".localconfig.bigame.{}", std::process::id()));
    std::fs::write(&tmp, content).with_context(|| format!("write {}", tmp.display()))?;
    if let Err(e) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e).context("replace localconfig.vdf");
    }
    Ok(())
}

// ── Auditing what is already there ───────────────────────────────────────────

/// A launch-options string that names a program which is not installed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrokenLaunchOption {
    /// Steam `AppID`.
    pub app_id: String,
    /// The stored launch options.
    pub options: String,
    /// The missing program.
    pub missing: String,
}

/// Wrapper programs commonly found in launch options.
const WRAPPERS: &[&str] = &[
    "gamemoderun",
    "mangohud",
    "gamescope",
    "obs-gamecapture",
    "strangle",
];

/// Find launch options that invoke a program this system does not have.
///
/// A common case is `gamemoderun %command%` without Feral `GameMode`
/// installed: Steam runs the string through a shell, the wrapper is not found,
/// and the game does not start. Nothing in Steam's UI says why.
#[must_use]
pub fn broken_launch_options(config: &Path) -> Vec<BrokenLaunchOption> {
    let Ok(content) = std::fs::read_to_string(config) else {
        return Vec::new();
    };
    let lines: Vec<&str> = content.lines().collect();
    let Some((from, to)) = find_block(&lines, APPS_PATH) else {
        return Vec::new();
    };
    let app_depth = depth(lines[from]);

    let mut out = Vec::new();
    let mut current_app: Option<String> = None;
    for line in &lines[from..to] {
        if depth(line) == app_depth {
            if let Some(key) = block_key(line) {
                if key.chars().all(|c| c.is_ascii_digit()) {
                    current_app = Some(key.to_owned());
                }
            }
        }
        if depth(line) != app_depth + 1 || pair_key(line) != Some("LaunchOptions") {
            continue;
        }
        let Some(options) = pair_value(line) else {
            continue;
        };
        if options.trim().is_empty() {
            continue;
        }
        for wrapper in WRAPPERS {
            if !mentions_wrapper(options, wrapper) {
                continue;
            }
            if crate::capabilities::which(wrapper).is_some() {
                continue;
            }
            out.push(BrokenLaunchOption {
                app_id: current_app.clone().unwrap_or_default(),
                options: options.to_owned(),
                missing: (*wrapper).to_owned(),
            });
        }
    }
    out
}

/// Whether `options` invokes `wrapper` as a program rather than merely
/// containing the word (`MANGOHUD=1` is a variable, `mangohud %command%` is a
/// wrapper).
#[must_use]
pub fn mentions_wrapper(options: &str, wrapper: &str) -> bool {
    options
        .split_whitespace()
        .any(|token| token == wrapper || token.ends_with(&format!("/{wrapper}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Shaped like a real `localconfig.vdf`, including the `cloud` sub-block
    /// that also contains a `LaunchOptions` key.
    const VDF: &str = "\
\"UserLocalConfigStore\"
{
\t\"Software\"
\t{
\t\t\"Valve\"
\t\t{
\t\t\t\"Steam\"
\t\t\t{
\t\t\t\t\"apps\"
\t\t\t\t{
\t\t\t\t\t\"381210\"
\t\t\t\t\t{
\t\t\t\t\t\t\"LastPlayed\"\t\t\"1789802221\"
\t\t\t\t\t\t\"cloud\"
\t\t\t\t\t\t{
\t\t\t\t\t\t\t\"LaunchOptions\"\t\t\"gamemoderun %command%\"
\t\t\t\t\t\t}
\t\t\t\t\t\t\"LaunchOptions\"\t\t\"mangohud %command%\"
\t\t\t\t\t}
\t\t\t\t\t\"1808500\"
\t\t\t\t\t{
\t\t\t\t\t\t\"LastPlayed\"\t\t\"1789800000\"
\t\t\t\t\t}
\t\t\t\t}
\t\t\t}
\t\t}
\t}
}
";

    fn write_temp(name: &str, content: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "bigame_steam_{name}_{}_{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("localconfig.vdf");
        std::fs::write(&path, content).unwrap();
        path
    }

    #[test]
    fn reads_the_apps_own_launch_options_not_the_cloud_copy() {
        // Both keys are called LaunchOptions; only one is the app's.
        let path = write_temp("read", VDF);
        assert_eq!(
            launch_options(&path, "381210").as_deref(),
            Some("mangohud %command%")
        );
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn an_app_with_no_launch_options_reads_none() {
        let path = write_temp("none", VDF);
        assert_eq!(launch_options(&path, "1808500"), None);
        assert_eq!(launch_options(&path, "999999"), None);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn writing_replaces_only_the_apps_own_key() {
        let path = write_temp("write", VDF);
        write_launch_options(&path, "381210", "gamescope -f -- %command%").unwrap();

        assert_eq!(
            launch_options(&path, "381210").as_deref(),
            Some("gamescope -f -- %command%")
        );
        // The cloud copy is untouched, and the file is still well formed.
        let after = std::fs::read_to_string(&path).unwrap();
        assert!(after.contains("\t\t\t\t\t\t\t\"LaunchOptions\"\t\t\"gamemoderun %command%\""));
        assert_eq!(after.matches("\"LaunchOptions\"").count(), 2);
        assert_eq!(after.matches('{').count(), VDF.matches('{').count());
        assert_eq!(after.matches('}').count(), VDF.matches('}').count());
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn writing_inserts_the_key_when_the_app_has_none() {
        let path = write_temp("insert", VDF);
        write_launch_options(&path, "1808500", "mangohud %command%").unwrap();
        assert_eq!(
            launch_options(&path, "1808500").as_deref(),
            Some("mangohud %command%")
        );
        // And the other app is unaffected.
        assert_eq!(
            launch_options(&path, "381210").as_deref(),
            Some("mangohud %command%")
        );
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn writing_leaves_a_backup() {
        let path = write_temp("backup", VDF);
        write_launch_options(&path, "381210", "mangohud %command%").unwrap();
        let backup = path.with_extension("vdf.bigame-backup");
        assert!(backup.is_file());
        assert_eq!(std::fs::read_to_string(&backup).unwrap(), VDF);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn writing_refuses_an_unknown_app() {
        let path = write_temp("unknown", VDF);
        let err = write_launch_options(&path, "999999", "x %command%").unwrap_err();
        assert!(err.to_string().contains("no entry for app"));
        // Nothing was written, so no backup either.
        assert!(!path.with_extension("vdf.bigame-backup").exists());
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn writing_refuses_characters_that_would_corrupt_the_file() {
        let path = write_temp("quotes", VDF);
        for bad in ["say \"hi\" %command%", "a\nb", "back\\slash"] {
            assert!(
                write_launch_options(&path, "381210", bad).is_err(),
                "{bad:?}"
            );
        }
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn detects_wrappers_that_are_not_installed() {
        let path = write_temp("broken", VDF);
        let broken = broken_launch_options(&path);
        // The gamemoderun entry is inside `cloud`, so only the app-level key
        // is considered, and the result depends on what is installed: whatever
        // is reported must really be missing.
        for b in &broken {
            assert_eq!(b.app_id, "381210");
            assert!(crate::capabilities::which(&b.missing).is_none());
        }
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn a_wrapper_is_a_program_not_a_variable() {
        assert!(mentions_wrapper("gamemoderun %command%", "gamemoderun"));
        assert!(mentions_wrapper("/usr/bin/mangohud %command%", "mangohud"));
        assert!(mentions_wrapper(
            "mangohud gamemoderun %command%",
            "gamemoderun"
        ));

        // These set a variable or name a file; neither invokes the program.
        assert!(!mentions_wrapper("MANGOHUD=1 %command%", "mangohud"));
        assert!(!mentions_wrapper("MANGOHUD_CONFIG=x %command%", "mangohud"));
        assert!(!mentions_wrapper("", "gamemoderun"));
    }

    #[test]
    fn the_public_entry_point_refuses_while_steam_is_running() {
        // The guard is on set_launch_options, not on the writer, so these tests
        // do not depend on whether Steam happens to be open.
        let path = write_temp("guard", VDF);
        if is_running() {
            let err = set_launch_options(&path, "381210", "mangohud %command%").unwrap_err();
            assert!(err.to_string().contains("Steam is running"));
        } else {
            assert!(set_launch_options(&path, "381210", "mangohud %command%").is_ok());
        }
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn block_lookup_respects_nesting_depth() {
        let lines: Vec<&str> = VDF.lines().collect();
        assert!(find_block(&lines, APPS_PATH).is_some());
        // `Steam` exists, but nested — it is not a document root, so a path
        // starting there must not resolve.
        assert!(find_block(&lines, &["Steam"]).is_none());
        assert!(find_block(&lines, &["Software"]).is_none());
        assert!(find_block(&lines, &["UserLocalConfigStore", "nope"]).is_none());
        // And a partial prefix of the real path still resolves.
        assert!(find_block(&lines, &["UserLocalConfigStore", "Software"]).is_some());
    }
}
