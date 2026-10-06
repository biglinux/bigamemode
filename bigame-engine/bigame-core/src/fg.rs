//! Frame Generation (lsfg-vk) config management.
//!
//! lsfg-vk is a Vulkan implicit layer — NOT a kernel module. It reads
//! `$XDG_CONFIG_HOME/lsfg-vk/conf.toml` and reloads it while a game runs
//! ("Failed to update configuration, continuing using old" when a new version
//! does not parse) — for a game that started with an entry, and then only to
//! apply new values: a new multiplier takes effect live, but removing the
//! entry does not stop generating. A game that started while the file had no
//! entry for it, or did not parse, runs without frame generation until its
//! next start. So on and off take effect at the next start (checked with
//! Shadow of the Tomb Raider on the reference desktop: x2 → removed stayed
//! at x2's cost, → x3 applied). Big Game Mode writes per-game entries there.
//!
//! Two formats exist and the installed layer reads exactly one, so the format
//! is read from the library itself ([`installed`]), never assumed. lsfg-vk
//! 1.x (`liblsfg-vk.so`, checked against its strings):
//!
//! ```toml
//! version = 1
//! [global]
//! dll = "/path/to/Lossless.dll"
//! [[game]]
//! exe = "Game.exe"
//! multiplier = 3            # at least 2
//! flow_scale = 0.7          # 0.25–1.0
//! performance_mode = true
//! hdr_mode = false
//! experimental_present_mode = "fifo"   # or "mailbox", "immediate"
//! ```
//!
//! lsfg-vk 2.x (`liblsfg-vk-layer.so`, checked with its own
//! `lsfg-vk-cli validate`):
//!
//! ```toml
//! version = 2
//! [global]                  # required, even when empty
//! dll = "/path/to/Lossless.dll"
//! [[profile]]               # at least one
//! name = "Game.exe"
//! active_in = ["Game.exe"]  # Wine executable, process name or Steam app id
//! multiplier = 3
//! flow_scale = 0.7
//! performance_mode = true
//! override_present_mode = false   # absent or true: FIFO
//! ```
//!
//! What the two parsers share shapes everything here:
//!
//! * One invalid value — and, in 2.x, one key it does not know — makes
//!   lsfg-vk ignore the **whole** file, so a game with frame generation off
//!   has **no** entry, never `multiplier = 1`, and a 2.x file carries only
//!   the keys 2.x reads.
//! * 2.x rejects a file without a profile: with no game left, a placeholder
//!   profile that matches nothing keeps it valid ([`PLACEHOLDER`]).
//! * HDR mode and the choice among four present modes exist only in 1.x;
//!   2.x presents with FIFO unless told to keep the game's own mode
//!   ([`per_game_hdr_and_present_modes`]).
//! * A file in the other format — an earlier Big Game Mode's `[[profile]]`
//!   layout under 1.x, a 1.x file after the package moved to 2.x — makes
//!   lsfg-vk ignore it entirely. It is converted when the application starts
//!   ([`convert_legacy_file`]) with a copy kept beside it, and on every write.
//! * Keys and entries Big Game Mode did not write are kept as they are: the
//!   file is also the user's, and lsfg-vk-ui's. A 2.x profile lsfg-vk-ui
//!   shares between several games is never edited from here.
//!
//! Entries set aside while the general switch is off are kept in Big Game
//! Mode's state directory in the 1.x shape, whatever the installed format.
//! Big Game Mode stores `flow_scale` as percent (25–100); lsfg-vk as 0.25–1.0.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use anyhow::{Context, Result};
use toml::{Table, Value};

use crate::error::UserError;
use crate::models::{FrameGenBackend, FrameGenSettings};
use crate::text::N_;

// ── Paths ───────────────────────────────────────────────────────────────────

/// `$XDG_CONFIG_HOME/lsfg-vk/conf.toml`, where lsfg-vk reads it.
#[must_use]
pub fn config_path() -> PathBuf {
    crate::paths::config_home().join("lsfg-vk/conf.toml")
}

/// Entries Big Game Mode wrote, and those set aside while frame generation is
/// turned off globally.
fn state_path() -> PathBuf {
    crate::paths::state_home().join("bigame-mode/lsfg-vk.toml")
}

// ── Format ──────────────────────────────────────────────────────────────────

/// A configuration format of lsfg-vk.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    /// lsfg-vk 1.x: `[[game]]` entries keyed by `exe`.
    V1,
    /// lsfg-vk 2.x: `[[profile]]` entries matched through `active_in`.
    V2,
}

impl Format {
    fn number(self) -> i64 {
        match self {
            Self::V1 => 1,
            Self::V2 => 2,
        }
    }

    /// The key of the list of per-game entries.
    fn list(self) -> &'static str {
        match self {
            Self::V1 => "game",
            Self::V2 => "profile",
        }
    }
}

/// What is installed of lsfg-vk.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Install {
    /// No lsfg-vk library.
    Missing,
    /// A library whose configuration format is known.
    Reads(Format),
    /// A library that reads neither known format.
    Unknown,
}

/// lsfg-vk's libraries, where the packages install them: 2.x's layer first.
const LIBRARIES: &[&str] = &[
    "/usr/lib/liblsfg-vk-layer.so",
    "/usr/local/lib/liblsfg-vk-layer.so",
    "/usr/lib/liblsfg-vk.so",
    "/usr/local/lib/liblsfg-vk.so",
];

/// A string only 2.x's parser contains.
const V2_MARK: &[u8] = b"Unsupported configuration version (must be 2)";
/// A string only 1.x's parser contains.
const V1_MARK: &[u8] = b"Game override missing 'exe' field";

/// The variables that switch the layer off for one process: each version's
/// `disable_environment` (1.x `DISABLE_LSFG`, 2.x `DISABLE_LSFGVK`). Both are
/// set, so a launch is covered whichever version the package brings.
pub const DISABLE_VARIABLES: [&str; 2] = ["DISABLE_LSFG", "DISABLE_LSFGVK"];

/// The installed lsfg-vk and the format it reads, from the library itself.
#[must_use]
pub fn installed() -> Install {
    static INSTALL: OnceLock<Install> = OnceLock::new();
    *INSTALL.get_or_init(|| detect(LIBRARIES.iter().map(Path::new)))
}

