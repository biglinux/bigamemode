//! Steam per-game launch options: reading them, and clearing them.
//!
//! Launch options are the string Steam runs a game with, `%command%` standing
//! for the game's own command line. Diagnostics reads them to find options
//! that call a program which is not installed — a leftover that makes the game
//! fail to start — and offers to clear them.
//!
//! Editing Steam's configuration is delicate and this module is built around
//! that:
//!
//! * **Steam must not be running.** It holds `localconfig.vdf` in memory and
//!   rewrites it on exit, so an edit made underneath a running client is simply
//!   discarded — silently, which is worse than failing.
//! * **A backup is written first**, beside the original, once: it stays the
//!   file as it was before Big Game Mode's first change.
//! * **Only the one key is touched**, located by its full path rather than by
//!   name. `LaunchOptions` appears at several nesting depths in a real file —
//!   including inside `cloud` blocks — and editing the wrong one does nothing.
//! * **The result is read back** and compared.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::error::UserError;
use crate::text::N_;

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

impl SteamUser {
    /// Whether the account is the Flatpak Steam's
    /// (`~/.var/app/com.valvesoftware.Steam`), whose games run in its
    /// sandbox and see only what its Flatpak extensions bring.
    #[must_use]
    pub fn flatpak(&self) -> bool {
        self.config
            .to_string_lossy()
            .contains(&format!("/.var/app/{FLATPAK}/"))
    }
}

/// The Flatpak Steam's application id.
pub const FLATPAK: &str = "com.valvesoftware.Steam";

/// The Flatpak Steam's own Gamescope extension, which Flathub keeps for
/// older installs (`org.freedesktop.Platform.VulkanLayer.gamescope` replaced
/// it).
const FLATPAK_GAMESCOPE_UTILITY: &str = "com.valvesoftware.Steam.Utility.gamescope";

/// Whether any of `users` is the Flatpak Steam's.
#[must_use]
pub fn any_flatpak(users: &[SteamUser]) -> bool {
    users.iter().any(SteamUser::flatpak)
}

/// Whether every one of `users` is the Flatpak Steam's (and there is one).
#[must_use]
pub fn only_flatpak(users: &[SteamUser]) -> bool {
    !users.is_empty() && users.iter().all(SteamUser::flatpak)
}

/// The command that installs Gamescope for the Flatpak Steam, when it has
/// none: without it `gamescope` in a game's launch options is not found in
/// the sandbox, and the game does not start.
#[must_use]
pub fn flatpak_gamescope_missing() -> Option<String> {
    let missing =
        crate::mangohud::missing_flatpak_layer(FLATPAK, crate::heroic_launch::GAMESCOPE_LAYER)?;
    let home = crate::paths::home_dir();
    let utility = [
        PathBuf::from("/var/lib/flatpak"),
        home.join(".local/share/flatpak"),
    ]
    .iter()
    .any(|base| {
        base.join("runtime")
            .join(FLATPAK_GAMESCOPE_UTILITY)
            .is_dir()
    });
    (!utility).then_some(missing)
}

/// The command that installs `MangoHud` for the Flatpak Steam, when it has
/// none: without it the overlay cannot load, and `mangohud %command%` is not
/// found in the sandbox.
#[must_use]
pub fn flatpak_mangohud_missing() -> Option<String> {
    crate::mangohud::missing_flatpak_extension(FLATPAK)
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
    // One account signed in to both the native Steam and the Flatpak has a
    // `userdata/<id>` in each, and each client reads only its own: they are
    // two configurations to write, told apart by their file, not by the id.
    out.sort_by(|a, b| a.id.cmp(&b.id).then_with(|| a.config.cmp(&b.config)));
    out.dedup_by(|a, b| a.config == b.config);
    out
}

/// Whether a Steam client is currently running.
///
/// Editing the configuration while it is would be discarded on exit. Only
/// this user's client counts: another user's Steam keeps another
/// configuration, and cannot be closed from here.
#[must_use]
pub fn is_running() -> bool {
    // SAFETY: getuid cannot fail and has no side effects.
    running_in(std::path::Path::new("/proc"), unsafe { libc::getuid() })
}

/// Whether a process of `uid` in the `/proc`-like tree at `root` is called
/// `steam`.
fn running_in(root: &Path, uid: u32) -> bool {
    use std::os::unix::fs::MetadataExt;
    let Ok(entries) = std::fs::read_dir(root) else {
        return false;
    };
    entries.flatten().any(|entry| {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.is_empty() || !name.chars().all(|c| c.is_ascii_digit()) {
            return false;
        }
        entry.metadata().is_ok_and(|m| m.uid() == uid)
            && std::fs::read_to_string(entry.path().join("comm"))
                .is_ok_and(|comm| comm.trim() == "steam")
    })
}

