//! falcond game profile management (CRUD + sync).

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// Default system profiles directory.
pub const SYSTEM_PROFILES_DIR: &str = "/usr/share/falcond/profiles";

/// User override profiles directory.
pub const USER_PROFILES_DIR: &str = "/usr/share/falcond/profiles/user";

/// A falcond game profile.
///
/// Mirrors falcond's own on-disk shape, which is a flat list of independent
/// switches. Restructuring it here would only make the round trip harder to
/// verify against the file falcond actually reads.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameProfile {
    /// Executable/process name to match.
    pub name: String,
    /// Enable performance mode for this game.
    #[serde(default)]
    pub performance_mode: bool,
    /// sched-ext scheduler (none, bpfland, lavd, rusty, flash).
    #[serde(default)]
    pub scx_sched: String,
    /// Scheduler mode (default, gaming, power, latency, server).
    #[serde(default)]
    pub scx_sched_props: String,
    /// `VCache` mode (none, cache, freq).
    #[serde(default)]
    pub vcache_mode: String,
    /// Script to run when game starts.
    pub start_script: Option<String>,
    /// Script to run when game stops.
    pub stop_script: Option<String>,
    /// Suppress screensaver while running.
    #[serde(default)]
    pub idle_inhibit: bool,
    /// CPU frequency governor override (empty = no change).
    #[serde(default)]
    pub cpu_governor: String,
    /// Custom sched-ext flags (e.g. `--slice-us=800 --verbose`).
    #[serde(default)]
    pub scx_custom_flags: String,
    /// Is this profile active?
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    /// Per-game Gamescope configuration (None = use global defaults).
    #[serde(default)]
    pub gamescope: Option<crate::gamescope::Config>,
    /// Whether Gamescope wraps this game: automatically, always, or never.
    ///
    /// Defaults to `Auto`, so profiles written before this field existed keep
    /// working and get the decision made for them.
    #[serde(default)]
    pub gamescope_mode: crate::gamescope::Mode,
    /// Frame generation multiplier (1-4).
    #[serde(default = "default_fg_multiplier")]
    pub fg_multiplier: u32,
    /// Optical flow scale (0-100).
    #[serde(default = "default_fg_flow_scale")]
    pub fg_flow_scale: u32,
    /// Performance mode for frame generation.
    #[serde(default)]
    pub fg_perf_mode: bool,
    /// OTF quality preset: 0=Performance, 1=Balanced, 2=Quality.
    #[serde(default = "default_fg_quality")]
    pub fg_quality: u32,
    /// Path to custom Lossless.dll.
    pub fg_dll_path: Option<String>,
    /// HDR support for frame generation.
    #[serde(default)]
    pub fg_hdr: bool,
    /// Present mode for frame generation (0=VSync/FIFO, 1=Mailbox, 2=Immediate).
    #[serde(default)]
    pub fg_present_mode: u32,
    /// Keys this build does not recognise, preserved verbatim.
    ///
    /// falcond gains fields faster than this project can track them — 2.0.8
    /// added `dmem_protect` and `disable_split_lock`, neither of which older
    /// BiGame-mode builds knew about. Without this, opening such a profile in
    /// the editor and pressing Save would silently delete them, because
    /// serialization only emitted the fields it happened to know.
    ///
    /// A `BTreeMap` keeps the output order stable so a save with no edits
    /// produces no diff.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub extra: std::collections::BTreeMap<String, String>,
}

fn default_enabled() -> bool {
    true
}
fn default_fg_multiplier() -> u32 {
    1
}
fn default_fg_flow_scale() -> u32 {
    100
}
fn default_fg_quality() -> u32 {
    1
}

impl Default for GameProfile {
    fn default() -> Self {
        Self {
            name: String::new(),
            enabled: true,
            performance_mode: true,
            scx_sched: "none".into(),
            scx_sched_props: "default".into(),
            vcache_mode: "none".into(),
            start_script: None,
            stop_script: None,
            idle_inhibit: false,
            cpu_governor: String::new(),
            scx_custom_flags: String::new(),
            gamescope: None,
            gamescope_mode: crate::gamescope::Mode::default(),
            fg_multiplier: 1,
            fg_flow_scale: 100,
            fg_perf_mode: false,
            fg_quality: 1,
            fg_dll_path: None,
            fg_hdr: false,
            fg_present_mode: 0,
            extra: std::collections::BTreeMap::new(),
        }
    }
}

