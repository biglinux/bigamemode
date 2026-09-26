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
//! at x2's cost, → x3 applied). BiGame-mode writes per-game entries there.
//!
//! The format is the one lsfg-vk 1.0.0 — the package `BigLinux` ships — reads,
//! checked against the strings of its `liblsfg-vk.so`:
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
//! Three properties of that parser shape everything here:
//!
//! * One invalid entry makes lsfg-vk ignore the **whole** file ("Global
//!   Multiplier cannot be less than 2" … "IGNORING"), so a game with frame
//!   generation off has **no** entry — never `multiplier = 1`.
//! * It knows nothing of the `[[profile]]`/`active_in` layout an earlier
//!   version of this module wrote; that layout did nothing with lsfg-vk 1.0
//!   and made it ignore the whole file. It is converted when the application
//!   starts ([`convert_legacy_file`]) and on every write.
//! * Keys and entries BiGame-mode did not write are kept as they are: the file
//!   is also the user's, and lsfg-vk-ui's.
//!
//! BiGame-mode stores `flow_scale` as percent (25–100); lsfg-vk as 0.25–1.0.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use anyhow::{Context, Result};
use toml::{Table, Value};

use crate::models::{FrameGenBackend, FrameGenSettings};

// ── Paths ───────────────────────────────────────────────────────────────────

/// `$XDG_CONFIG_HOME/lsfg-vk/conf.toml`, where lsfg-vk reads it.
#[must_use]
pub fn config_path() -> PathBuf {
    crate::paths::config_home().join("lsfg-vk/conf.toml")
}

/// Entries BiGame-mode wrote, and those set aside while frame generation is
/// turned off globally.
fn state_path() -> PathBuf {
    crate::paths::state_home().join("bigame-mode/lsfg-vk.toml")
}

/// lsfg-vk's library, where the packages install it.
const LIBRARY: &[&str] = &["/usr/lib/liblsfg-vk.so", "/usr/local/lib/liblsfg-vk.so"];

// ── Format support ──────────────────────────────────────────────────────────

/// Whether the installed lsfg-vk reads the format written here, from a string
/// only its 1.x parser contains. `None` when no library is installed.
fn format_supported() -> Option<bool> {
    static SUPPORTED: OnceLock<Option<bool>> = OnceLock::new();
    *SUPPORTED.get_or_init(|| {
        let lib = LIBRARY.iter().find(|p| Path::new(p).is_file())?;
        let bytes = std::fs::read(lib).ok()?;
        let needle = b"Game override missing 'exe' field";
        Some(bytes.windows(needle.len()).any(|w| w == needle))
    })
}

fn ensure_format_supported() -> Result<()> {
    anyhow::ensure!(
        format_supported() != Some(false),
        "the installed lsfg-vk uses a configuration format BiGame-mode does not write \
         (it writes lsfg-vk 1.x's); change it in lsfg-vk-ui instead"
    );
    Ok(())
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
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    let text = toml::to_string_pretty(table).context("serialize")?;
    // Written whole and renamed, so lsfg-vk's reload never sees half a file.
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, text).with_context(|| format!("write {}", tmp.display()))?;
    std::fs::rename(&tmp, path).with_context(|| format!("replace {}", path.display()))
}

/// lsfg-vk's file, with anything an earlier BiGame-mode left there converted.
fn read_config() -> Result<Table> {
    let mut t = read_table(&config_path())?;
    migrate_legacy(&mut t);
    Ok(t)
}

/// Convert the file if it still has the layout an earlier BiGame-mode
/// wrote, keeping a copy of it beside it (`conf.toml.bigame-legacy`). While
/// it is in that layout lsfg-vk ignores the whole file, so every game started
/// meanwhile would run without frame generation. Returns whether it changed
/// anything.
///
/// # Errors
/// Returns an error if the file cannot be read, backed up or written.
pub fn convert_legacy_file() -> Result<bool> {
    let path = config_path();
    let original = read_table(&path)?;
    if !original.contains_key("profile") || format_supported() == Some(false) {
        return Ok(false);
    }
    let backup = path.with_extension("toml.bigame-legacy");
    if !backup.exists() {
        std::fs::copy(&path, &backup)
            .with_context(|| format!("back up to {}", backup.display()))?;
    }
    let mut t = original;
    migrate_legacy(&mut t);
    write_config(&t)?;
    Ok(true)
}

fn write_config(t: &Table) -> Result<()> {
    let mut t = t.clone();
    t.insert("version".into(), Value::Integer(1));
    write_table(&config_path(), &t)
}

