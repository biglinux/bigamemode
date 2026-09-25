//! User settings persistence (window geometry, last tab).
//!
//! Settings stored as TOML in `$XDG_CONFIG_HOME/bigame-mode/settings.toml`.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Persisted user preferences.
///
/// A flat set of independent switches, which is what a preferences file is;
/// grouping them to satisfy a lint would only add indirection.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub window_width: i32,
    pub window_height: i32,
    pub maximized: bool,
    pub last_tab: String,
    /// The colour scheme the user chose from the menu (`"dark"` or
    /// `"light"`); absent, the desktop's is followed. (An older `dark_mode`
    /// key is ignored: it was never read or written.)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color_scheme: Option<String>,
    /// Enable desktop notifications on game launch/exit.
    pub notifications_enabled: bool,
    /// Ping target for network latency telemetry.
    pub ping_target: String,
    /// Offer a profile when a game falcond has no profile for starts.
    pub offer_profiles: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            window_width: 800,
            window_height: 600,
            maximized: false,
            // Home is where a first start lands: Turbo is the one thing a
            // beginner needs (views/home.rs).
            last_tab: String::from("home"),
            color_scheme: None,
            notifications_enabled: true,
            ping_target: String::from("1.1.1.1"),
            offer_profiles: true,
        }
    }
}

/// Settings file path: `$XDG_CONFIG_HOME/bigame-mode/settings.toml`.
fn settings_path() -> PathBuf {
    bigame_core::paths::config_home()
        .join("bigame-mode")
        .join("settings.toml")
}

/// Load settings from disk. Returns defaults on any error.
#[must_use]
pub fn load() -> Settings {
    let path = settings_path();
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| toml::from_str(&s).ok())
        .unwrap_or_default()
}

/// Save settings to disk. Silently ignores errors.
pub fn save(settings: &Settings) {
    let path = settings_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(content) = toml::to_string_pretty(settings) {
        let _ = std::fs::write(&path, content);
    }
}

/// The XDG autostart entry that starts BiGame-mode hidden at login.
fn autostart_path() -> Option<PathBuf> {
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(
        config
            .join("autostart")
            .join("com.biglinux.BiGameMode.desktop"),
    )
}

/// Whether BiGame-mode starts in the background at login.
#[must_use]
pub fn starts_at_login() -> bool {
    autostart_path().is_some_and(|p| p.exists())
}

/// Start, or stop starting, in the background at login.
///
/// A per-user XDG autostart entry, so nothing system-wide changes and
/// removing the file undoes it completely.
///
/// # Errors
/// Returns an error if the entry could not be written or removed.
pub fn set_starts_at_login(enabled: bool) -> std::io::Result<()> {
    let Some(path) = autostart_path() else {
        return Ok(());
    };
    if !enabled {
        return match std::fs::remove_file(&path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        };
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(
        path,
        "[Desktop Entry]\nType=Application\nName=BiGame-mode\nExec=bigame-ui --background\n\
         Icon=com.biglinux.BiGameMode\nNoDisplay=true\nX-GNOME-Autostart-enabled=true\n",
    )
}