/// Validate a profile and return a list of warnings (empty = valid).
///
/// Checks for common misconfigurations that would prevent falcond from
/// applying the profile correctly.
#[must_use]
pub fn validate(profile: &GameProfile) -> Vec<String> {
    let mut warnings = Vec::new();

    if profile.name.trim().is_empty() {
        warnings.push("Profile name is required".into());
    }
    if profile.name.contains(std::path::MAIN_SEPARATOR) || profile.name.contains("..") {
        warnings.push("Profile name contains invalid path characters".into());
    }
    // Scheduler mode without scheduler selected
    if profile.scx_sched == "none" && profile.scx_sched_props != "default" {
        warnings.push("Scheduler mode set but no scheduler selected".into());
    }
    // Custom flags without scheduler
    if profile.scx_sched == "none" && !profile.scx_custom_flags.trim().is_empty() {
        warnings.push("Custom sched-ext flags set but no scheduler selected".into());
    }
    // VCache on non-AMD
    if profile.vcache_mode != "none" && !crate::vcache::is_available() {
        warnings.push("VCache mode set but AMD 3D V-Cache not detected".into());
    }
    // Gamescope resolution sanity. Zero on *both* axes is valid and means
    // "let Gamescope follow the game"; only a half-specified resolution is
    // wrong, because it makes Gamescope infer the wrong aspect ratio.
    if let Some(ref gs) = profile.gamescope {
        if (gs.render_width == 0) != (gs.render_height == 0) {
            warnings.push("Gamescope render resolution needs both width and height".into());
        }
        if (gs.output_width == 0) != (gs.output_height == 0) {
            warnings.push("Gamescope output resolution needs both width and height".into());
        }
    }
    // Script paths: check they look like absolute paths
    if let Some(ref s) = profile.start_script {
        if !s.starts_with('/') {
            warnings.push("Start script should be an absolute path".into());
        }
    }
    if let Some(ref s) = profile.stop_script {
        if !s.starts_with('/') {
            warnings.push("Stop script should be an absolute path".into());
        }
    }
    warnings
}

/// Hard errors that MUST block saving.
///
/// Subset of `validate()`: only checks that make the profile completely unusable
/// (empty/invalid name, zero Gamescope resolution). Advisory checks like
/// VCache-on-non-AMD or missing scheduler are returned by `validate()` but
/// should not prevent the user from saving.
#[must_use]
pub fn critical_errors(profile: &GameProfile) -> Vec<String> {
    let mut errors = Vec::new();
    if profile.name.trim().is_empty() {
        errors.push("Profile name is required".into());
    }
    if profile.name.contains(std::path::MAIN_SEPARATOR) || profile.name.contains("..") {
        errors.push("Profile name contains invalid path characters".into());
    }
    if let Some(ref gs) = profile.gamescope {
        if (gs.render_width == 0) != (gs.render_height == 0)
            || (gs.output_width == 0) != (gs.output_height == 0)
        {
            errors.push("Gamescope resolution needs both width and height".into());
        }
    }
    errors
}

/// List all profile names from system + user directories.
///
/// User profiles override system ones (same filename = same profile).
#[must_use]
pub fn list_names() -> Vec<String> {
    let mut names = Vec::new();
    for dir in [Path::new(USER_PROFILES_DIR), Path::new(SYSTEM_PROFILES_DIR)] {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().is_some_and(|e| e == "conf") {
                    if let Some(stem) = path.file_stem() {
                        let name = stem.to_string_lossy().into_owned();
                        if !names.contains(&name) {
                            names.push(name);
                        }
                    }
                }
            }
        }
    }
    names.sort();
    names
}