/// The `[[profile]]` layout an earlier BiGame-mode wrote, as `[[game]]`
/// entries; `allow_fp16`, which lsfg-vk 1.0 does not read, is dropped.
fn migrate_legacy(t: &mut Table) {
    if let Some(Value::Array(old)) = t.remove("profile") {
        for p in old.iter().filter_map(Value::as_table) {
            let Some(exe) = p
                .get("active_in")
                .and_then(Value::as_array)
                .and_then(|a| a.first())
                .and_then(Value::as_str)
            else {
                continue;
            };
            let mult = p.get("multiplier").and_then(Value::as_integer).unwrap_or(1);
            if mult < 2 || find(t, exe).is_some() {
                continue;
            }
            let mut g = Table::new();
            g.insert("exe".into(), exe.into());
            g.insert("multiplier".into(), Value::Integer(mult.min(20)));
            if let Some(f) = p.get("flow_scale").and_then(Value::as_float) {
                // Older versions wrote it through an f32 (0.6000000238418579);
                // the UI works in whole percent.
                let f = (f.clamp(0.25, 1.0) * 100.0).round() / 100.0;
                g.insert("flow_scale".into(), Value::Float(f));
            }
            for (from, to) in [
                ("performance_mode", "performance_mode"),
                ("hdr", "hdr_mode"),
            ] {
                if let Some(b) = p.get(from).and_then(Value::as_bool) {
                    g.insert(to.into(), Value::Boolean(b));
                }
            }
            games_mut(t).push(Value::Table(g));
        }
    }
    if let Some(Value::Table(global)) = t.get_mut("global") {
        global.remove("allow_fp16");
    }
}

fn games_mut(t: &mut Table) -> &mut Vec<Value> {
    let v = t.entry("game").or_insert_with(|| Value::Array(Vec::new()));
    if !v.is_array() {
        *v = Value::Array(Vec::new());
    }
    v.as_array_mut().expect("just made an array")
}

/// Index of the `[[game]]` entry for `exe`.
fn find(t: &Table, exe: &str) -> Option<usize> {
    t.get("game")?
        .as_array()?
        .iter()
        .position(|g| g.get("exe").and_then(Value::as_str) == Some(exe))
}

fn entry<'a>(t: &'a Table, exe: &str) -> Option<&'a Table> {
    let i = find(t, exe)?;
    t.get("game")?.as_array()?.get(i)?.as_table()
}

/// Remove and return the entry for `exe`.
fn take(t: &mut Table, exe: &str) -> Option<Value> {
    let i = find(t, exe)?;
    Some(games_mut(t).remove(i))
}

/// The game names BiGame-mode manages, and the entries set aside.
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

// ── Entries ─────────────────────────────────────────────────────────────────

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

fn game_entry(
    exe: &str,
    multiplier: u32,
    flow_scale_pct: u32,
    perf_mode: bool,
    hdr: bool,
    present_mode: u32,
) -> Table {
    let mut g = Table::new();
    g.insert("exe".into(), exe.into());
    g.insert("multiplier".into(), Value::Integer(i64::from(multiplier)));
    g.insert(
        "flow_scale".into(),
        Value::Float(f64::from(flow_scale_pct) / 100.0),
    );
    g.insert("performance_mode".into(), Value::Boolean(perf_mode));
    g.insert("hdr_mode".into(), Value::Boolean(hdr));
    if let Some(p) = present_name(present_mode) {
        g.insert("experimental_present_mode".into(), p.into());
    }
    g
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
    ensure_format_supported()?;
    anyhow::ensure!(
        is_lossless_dll_ready(),
        "Lossless.dll not found in configured LSFG path"
    );
    anyhow::ensure!(
        (25..=100).contains(&flow_scale_pct),
        "flow_scale_pct must be 25–100"
    );
    let mut t = read_config()?;
    let new = game_entry(
        name,
        multiplier.clamp(2, 20),
        flow_scale_pct,
        perf_mode,
        hdr,
        present_mode,
    );
    match find(&t, name) {
        Some(i) => {
            // Keys lsfg-vk-ui or the user added to the entry stay.
            if let Some(Value::Table(old)) = games_mut(&mut t).get_mut(i) {
                old.remove("experimental_present_mode");
                old.extend(new);
            }
        }
        None => games_mut(&mut t).push(Value::Table(new)),
    }
    write_config(&t)?;
    let mut s = read_state();
    if !s.managed.iter().any(|m| m == name) {
        s.managed.push(name.to_owned());
    }
    s.paused
        .retain(|g| g.get("exe").and_then(Value::as_str) != Some(name));
    write_state(&s)
}

/// Remove the frame generation entry for a game (its profile was deleted).
///
/// # Errors
/// Returns error if the file cannot be read or written.
pub fn delete_profile(name: &str) -> Result<()> {
    disable_for_game(name)?;
    let mut s = read_state();
    s.managed.retain(|m| m != name);
    s.paused
        .retain(|g| g.get("exe").and_then(Value::as_str) != Some(name));
    write_state(&s)
}