/// Close the Steam client and open it again in the session's current
/// environment, so the games it starts get what was set there since it
/// started (a Turbo preset, Wine FSR).
///
/// # Errors
/// Returns an error when Steam does not close within a minute or cannot be
/// started again.
pub fn restart_in_session() -> anyhow::Result<()> {
    // `while_closed` opens it again itself when it was open; asking a second
    // time, before the new client is up, started a second unit.
    if is_running() {
        while_closed(|| ())
    } else {
        start_in_session(installed_flatpak_only())
    }
}

/// Whether the Steam that runs now is the Flatpak: its processes see the
/// sandbox's root, which has `/.flatpak-info`.
fn running_flatpak() -> bool {
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return false;
    };
    entries.flatten().any(|entry| {
        std::fs::read_to_string(entry.path().join("comm")).is_ok_and(|c| c.trim() == "steam")
            && entry.path().join("root/.flatpak-info").exists()
    })
}

/// Whether the only Steam installed is the Flatpak.
fn installed_flatpak_only() -> bool {
    crate::capabilities::which("steam").is_none()
        && crate::capabilities::which("flatpak").is_some()
        && [
            PathBuf::from("/var/lib/flatpak"),
            crate::paths::home_dir().join(".local/share/flatpak"),
        ]
        .iter()
        .any(|root| root.join("app").join(FLATPAK).join("current").exists())
}

/// The command that runs the Steam client (`flatpak` for the Flatpak),
/// then `extra`.
fn client_command(flatpak: bool, extra: &[&str]) -> Vec<String> {
    let mut argv: Vec<String> = if flatpak {
        vec!["flatpak".into(), "run".into(), FLATPAK.into()]
    } else {
        vec!["steam".into()]
    };
    argv.extend(extra.iter().map(|s| (*s).to_owned()));
    argv
}

/// Start the Steam client as a unit of the user's systemd manager, under a
/// name no other start can have taken ([`crate::launchers`]).
fn start_in_session(flatpak: bool) -> anyhow::Result<()> {
    crate::launchers::start_in_session("steam", &client_command(flatpak, &[]))
}

/// Run `f` with the Steam client closed — it keeps its configuration in
/// memory and writes it back on exit, so a launch option changed while it
/// runs is lost — and open Steam again afterwards if it was open.
///
/// Closed the way Steam closes itself (`steam -shutdown`, through
/// `flatpak run` for the Flatpak), which lets it save its state; opened as
/// a unit of the user's systemd manager, which is how the desktop's menu
/// starts it (KDE Plasma: `app-…@.service`), so it inherits the manager's
/// environment rather than Big Game Mode's own. The Steam that was open is
/// the one opened again.
///
/// # Errors
/// Returns an error when a Steam game is running, when Steam does not close
/// within a minute or cannot be started again; `f` has not run in the first
/// two cases.
pub fn while_closed<T>(f: impl FnOnce() -> T) -> anyhow::Result<T> {
    use anyhow::Context;
    let was_open = is_running();
    let flatpak = was_open && running_flatpak();
    if was_open {
        // Closing the client would close a game it runs.
        crate::launchers::ensure_launcher_idle(crate::launchers::Launcher::Steam)?;
        let argv = client_command(flatpak, &["-shutdown"]);
        std::process::Command::new(&argv[0])
            .args(&argv[1..])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .context("ask Steam to close")?;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        while is_running() {
            anyhow::ensure!(
                std::time::Instant::now() < deadline,
                UserError::plain(N_("Steam did not close within a minute"))
            );
            std::thread::sleep(std::time::Duration::from_millis(500));
        }
    }
    let result = f();
    if was_open {
        start_in_session(flatpak)?;
    }
    Ok(result)
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
            .then(|| unescape(pair_value(line).unwrap_or_default()))
    })
}

/// A value as Steam's text format stores it, read: `\"`, `\\`, `\n` and
/// `\t` are its escapes; any other backslash is kept as it is.
fn unescape(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some(c @ ('"' | '\\')) => out.push(c),
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

/// `value` in Steam's text format: the four characters it escapes, escaped.
fn escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out
}