/// Load a profile by name. Checks user dir first, then system.
///
/// Supports both TOML (quoted strings) and `otter_conf` (bare identifiers) formats.
///
/// # Errors
/// Returns error if file is unreadable or unparseable.
pub fn load(name: &str) -> Result<GameProfile> {
    let path = resolve_path(name);
    let content = std::fs::read_to_string(&path)
        .with_context(|| format!("read profile: {}", path.display()))?;
    // Try TOML first (backwards compat)
    if let Ok(p) = toml::from_str::<GameProfile>(&content) {
        return Ok(p);
    }
    // Fall back to otter_conf key=value parsing
    Ok(parse_profile_otter_conf(&content))
}

/// Parse a profile from `otter_conf` format (bare identifiers for enums).
fn parse_profile_otter_conf(content: &str) -> GameProfile {
    let mut p = GameProfile::default();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, val)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        let val = val.trim().trim_matches('"');
        match key {
            "name" => p.name = val.to_string(),
            "performance_mode" => p.performance_mode = val == "true",
            "scx_sched" => p.scx_sched = val.to_string(),
            "scx_sched_props" => p.scx_sched_props = val.to_string(),
            "vcache_mode" => p.vcache_mode = val.to_string(),
            "idle_inhibit" => p.idle_inhibit = val == "true",
            "start_script" => {
                if !val.is_empty() {
                    p.start_script = Some(val.to_string());
                }
            }
            "stop_script" => {
                if !val.is_empty() {
                    p.stop_script = Some(val.to_string());
                }
            }
            "cpu_governor" => p.cpu_governor = val.to_string(),
            "scx_custom_flags" => p.scx_custom_flags = val.to_string(),
            "enabled" => p.enabled = val == "true",
            "fg_multiplier" => p.fg_multiplier = val.parse().unwrap_or(1),
            "fg_flow_scale" => p.fg_flow_scale = val.parse().unwrap_or(100),
            "fg_perf_mode" => p.fg_perf_mode = val == "true",
            "fg_quality" => p.fg_quality = val.parse().unwrap_or(1),
            "fg_dll_path" => {
                if !val.is_empty() {
                    p.fg_dll_path = Some(val.to_string());
                }
            }
            "fg_hdr" => p.fg_hdr = val == "true",
            "gamescope_mode" => {
                p.gamescope_mode = match val {
                    "enabled" => crate::gamescope::Mode::Enabled,
                    "disabled" => crate::gamescope::Mode::Disabled,
                    _ => crate::gamescope::Mode::Auto,
                };
            }
            "fg_present_mode" => p.fg_present_mode = val.parse().unwrap_or(0),
            // Anything this build does not know is kept so saving cannot
            // destroy a falcond feature we have not caught up with yet.
            other => {
                p.extra.insert(other.to_owned(), val.to_owned());
            }
        }
    }
    p
}

