//! `MangoHud` per game: off, on, or forced.
//!
//! `MangoHud` reaches a game in one of two ways, and they cover different games:
//!
//! * **On** — its implicit Vulkan layer, switched on by `MANGOHUD=1`. That is
//!   every Vulkan game, and every Proton game (DXVK and `VKD3D-Proton` are
//!   Vulkan).
//! * **Forced** — the `mangohud` wrapper, which also preloads it into `OpenGL`
//!   games, where the Vulkan layer never loads.
//!
//! A game Big Game Mode starts gets it through its launch plan (inside Gamescope,
//! as Gamescope's own `--mangoapp`). A game its own launcher starts runs in
//! that launcher's process tree, which Big Game Mode cannot reach, so the setting
//! goes where that launcher reads it:
//!
//! * **Steam** — the game's launch options, edited with Steam closed, backed
//!   up and read back (`steam::set_launch_options`). Options the user wrote
//!   stay; only the word this module added (recorded in Big Game Mode's state
//!   directory) is ever removed.
//! * **Heroic** — the game's `GamesConfig/<app>.json`, with the rest of the
//!   game's launch settings (`crate::heroic_launch`): Forced is Heroic's own
//!   `MangoHud` switch (`showMangohud`, its `mangohud --dlsym` wrapper), On
//!   is `MANGOHUD=1` in the game's environment variables, seeded from
//!   Heroic's defaults so the game keeps the others. What was written is
//!   recorded, and Off takes out exactly that. Heroic keeps a game's
//!   settings in memory and writes them back when one changes, so the file
//!   is written with Heroic closed.
//! * **Lutris** — the game's YAML: Lutris's own *FPS counter (`MangoHud`)*
//!   option, `system: mangohud: true`, for On and Forced alike.
//!
//! A launcher running as a Flatpak cannot see the system's `mangohud`: it
//! needs Flathub's `org.freedesktop.Platform.VulkanLayer.MangoHud` for its
//! runtime's branch, and Heroic refuses to start a game with its switch on
//! and no `mangohud` on its PATH. That is detected and said, with the
//! command; for the Flatpak Steam, where `mangohud %command%` would not
//! start the game, nothing is written until it is installed.

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::error::UserError;
use crate::text::N_;

/// A game's `MangoHud` setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    /// Not added by Big Game Mode (`MangoHud` may still come from elsewhere).
    #[default]
    Off,
    /// The Vulkan layer (`MANGOHUD=1`): Vulkan and Proton games.
    On,
    /// The `mangohud` wrapper: `OpenGL` games too.
    Forced,
}

/// What [`Mode::On`] adds in front of Steam's launch options.
const LAYER: &str = "MANGOHUD=1";
/// What [`Mode::Forced`] puts in front of `%command%`.
const WRAPPER: &str = "mangohud";

/// Steam launch options with `mode` applied to `current`, and the word added
/// for it, to record.
///
/// `added` is the word an earlier call added (`MANGOHUD=1`, or `mangohud`
/// right before `%command%`): it is taken out, and nothing else. A word the
/// user typed themselves is never removed, nor added a second time.
/// Everything else — other variables, other wrappers, the game's own
/// arguments — is kept in place. Launch options without `%command%` are the
/// game's arguments, and Steam appends them to the command; they are kept
/// after it.
#[must_use]
pub fn launch_options(
    current: &str,
    added: Option<&str>,
    mode: Mode,
) -> (String, Option<&'static str>) {
    use crate::steam::{COMMAND, is_command};
    let mut words = crate::steam::option_words(current);
    let command = |w: &[&str]| w.iter().position(|w| is_command(w));
    match added {
        Some(LAYER) => {
            if let Some(i) = words.iter().position(|w| *w == LAYER) {
                words.remove(i);
            }
        }
        Some(WRAPPER) => {
            if let Some(i) = command(&words).filter(|i| *i > 0 && words[i - 1] == WRAPPER) {
                words.remove(i - 1);
            }
        }
        _ => {}
    }
    let add = match mode {
        Mode::On if !words.contains(&LAYER) => Some(LAYER),
        Mode::Forced if !command(&words).is_some_and(|i| i > 0 && words[i - 1] == WRAPPER) => {
            Some(WRAPPER)
        }
        _ => None,
    };
    if let Some(word) = add {
        if command(&words).is_none() {
            // Plain arguments: they follow the command.
            words.insert(0, COMMAND);
        }
        let at = if word == LAYER {
            0
        } else {
            command(&words).unwrap_or(0)
        };
        words.insert(at, word);
    }
    let rest = words.join(" ");
    (if rest == COMMAND { String::new() } else { rest }, add)
}

/// Where the words added to Steam's launch options are recorded.
fn added_path() -> std::path::PathBuf {
    crate::paths::state_home().join("bigame-mode/mangohud/steam-added.toml")
}

/// The word recorded in `path` as added for `process`. With no record (an
/// older version wrote the options), the word for the mode it saved
/// (`saved`): it added that word whether or not the user had typed it.
fn read_added(path: &std::path::Path, process: &str, saved: Mode) -> Result<Option<String>> {
    let records = read_records(path)?;
    Ok(match records.get(process) {
        Some(word) => Some(word.clone()).filter(|w| !w.is_empty()),
        None => legacy_added(saved),
    })
}