/// Set the launch options Steam stores for `app_id`.
///
/// # Errors
/// Returns an error if Steam is running, if the account's configuration has
/// no apps section, or if the file cannot be written or verified. An app
/// without a block of its own gets one.
pub fn set_launch_options(config: &Path, app_id: &str, value: &str) -> Result<()> {
    anyhow::ensure!(
        !is_running(),
        UserError::plain(N_(
            "Steam is running. It keeps localconfig.vdf in memory and rewrites it on \
             exit, so this edit would be discarded. Close Steam and try again."
        ))
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
    // The id becomes a key of the file: anything but digits (from a crafted
    // appmanifest in a shared library) could write a block for another game.
    anyhow::ensure!(
        !app_id.is_empty() && app_id.bytes().all(|b| b.is_ascii_digit()),
        UserError::with(N_("not a Steam app id: %s"), [app_id])
    );
    // Written in Steam's escapes: a bare quote or newline would corrupt the
    // file for every game, not just this one.
    let stored = escape(value);
    let content =
        std::fs::read_to_string(config).with_context(|| format!("read {}", config.display()))?;
    let mut lines: Vec<String> = content.lines().map(str::to_owned).collect();

    let updated = {
        let borrowed: Vec<&str> = lines.iter().map(String::as_str).collect();
        let mut path: Vec<&str> = APPS_PATH.to_vec();
        path.push(app_id);
        let Some((from, to)) = find_block(&borrowed, &path) else {
            // Steam keeps a block only for apps with settings of their own; a
            // game played with defaults has none. Nothing stored is already
            // "no launch options"; anything else gets a new block, at the end
            // of `apps`, in Steam's own layout.
            if value.is_empty() {
                return Ok(());
            }
            let (apps_from, apps_to) = find_block(&borrowed, APPS_PATH).ok_or_else(|| {
                UserError::with(
                    N_("Steam's configuration has no apps section: %s"),
                    [config.display().to_string()],
                )
            })?;
            let child = "\t".repeat(depth(borrowed[apps_from.saturating_sub(1)]) + 1);
            let block = [
                format!("{child}\"{app_id}\""),
                format!("{child}{{"),
                format!("{child}\t\"LaunchOptions\"\t\t\"{stored}\""),
                format!("{child}}}"),
            ];
            drop(borrowed);
            for (k, l) in block.into_iter().enumerate() {
                lines.insert(apps_to + k, l);
            }
            return finish_write(config, &content, &lines, app_id, value);
        };
        // An empty block's first line is its own closing brace, one level
        // out from the keys that belong inside.
        let app_depth = depth(borrowed[from]) + usize::from(from == to);
        let existing = (from..to).find(|i| {
            pair_key(borrowed[*i]) == Some("LaunchOptions") && depth(borrowed[*i]) == app_depth
        });
        let indent = "\t".repeat(app_depth);
        let line = format!("{indent}\"LaunchOptions\"\t\t\"{stored}\"");
        if let Some(i) = existing {
            lines[i] = line;
            i
        } else {
            lines.insert(from, line);
            from
        }
    };
    tracing::debug!(target: "launch", app_id, line = updated, "rewrote LaunchOptions");
    finish_write(config, &content, &lines, app_id, value)
}

/// Back up, write atomically and read back.
fn finish_write(
    config: &Path,
    content: &str,
    lines: &[String],
    app_id: &str,
    value: &str,
) -> Result<()> {
    // Keep a copy of the user's Steam configuration before Big Game Mode first
    // touches it; a later write must not replace it with its own.
    let backup = config.with_extension("vdf.bigame-backup");
    if !backup.exists() {
        std::fs::copy(config, &backup)
            .with_context(|| format!("back up to {}", backup.display()))?;
    }

    let mut out = lines.join("\n");
    if content.ends_with('\n') {
        out.push('\n');
    }
    write_atomic(config, out.as_bytes())?;

    // Read back rather than trusting the write.
    let readback = launch_options(config, app_id);
    anyhow::ensure!(
        readback.as_deref() == Some(value),
        UserError::with(
            N_("wrote launch options but the file reads back %s"),
            [format!("{readback:?}")]
        )
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

// ── Launch options as words ──────────────────────────────────────────────────

/// The word Steam replaces with the game's command.
pub const COMMAND: &str = "%command%";

/// The words of launch options as the shell Steam runs them with reads
/// them: split at blanks outside quotes, each word kept as typed (its quotes
/// and inner spaces included), so the words joined again are the same text.
#[must_use]
pub fn option_words(options: &str) -> Vec<&str> {
    let mut words = Vec::new();
    let mut start = None;
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for (i, c) in options.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match (quote, c) {
            (Some('\''), '\'') | (Some('"'), '"') => quote = None,
            (Some('\''), _) => {}
            (_, '\\') => {
                escaped = true;
                start.get_or_insert(i);
            }
            (Some(_), _) => {}
            (None, '"' | '\'') => {
                quote = Some(c);
                start.get_or_insert(i);
            }
            (None, c) if c.is_whitespace() => {
                if let Some(s) = start.take() {
                    words.push(&options[s..i]);
                }
            }
            (None, _) => {
                start.get_or_insert(i);
            }
        }
    }
    if let Some(s) = start {
        words.push(&options[s..]);
    }
    words
}

/// Whether `word` is where Steam puts the game's command: `%command%`,
/// quoted or not.
#[must_use]
pub fn is_command(word: &str) -> bool {
    word.contains(COMMAND)
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
        if depth(line) == app_depth
            && let Some(key) = block_key(line)
            && key.chars().all(|c| c.is_ascii_digit())
        {
            current_app = Some(key.to_owned());
        }
        if depth(line) != app_depth + 1 || pair_key(line) != Some("LaunchOptions") {
            continue;
        }
        let Some(options) = pair_value(line).map(unescape) else {
            continue;
        };
        if options.trim().is_empty() {
            continue;
        }
        for wrapper in WRAPPERS {
            if !mentions_wrapper(&options, wrapper) {
                continue;
            }
            if crate::capabilities::which(wrapper).is_some() {
                continue;
            }
            out.push(BrokenLaunchOption {
                app_id: current_app.clone().unwrap_or_default(),
                options: options.clone(),
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

    #[test]
    fn an_app_id_that_is_not_a_number_is_refused_and_nothing_written() {
        let path = write_temp("bad-id", VDF);
        for bad in ["", "381210\"\n\"1", "1a", "-1"] {
            assert!(write_launch_options(&path, bad, "mangohud %command%").is_err());
        }
        assert_eq!(std::fs::read_to_string(&path).unwrap(), VDF);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

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
    fn only_this_users_steam_counts_as_running() {
        use std::os::unix::fs::MetadataExt;
        let root = std::env::temp_dir().join(format!("bigame_steam_proc_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("4242")).unwrap();
        std::fs::write(root.join("4242/comm"), "steam\n").unwrap();
        std::fs::create_dir_all(root.join("self")).unwrap();
        let owner = std::fs::metadata(root.join("4242")).unwrap().uid();
        assert!(running_in(&root, owner));
        // The same process seen by another user is that user's Steam.
        assert!(!running_in(&root, owner + 1));
        std::fs::write(root.join("4242/comm"), "steamwebhelper\n").unwrap();
        assert!(!running_in(&root, owner));
        let _ = std::fs::remove_dir_all(&root);
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
    fn an_empty_app_block_gets_the_key_inside_it() {
        let vdf = VDF.replace(
            "\t\t\t\t\t\"1808500\"\n\t\t\t\t\t{\n\t\t\t\t\t\t\"LastPlayed\"\t\t\"1789800000\"\n",
            "\t\t\t\t\t\"1808500\"\n\t\t\t\t\t{\n",
        );
        assert_ne!(vdf, VDF);
        let path = write_temp("empty-block", &vdf);
        write_launch_options(&path, "1808500", "MANGOHUD=1 %command%").unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(
            text.contains(
                "\t\t\t\t\t\"1808500\"\n\t\t\t\t\t{\n\t\t\t\t\t\t\"LaunchOptions\"\t\t\"MANGOHUD=1 %command%\"\n\t\t\t\t\t}\n"
            ),
            "{text}"
        );
        assert_eq!(
            launch_options(&path, "1808500").as_deref(),
            Some("MANGOHUD=1 %command%")
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
    fn an_app_without_a_block_gets_one_in_steams_layout() {
        // Steam keeps no block for a game played with default settings (the
        // reference desktop had none for Shadow of the Tomb Raider).
        let path = write_temp("unknown", VDF);
        write_launch_options(&path, "999999", "MANGOHUD=1 %command%").unwrap();
        assert_eq!(
            launch_options(&path, "999999").as_deref(),
            Some("MANGOHUD=1 %command%")
        );
        // The other apps are untouched.
        let before = VDF.lines().filter(|l| !l.trim().is_empty()).count();
        let after_text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(
            after_text.lines().filter(|l| !l.trim().is_empty()).count(),
            before + 4
        );
        assert!(after_text.contains("\t\t\t\t\t\"999999\"\n\t\t\t\t\t{\n"));
        assert!(path.with_extension("vdf.bigame-backup").exists());
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn clearing_the_options_of_an_app_without_a_block_changes_nothing() {
        let path = write_temp("unknown-clear", VDF);
        write_launch_options(&path, "999999", "").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), VDF);
        assert!(!path.with_extension("vdf.bigame-backup").exists());
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn quotes_and_backslashes_are_written_in_steams_escapes_and_read_back() {
        // Steam stores `-name "My Game"` as `-name \"My Game\"`.
        let path = write_temp("quotes", VDF);
        for value in [
            "say \"hi\" %command%",
            "%command% -path C:\\Games",
            "a\nb",
            "PROTON_LOG=1 %command% -name \"two  spaces\"",
        ] {
            write_launch_options(&path, "381210", value).unwrap();
            assert_eq!(launch_options(&path, "381210").as_deref(), Some(value));
        }
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(
            text.contains(
                "\"LaunchOptions\"\t\t\"PROTON_LOG=1 %command% -name \\\"two  spaces\\\"\""
            ),
            "{text}"
        );
        // Still one line per key, and the braces balance.
        assert_eq!(text.matches('{').count(), VDF.matches('{').count());
        assert_eq!(text.lines().count(), VDF.lines().count());
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn options_steam_wrote_with_escapes_are_read_as_the_user_typed_them() {
        let vdf = VDF.replace(
            "\"mangohud %command%\"",
            "\"%command% -name \\\"A B\\\" -dir C:\\\\x\"",
        );
        let path = write_temp("escaped", &vdf);
        assert_eq!(
            launch_options(&path, "381210").as_deref(),
            Some("%command% -name \"A B\" -dir C:\\x")
        );
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn launch_options_split_into_words_as_the_shell_reads_them() {
        assert_eq!(
            option_words("A=1  gamemoderun \"%command%\" -name 'two  spaces' x\\ y"),
            [
                "A=1",
                "gamemoderun",
                "\"%command%\"",
                "-name",
                "'two  spaces'",
                "x\\ y"
            ]
        );
        assert!(is_command("\"%command%\"") && !is_command("-novid"));
        assert!(option_words("   ").is_empty());
    }

    #[test]
    fn the_backup_stays_the_file_before_the_first_change() {
        let path = write_temp("backup_once", VDF);
        write_launch_options(&path, "381210", "MANGOHUD=1 %command%").unwrap();
        write_launch_options(&path, "381210", "gamescope -f -- %command%").unwrap();
        let backup = path.with_extension("vdf.bigame-backup");
        assert_eq!(std::fs::read_to_string(&backup).unwrap(), VDF);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn an_account_of_the_flatpak_steam_is_told_apart() {
        let user = |config: &str| SteamUser {
            id: "1".into(),
            config: PathBuf::from(config),
        };
        let flatpak = user(
            "/home/u/.var/app/com.valvesoftware.Steam/.local/share/Steam/userdata/1/config/localconfig.vdf",
        );
        let native = user("/home/u/.local/share/Steam/userdata/1/config/localconfig.vdf");
        assert!(flatpak.flatpak() && !native.flatpak());
        assert!(any_flatpak(&[native.clone(), flatpak.clone()]));
        assert!(!only_flatpak(&[native.clone(), flatpak.clone()]));
        assert!(only_flatpak(std::slice::from_ref(&flatpak)));
        assert!(!only_flatpak(&[]));
        assert_eq!(
            client_command(true, &["-shutdown"]),
            ["flatpak", "run", "com.valvesoftware.Steam", "-shutdown"]
        );
        assert_eq!(
            client_command(false, &["-shutdown"]),
            ["steam", "-shutdown"]
        );
    }

    #[test]
    fn one_account_in_the_native_and_the_flatpak_steam_is_two_configurations() {
        let home = tempfile::tempdir().unwrap();
        let account = |root: &str| {
            let config = home
                .path()
                .join(root)
                .join("userdata/1234/config/localconfig.vdf");
            std::fs::create_dir_all(home.path().join(root).join("steamapps")).unwrap();
            std::fs::create_dir_all(config.parent().unwrap()).unwrap();
            std::fs::write(&config, VDF).unwrap();
        };
        account(".local/share/Steam");
        account(".var/app/com.valvesoftware.Steam/.local/share/Steam");
        let found = users(home.path());
        assert_eq!(found.len(), 2, "{found:?}");
        assert!(found.iter().all(|u| u.id == "1234"));
        assert_eq!(found.iter().filter(|u| u.flatpak()).count(), 1);
        // `~/.steam/steam`, a link to the native install, is not a third.
        std::fs::create_dir_all(home.path().join(".steam")).unwrap();
        std::os::unix::fs::symlink(
            home.path().join(".local/share/Steam"),
            home.path().join(".steam/steam"),
        )
        .unwrap();
        assert_eq!(users(home.path()).len(), 2);
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