fn detect<'a>(libraries: impl Iterator<Item = &'a Path>) -> Install {
    let mut found = false;
    for lib in libraries {
        let Ok(bytes) = std::fs::read(lib) else {
            continue;
        };
        found = true;
        if contains(&bytes, V2_MARK) {
            return Install::Reads(Format::V2);
        }
        if contains(&bytes, V1_MARK) {
            return Install::Reads(Format::V1);
        }
    }
    if found {
        Install::Unknown
    } else {
        Install::Missing
    }
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

/// Whether the installed lsfg-vk reads a per-game HDR mode and a choice of
/// four present modes (1.x). 2.x has neither: it presents with FIFO, or
/// keeps the game's own mode.
#[must_use]
pub fn per_game_hdr_and_present_modes() -> bool {
    installed() != Install::Reads(Format::V2)
}

/// The format to read the file in: the installed one, or 1.x's when none is
/// known (lookups then search both layouts).
fn read_format() -> Format {
    match installed() {
        Install::Reads(f) => f,
        Install::Missing | Install::Unknown => Format::V1,
    }
}

/// The format to write, or why nothing can be written.
fn writable_format() -> Result<Format> {
    match installed() {
        Install::Reads(f) => Ok(f),
        Install::Missing => Err(UserError::plain(N_("lsfg-vk is not installed")).into()),
        Install::Unknown => Err(UserError::plain(N_(
            "the installed lsfg-vk uses a configuration format Big Game Mode does not write \
             (it writes lsfg-vk 1.x's and 2.x's); change it in lsfg-vk-ui instead",
        ))
        .into()),
    }
}

// ── The two files ───────────────────────────────────────────────────────────

fn read_table(path: &Path) -> Result<Table> {
    match std::fs::read_to_string(path) {
        Ok(text) => text
            .parse::<Table>()
            .with_context(|| format!("parse {}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Table::new()),
        Err(e) => Err(e).with_context(|| format!("read {}", path.display())),
    }
}

fn write_table(path: &Path, table: &Table) -> Result<()> {
    use std::io::Write as _;
    use std::os::unix::fs::OpenOptionsExt as _;
    static SEQUENCE: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    // A dotfile manager's symlink stays a symlink: the file it points to is
    // the one replaced.
    let path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    let text = toml::to_string_pretty(table).context("serialize")?;
    // Written whole and renamed, so lsfg-vk's reload never sees half a file;
    // a name of its own per write, so two writers never share a temporary.
    let tmp = path.with_extension(format!(
        "toml.{}-{}.tmp",
        std::process::id(),
        SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    let written = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .custom_flags(libc::O_NOFOLLOW)
        .mode(0o644)
        .open(&tmp)
        .and_then(|mut f| f.write_all(text.as_bytes()).and_then(|()| f.sync_all()));
    if let Err(e) = written {
        // Nothing to undo when the temporary was never created.
        if let Err(cleanup) = std::fs::remove_file(&tmp)
            && cleanup.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!(target: "fg", path = %tmp.display(), error = %cleanup, "could not remove a temporary file");
        }
        return Err(e).with_context(|| format!("write {}", tmp.display()));
    }
    std::fs::rename(&tmp, &path).with_context(|| format!("replace {}", path.display()))
}

/// lsfg-vk's file in `format`, with anything in another layout converted.
fn read_config_as(format: Format) -> Result<Table> {
    let mut t = read_table(&config_path())?;
    migrate(format, &mut t);
    Ok(t)
}

fn read_config() -> Result<Table> {
    read_config_as(read_format())
}

fn write_config(format: Format, t: &Table) -> Result<()> {
    write_table(&config_path(), &finished(format, t.clone()))
}

/// `t` as the file lsfg-vk reads: its version, and for 2.x the `[global]`
/// section and at least one profile it requires.
fn finished(format: Format, mut t: Table) -> Table {
    t.insert("version".into(), Value::Integer(format.number()));
    if format == Format::V2 {
        if !t.get("global").is_some_and(Value::is_table) {
            t.insert("global".into(), Value::Table(Table::new()));
        }
        let profiles = list_mut(format, &mut t);
        if profiles.is_empty() {
            profiles.push(Value::Table(placeholder()));
        }
    }
    t
}

/// The name of the 2.x profile that matches no game, which keeps the file
/// valid when no game has frame generation.
pub const PLACEHOLDER: &str = "Big Game Mode: no game";

fn placeholder() -> Table {
    let mut p = Table::new();
    p.insert("name".into(), PLACEHOLDER.into());
    p.insert("active_in".into(), Value::Array(Vec::new()));
    p
}

fn is_placeholder(entry: &Table) -> bool {
    entry.get("name").and_then(Value::as_str) == Some(PLACEHOLDER) && names(entry).is_empty()
}

/// Convert the file if lsfg-vk would ignore it for its layout — the
/// `[[profile]]` layout an earlier Big Game Mode wrote under 1.x, a 1.x file
/// under 2.x — keeping a copy beside it (`conf.toml.bigame-legacy` for the
/// first, `conf.toml.bigame-v1` for the second). While it is in that layout
/// lsfg-vk ignores the whole file, so every game started meanwhile would run
/// without frame generation. Returns whether it changed anything.
///
/// # Errors
/// Returns an error if the file cannot be read, backed up or written.
pub fn convert_legacy_file() -> Result<bool> {
    let Install::Reads(format) = installed() else {
        return Ok(false);
    };
    let path = config_path();
    if !path.exists() {
        return Ok(false);
    }
    let original = read_table(&path)?;
    let mut t = original.clone();
    migrate(format, &mut t);
    let converted = finished(format, t);
    let stale = match format {
        // 1.x: only the layout it cannot read; its own file is left as is.
        Format::V1 => original.contains_key("profile"),
        Format::V2 => converted != original,
    };
    if !stale {
        return Ok(false);
    }
    let backup = path.with_extension(match format {
        Format::V1 => "toml.bigame-legacy",
        Format::V2 => "toml.bigame-v1",
    });
    if !backup.exists() {
        std::fs::copy(&path, &backup)
            .with_context(|| format!("back up to {}", backup.display()))?;
    }
    write_table(&path, &converted)?;
    Ok(true)
}

/// Bring `t` to `format`'s layout.
fn migrate(format: Format, t: &mut Table) {
    match format {
        Format::V1 => migrate_to_v1(t),
        Format::V2 => migrate_to_v2(t),
    }
}

/// The `[[profile]]` layout an earlier Big Game Mode wrote, as `[[game]]`
/// entries; `allow_fp16`, which lsfg-vk 1.0 does not read, is dropped.
fn migrate_to_v1(t: &mut Table) {
    if let Some(Value::Array(old)) = t.remove("profile") {
        for p in old.iter().filter_map(Value::as_table) {
            let Some(exe) = names(p).first().copied() else {
                continue;
            };
            let v = values_of(Format::V2, p);
            // Old writers wrote `multiplier = 1` for off.
            let explicit = p.get("multiplier").and_then(Value::as_integer).unwrap_or(1);
            if explicit < 2 || find(Format::V1, t, exe).is_some() {
                continue;
            }
            let mut g = entry_table(Format::V1, exe, v);
            // The old layout's own HDR key.
            if let Some(hdr) = p.get("hdr").and_then(Value::as_bool) {
                g.insert("hdr_mode".into(), Value::Boolean(hdr));
            }
            // It wrote the present mode lsfg-vk chose; keep that choice.
            g.remove("experimental_present_mode");
            list_mut(Format::V1, t).push(Value::Table(g));
        }
    }
    if let Some(Value::Table(global)) = t.get_mut("global") {
        global.remove("allow_fp16");
    }
}

/// The keys lsfg-vk 2.x reads; any other makes it ignore the file.
const V2_TOP: &[&str] = &["version", "global", "profile"];
const V2_GLOBAL: &[&str] = &["dll", "allow_fp16", "log_level", "log_file"];
const V2_PROFILE: &[&str] = &[
    "name",
    "active_in",
    "pacing_mode",
    "multiplier",
    "flow_scale",
    "performance_mode",
    "override_present_mode",
    "preserve_swapchain_image_count",
];

/// 1.x's `[[game]]` entries — and an earlier Big Game Mode's `[[profile]]`
/// entries with the keys 1.x-era writers added — as 2.x profiles. Keys 2.x
/// does not know are removed (the copy kept by [`convert_legacy_file`] has
/// them); an entry that is off is removed, as off is no entry.
fn migrate_to_v2(t: &mut Table) {
    let games = t.remove("game");
    t.retain(|k, _| V2_TOP.contains(&k));
    if let Some(Value::Table(global)) = t.get_mut("global") {
        global.retain(|k, _| V2_GLOBAL.contains(&k));
    }
    if let Some(v) = t.get_mut("profile") {
        if let Value::Array(profiles) = v {
            profiles.retain_mut(|p| {
                let Some(p) = p.as_table_mut() else {
                    return false;
                };
                // Earlier Big Game Mode versions wrote `multiplier = 1`
                // for off, with keys of their own (`pacing`, `hdr`,
                // `present_mode`).
                let off = p
                    .get("multiplier")
                    .and_then(Value::as_integer)
                    .is_some_and(|m| m < 2);
                if p.contains_key("pacing") && p.contains_key("present_mode") {
                    if let Some(f) = p.get("flow_scale").and_then(Value::as_float) {
                        p.insert("flow_scale".into(), Value::Float(round_flow(f)));
                    }
                }
                p.retain(|k, _| V2_PROFILE.contains(&k));
                !off
            });
        } else {
            *v = Value::Array(Vec::new());
        }
    }
    let Some(Value::Array(games)) = games else {
        return;
    };
    for g in games.iter().filter_map(Value::as_table) {
        let Some(exe) = g.get("exe").and_then(Value::as_str) else {
            continue;
        };
        let v = values_of(Format::V1, g);
        if v.multiplier < 2 || find(Format::V2, t, exe).is_some() {
            continue;
        }
        let p = entry_table(Format::V2, exe, v);
        let profiles = list_mut(Format::V2, t);
        profiles.retain(|e| !e.as_table().is_some_and(is_placeholder));
        profiles.push(Value::Table(p));
    }
}

fn list_mut(format: Format, t: &mut Table) -> &mut Vec<Value> {
    let v = t
        .entry(format.list())
        .or_insert_with(|| Value::Array(Vec::new()));
    if !v.is_array() {
        *v = Value::Array(Vec::new());
    }
    v.as_array_mut().expect("just made an array")
}

/// The game names an entry applies to: 1.x's `exe`, 2.x's `active_in` (a
/// list, or one string).
fn names(entry: &Table) -> Vec<&str> {
    if let Some(exe) = entry.get("exe").and_then(Value::as_str) {
        return vec![exe];
    }
    match entry.get("active_in") {
        Some(Value::String(s)) => vec![s.as_str()],
        Some(Value::Array(a)) => a.iter().filter_map(Value::as_str).collect(),
        _ => Vec::new(),
    }
}

/// Index of the entry that applies to `exe`.
fn find(format: Format, t: &Table, exe: &str) -> Option<usize> {
    t.get(format.list())?
        .as_array()?
        .iter()
        .position(|g| g.as_table().is_some_and(|g| names(g).contains(&exe)))
}

/// The entry that applies to `exe`, in either layout: what lsfg-vk reads
/// when the format is known, and both when it is not.
fn entry<'a>(t: &'a Table, exe: &str) -> Option<(Format, &'a Table)> {
    [read_format(), Format::V1, Format::V2]
        .into_iter()
        .find_map(|f| {
            let i = find(f, t, exe)?;
            Some((f, t.get(f.list())?.as_array()?.get(i)?.as_table()?))
        })
}

/// Remove and return the entry for `exe` — only one Big Game Mode could have
/// written: a 2.x profile lsfg-vk-ui shares with other games stays.
fn take(format: Format, t: &mut Table, exe: &str) -> Option<Table> {
    let i = find(format, t, exe)?;
    let list = list_mut(format, t);
    if list[i].as_table().is_none_or(|g| names(g) != [exe]) {
        return None;
    }
    match list.remove(i) {
        Value::Table(g) => Some(g),
        _ => None,
    }
}

/// The game names Big Game Mode manages, and the entries set aside.
#[derive(Default)]
struct State {
    managed: Vec<String>,
    paused: Vec<Value>,
}

fn read_state() -> State {
    let t = read_table(&state_path()).unwrap_or_default();
    State {
        managed: t
            .get("managed")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default(),
        paused: t
            .get("paused")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default(),
    }
}

fn write_state(s: &State) -> Result<()> {
    let mut t = Table::new();
    t.insert(
        "managed".into(),
        Value::Array(s.managed.iter().map(|m| Value::String(m.clone())).collect()),
    );
    t.insert("paused".into(), Value::Array(s.paused.clone()));
    write_table(&state_path(), &t)
}

fn paused_exe(g: &Value) -> Option<&str> {
    g.get("exe").and_then(Value::as_str)
}

// ── Entries ─────────────────────────────────────────────────────────────────

/// One game's frame generation as Big Game Mode keeps it.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Values {
    multiplier: u32,
    flow_pct: u32,
    performance: bool,
    hdr: bool,
    present: u32,
}

impl Values {
    fn tuple(self) -> (u32, u32, bool, bool, u32) {
        (
            self.multiplier,
            self.flow_pct,
            self.performance,
            self.hdr,
            self.present,
        )
    }
}

/// lsfg-vk's present mode name for the UI's index (0 FIFO, 1 recommended,
/// 2 mailbox, 3 immediate); `None` leaves lsfg-vk's own choice.
fn present_name(mode: u32) -> Option<&'static str> {
    match mode {
        0 => Some("fifo"),
        2 => Some("mailbox"),
        3 => Some("immediate"),
        _ => None,
    }
}

fn present_index(name: Option<&str>) -> u32 {
    match name {
        Some("fifo") => 0,
        Some("mailbox") => 2,
        Some("immediate") => 3,
        _ => 1,
    }
}

/// 2.x knows FIFO (its default) or the game's own mode: mailbox and
/// immediate are the game's own choice of a mode without vsync.
fn keeps_game_present_mode(present: u32) -> bool {
    present >= 2
}

/// Older versions wrote it through an f32 (0.6000000238418579); the UI works
/// in whole percent.
fn round_flow(f: f64) -> f64 {
    (f.clamp(0.25, 1.0) * 100.0).round() / 100.0
}

fn entry_table(format: Format, exe: &str, v: Values) -> Table {
    let mut g = Table::new();
    match format {
        Format::V1 => {
            g.insert("exe".into(), exe.into());
        }
        Format::V2 => {
            g.insert("name".into(), exe.into());
            g.insert("active_in".into(), Value::Array(vec![exe.into()]));
        }
    }
    g.insert("multiplier".into(), Value::Integer(i64::from(v.multiplier)));
    g.insert(
        "flow_scale".into(),
        Value::Float(f64::from(v.flow_pct) / 100.0),
    );
    g.insert("performance_mode".into(), Value::Boolean(v.performance));
    match format {
        Format::V1 => {
            g.insert("hdr_mode".into(), Value::Boolean(v.hdr));
            if let Some(p) = present_name(v.present) {
                g.insert("experimental_present_mode".into(), p.into());
            }
        }
        Format::V2 => {
            if keeps_game_present_mode(v.present) {
                g.insert("override_present_mode".into(), Value::Boolean(false));
            }
        }
    }
    g
}

/// The values of one entry, lsfg-vk's defaults where it has none: 1.x's
/// entry without a multiplier is off, 2.x's profile defaults to 2.
fn values_of(format: Format, g: &Table) -> Values {
    let default_multiplier = match format {
        Format::V1 => 1,
        Format::V2 => 2,
    };
    let multiplier = g
        .get("multiplier")
        .and_then(Value::as_integer)
        .and_then(|m| u32::try_from(m).ok())
        .unwrap_or(default_multiplier);
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let flow = g
        .get("flow_scale")
        // lsfg-vk accepts a whole number (`flow_scale = 1`); within 0–1 it
        // converts exactly.
        .and_then(|v| {
            v.as_float().or_else(|| {
                v.as_integer()
                    .and_then(|i| i32::try_from(i).ok())
                    .map(f64::from)
            })
        })
        .map_or(100, |f| ((f * 100.0).round() as u32).clamp(25, 100));
    let present = match (
        g.get("experimental_present_mode").and_then(Value::as_str),
        g.get("override_present_mode").and_then(Value::as_bool),
    ) {
        (Some(name), _) => present_index(Some(name)),
        (None, Some(false)) => 2,
        (None, _) => 1,
    };
    Values {
        multiplier,
        flow_pct: flow,
        performance: g
            .get("performance_mode")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        hdr: g.get("hdr_mode").and_then(Value::as_bool).unwrap_or(false),
        present,
    }
}

/// Put `exe`'s entry into `t`, keeping keys lsfg-vk-ui or the user added
/// to it.
fn upsert(format: Format, t: &mut Table, exe: &str, v: Values) -> Result<()> {
    let new = entry_table(format, exe, v);
    let Some(i) = find(format, t, exe) else {
        let list = list_mut(format, t);
        list.retain(|e| !e.as_table().is_some_and(is_placeholder));
        list.push(Value::Table(new));
        return Ok(());
    };
    let Some(Value::Table(old)) = list_mut(format, t).get_mut(i) else {
        unreachable!("find returns tables only");
    };
    if names(old) != [exe] {
        let profile = old
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        return Err(UserError::with(
            N_("lsfg-vk's profile “%s” also covers other games; change this game's frame generation in lsfg-vk-ui"),
            [profile],
        )
        .into());
    }
    old.remove("experimental_present_mode");
    old.remove("override_present_mode");
    old.extend(new);
    Ok(())
}

// ── Public API ──────────────────────────────────────────────────────────────

/// Write or update frame generation for the game whose process is `name`.
///
/// `multiplier` 1 (or 0) means off: the game's entry is removed. Otherwise
/// the multiplier is clamped to 2–20 and `flow_scale_pct` must be 25–100.
///
/// # Errors
/// Returns error if the DLL is not configured, a value is out of range, the
/// installed lsfg-vk reads another format, or the file cannot be written.
pub fn write_profile(
    name: &str,
    multiplier: u32,
    flow_scale_pct: u32,
    perf_mode: bool,
    hdr: bool,
    present_mode: u32,
) -> Result<()> {
    if multiplier <= 1 {
        return disable_for_game(name);
    }
    let format = writable_format()?;
    // A game whose lsfg-vk cannot load its shaders may close at start; said
    // before "not found", which is all a DLL known unusable reads as.
    if let Some(DllCheck::Unusable(reason)) =
        read_global_dll().and_then(|d| check_dll(Path::new(&d)))
    {
        return Err(unusable_error(&reason));
    }
    anyhow::ensure!(
        is_lossless_dll_ready(),
        UserError::plain(N_("Lossless.dll not found in configured LSFG path"))
    );
    anyhow::ensure!(
        (25..=100).contains(&flow_scale_pct),
        UserError::plain(N_("flow_scale_pct must be 25–100"))
    );
    let mut t = read_config_as(format)?;
    upsert(
        format,
        &mut t,
        name,
        Values {
            multiplier: multiplier.clamp(2, 20),
            flow_pct: flow_scale_pct,
            performance: perf_mode,
            hdr,
            present: present_mode,
        },
    )?;
    write_config(format, &t)?;
    let mut s = read_state();
    if !s.managed.iter().any(|m| m == name) {
        s.managed.push(name.to_owned());
    }
    s.paused.retain(|g| paused_exe(g) != Some(name));
    write_state(&s)
}

/// Write `name`'s entry where it belongs: in lsfg-vk's file when the general
/// switch is on, set aside with the other paused entries when it is off, so
/// saving a profile never switches frame generation on behind the general
/// switch. Multiplier 1 removes the entry from both.
///
/// # Errors
/// Returns error if the DLL is not configured, a value is out of range, or
/// the files cannot be written.
pub fn save_for_game(
    name: &str,
    multiplier: u32,
    flow_scale_pct: u32,
    perf_mode: bool,
    hdr: bool,
    present_mode: u32,
    general_on: bool,
) -> Result<()> {
    if general_on || multiplier <= 1 {
        write_profile(
            name,
            multiplier,
            flow_scale_pct,
            perf_mode,
            hdr,
            present_mode,
        )?;
        if multiplier <= 1 {
            let mut s = read_state();
            let before = s.paused.len();
            s.paused.retain(|g| paused_exe(g) != Some(name));
            if s.paused.len() != before {
                write_state(&s)?;
            }
        }
        return Ok(());
    }
    anyhow::ensure!(
        is_lossless_dll_ready(),
        UserError::plain(N_("Lossless.dll not found in configured LSFG path"))
    );
    anyhow::ensure!(
        (25..=100).contains(&flow_scale_pct),
        UserError::plain(N_("flow_scale_pct must be 25–100"))
    );
    // Not in lsfg-vk's file while the switch is off — when it is Big Game
    // Mode's entry. One the user made in lsfg-vk-ui is theirs: the general
    // switch does not set it aside, and saving a profile does not take it.
    let mut s = read_state();
    if !s.managed.iter().any(|m| m == name) && read_profile(name).0 > 1 {
        return Ok(());
    }
    disable_for_game(name)?;
    s.paused.retain(|g| paused_exe(g) != Some(name));
    s.paused.push(Value::Table(entry_table(
        Format::V1,
        name,
        Values {
            multiplier: multiplier.clamp(2, 20),
            flow_pct: flow_scale_pct,
            performance: perf_mode,
            hdr,
            present: present_mode,
        },
    )));
    if !s.managed.iter().any(|m| m == name) {
        s.managed.push(name.to_owned());
    }
    write_state(&s)
}

/// Remove the frame generation entry for a game (its profile was deleted).
///
/// # Errors
/// Returns error if the file cannot be read or written.
pub fn delete_profile(name: &str) -> Result<()> {
    let mut s = read_state();
    // An entry the user made in lsfg-vk-ui outlives a deleted profile.
    if s.managed.iter().any(|m| m == name) {
        disable_for_game(name)?;
    }
    s.managed.retain(|m| m != name);
    s.paused.retain(|g| paused_exe(g) != Some(name));
    write_state(&s)
}

/// Frame generation for `name`: `(multiplier, flow_scale_pct, perf_mode, hdr,
/// present_mode)`. Multiplier 1 means off (no entry).
#[must_use]
pub fn read_profile(name: &str) -> (u32, u32, bool, bool, u32) {
    read_config()
        .ok()
        .and_then(|t| entry(&t, name).map(|(f, g)| values_of(f, g).tuple()))
        .unwrap_or(OFF)
}

/// What Big Game Mode set for `name`, whether lsfg-vk reads it now or the
/// global switch has set it aside: the value a game's profile shows. The
/// entry lsfg-vk reads comes first; a paused one is what the switch puts
/// back when it is turned on again.
#[must_use]
pub fn read_profile_any(name: &str) -> (u32, u32, bool, bool, u32) {
    if let Some(values) = read_config()
        .ok()
        .and_then(|t| entry(&t, name).map(|(f, g)| values_of(f, g).tuple()))
    {
        return values;
    }
    read_state()
        .paused
        .iter()
        .filter_map(Value::as_table)
        .find(|g| g.get("exe").and_then(Value::as_str) == Some(name))
        .map_or(OFF, |g| values_of(Format::V1, g).tuple())
}

/// No entry: frame generation off.
const OFF: (u32, u32, bool, bool, u32) = (1, 100, false, false, 1);

/// The games lsfg-vk's file generates frames for (multiplier above 1), by
/// the names its entries match.
#[must_use]
pub fn generating_games() -> Vec<String> {
    read_config().map_or_else(|_| Vec::new(), |t| generating_in(&t))
}

pub(crate) fn generating_in(t: &Table) -> Vec<String> {
    let mut games = Vec::new();
    for format in [Format::V1, Format::V2] {
        let Some(list) = t.get(format.list()).and_then(Value::as_array) else {
            continue;
        };
        for g in list.iter().filter_map(Value::as_table) {
            if values_of(format, g).multiplier < 2 {
                continue;
            }
            games.extend(
                names(g)
                    .into_iter()
                    // 1.x's catch-all entry for Proton's own processes.
                    .filter(|n| !n.eq_ignore_ascii_case("proton"))
                    .map(str::to_owned),
            );
        }
    }
    games
}

/// `[global].dll`, when set.
#[must_use]
pub fn read_global_dll() -> Option<String> {
    read_config()
        .ok()?
        .get("global")?
        .get("dll")?
        .as_str()
        .map(str::to_owned)
}

/// Whether the lsfg-vk layer is installed (system-wide, in `/usr/local` or
/// for this user).
#[must_use]
pub fn layer_installed() -> bool {
    const DIRS: &[&str] = &[
        "/etc/vulkan/implicit_layer.d",
        "/usr/share/vulkan/implicit_layer.d",
        "/usr/local/share/vulkan/implicit_layer.d",
    ];
    let user = crate::paths::data_home().join("vulkan/implicit_layer.d");
    DIRS.iter()
        .map(PathBuf::from)
        .chain(std::iter::once(user))
        .any(|d| {
            [
                "VkLayer_LS_frame_generation.json",
                "VkLayer_LSFGVK_frame_generation.json",
            ]
            .iter()
            .any(|f| d.join(f).is_file())
        })
        || installed() != Install::Missing
}

/// Lossless Scaling's Steam app id.
pub const LOSSLESS_SCALING_APP: u32 = 993_090;

/// `Lossless.dll` where Steam installs Lossless Scaling, in any library.
#[must_use]
pub fn find_steam_dll() -> Option<std::path::PathBuf> {
    crate::games::steam_libraries(&crate::paths::home_dir())
        .into_iter()
        .map(|root| root.join("steamapps/common/Lossless Scaling/Lossless.dll"))
        .find(|p| p.is_file())
}

/// Whether a `Lossless.dll` is configured, exists, and was not found
/// unusable by the installed lsfg-vk ([`check_dll`]; only its saved answer
/// is read here, nothing is run).
#[must_use]
pub fn is_lossless_dll_ready() -> bool {
    read_global_dll().is_some_and(|p| {
        let dll = Path::new(&p);
        dll.is_file() && !matches!(cached_dll_check(dll), Some(DllCheck::Unusable(_)))
    })
}

// ── Lossless.dll and the installed lsfg-vk ─────────────────────────────────

/// lsfg-vk 2.x's own command-line tool.
const CLI: &str = "/usr/bin/lsfg-vk-cli";

/// What lsfg-vk said about a `Lossless.dll`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DllCheck {
    /// It generated frames with it.
    Works,
    /// It cannot, for the reason it gave.
    Unusable(String),
}