fn read_records(path: &std::path::Path) -> Result<std::collections::BTreeMap<String, String>> {
    use anyhow::Context;
    match std::fs::read_to_string(path) {
        Ok(text) => toml::from_str(&text).with_context(|| format!("parse {}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(std::collections::BTreeMap::new()),
        Err(e) => Err(e).with_context(|| format!("read {}", path.display())),
    }
}

/// Record `word` as added for `process` in `path`.
fn write_added(path: &std::path::Path, process: &str, word: Option<&str>) -> Result<()> {
    use anyhow::Context;
    let mut records = read_records(path)?;
    records.insert(process.to_owned(), word.unwrap_or_default().to_owned());
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("create {}", dir.display()))?;
    }
    replace_file(path, toml::to_string(&records)?.as_bytes())
}

/// The word an older version added for a game saved at `mode`.
fn legacy_added(mode: Mode) -> Option<String> {
    match mode {
        Mode::Off => None,
        Mode::On => Some(LAYER.to_owned()),
        Mode::Forced => Some(WRAPPER.to_owned()),
    }
}

/// Where a `MangoHud` setting took effect.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Applied {
    /// In the game's settings in its launcher (Heroic, Lutris).
    Launcher {
        /// The launcher's name.
        name: &'static str,
        /// The launcher is a Flatpak without `MangoHud`'s Flatpak extension
        /// for its runtime: the command that installs it. Without it the
        /// overlay cannot load (and Heroic refuses to start the game with
        /// its switch on).
        missing_extension: Option<String>,
    },
    /// Nothing changed: this launcher is running and keeps the game's
    /// settings in memory, so it would overwrite the change.
    LauncherRunning(&'static str),
    /// In the launch plan of games Big Game Mode starts; the game is not a
    /// Steam game.
    LaunchPlan,
    /// In Steam's launch options, which now read as given (read back).
    SteamLaunchOptions(String),
    /// Nothing changed: Steam is running and holds its configuration in
    /// memory, so the launch options could not be written.
    SteamRunning,
}

/// Save `mode` for the game whose process is `process`, and write it where
/// the game will see it.
///
/// # Errors
/// Returns an error if the setting cannot be saved, Steam's configuration
/// cannot be written or verified, or the Flatpak Steam has no `MangoHud`.
pub fn apply(process: &str, mode: Mode) -> Result<Applied> {
    let games: Vec<crate::games::DetectedGame> = crate::games::detect_all()
        .into_iter()
        .filter(|g| g.profile_key() == process)
        .collect();
    if let Some(launcher) = games.iter().find_map(|g| g.launcher.clone()) {
        return apply_to_launcher(process, &launcher, mode);
    }
    let apps: Vec<String> = games
        .into_iter()
        .filter(|g| g.source == crate::games::Source::Steam)
        .filter_map(|g| g.app_id)
        .collect();
    // A Steam game whose launch options cannot be written now keeps its old
    // setting too: a saved choice that is not in effect would be a lie.
    if !apps.is_empty() && crate::steam::is_running() {
        return Ok(Applied::SteamRunning);
    }
    let users = crate::steam::users(&crate::paths::home_dir());
    if !apps.is_empty()
        && mode != Mode::Off
        && crate::steam::any_flatpak(&users)
        && let Some(command) = crate::steam::flatpak_mangohud_missing()
    {
        anyhow::bail!(UserError::with(
            N_(
                "Steam's Flatpak finds MangoHud only in Flathub's MangoHud extension, which is not installed. Nothing was written. Install it and restart Steam: %s"
            ),
            [command]
        ));
    }
    let mut settings = crate::game_settings::load(process)?;
    let record = added_path();
    let added = read_added(&record, process, settings.mangohud)?;
    // The launch options first, the saved choice after: if Steam's file cannot
    // be written, the choice stays as it was instead of claiming a mode the
    // game will not get.
    let mut last = String::new();
    let mut now_added = None;
    for user in &users {
        for app in &apps {
            let current = crate::steam::launch_options(&user.config, app).unwrap_or_default();
            let (wanted, add) = launch_options(&current, added.as_deref(), mode);
            if wanted != current {
                crate::steam::set_launch_options(&user.config, app, &wanted)?;
            }
            now_added = now_added.or(add);
            last = wanted;
        }
    }
    if !apps.is_empty() {
        write_added(&record, process, now_added)?;
    }
    settings.mangohud = mode;
    crate::game_settings::save(process, &settings)?;
    if apps.is_empty() {
        return Ok(Applied::LaunchPlan);
    }
    Ok(Applied::SteamLaunchOptions(last))
}

/// Write `mode` into the game's settings in its launcher, then save it.
fn apply_to_launcher(
    process: &str,
    launcher: &crate::games::LauncherRef,
    mode: Mode,
) -> Result<Applied> {
    use crate::games::LauncherRef;
    match launcher {
        LauncherRef::Heroic { .. } => {
            // Written with the game's other launch settings in Heroic, from
            // the saved choice; the choice goes back if it is not written.
            let mut settings = crate::game_settings::load(process)?;
            let before = settings.mangohud;
            settings.mangohud = mode;
            crate::game_settings::save(process, &settings)?;
            let applied = crate::optimization::apply_heroic(process);
            if !matches!(
                applied,
                Ok(crate::heroic_launch::Applied::Written
                    | crate::heroic_launch::Applied::Unchanged
                    | crate::heroic_launch::Applied::NotHeroic)
            ) {
                let mut settings = crate::game_settings::load(process)?;
                settings.mangohud = before;
                crate::game_settings::save(process, &settings)?;
            }
            if let crate::heroic_launch::Applied::HeroicRunning { .. } = applied? {
                return Ok(Applied::LauncherRunning("Heroic"));
            }
        }
        LauncherRef::Lutris { config_file } => {
            let current = std::fs::read_to_string(config_file)?;
            let wanted = lutris_config(&current, mode);
            if wanted != current {
                write_keeping_backup(config_file, &wanted)?;
            }
            let mut settings = crate::game_settings::load(process)?;
            settings.mangohud = mode;
            crate::game_settings::save(process, &settings)?;
        }
    }
    Ok(Applied::Launcher {
        name: launcher.launcher_name(),
        missing_extension: (mode != Mode::Off)
            .then(|| launcher.flatpak_id().and_then(missing_flatpak_extension))
            .flatten(),
    })
}