/// Serialize a game profile to `otter_conf` format (bare identifiers for enums).
///
/// Only emits fields that falcond's `UserProfileConfig` / `ProfileConfig` understand.
/// Extra UI-only fields (`enabled`, `scx_custom_flags`, `fg_*`, `gamescope`) are
/// appended with quotes so `otter_conf` skips them (unknown fields are ignored).
fn serialize_profile_otter_conf(profile: &GameProfile) -> String {
    let mut out = String::new();
    // name: always a quoted string
    let _ = writeln!(out, "name = \"{}\"", profile.name);
    // Booleans: bare
    let _ = writeln!(out, "performance_mode = {}", profile.performance_mode);
    // Enums: bare identifiers (no quotes!)
    let _ = writeln!(out, "scx_sched = {}", profile.scx_sched);
    let _ = writeln!(out, "scx_sched_props = {}", profile.scx_sched_props);
    let _ = writeln!(out, "vcache_mode = {}", profile.vcache_mode);
    let _ = writeln!(out, "idle_inhibit = {}", profile.idle_inhibit);
    // Strings: quoted
    if let Some(ref s) = profile.start_script {
        if !s.is_empty() {
            let _ = writeln!(out, "start_script = \"{s}\"");
        }
    }
    if let Some(ref s) = profile.stop_script {
        if !s.is_empty() {
            let _ = writeln!(out, "stop_script = \"{s}\"");
        }
    }
    if !profile.cpu_governor.is_empty() {
        let _ = writeln!(out, "cpu_governor = \"{}\"", profile.cpu_governor);
    }
    // UI-only fields (otter_conf ignores unknown keys via skipValue)
    let _ = writeln!(out, "scx_custom_flags = \"{}\"", profile.scx_custom_flags);
    let _ = writeln!(out, "enabled = {}", profile.enabled);
    let _ = writeln!(out, "fg_multiplier = {}", profile.fg_multiplier);
    let _ = writeln!(out, "fg_flow_scale = {}", profile.fg_flow_scale);
    let _ = writeln!(out, "fg_perf_mode = {}", profile.fg_perf_mode);
    let _ = writeln!(out, "fg_quality = {}", profile.fg_quality);
    if let Some(ref s) = profile.fg_dll_path {
        let _ = writeln!(out, "fg_dll_path = \"{s}\"");
    }
    let _ = writeln!(out, "fg_hdr = {}", profile.fg_hdr);
    let _ = writeln!(out, "fg_present_mode = {}", profile.fg_present_mode);
    let _ = writeln!(
        out,
        "gamescope_mode = \"{}\"",
        match profile.gamescope_mode {
            crate::gamescope::Mode::Auto => "auto",
            crate::gamescope::Mode::Enabled => "enabled",
            crate::gamescope::Mode::Disabled => "disabled",
        }
    );
    // Unrecognised keys, written back exactly as they were read.
    for (key, value) in &profile.extra {
        let _ = writeln!(out, "{key} = {value}");
    }
    out
}

/// Save a profile to the user directory via D-Bus.
///
/// Synchronous on purpose. It was `async` while containing no `await` — it uses
/// the blocking proxy throughout — and that mismatch caused a real bug: a call
/// site wrote `let _ = profiles::delete(&name)` inside a blocking closure,
/// which built a future and dropped it. The button reported "Profile deleted"
/// and nothing was deleted. A function that cannot suspend should not claim it
/// might.
///
/// # Errors
/// Returns an error if serialization or the D-Bus call fails.
pub fn save(profile: &GameProfile) -> Result<()> {
    let content = serialize_profile_otter_conf(profile);

    // Use blocking proxy to avoid requiring a Tokio reactor in GTK main-thread flows.
    let proxy = crate::dbus_client::daemon_proxy_blocking()?;
    proxy.save_profile(&profile.name, &content)?;

    // Sync FG parameters to ~/.config/lsfg-vk/conf.toml (best-effort).
    // Do not fail profile save if lsfg-vk config is invalid/incompatible.
    // This keeps profile creation reliable even when external FG config is broken.
    if let Err(e) = crate::fg::write_profile(
        &profile.name,
        profile.fg_multiplier,
        profile.fg_flow_scale,
        profile.fg_perf_mode,
        profile.fg_hdr,
        profile.fg_present_mode,
    ) {
        tracing::warn!(
            profile = %profile.name,
            error = %e,
            "failed to sync lsfg-vk profile; profile save will continue"
        );
    }

    // The profile's `cpu_governor` is deliberately NOT applied here.
    //
    // It is a *per-game* setting, and falcond applies it when the game starts.
    // Writing it at save time changed the governor system-wide, immediately,
    // with no record of the previous value and no way back — so merely editing
    // a profile silently repinned every core on the machine.
    Ok(())
}

/// Delete a user profile by name via D-Bus.
///
/// Synchronous for the same reason as [`save`].
///
/// # Errors
/// Returns an error if the profile does not exist or the D-Bus call fails.
pub fn delete(name: &str) -> Result<()> {
    let path = user_path(name);
    anyhow::ensure!(path.exists(), "profile not found: {}", path.display());

    let proxy = crate::dbus_client::daemon_proxy_blocking()?;
    // The helper reloads falcond itself, through systemd. This used to shell
    // out to `sudo -n pkill -HUP falcond` from the GUI thread: it blocked the
    // main loop on a subprocess, signalled every process sharing the name, and
    // depended on a passwordless sudoers rule that has since been removed as a
    // root escalation. It also silently did nothing, because `sudo -n` already
    // failed on any normally configured machine.
    proxy.delete_profile(name)?;

    // Remove FG entry from lsfg-vk config (best-effort).
    let _ = crate::fg::delete_profile(name);

    Ok(())
}