fn dll_check_path() -> PathBuf {
    crate::paths::cache_home().join("bigame-mode/lsfg-dll-check.toml")
}

/// What a check is valid for: the DLL as it is and the tool as it is.
fn check_key(dll: &Path) -> Option<String> {
    let stamp = |m: &std::fs::Metadata| {
        m.modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_secs())
    };
    let d = std::fs::metadata(dll).ok()?;
    let c = std::fs::metadata(CLI).ok()?;
    Some(format!(
        "{}|{}|{}|{}",
        dll.display(),
        d.len(),
        stamp(&d),
        stamp(&c)
    ))
}

/// The saved answer of [`check_dll`] for `dll` as it is now.
#[must_use]
pub fn cached_dll_check(dll: &Path) -> Option<DllCheck> {
    let key = check_key(dll)?;
    let t = read_table(&dll_check_path()).ok()?;
    if t.get("key").and_then(Value::as_str) != Some(key.as_str()) {
        return None;
    }
    match t.get("unusable").and_then(Value::as_str) {
        Some(reason) => Some(DllCheck::Unusable(reason.to_owned())),
        None => Some(DllCheck::Works),
    }
}

/// Ask lsfg-vk 2.x whether it can generate frames with `dll`: its own
/// benchmark, one second at 64×64. Run again only when the DLL or the tool
/// changed.
///
/// The configuration file names a DLL and lsfg-vk loads the profile, but its
/// shaders come from the DLL: one from an older Lossless Scaling fails when
/// the game creates its swapchain ("Unable to find base shader 'mipmaps'",
/// "The specified shader DLL does not exist"), and Shadow of the Tomb Raider
/// then closed at start (checked on the reference desktop with lsfg-vk
/// 2.0.0 and a Lossless.dll that worked with 1.0). Blocking: for a worker
/// thread. `None` when lsfg-vk 1.x is installed (it has no such tool) or the
/// tool gives no clear answer.
#[must_use]
pub fn check_dll(dll: &Path) -> Option<DllCheck> {
    if installed() != Install::Reads(Format::V2) || !Path::new(CLI).is_file() {
        return None;
    }
    if let Some(known) = cached_dll_check(dll) {
        return Some(known);
    }
    let key = check_key(dll)?;
    let output = run_with_timeout(
        std::process::Command::new(CLI)
            .args(["benchmark", "-d"])
            .arg(dll)
            .args(["-w", "64", "-h", "64", "-m", "2", "-t", "1"]),
        std::time::Duration::from_secs(20),
    )?;
    let answer = cli_answer(output.0, &output.1)?;
    let mut t = Table::new();
    t.insert("key".into(), key.into());
    if let DllCheck::Unusable(reason) = &answer {
        t.insert("unusable".into(), reason.clone().into());
    }
    if let Err(e) = write_table(&dll_check_path(), &t) {
        tracing::warn!(target: "fg", error = %format!("{e:#}"), "could not save the Lossless.dll check");
    }
    Some(answer)
}