/// Replace `file` with `text`, atomically. The launcher's own version is kept
/// once, the first time Big Game Mode changes the file, under
/// `$XDG_STATE_HOME/bigame-mode/launcher-backups/` — never beside it, where
/// the launcher might read a stray file.
pub(crate) fn write_keeping_backup(file: &std::path::Path, text: &str) -> Result<()> {
    write_keeping_backup_in(
        file,
        text,
        &crate::paths::state_home().join("bigame-mode/launcher-backups"),
    )
}

/// [`write_keeping_backup`], keeping the launcher's version in `dir`.
pub(crate) fn write_keeping_backup_in(
    file: &std::path::Path,
    text: &str,
    dir: &std::path::Path,
) -> Result<()> {
    use anyhow::Context;
    if file.exists() {
        std::fs::create_dir_all(dir)?;
        let name = file
            .to_string_lossy()
            .trim_start_matches('/')
            .replace('/', "__");
        let bak = dir.join(name);
        if !bak.exists() {
            std::fs::copy(file, &bak).with_context(|| format!("back up {}", file.display()))?;
        }
    } else if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    replace_file(file, text.as_bytes())
}

/// Replace `path` with `content`: written whole to a temporary of its own
/// beside it, flushed, renamed over it and the folder flushed, so a crash
/// leaves the old file or the new one, never half of either, and two
/// writers never share a temporary. The file keeps its permissions, and a
/// dotfile manager's symlink (stow, chezmoi) stays a symlink: the file it
/// points to is the one replaced.
pub(crate) fn replace_file(path: &std::path::Path, content: &[u8]) -> Result<()> {
    use anyhow::Context;
    use std::io::Write as _;
    use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _};
    static SEQUENCE: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let dir = path.parent().context("path has no parent")?;
    let name = path
        .file_name()
        .context("path has no file name")?
        .to_string_lossy();
    let tmp = dir.join(format!(
        ".{name}.bigame-{}-{}.tmp",
        std::process::id(),
        SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    let mode = std::fs::metadata(&path)
        .ok()
        .map(|m| m.permissions().mode() & 0o777);
    let written = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .custom_flags(libc::O_NOFOLLOW)
        .mode(mode.unwrap_or(0o644))
        .open(&tmp)
        .and_then(|mut f| {
            // The umask narrowed the mode it was created with.
            if let Some(mode) = mode {
                f.set_permissions(std::fs::Permissions::from_mode(mode))?;
            }
            f.write_all(content)?;
            f.sync_all()
        });
    let renamed = written
        .with_context(|| format!("write {}", tmp.display()))
        .and_then(|()| {
            std::fs::rename(&tmp, &path).with_context(|| format!("replace {}", path.display()))
        });
    if let Err(e) = renamed {
        if let Err(cleanup) = std::fs::remove_file(&tmp)
            && cleanup.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!(path = %tmp.display(), error = %cleanup, "could not remove a temporary file");
        }
        return Err(e);
    }
    // The file is in place; only the rename's durability is at stake.
    if let Err(e) = std::fs::File::open(dir).and_then(|d| d.sync_all()) {
        tracing::warn!(path = %dir.display(), error = %e, "could not flush a folder");
    }
    Ok(())
}

/// A Lutris game YAML with `mode` applied to its `system:` section's
/// `mangohud` option. Only that line (and, when there was none, the section
/// header) changes; comments, order and every other key stay byte for byte.
#[must_use]
pub fn lutris_config(current: &str, mode: Mode) -> String {
    let lines: Vec<&str> = current.lines().collect();
    let header = lines
        .iter()
        .position(|l| *l == "system:" || l.starts_with("system: "));
    let on = mode != Mode::Off;
    let mut out: Vec<String> = lines.iter().map(|l| (*l).to_owned()).collect();
    match header {
        None if !on => return current.to_owned(),
        None => {
            out.push("system:".into());
            out.push("  mangohud: true".into());
        }
        Some(h) => {
            // `system: {}` or another flow value: make it a block.
            if out[h] != "system:" {
                out[h] = "system:".into();
            }
            let end = (h + 1..out.len())
                .find(|&i| !out[i].is_empty() && !out[i].starts_with(' '))
                .unwrap_or(out.len());
            let found = (h + 1..end).find(|&i| {
                let t = out[i].trim_start();
                out[i].starts_with("  ") && !out[i].starts_with("   ") && t.starts_with("mangohud:")
            });
            match (found, on) {
                (Some(i), true) => out[i] = "  mangohud: true".into(),
                (Some(i), false) => {
                    out.remove(i);
                    // A section left with nothing in it goes too.
                    if (h + 1..end - 1).all(|j| out[j].trim().is_empty()) {
                        out.remove(h);
                    }
                }
                (None, true) => out.insert(h + 1, "  mangohud: true".into()),
                (None, false) => {}
            }
        }
    }
    let mut text = out.join("\n");
    if current.ends_with('\n') || current.is_empty() {
        text.push('\n');
    }
    text
}

/// The command that installs `MangoHud`'s Flatpak extension for the runtime
/// of Flatpak app `app_id`, when it is missing; `None` when it is there, or
/// the app is not a Flatpak this machine has.
#[must_use]
pub fn missing_flatpak_extension(app_id: &str) -> Option<String> {
    missing_flatpak_layer(app_id, "org.freedesktop.Platform.VulkanLayer.MangoHud")
}