/// Resolve profile path: user dir first, then system.
fn resolve_path(name: &str) -> PathBuf {
    let user = user_path(name);
    if user.exists() {
        return user;
    }
    system_path(name)
}

fn user_path(name: &str) -> PathBuf {
    Path::new(USER_PROFILES_DIR).join(format!("{name}.conf"))
}

fn system_path(name: &str) -> PathBuf {
    Path::new(SYSTEM_PROFILES_DIR).join(format!("{name}.conf"))
}

/// Check if a profile exists in the user directory (meaning it can be deleted/reverted).
#[must_use]
pub fn is_user_profile(name: &str) -> bool {
    user_path(name).exists()
}

/// Check if a profile exists in the system directory.
#[must_use]
pub fn is_system_profile(name: &str) -> bool {
    system_path(name).exists()
}

/// Export a profile to a local file (no root required).
///
/// # Errors
/// Returns error if the profile cannot be loaded or the target path is unwritable.
pub fn export(name: &str, dest: &Path) -> Result<()> {
    let profile = load(name)?;
    let ai = crate::game_settings::load(name)
        .map(|s| s.ai_graphics)
        .unwrap_or_default();
    let content = export_text(&profile, &ai)?;
    std::fs::write(dest, content).with_context(|| format!("write export: {}", dest.display()))
}

/// A profile as an export file: the profile, then — when AI Graphics was set
/// up for the game — its choices as `[ai_graphics]`. Only intent travels:
/// no paths, no installed files, nothing about this machine's update
/// offers; the game is analysed again wherever the file is imported.
///
/// # Errors
/// Returns an error if serialization fails.
pub fn export_text(
    profile: &GameProfile,
    ai: &crate::graphics::config::AiGraphicsConfig,
) -> Result<String> {
    let mut content = toml::to_string_pretty(profile).context("serialize profile for export")?;
    if *ai != crate::graphics::config::AiGraphicsConfig::default() {
        let portable = crate::graphics::config::AiGraphicsConfig {
            skipped_update: None,
            ..ai.clone()
        };
        #[derive(Serialize)]
        struct Section<'a> {
            ai_graphics: &'a crate::graphics::config::AiGraphicsConfig,
        }
        content.push('\n');
        content.push_str(
            &toml::to_string_pretty(&Section {
                ai_graphics: &portable,
            })
            .context("serialize AI Graphics for export")?,
        );
    }
    Ok(content)
}

/// The AI Graphics choices in an export file, if it has them.
///
/// # Errors
/// Returns an error if the section is there but does not parse.
pub fn imported_ai_graphics(
    content: &str,
) -> Result<Option<crate::graphics::config::AiGraphicsConfig>> {
    #[derive(Deserialize)]
    struct Section {
        #[serde(default)]
        ai_graphics: Option<crate::graphics::config::AiGraphicsConfig>,
    }
    Ok(toml::from_str::<Section>(content)
        .context("parse the AI Graphics section")?
        .ai_graphics)
}