/// The benchmark's verdict: success, or the reason it printed under "An
/// error occured…" (`- …`). `None` for a failure that gives no reason.
fn cli_answer(success: bool, output: &str) -> Option<DllCheck> {
    if success {
        return Some(DllCheck::Works);
    }
    let reason = output
        .lines()
        .filter_map(|l| l.trim().strip_prefix("- "))
        .collect::<Vec<_>>()
        .join("; ");
    (!reason.is_empty()).then_some(DllCheck::Unusable(reason))
}

/// Run `command`, its output captured, killed after `limit`. `(success,
/// stdout and stderr)`, or `None` when it could not run or took too long.
fn run_with_timeout(
    command: &mut std::process::Command,
    limit: std::time::Duration,
) -> Option<(bool, String)> {
    use std::io::Read as _;
    let mut child = command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .ok()?;
    let deadline = std::time::Instant::now() + limit;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if std::time::Instant::now() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            _ => {
                if let Err(e) = child.kill() {
                    tracing::warn!(target: "fg", error = %e, "could not stop lsfg-vk-cli");
                }
                // Reaped so it leaves no zombie.
                let _reaped = child.wait();
                return None;
            }
        }
    };
    // The run is a second at 64x64: its output fits in the pipes' buffers,
    // read after it ended.
    let mut text = String::new();
    for stream in [
        child
            .stdout
            .take()
            .map(|s| Box::new(s) as Box<dyn std::io::Read>),
        child
            .stderr
            .take()
            .map(|s| Box::new(s) as Box<dyn std::io::Read>),
    ]
    .into_iter()
    .flatten()
    {
        let mut part = String::new();
        if stream.take(64 * 1024).read_to_string(&mut part).is_ok() {
            text.push_str(&part);
        }
    }
    Some((status.success(), text))
}