/// The command that installs Flatpak extension `ext` (one of the runtime's
/// `org.freedesktop.Platform.VulkanLayer.*`) for the runtime of Flatpak app
/// `app_id`, when it is missing; `None` when it is there, or the app is not
/// a Flatpak this machine has.
#[must_use]
pub fn missing_flatpak_layer(app_id: &str, ext: &str) -> Option<String> {
    let home = crate::paths::home_dir();
    let installs = [
        std::path::PathBuf::from("/var/lib/flatpak"),
        home.join(".local/share/flatpak"),
    ];
    let branch = installs.iter().find_map(|base| {
        let meta = std::fs::read_to_string(
            base.join("app")
                .join(app_id)
                .join("current/active/metadata"),
        )
        .ok()?;
        flatpak_runtime_branch(&meta)
    })?;
    let present = installs.iter().any(|base| {
        base.join("runtime")
            .join(ext)
            .join("x86_64")
            .join(&branch)
            .is_dir()
    });
    (!present).then(|| format!("flatpak install flathub {ext}//{branch}"))
}

/// The branch of the runtime a Flatpak app's `metadata` names:
/// `runtime=org.freedesktop.Platform/x86_64/25.08` → `25.08`.
fn flatpak_runtime_branch(metadata: &str) -> Option<String> {
    metadata
        .lines()
        .find_map(|l| l.strip_prefix("runtime="))
        .and_then(|r| r.rsplit('/').next())
        .filter(|b| !b.is_empty())
        .map(str::to_owned)
}

/// The game's `MangoHud` setting, as saved.
#[must_use]
pub fn mode_for(process: &str) -> Mode {
    crate::game_settings::load(process).map_or(Mode::Off, |s| s.mangohud)
}

// ── Overlay style ───────────────────────────────────────────────────────────

/// How the overlay looks, for every game that shows it.
///
/// `MangoHud` reads `$XDG_CONFIG_HOME/MangoHud/MangoHud.conf` (a per-game file
/// there, `wine-<game>.conf` or `<program>.conf`, takes precedence, and so
/// do `MANGOHUD_CONFIG`/`MANGOHUD_CONFIGFILE`). Big Game Mode writes that file
/// only when a style is chosen, marks it as its own, and keeps the user's
/// file to put back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Style {
    /// The user's own file, or `MangoHud`'s defaults when there is none.
    Own,
    /// One line across the top, as the Steam Deck's level 2: frame rate,
    /// frame times, CPU, GPU, memory and power.
    Basic,
    /// A column, as the Steam Deck's level 3: each processor with its load,
    /// temperature, clock and power, then memory and the frame times.
    Full,
}

/// The first line of a file Big Game Mode wrote.
const STYLE_MARKER: &str = "# Managed by BiGame-mode";

/// Options every style shares. Only options `MangoHud` 0.8.4 parses
/// (`overlay_params.h`); a key it does not know is ignored with a warning.
const STYLE_COMMON: &[&str] = &[
    "legacy_layout=0",
    "position=top-left",
    "background_alpha=0.5",
    "round_corners=8",
    "font_size=20",
    "text_outline",
];

const STYLE_BASIC: &[&str] = &[
    "horizontal",
    "hud_no_margin",
    "table_columns=20",
    "fps",
    "frame_timing",
    "cpu_stats",
    "cpu_power",
    "gpu_stats",
    "gpu_power",
    "ram",
    "vram",
    "battery",
];

const STYLE_FULL: &[&str] = &[
    "gpu_stats",
    "gpu_temp",
    "gpu_core_clock",
    "gpu_mem_clock",
    "gpu_power",
    "vram",
    "cpu_stats",
    "cpu_temp",
    "cpu_mhz",
    "cpu_power",
    "ram",
    "fps",
    "frametime",
    "frame_timing",
    "battery",
];

/// What this machine can show: `MangoHud` logs an error and leaves an empty
/// line for a metric it cannot read, so a style asks only for these.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Metrics {
    /// CPU power: RAPL readable by the user, or a `zenpower`/`amd_energy`
    /// sensor (RAPL is root-only on many systems).
    pub cpu_power: bool,
    /// A battery (a laptop or a handheld).
    pub battery: bool,
}

impl Metrics {
    /// Read them from sysfs.
    #[must_use]
    pub fn detect() -> Self {
        let rapl = std::fs::File::open("/sys/class/powercap/intel-rapl:0/energy_uj").is_ok();
        let sensor = std::fs::read_dir("/sys/class/hwmon").is_ok_and(|d| {
            d.flatten().any(|h| {
                std::fs::read_to_string(h.path().join("name"))
                    .is_ok_and(|n| matches!(n.trim(), "zenpower" | "amd_energy"))
            })
        });
        let battery = std::fs::read_dir("/sys/class/power_supply").is_ok_and(|d| {
            d.flatten()
                .any(|p| p.file_name().to_string_lossy().starts_with("BAT"))
        });
        Self {
            cpu_power: rapl || sensor,
            battery,
        }
    }
}

/// The file for `style` on this machine; `None` for [`Style::Own`].
#[must_use]
pub fn style_config(style: Style) -> Option<String> {
    style_config_for(style, Metrics::detect())
}