/// Import a profile from a local TOML file into the user profiles directory.
///
/// # Errors
/// Returns error if the file is unreadable, contains invalid TOML, or `DBus` write fails.
pub fn import(src: &Path) -> Result<String> {
    let content =
        std::fs::read_to_string(src).with_context(|| format!("read import: {}", src.display()))?;
    let profile: GameProfile = toml::from_str(&content).context("parse imported profile TOML")?;
    anyhow::ensure!(!profile.name.is_empty(), "imported profile has no name");
    let name = profile.name.clone();
    let ai = imported_ai_graphics(&content)?;
    save(&profile)?;
    if let Some(ai) = ai {
        let mut settings = crate::game_settings::load(&name).unwrap_or_default();
        settings.ai_graphics = ai;
        crate::game_settings::save(&name, &settings)?;
    }
    Ok(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gamescope_mode_survives_the_otter_conf_round_trip() {
        use crate::gamescope::Mode;
        for mode in [Mode::Auto, Mode::Enabled, Mode::Disabled] {
            let p = GameProfile {
                name: "x".into(),
                gamescope_mode: mode,
                ..GameProfile::default()
            };
            let text = serialize_profile_otter_conf(&p);
            assert_eq!(parse_profile_otter_conf(&text).gamescope_mode, mode);
        }
        // A profile written before the field existed reads back as Auto.
        assert_eq!(
            parse_profile_otter_conf("name = \"x\"\n").gamescope_mode,
            Mode::Auto
        );
    }

    #[test]
    fn saving_preserves_falcond_fields_this_build_does_not_know() {
        // falcond 2.0.8 added dmem_protect and disable_split_lock. Opening such
        // a profile and saving it must not silently drop them.
        let original = "\
name = \"Cyberpunk2077.exe\"
performance_mode = true
vcache_mode = cache
dmem_protect = true
disable_split_lock = true
some_future_falcond_key = 42
";
        let parsed = parse_profile_otter_conf(original);
        assert_eq!(parsed.name, "Cyberpunk2077.exe");
        assert_eq!(
            parsed.extra.get("dmem_protect").map(String::as_str),
            Some("true")
        );
        assert_eq!(
            parsed.extra.get("disable_split_lock").map(String::as_str),
            Some("true")
        );
        assert_eq!(
            parsed
                .extra
                .get("some_future_falcond_key")
                .map(String::as_str),
            Some("42")
        );

        let written = serialize_profile_otter_conf(&parsed);
        assert!(written.contains("dmem_protect = true"));
        assert!(written.contains("disable_split_lock = true"));
        assert!(written.contains("some_future_falcond_key = 42"));

        // And the values survive a second round trip unchanged.
        let again = parse_profile_otter_conf(&written);
        assert_eq!(again.extra, parsed.extra);
    }

    #[test]
    fn unknown_keys_do_not_leak_into_known_fields() {
        let parsed = parse_profile_otter_conf("name = \"x\"\nvcache_mode = cache\n");
        assert_eq!(parsed.vcache_mode, "cache");
        assert!(parsed.extra.is_empty(), "known keys must not land in extra");
    }

    #[test]
    fn default_profile_values() {
        let p = GameProfile::default();
        assert!(p.name.is_empty());
        assert!(p.performance_mode);
        assert_eq!(p.scx_sched, "none");
        assert_eq!(p.scx_sched_props, "default");
        assert_eq!(p.vcache_mode, "none");
        assert!(p.start_script.is_none());
        assert!(p.stop_script.is_none());
        assert!(!p.idle_inhibit);
    }

    #[test]
    fn round_trip_serialization() {
        let p = GameProfile {
            name: "Cyberpunk2077.exe".into(),
            performance_mode: true,
            scx_sched: "bpfland".into(),
            scx_sched_props: "gaming".into(),
            vcache_mode: "cache".into(),
            start_script: Some("/opt/scripts/start.sh".into()),
            stop_script: None,
            idle_inhibit: true,
            cpu_governor: "performance".into(),
            scx_custom_flags: "--slice-us=800".into(),
            gamescope: None,
            ..Default::default()
        };
        let toml_str = toml::to_string_pretty(&p).unwrap();
        let parsed: GameProfile = toml::from_str(&toml_str).unwrap();
        assert_eq!(parsed.name, "Cyberpunk2077.exe");
        assert!(parsed.performance_mode);
        assert_eq!(parsed.scx_sched, "bpfland");
        assert_eq!(parsed.vcache_mode, "cache");
        assert_eq!(
            parsed.start_script.as_deref(),
            Some("/opt/scripts/start.sh")
        );
        assert!(parsed.stop_script.is_none());
        assert!(parsed.idle_inhibit);
    }

    #[test]
    fn partial_toml_uses_defaults() {
        let content = "name = \"test.exe\"\n";
        let p: GameProfile = toml::from_str(content).unwrap();
        assert_eq!(p.name, "test.exe");
        assert!(!p.performance_mode); // serde default = false (no custom default fn)
        assert_eq!(p.scx_sched, "");
        assert!(!p.idle_inhibit);
    }

    #[test]
    fn resolve_path_prefers_user() {
        // Without real directories, resolve_path falls back to system
        let rp = resolve_path("test_game");
        assert!(rp.to_string_lossy().contains("test_game.conf"));
    }

    #[test]
    fn an_export_carries_the_ai_graphics_choice_and_no_path() {
        use crate::graphics::config::{AiGraphicsConfig, Mode, Upscaler, VersionPolicy};
        let p = GameProfile {
            name: "SOTTR.exe".into(),
            ..GameProfile::default()
        };
        // Nothing set up: nothing written, and an old export reads as none.
        let plain = export_text(&p, &AiGraphicsConfig::default()).unwrap();
        assert!(!plain.contains("ai_graphics"));
        assert_eq!(imported_ai_graphics(&plain).unwrap(), None);

        let ai = AiGraphicsConfig {
            mode: Mode::Advanced,
            upscaler: Upscaler::Fsr,
            version: VersionPolicy::Pinned("0.9.4".into()),
            skipped_update: Some("0.9.5".into()),
            ..AiGraphicsConfig::default()
        };
        let text = export_text(&p, &ai).unwrap();
        assert!(!text.contains('/'), "portable: no paths\n{text}");
        assert!(!text.contains("skipped_update"), "{text}");
        let back: GameProfile = toml::from_str(&text).unwrap();
        assert_eq!(back.name, "SOTTR.exe");
        let back_ai = imported_ai_graphics(&text).unwrap().unwrap();
        assert_eq!(
            back_ai,
            AiGraphicsConfig {
                skipped_update: None,
                ..ai
            }
        );
    }

    #[test]
    fn export_import_round_trip() {
        let tmp = crate::tests::tempdir("export_import");
        let src = tmp.join("test_profile.conf");

        // Create a profile file manually
        let profile = GameProfile {
            name: "export_test".into(),
            performance_mode: true,
            scx_sched: "bpfland".into(),
            scx_sched_props: "gaming".into(),
            vcache_mode: "cache".into(),
            start_script: Some("/opt/start.sh".into()),
            stop_script: Some("/opt/stop.sh".into()),
            idle_inhibit: true,
            cpu_governor: "performance".into(),
            scx_custom_flags: "--verbose".into(),
            gamescope: None,
            ..Default::default()
        };
        let toml_str = toml::to_string_pretty(&profile).unwrap();
        std::fs::write(&src, &toml_str).unwrap();

        // Export path
        let export_dst = tmp.join("exported.toml");
        std::fs::write(&export_dst, &toml_str).unwrap();

        // Import back
        let content = std::fs::read_to_string(&export_dst).unwrap();
        let imported: GameProfile = toml::from_str(&content).unwrap();
        assert_eq!(imported.name, "export_test");
        assert_eq!(imported.scx_sched, "bpfland");
        assert_eq!(imported.scx_sched_props, "gaming");
        assert_eq!(imported.cpu_governor, "performance");
        assert_eq!(imported.scx_custom_flags, "--verbose");
        assert!(imported.idle_inhibit);
        assert_eq!(imported.start_script.as_deref(), Some("/opt/start.sh"));
        assert_eq!(imported.stop_script.as_deref(), Some("/opt/stop.sh"));
    }

    #[test]
    fn crud_file_operations() {
        let tmp = crate::tests::tempdir("crud_file");

        // CREATE: write profile to temp dir
        let profile = GameProfile {
            name: "crud_game".into(),
            performance_mode: true,
            scx_sched: "lavd".into(),
            scx_sched_props: "latency".into(),
            vcache_mode: "freq".into(),
            start_script: None,
            stop_script: None,
            idle_inhibit: false,
            cpu_governor: "schedutil".into(),
            scx_custom_flags: String::new(),
            gamescope: None,
            ..Default::default()
        };
        let path = tmp.join("crud_game.conf");
        let content = toml::to_string_pretty(&profile).unwrap();
        std::fs::write(&path, &content).unwrap();
        assert!(path.exists());

        // READ: load back
        let loaded: GameProfile = toml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(loaded.name, "crud_game");
        assert_eq!(loaded.scx_sched, "lavd");
        assert_eq!(loaded.cpu_governor, "schedutil");

        // UPDATE: modify and rewrite
        let mut updated = loaded;
        updated.scx_sched = "flash".into();
        updated.cpu_governor = "performance".into();
        updated.scx_custom_flags = "--slice-us=500".into();
        let updated_content = toml::to_string_pretty(&updated).unwrap();
        std::fs::write(&path, &updated_content).unwrap();

        let reloaded: GameProfile =
            toml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(reloaded.scx_sched, "flash");
        assert_eq!(reloaded.cpu_governor, "performance");
        assert_eq!(reloaded.scx_custom_flags, "--slice-us=500");

        // DELETE: remove file
        std::fs::remove_file(&path).unwrap();
        assert!(!path.exists());
    }

    #[test]
    fn list_profiles_in_temp_dir() {
        let tmp = crate::tests::tempdir("list_profiles");

        // Create 3 profile files
        for name in &["alpha", "beta", "gamma"] {
            let p = GameProfile {
                name: (*name).to_string(),
                ..Default::default()
            };
            let path = tmp.join(format!("{name}.conf"));
            std::fs::write(&path, toml::to_string_pretty(&p).unwrap()).unwrap();
        }

        // List .conf files in tmp dir
        let mut names: Vec<String> = std::fs::read_dir(&tmp)
            .unwrap()
            .flatten()
            .filter_map(|e| {
                let p = e.path();
                if p.extension().is_some_and(|x| x == "conf") {
                    p.file_stem().map(|s| s.to_string_lossy().into_owned())
                } else {
                    None
                }
            })
            .collect();
        names.sort();
        assert_eq!(names, vec!["alpha", "beta", "gamma"]);
    }

    #[test]
    fn new_fields_default_empty() {
        let content = "name = \"minimal\"\n";
        let p: GameProfile = toml::from_str(content).unwrap();
        assert_eq!(p.cpu_governor, "");
        assert_eq!(p.scx_custom_flags, "");
    }

    #[test]
    fn validate_empty_name() {
        let p = GameProfile::default();
        let w = super::validate(&p);
        assert!(w.iter().any(|s| s.contains("name is required")));
    }

    #[test]
    fn validate_path_traversal() {
        let p = GameProfile {
            name: "../etc/passwd".into(),
            ..Default::default()
        };
        let w = super::validate(&p);
        assert!(w.iter().any(|s| s.contains("invalid path")));
    }

    #[test]
    fn validate_sched_mode_without_scheduler() {
        let p = GameProfile {
            name: "test".into(),
            scx_sched: "none".into(),
            scx_sched_props: "gaming".into(),
            ..Default::default()
        };
        let w = super::validate(&p);
        assert!(w.iter().any(|s| s.contains("no scheduler selected")));
    }

    #[test]
    fn validate_custom_flags_without_scheduler() {
        let p = GameProfile {
            name: "test".into(),
            scx_sched: "none".into(),
            scx_custom_flags: "--verbose".into(),
            ..Default::default()
        };
        let w = super::validate(&p);
        assert!(w.iter().any(|s| s.contains("no scheduler selected")));
    }

    #[test]
    fn validate_relative_script_path() {
        let p = GameProfile {
            name: "test".into(),
            start_script: Some("relative/path.sh".into()),
            ..Default::default()
        };
        let w = super::validate(&p);
        assert!(w.iter().any(|s| s.contains("absolute path")));
    }

    #[test]
    fn validate_valid_profile() {
        let p = GameProfile {
            name: "Cyberpunk2077.exe".into(),
            performance_mode: true,
            scx_sched: "bpfland".into(),
            scx_sched_props: "gaming".into(),
            vcache_mode: "none".into(),
            start_script: Some("/opt/scripts/start.sh".into()),
            stop_script: None,
            idle_inhibit: true,
            cpu_governor: "performance".into(),
            scx_custom_flags: "--slice-us=800".into(),
            gamescope: None,
            ..Default::default()
        };
        let w = super::validate(&p);
        assert!(w.is_empty(), "expected no warnings, got: {w:?}");
    }
}