/// The error for a DLL lsfg-vk cannot generate frames with.
fn unusable_error(reason: &str) -> anyhow::Error {
    UserError::with(
        N_("lsfg-vk cannot generate frames with this Lossless.dll (%s): update Lossless Scaling and choose its Lossless.dll again"),
        [reason.to_owned()],
    )
    .into()
}

/// Set (or with `None` remove) `[global].dll`.
///
/// # Errors
/// Returns error if lsfg-vk's format is unknown or the file cannot be read
/// or written.
pub fn write_global_dll(dll: Option<String>) -> Result<()> {
    let format = writable_format()?;
    let mut t = read_config_as(format)?;
    let global = t
        .entry("global")
        .or_insert_with(|| Value::Table(Table::new()));
    if let Some(g) = global.as_table_mut() {
        match dll {
            Some(d) => {
                g.insert("dll".into(), d.into());
            }
            None => {
                g.remove("dll");
            }
        }
    }
    write_config(format, &t)
}

/// Whether the global video settings allow lsfg-vk.
#[must_use]
pub fn global_state_allows_lsfg(frame_gen: &FrameGenSettings) -> bool {
    frame_gen.enabled && frame_gen.backend == FrameGenBackend::LsfgVk
}

/// Bring lsfg-vk's file in line with the global switch: off sets
/// Big Game Mode's entries aside, on puts them back. Returns whether anything
/// changed.
///
/// # Errors
/// Returns error if the files cannot be read or written.
pub fn sync_global_enablement(frame_gen: &FrameGenSettings) -> Result<bool> {
    if global_state_allows_lsfg(frame_gen) {
        resume_all_profiles()
    } else {
        disable_all_profiles()
    }
}