/// The file for `style` with the metrics `m` says exist.
#[must_use]
pub fn style_config_for(style: Style, m: Metrics) -> Option<String> {
    let (name, options) = match style {
        Style::Own => return None,
        Style::Basic => ("basic", STYLE_BASIC),
        Style::Full => ("full", STYLE_FULL),
    };
    let mut out = format!(
        "{STYLE_MARKER}: style {name}.\n\
         # Written by Big Game Mode (Tuning → Monitoring). Choose \"My own file\" there to\n\
         # put back the file that was here. Shift_R+F12 shows or hides the overlay.\n"
    );
    for o in STYLE_COMMON.iter().chain(options) {
        if (*o == "cpu_power" && !m.cpu_power) || (*o == "battery" && !m.battery) {
            continue;
        }
        out.push_str(o);
        out.push('\n');
    }
    Some(out)
}

/// `MangoHud`'s configuration file.
#[must_use]
pub fn style_path() -> std::path::PathBuf {
    crate::paths::config_home().join("MangoHud/MangoHud.conf")
}

/// Where the user's own file is kept while a style is in place.
fn style_backup() -> std::path::PathBuf {
    crate::paths::state_home().join("bigame-mode/mangohud/MangoHud.conf.user")
}

/// What `MangoHud.conf` holds now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StyleState {
    /// No file: `MangoHud`'s own defaults.
    Defaults,
    /// A file Big Game Mode did not write.
    Own,
    /// A style Big Game Mode wrote.
    Style(Style),
}

/// Read which style is in place.
#[must_use]
pub fn current_style() -> StyleState {
    style_state_of(config_text(&style_path()).as_deref())
}

fn style_state_of(text: Option<&str>) -> StyleState {
    let Some(text) = text else {
        return StyleState::Defaults;
    };
    let first = text.lines().next().unwrap_or_default();
    match first.strip_prefix(STYLE_MARKER) {
        Some(rest) if rest.contains("style full") => StyleState::Style(Style::Full),
        Some(rest) if rest.contains("style basic") => StyleState::Style(Style::Basic),
        _ => StyleState::Own,
    }
}

/// Put `style` in place. The first time, the user's own file (if any) is
/// moved aside; [`Style::Own`] puts it back, or removes Big Game Mode's file
/// when there was none. A file Big Game Mode did not write is never replaced
/// without being kept first.
///
/// # Errors
/// Returns an error if a file cannot be read, moved or written.
pub fn set_style(style: Style) -> Result<()> {
    set_style_at(style, &style_path(), &style_backup())
}

fn set_style_at(style: Style, path: &std::path::Path, backup: &std::path::Path) -> Result<()> {
    use anyhow::Context;
    let state = style_state_of(config_text(path).as_deref());
    match style_config(style) {
        None => {
            if !matches!(state, StyleState::Style(_)) {
                return Ok(()); // Already the user's own.
            }
            if !put_back(backup, path)? {
                std::fs::remove_file(path).with_context(|| format!("remove {}", path.display()))?;
            }
        }
        Some(text) => {
            if state == StyleState::Own {
                keep_aside(path, backup)?;
            }
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            replace_file(path, text.as_bytes())?;
        }
    }
    Ok(())
}

// ── A configuration file that may be the user's ─────────────────────────────

/// The text of the configuration file at `path`, to look for Big Game
/// Mode's marker in its first line; `None` only when nothing is there. A
/// file that is not UTF-8, cannot be read, or is a symlink to nothing is
/// still something the user put there: it reads as text without the
/// marker, so it is kept before anything replaces it.
pub(crate) fn config_text(path: &std::path::Path) -> Option<String> {
    match std::fs::read(path) {
        Ok(bytes) => Some(String::from_utf8_lossy(&bytes).into_owned()),
        Err(e)
            if e.kind() == std::io::ErrorKind::NotFound
                && std::fs::symlink_metadata(path).is_err() =>
        {
            None
        }
        Err(_) => Some(String::new()),
    }
}

/// Keep the user's file at `path` in `backup` before Big Game Mode's
/// replaces it: what it holds, byte for byte, or a symlink to nothing as
/// that symlink.
///
/// # Errors
/// Returns an error, and `path` must then not be replaced, when it cannot
/// be kept.
pub(crate) fn keep_aside(path: &std::path::Path, backup: &std::path::Path) -> Result<()> {
    use anyhow::Context;
    if let Some(dir) = backup.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("create {}", dir.display()))?;
    }
    // Copying onto a symlink kept earlier would write where it points.
    if std::fs::symlink_metadata(backup).is_ok_and(|m| m.file_type().is_symlink()) {
        std::fs::remove_file(backup).with_context(|| format!("remove {}", backup.display()))?;
    }
    let dangling = std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink())
        && std::fs::metadata(path).is_err();
    if dangling {
        let target =
            std::fs::read_link(path).with_context(|| format!("read link {}", path.display()))?;
        std::os::unix::fs::symlink(target, backup)
    } else {
        std::fs::copy(path, backup).map(drop)
    }
    .with_context(|| format!("keep {}", path.display()))
}