/// Frame generation for `name`: `(multiplier, flow_scale_pct, perf_mode, hdr,
/// present_mode)`. Multiplier 1 means off (no entry).
#[must_use]
pub fn read_profile(name: &str) -> (u32, u32, bool, bool, u32) {
    let off = (1, 100, false, false, 1);
    let Ok(t) = read_config() else {
        return off;
    };
    let Some(g) = entry(&t, name) else {
        return off;
    };
    let multiplier = g
        .get("multiplier")
        .and_then(Value::as_integer)
        .and_then(|m| u32::try_from(m).ok())
        .unwrap_or(1);
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let flow = g
        .get("flow_scale")
        .and_then(Value::as_float)
        .map_or(100, |f| ((f * 100.0).round() as u32).clamp(25, 100));
    (
        multiplier,
        flow,
        g.get("performance_mode")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        g.get("hdr_mode").and_then(Value::as_bool).unwrap_or(false),
        present_index(g.get("experimental_present_mode").and_then(Value::as_str)),
    )
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
        || LIBRARY.iter().any(|p| Path::new(p).is_file())
}

/// Whether a `Lossless.dll` is configured and exists.
#[must_use]
pub fn is_lossless_dll_ready() -> bool {
    read_global_dll().is_some_and(|p| Path::new(&p).is_file())
}

/// Set (or with `None` remove) `[global].dll`.
///
/// # Errors
/// Returns error if the file cannot be read or written.
pub fn write_global_dll(dll: Option<String>) -> Result<()> {
    let mut t = read_config()?;
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
    write_config(&t)
}

/// Whether the global video settings allow lsfg-vk.
#[must_use]
pub fn global_state_allows_lsfg(frame_gen: &FrameGenSettings) -> bool {
    frame_gen.enabled && frame_gen.backend == FrameGenBackend::LsfgVk
}

/// Bring lsfg-vk's file in line with the global switch: off sets
/// BiGame-mode's entries aside, on puts them back. Returns whether anything
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

/// Set aside every entry BiGame-mode wrote (the global switch turned off).
/// They are kept, and [`sync_global_enablement`] puts them back.
///
/// # Errors
/// Returns error if the files cannot be read or written.
pub fn disable_all_profiles() -> Result<bool> {
    let mut t = read_config()?;
    let mut s = read_state();
    let mut changed = false;
    for name in s.managed.clone() {
        if let Some(g) = take(&mut t, &name) {
            s.paused.push(g);
            changed = true;
        }
    }
    if changed {
        write_config(&t)?;
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
    ensure_format_supported()?;
    let mut t = read_config()?;
    for g in std::mem::take(&mut s.paused) {
        let exe = g
            .get("exe")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        if !exe.is_empty() && find(&t, &exe).is_none() {
            games_mut(&mut t).push(g);
        }
    }
    write_config(&t)?;
    write_state(&s)?;
    Ok(true)
}

/// Turn frame generation off for one game: its entry is removed.
///
/// # Errors
/// Returns error if the file cannot be read or written.
pub fn disable_for_game(name: &str) -> Result<()> {
    let mut t = read_config()?;
    if take(&mut t, name).is_some() {
        write_config(&t)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{FrameGenBackend, FrameGenSettings};

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
        let g = game_entry("SOTTR.exe", 2, 70, true, false, 0);
        let text = toml::to_string(&g).unwrap();
        assert!(text.contains("exe = \"SOTTR.exe\""), "{text}");
        assert!(text.contains("multiplier = 2"));
        assert!(text.contains("flow_scale = 0.7"));
        assert!(text.contains("experimental_present_mode = \"fifo\""));
        // The recommended mode is lsfg-vk's own choice: no key at all.
        assert!(
            !toml::to_string(&game_entry("x", 2, 100, false, false, 1))
                .unwrap()
                .contains("present_mode")
        );
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
        migrate_legacy(&mut t);
        assert!(!t.contains_key("profile"));
        assert_eq!(t["tweak"].as_str(), Some("kept"));
        assert!(!t["global"].as_table().unwrap().contains_key("allow_fp16"));
        assert_eq!(t["global"]["dll"].as_str(), Some("/home/u/Lossless.dll"));
        let g = entry(&t, "SOTTR.exe").unwrap();
        assert_eq!(g["multiplier"].as_integer(), Some(3));
        // Written through an f32 by the old code; back to whole percent.
        assert_eq!(g["flow_scale"].as_float(), Some(0.6));
        // multiplier 1 would make lsfg-vk reject the whole file.
        assert!(entry(&t, "Off.exe").is_none());
        assert!(entry(&t, "vkcube").is_some(), "a user's entry is kept");
    }

    #[test]
    fn present_modes_round_trip() {
        for i in 0..4 {
            assert_eq!(present_index(present_name(i)), i);
        }
    }
}