/// Whether `name` has frame generation on.
#[must_use]
pub fn is_active_for_game(name: &str) -> bool {
    is_lossless_dll_ready() && read_profile(name).0 > 1
}

/// Set aside every entry Big Game Mode wrote (the global switch turned off).
/// They are kept, and [`sync_global_enablement`] puts them back.
///
/// # Errors
/// Returns error if the files cannot be read or written.
pub fn disable_all_profiles() -> Result<bool> {
    let format = read_format();
    let mut t = read_config_as(format)?;
    let mut s = read_state();
    let mut changed = false;
    for name in s.managed.clone() {
        if let Some(g) = take(format, &mut t, &name) {
            // Kept in 1.x's shape, whatever the file's.
            let kept = match format {
                Format::V1 => g,
                Format::V2 => entry_table(Format::V1, &name, values_of(Format::V2, &g)),
            };
            s.paused.push(Value::Table(kept));
            changed = true;
        }
    }
    if changed {
        write_config(format, &t)?;
        write_state(&s)?;
    }
    Ok(changed)
}

/// Put back the entries set aside by [`disable_all_profiles`].
fn resume_all_profiles() -> Result<bool> {
    let mut s = read_state();
    if s.paused.is_empty() {
        return Ok(false);
    }
    let format = writable_format()?;
    let mut t = read_config_as(format)?;
    for g in std::mem::take(&mut s.paused) {
        let Some(g) = g.as_table() else {
            continue;
        };
        let Some(exe) = g.get("exe").and_then(Value::as_str) else {
            continue;
        };
        if find(format, &t, exe).is_some() {
            continue;
        }
        match format {
            Format::V1 => list_mut(format, &mut t).push(Value::Table(g.clone())),
            Format::V2 => {
                let v = values_of(Format::V1, g);
                if v.multiplier >= 2 {
                    upsert(format, &mut t, exe, v)?;
                }
            }
        }
    }
    write_config(format, &t)?;
    write_state(&s)?;
    Ok(true)
}