/// Put the file kept in `backup` back at `path`; whether one was kept. A
/// symlink at `path` stays: the file it points to is the one put back.
///
/// # Errors
/// Returns an error when the kept file cannot be moved back.
pub(crate) fn put_back(backup: &std::path::Path, path: &std::path::Path) -> Result<bool> {
    use anyhow::Context;
    if std::fs::symlink_metadata(backup).is_err() {
        return Ok(false);
    }
    let path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    std::fs::rename(backup, &path).with_context(|| format!("put back {}", path.display()))?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every option `MangoHud` 0.8.4 parses that a style uses
    /// (`src/overlay_params.h`, v0.8.4).
    const KNOWN_084: &[&str] = &[
        "legacy_layout",
        "position",
        "background_alpha",
        "round_corners",
        "font_size",
        "text_outline",
        "horizontal",
        "hud_no_margin",
        "table_columns",
        "fps",
        "frametime",
        "frame_timing",
        "cpu_stats",
        "cpu_temp",
        "cpu_mhz",
        "cpu_power",
        "gpu_stats",
        "gpu_temp",
        "gpu_core_clock",
        "gpu_mem_clock",
        "gpu_power",
        "ram",
        "vram",
        "battery",
    ];

    #[test]
    fn styles_use_only_options_mangohud_084_knows() {
        for style in [Style::Basic, Style::Full] {
            let text = style_config(style).unwrap();
            for line in text.lines().filter(|l| !l.starts_with('#')) {
                let key = line.split('=').next().unwrap();
                assert!(KNOWN_084.contains(&key), "{style:?}: {key}");
            }
        }
        assert_eq!(style_config(Style::Own), None);
    }

    #[test]
    fn a_style_asks_only_for_what_the_machine_can_show() {
        let desktop = Metrics {
            cpu_power: false,
            battery: false,
        };
        let text = style_config_for(Style::Full, desktop).unwrap();
        assert!(
            !text.lines().any(|l| l == "cpu_power" || l == "battery"),
            "{text}"
        );
        let laptop = Metrics {
            cpu_power: true,
            battery: true,
        };
        let text = style_config_for(Style::Basic, laptop).unwrap();
        assert!(text.lines().any(|l| l == "cpu_power") && text.lines().any(|l| l == "battery"));
    }

    #[test]
    fn a_style_is_recognised_by_its_first_line() {
        assert_eq!(style_state_of(None), StyleState::Defaults);
        assert_eq!(style_state_of(Some("fps\n")), StyleState::Own);
        for style in [Style::Basic, Style::Full] {
            let text = style_config(style).unwrap();
            assert_eq!(style_state_of(Some(&text)), StyleState::Style(style));
        }
    }

    #[test]
    fn the_users_own_file_is_kept_and_put_back() {
        let dir = crate::tests::tempdir("mangohud_style");
        let path = dir.join("MangoHud/MangoHud.conf");
        let backup = dir.join("state/MangoHud.conf.user");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "# Goverlay\nfps\n").unwrap();

        set_style_at(Style::Basic, &path, &backup).unwrap();
        assert!(
            std::fs::read_to_string(&path)
                .unwrap()
                .starts_with(STYLE_MARKER)
        );
        assert_eq!(
            std::fs::read_to_string(&backup).unwrap(),
            "# Goverlay\nfps\n"
        );
        // Switching between styles keeps the first backup.
        set_style_at(Style::Full, &path, &backup).unwrap();
        assert_eq!(
            std::fs::read_to_string(&backup).unwrap(),
            "# Goverlay\nfps\n"
        );

        set_style_at(Style::Own, &path, &backup).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "# Goverlay\nfps\n");
        assert!(!backup.exists());
        // Own again changes nothing.
        set_style_at(Style::Own, &path, &backup).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "# Goverlay\nfps\n");
    }

    #[test]
    fn with_no_file_before_own_removes_the_style() {
        let dir = crate::tests::tempdir("mangohud_style_none");
        let path = dir.join("MangoHud/MangoHud.conf");
        let backup = dir.join("state/MangoHud.conf.user");
        set_style_at(Style::Full, &path, &backup).unwrap();
        assert!(path.exists() && !backup.exists());
        set_style_at(Style::Own, &path, &backup).unwrap();
        assert!(!path.exists());
    }

    #[test]
    fn a_file_that_is_not_utf8_or_a_symlink_to_nothing_is_kept() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("MangoHud/MangoHud.conf");
        let backup = dir.path().join("state/MangoHud.conf.user");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();

        // Latin-1, as an old editor saved it.
        let latin1 = b"# r\xe9glages\nfps\n".to_vec();
        std::fs::write(&path, &latin1).unwrap();
        assert_eq!(
            style_state_of(config_text(&path).as_deref()),
            StyleState::Own
        );
        set_style_at(Style::Basic, &path, &backup).unwrap();
        assert_eq!(std::fs::read(&backup).unwrap(), latin1);
        set_style_at(Style::Own, &path, &backup).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), latin1);

        // A link whose file is gone (an unmounted dotfiles folder).
        std::fs::remove_file(&path).unwrap();
        let gone = dir.path().join("unmounted/MangoHud.conf");
        std::os::unix::fs::symlink(&gone, &path).unwrap();
        assert_eq!(
            style_state_of(config_text(&path).as_deref()),
            StyleState::Own
        );
        set_style_at(Style::Full, &path, &backup).unwrap();
        assert_eq!(std::fs::read_link(&backup).unwrap(), gone);
        set_style_at(Style::Own, &path, &backup).unwrap();
        assert_eq!(std::fs::read_link(&path).unwrap(), gone);
        assert!(std::fs::symlink_metadata(&backup).is_err());
    }

    #[test]
    fn a_dotfile_managers_symlink_stays_a_symlink() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("dotfiles/MangoHud.conf");
        std::fs::create_dir_all(real.parent().unwrap()).unwrap();
        std::fs::write(&real, "fps\n").unwrap();
        let path = dir.path().join("MangoHud/MangoHud.conf");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(&real, &path).unwrap();
        let backup = dir.path().join("state/MangoHud.conf.user");

        set_style_at(Style::Basic, &path, &backup).unwrap();
        assert_eq!(std::fs::read_link(&path).unwrap(), real);
        assert!(
            std::fs::read_to_string(&real)
                .unwrap()
                .starts_with(STYLE_MARKER)
        );
        set_style_at(Style::Own, &path, &backup).unwrap();
        assert_eq!(std::fs::read_link(&path).unwrap(), real);
        assert_eq!(std::fs::read_to_string(&real).unwrap(), "fps\n");
    }

    /// `launch_options` as a chain of saves: what it added is what the next
    /// call takes out.
    fn steam(current: &str, added: Option<&str>, mode: Mode) -> (String, Option<String>) {
        let (out, add) = launch_options(current, added, mode);
        (out, add.map(str::to_owned))
    }

    #[test]
    fn empty_launch_options_get_the_layer_or_the_wrapper() {
        assert_eq!(
            steam("", None, Mode::On),
            ("MANGOHUD=1 %command%".into(), Some(LAYER.into()))
        );
        assert_eq!(
            steam("", None, Mode::Forced),
            ("mangohud %command%".into(), Some(WRAPPER.into()))
        );
        assert_eq!(steam("", None, Mode::Off), (String::new(), None));
    }

    #[test]
    fn the_users_own_options_are_kept_in_place() {
        let mine = "PROTON_LOG=1 gamemoderun %command% -dx12";
        assert_eq!(
            steam(mine, None, Mode::On).0,
            "MANGOHUD=1 PROTON_LOG=1 gamemoderun %command% -dx12"
        );
        assert_eq!(
            steam(mine, None, Mode::Forced).0,
            "PROTON_LOG=1 gamemoderun mangohud %command% -dx12"
        );
        // Plain arguments follow the command.
        assert_eq!(
            steam("-novid", None, Mode::On).0,
            "MANGOHUD=1 %command% -novid"
        );
        // A quoted command is the command.
        assert_eq!(
            steam("\"%command%\" -name 'A  B'", None, Mode::Forced).0,
            "mangohud \"%command%\" -name 'A  B'"
        );
    }

    #[test]
    fn switching_mode_replaces_what_this_module_added_and_off_removes_it() {
        let (on, added) = steam("gamemoderun %command%", None, Mode::On);
        let (forced, added) = steam(&on, added.as_deref(), Mode::Forced);
        assert_eq!(forced, "gamemoderun mangohud %command%");
        let (off, added) = steam(&forced, added.as_deref(), Mode::Off);
        assert_eq!((off.as_str(), added), ("gamemoderun %command%", None));
        // Applying twice changes nothing.
        let (again, _) = steam(&on, Some(LAYER), Mode::On);
        assert_eq!(again, on);
    }

    #[test]
    fn what_the_user_typed_is_never_taken_out() {
        // Their own MANGOHUD=1: On adds nothing, so Off removes nothing.
        let mine = "MANGOHUD=1 %command%";
        let (on, added) = steam(mine, None, Mode::On);
        assert_eq!((on.as_str(), added.as_deref()), (mine, None));
        assert_eq!(steam(&on, added.as_deref(), Mode::Off).0, mine);
        // Their own wrapper, likewise.
        let mine = "mangohud %command%";
        let (forced, added) = steam(mine, None, Mode::Forced);
        assert_eq!(added, None);
        assert_eq!(steam(&forced, added.as_deref(), Mode::Off).0, mine);
        // An older version recorded nothing: the word for the saved mode was
        // its own.
        assert_eq!(legacy_added(Mode::On).as_deref(), Some(LAYER));
        assert_eq!(
            steam(mine, legacy_added(Mode::Forced).as_deref(), Mode::Off).0,
            ""
        );
    }

    #[test]
    fn the_record_of_added_words_reads_back() {
        let dir = crate::tests::tempdir("mangohud_added");
        let path = dir.join("state/steam-added.toml");
        // Nothing recorded: what an older version added for the saved mode.
        assert_eq!(
            read_added(&path, "Game.exe", Mode::Forced)
                .unwrap()
                .as_deref(),
            Some(WRAPPER)
        );
        write_added(&path, "Game.exe", Some(LAYER)).unwrap();
        write_added(&path, "Other.exe", None).unwrap();
        assert_eq!(
            read_added(&path, "Game.exe", Mode::Forced)
                .unwrap()
                .as_deref(),
            Some(LAYER)
        );
        assert_eq!(read_added(&path, "Other.exe", Mode::On).unwrap(), None);
        std::fs::write(&path, "not = [toml").unwrap();
        assert!(
            read_added(&path, "Game.exe", Mode::Off).is_err(),
            "a broken record is said"
        );
    }

    #[test]
    fn heroic_gets_its_own_switch_or_the_variable_and_off_takes_out_only_that() {
        use crate::heroic_launch::{Defaults, Wanted, Written, transform};
        let value = |t: &str| serde_json::from_str::<serde_json::Value>(t).unwrap();
        // A game with no variables of its own, and Heroic's defaults with
        // some: the game's list starts from them, so it keeps them.
        let file = r#"{
  "cb3bf": {
    "wineVersion": {"name": "Proton - GE-Proton-latest", "type": "proton"}
  },
  "version": "v0",
  "explicit": true
}"#;
        let defaults = Defaults {
            env: vec![serde_json::json!({"key": "DXVK_ASYNC", "value": "1"})],
            ..Defaults::default()
        };
        let on = Wanted {
            env: vec![("MANGOHUD".into(), "1".into())],
            ..Wanted::default()
        };
        let (text, written) =
            transform(file, "cb3bf", &Written::default(), &on, &defaults).unwrap();
        let v: serde_json::Value = serde_json::from_str(&text).unwrap();
        let env = v["cb3bf"]["enviromentOptions"].as_array().unwrap();
        assert!(env.iter().any(|e| e["key"] == "DXVK_ASYNC"), "{v}");
        assert!(
            env.iter()
                .any(|e| e["key"] == "MANGOHUD" && e["value"] == "1")
        );
        assert!(
            v["cb3bf"].get("showMangohud").is_none(),
            "Heroic's default stays"
        );
        // Off: back to the file as it was.
        let (off, _) = transform(&text, "cb3bf", &written, &Wanted::default(), &defaults).unwrap();
        assert_eq!(value(&off), value(file));

        let forced = Wanted {
            show_mangohud: true,
            ..Wanted::default()
        };
        let (text, written) =
            transform(file, "cb3bf", &Written::default(), &forced, &defaults).unwrap();
        let v: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(v["cb3bf"]["showMangohud"], true);
        assert!(v["cb3bf"].get("enviromentOptions").is_none());
        let (off, _) = transform(&text, "cb3bf", &written, &Wanted::default(), &defaults).unwrap();
        assert_eq!(value(&off), value(file));

        // The user's own MANGOHUD in the game's list is theirs: On writes
        // nothing over it, and Off leaves it.
        let mine = file.replace(
            "\"wineVersion\"",
            "\"enviromentOptions\": [{\"key\": \"MANGOHUD\", \"value\": \"1\"}],\n    \"wineVersion\"",
        );
        let (same, written) =
            transform(&mine, "cb3bf", &Written::default(), &on, &defaults).unwrap();
        assert!(written.is_empty());
        let (off, _) = transform(&same, "cb3bf", &written, &Wanted::default(), &defaults).unwrap();
        assert_eq!(value(&off), value(&mine));
    }

    #[test]
    fn lutris_gets_its_own_option_and_nothing_else_moves() {
        // As Lutris writes a game with no system options.
        let plain = "game:\n  exe: /games/Game/run.sh\nname: Game\nrunner: linux\n";
        let on = lutris_config(plain, Mode::Forced);
        assert_eq!(on, format!("{plain}system:\n  mangohud: true\n"));
        assert_eq!(
            lutris_config(&on, Mode::On),
            on,
            "On is Lutris's own option too"
        );
        assert_eq!(
            lutris_config(&on, Mode::Off),
            plain,
            "the section it added goes"
        );

        // An existing section keeps its other options.
        let with = "game:\n  exe: /g/x.exe\nsystem:\n  env:\n    LANG: C\n  mangohud: false\nwine:\n  version: ge\n";
        let on = lutris_config(with, Mode::On);
        assert!(
            on.contains("system:\n  env:\n    LANG: C\n  mangohud: true\nwine:"),
            "{on}"
        );
        let off = lutris_config(&on, Mode::Off);
        assert_eq!(
            off,
            "game:\n  exe: /g/x.exe\nsystem:\n  env:\n    LANG: C\nwine:\n  version: ge\n"
        );

        // A flow mapping becomes a block.
        assert_eq!(
            lutris_config("system: {}\n", Mode::On),
            "system:\n  mangohud: true\n"
        );
        // Off with nothing to remove changes nothing.
        assert_eq!(lutris_config(plain, Mode::Off), plain);
    }

    #[test]
    fn a_replaced_file_keeps_its_mode_its_symlink_and_leaves_no_temporary() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("dotfiles/game.yml");
        std::fs::create_dir_all(real.parent().unwrap()).unwrap();
        std::fs::write(&real, "old\n").unwrap();
        std::fs::set_permissions(&real, std::fs::Permissions::from_mode(0o600)).unwrap();
        let link = dir.path().join("game.yml");
        std::os::unix::fs::symlink(&real, &link).unwrap();

        replace_file(&link, b"new\n").unwrap();
        assert!(
            std::fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(std::fs::read_to_string(&real).unwrap(), "new\n");
        assert_eq!(
            std::fs::metadata(&real).unwrap().permissions().mode() & 0o777,
            0o600
        );
        // A file that was not there gets an ordinary one.
        replace_file(&dir.path().join("fresh.yml"), b"x").unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.path().join("fresh.yml")).unwrap(),
            "x"
        );

        // Writers at once each use a temporary of their own.
        std::thread::scope(|s| {
            for i in 0..8 {
                let real = &real;
                s.spawn(move || replace_file(real, format!("{i}\n").as_bytes()).unwrap());
            }
        });
        let last: u32 = std::fs::read_to_string(&real)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        assert!(last < 8);
        for d in [dir.path(), real.parent().unwrap()] {
            let names: Vec<String> = std::fs::read_dir(d)
                .unwrap()
                .flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect();
            assert!(!names.iter().any(|n| n.starts_with('.')), "{names:?}");
        }
    }

    #[test]
    fn the_launchers_file_is_kept_once_and_then_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("lutris/games/game.yml");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, "theirs\n").unwrap();
        let backups = dir.path().join("backups");
        write_keeping_backup_in(&file, "ours 1\n", &backups).unwrap();
        write_keeping_backup_in(&file, "ours 2\n", &backups).unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "ours 2\n");
        let kept: Vec<_> = std::fs::read_dir(&backups).unwrap().flatten().collect();
        assert_eq!(kept.len(), 1);
        assert_eq!(std::fs::read_to_string(kept[0].path()).unwrap(), "theirs\n");
        let names: Vec<String> = std::fs::read_dir(file.parent().unwrap())
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["game.yml"]);
    }

    #[test]
    fn the_runtime_branch_is_read_from_the_flatpak_metadata() {
        let meta = "[Application]\nname=com.heroicgameslauncher.hgl\nruntime=org.freedesktop.Platform/x86_64/25.08\nsdk=org.freedesktop.Sdk/x86_64/25.08\n";
        assert_eq!(flatpak_runtime_branch(meta).as_deref(), Some("25.08"));
        assert_eq!(flatpak_runtime_branch("[Application]\n"), None);
    }
}