/// Turn frame generation off for one game: its entry is removed.
///
/// # Errors
/// Returns error if the file cannot be read or written.
pub fn disable_for_game(name: &str) -> Result<()> {
    let format = read_format();
    let mut t = read_config_as(format)?;
    if take(format, &mut t, name).is_some() {
        write_config(format, &t)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{FrameGenBackend, FrameGenSettings};

    fn values(multiplier: u32, flow_pct: u32, present: u32) -> Values {
        Values {
            multiplier,
            flow_pct,
            performance: true,
            hdr: false,
            present,
        }
    }

    #[test]
    fn test_global_state_allows_lsfg_only_for_enabled_lsfg_backend() {
        let disabled = FrameGenSettings::default();
        assert!(!global_state_allows_lsfg(&disabled));
        let off = FrameGenSettings {
            enabled: true,
            backend: FrameGenBackend::None,
        };
        assert!(!global_state_allows_lsfg(&off));
        let lsfg = FrameGenSettings {
            enabled: true,
            backend: FrameGenBackend::LsfgVk,
        };
        assert!(global_state_allows_lsfg(&lsfg));
    }

    #[test]
    fn an_entry_is_what_lsfg_vk_1_reads() {
        let g = entry_table(Format::V1, "SOTTR.exe", values(2, 70, 0));
        let text = toml::to_string(&g).unwrap();
        assert!(text.contains("exe = \"SOTTR.exe\""), "{text}");
        assert!(text.contains("multiplier = 2"));
        assert!(text.contains("flow_scale = 0.7"));
        assert!(text.contains("experimental_present_mode = \"fifo\""));
        // The recommended mode is lsfg-vk's own choice: no key at all.
        assert!(
            !toml::to_string(&entry_table(Format::V1, "x", values(2, 100, 1)))
                .unwrap()
                .contains("present_mode")
        );
    }

    #[test]
    fn a_profile_is_what_lsfg_vk_2_reads_and_round_trips() {
        let v = values(3, 60, 2);
        let p = entry_table(Format::V2, "SOTTR.exe", v);
        assert_eq!(p["name"].as_str(), Some("SOTTR.exe"));
        assert_eq!(names(&p), ["SOTTR.exe"]);
        assert_eq!(p["override_present_mode"].as_bool(), Some(false));
        assert!(p.keys().all(|k| V2_PROFILE.contains(&k.as_str())), "{p:?}");
        assert_eq!(values_of(Format::V2, &p), v);
        // FIFO and lsfg-vk's choice are 2.x's default: no key.
        assert!(
            !entry_table(Format::V2, "x", values(2, 100, 0)).contains_key("override_present_mode")
        );
    }

    #[test]
    fn a_v2_file_is_never_left_without_global_or_profile() {
        let t = finished(Format::V2, Table::new());
        assert_eq!(t["version"].as_integer(), Some(2));
        assert!(t["global"].is_table());
        let profiles = t["profile"].as_array().unwrap();
        assert_eq!(profiles.len(), 1);
        assert!(is_placeholder(profiles[0].as_table().unwrap()));
        // The placeholder makes way for the first real entry, and comes back
        // when the last one goes.
        let mut t = t;
        upsert(Format::V2, &mut t, "Game.exe", values(2, 100, 1)).unwrap();
        let profiles = t["profile"].as_array().unwrap();
        assert_eq!(profiles.len(), 1);
        assert_eq!(names(profiles[0].as_table().unwrap()), ["Game.exe"]);
        assert!(take(Format::V2, &mut t, "Game.exe").is_some());
        assert!(is_placeholder(
            finished(Format::V2, t)["profile"][0].as_table().unwrap()
        ));
    }

    #[test]
    fn a_profile_lsfg_vk_ui_shares_between_games_is_never_edited() {
        let mut t: Table = r#"
version = 2
[global]
[[profile]]
name = "Shooters"
active_in = ["a.exe", "b.exe"]
multiplier = 3
"#
        .parse()
        .unwrap();
        let before = t.clone();
        assert!(upsert(Format::V2, &mut t, "a.exe", values(2, 100, 1)).is_err());
        assert!(take(Format::V2, &mut t, "a.exe").is_none());
        assert_eq!(t, before);
        // It still is what lsfg-vk applies to the game, and says so.
        assert_eq!(
            values_of(Format::V2, entry(&t, "b.exe").unwrap().1).multiplier,
            3
        );
        assert_eq!(generating_in(&t), ["a.exe", "b.exe"]);
    }

    #[test]
    fn a_1x_file_becomes_a_2x_file_and_keeps_its_games() {
        // This machine's file after lsfg-vk moved to 2.0, plus entries.
        let mut t: Table = r#"
version = 1
tweak = "1.x only"
[global]
dll = "/home/u/Lossless Scaling/Lossless.dll"
[[game]]
exe = "SOTTR.exe"
multiplier = 3
flow_scale = 0.6
performance_mode = true
hdr_mode = true
experimental_present_mode = "mailbox"
[[game]]
exe = "Off.exe"
multiplier = 1
"#
        .parse()
        .unwrap();
        migrate(Format::V2, &mut t);
        let t = finished(Format::V2, t);
        assert!(!t.contains_key("game") && !t.contains_key("tweak"));
        assert_eq!(
            t["global"]["dll"].as_str(),
            Some("/home/u/Lossless Scaling/Lossless.dll")
        );
        let (format, g) = entry(&t, "SOTTR.exe").unwrap();
        assert_eq!(format, Format::V2);
        let v = values_of(Format::V2, g);
        assert_eq!(
            (v.multiplier, v.flow_pct, v.performance, v.present),
            (3, 60, true, 2)
        );
        assert!(entry(&t, "Off.exe").is_none());
        // An empty 1.x file is still a valid 2.x one.
        let mut empty: Table = "version = 1\ngame = []\n[global]\ndll = \"/d\"\n"
            .parse()
            .unwrap();
        migrate(Format::V2, &mut empty);
        let empty = finished(Format::V2, empty);
        assert!(is_placeholder(empty["profile"][0].as_table().unwrap()));
    }

    #[test]
    fn an_earlier_big_game_mode_profile_layout_becomes_clean_2x_profiles() {
        let mut t: Table = r#"
version = 1
[global]
dll = "/home/u/Lossless.dll"
[[profile]]
name = "SOTTR.exe"
active_in = ["SOTTR.exe"]
multiplier = 3
flow_scale = 0.6000000238418579
performance_mode = true
pacing = "none"
hdr = true
present_mode = 3
[[profile]]
name = "civ7"
active_in = ["civ7"]
multiplier = 1
pacing = "none"
present_mode = 3
"#
        .parse()
        .unwrap();
        migrate(Format::V2, &mut t);
        let profiles = t["profile"].as_array().unwrap();
        assert_eq!(profiles.len(), 1, "an entry that is off goes");
        let p = profiles[0].as_table().unwrap();
        assert!(p.keys().all(|k| V2_PROFILE.contains(&k.as_str())), "{p:?}");
        assert_eq!(p["flow_scale"].as_float(), Some(0.6));
    }

    #[test]
    fn the_legacy_profile_layout_becomes_game_entries_and_nothing_else_is_lost() {
        // This machine's file before, plus a user's own key and entry.
        let mut t: Table = r#"
version = 1
tweak = "kept"
[global]
dll = "/home/u/Lossless.dll"
allow_fp16 = true
[[profile]]
name = "SOTTR.exe"
active_in = ["SOTTR.exe"]
multiplier = 3
flow_scale = 0.6000000238418579
performance_mode = true
pacing = "none"
hdr = false
present_mode = 0
[[profile]]
name = "off"
active_in = ["Off.exe"]
multiplier = 1
flow_scale = 1.0
performance_mode = false
pacing = "none"
[[game]]
exe = "vkcube"
multiplier = 4
"#
        .parse()
        .unwrap();
        migrate(Format::V1, &mut t);
        assert!(!t.contains_key("profile"));
        assert_eq!(t["tweak"].as_str(), Some("kept"));
        assert!(!t["global"].as_table().unwrap().contains_key("allow_fp16"));
        assert_eq!(t["global"]["dll"].as_str(), Some("/home/u/Lossless.dll"));
        let g = t["game"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_table)
            .find(|g| names(g) == ["SOTTR.exe"])
            .unwrap();
        assert_eq!(g["multiplier"].as_integer(), Some(3));
        // Written through an f32 by the old code; back to whole percent.
        assert_eq!(g["flow_scale"].as_float(), Some(0.6));
        // multiplier 1 would make lsfg-vk reject the whole file.
        assert!(find(Format::V1, &t, "Off.exe").is_none());
        assert!(
            find(Format::V1, &t, "vkcube").is_some(),
            "a user's entry is kept"
        );
    }

    #[test]
    fn lsfg_vk_clis_verdict_is_read() {
        // This machine's lsfg-vk 2.0.0 with a Lossless.dll that worked with 1.0.
        let old = "An error occured during the benchmark:\n- Unable to find base shader 'mipmaps' in DLL\n";
        assert_eq!(
            cli_answer(false, old),
            Some(DllCheck::Unusable(
                "Unable to find base shader 'mipmaps' in DLL".into()
            ))
        );
        assert_eq!(cli_answer(true, "anything"), Some(DllCheck::Works));
        // A failure with no reason is no answer.
        assert_eq!(cli_answer(false, "segfault"), None);
    }

    #[test]
    fn a_command_that_takes_too_long_is_stopped() {
        let start = std::time::Instant::now();
        let out = run_with_timeout(
            std::process::Command::new("sleep").arg("5"),
            std::time::Duration::from_millis(200),
        );
        assert!(out.is_none());
        assert!(start.elapsed() < std::time::Duration::from_secs(2));
        let ok = run_with_timeout(
            std::process::Command::new("sh").args(["-c", "echo '- reason'; exit 1"]),
            std::time::Duration::from_secs(5),
        );
        assert_eq!(ok, Some((false, "- reason\n".to_owned())));
    }

    #[test]
    fn present_modes_round_trip() {
        for i in 0..4 {
            assert_eq!(present_index(present_name(i)), i);
        }
    }

    #[test]
    fn the_format_is_read_from_the_library() {
        let dir = std::env::temp_dir().join(format!("bgm-fg-detect-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (v1, v2, other) = (dir.join("v1.so"), dir.join("v2.so"), dir.join("v3.so"));
        std::fs::write(&v1, [b"\0junk".as_slice(), V1_MARK, b"\0"].concat()).unwrap();
        std::fs::write(&v2, [b"\0junk".as_slice(), V2_MARK, b"\0"].concat()).unwrap();
        std::fs::write(&other, b"\0a future lsfg-vk\0").unwrap();
        assert_eq!(
            detect([v1.as_path()].into_iter()),
            Install::Reads(Format::V1)
        );
        assert_eq!(
            detect([v2.as_path(), v1.as_path()].into_iter()),
            Install::Reads(Format::V2)
        );
        assert_eq!(detect([other.as_path()].into_iter()), Install::Unknown);
        assert_eq!(
            detect([dir.join("none.so").as_path()].into_iter()),
            Install::Missing
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// What this module writes is what the installed lsfg-vk accepts, asked
    /// of lsfg-vk itself where its validator is installed.
    #[test]
    fn the_installed_lsfg_vk_accepts_what_is_written() {
        let Install::Reads(format) = installed() else {
            return;
        };
        if format != Format::V2 || !Path::new("/usr/bin/lsfg-vk-cli").is_file() {
            return;
        }
        let dir = std::env::temp_dir().join(format!("bgm-fg-validate-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut one_game = Table::new();
        upsert(
            format,
            &mut one_game,
            "Game Name With Spaces.exe",
            values(3, 75, 2),
        )
        .unwrap();
        let mut converted: Table = "version = 1\ngame = []\ntweak = 1\n[global]\ndll = \"/d\"\n"
            .parse()
            .unwrap();
        migrate(format, &mut converted);
        for (name, t) in [
            ("none", Table::new()),
            ("one", one_game),
            ("converted", converted),
        ] {
            let path = dir.join(format!("{name}.toml"));
            write_table(&path, &finished(format, t)).unwrap();
            let out = std::process::Command::new("/usr/bin/lsfg-vk-cli")
                .args(["validate", "-c"])
                .arg(&path)
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "{name}: {}{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            );
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
