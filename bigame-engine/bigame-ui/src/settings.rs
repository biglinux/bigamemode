//! User settings persistence (window geometry, last tab, theme).
//!
//! Settings stored as TOML in `$XDG_CONFIG_HOME/bigame-mode/settings.toml`.
//!
//! The theme keys carry a history. Before there was a choice, `theme` and
//! `color_scheme` did not exist, and their absence in a file *is* a choice:
//! the Default design and the desktop's colour scheme. A new installation
//! opens in Gamer + Dark, so that is what a *missing file* means — never a
//! missing key. [`load`] tells the two apart, and the first save writes the
//! values out, so the file's meaning stays fixed from then on.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Persisted user preferences.
///
/// A flat set of independent switches, which is what a preferences file is;
/// grouping them to satisfy a lint would only add indirection.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    /// The interface design (`"gamer"`); absent, the default design
    /// (theme.rs).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub theme: Option<String>,
    /// Enable desktop notifications on game launch/exit.
    pub notifications_enabled: bool,
    /// Ping target for network latency telemetry.
    pub ping_target: String,
    /// Offer a profile when a game falcond has no profile for starts.
    pub offer_profiles: bool,
}

impl Default for Settings {
    /// What a *key* means when it is missing from a file that exists.
    fn default() -> Self {
        Self {
            window_width: 800,
            window_height: 600,
            maximized: false,
            // Home is where a first start lands: Turbo is the one thing a
            // beginner needs (views/home.rs).
            last_tab: String::from("home"),
            color_scheme: None,
            theme: None,
            notifications_enabled: true,
            ping_target: String::from("1.1.1.1"),
            offer_profiles: true,
        }
    }
}

impl Settings {
    /// What a new installation gets: the Gamer design in its dark scheme.
    /// Everything else is as [`Settings::default`].
    #[must_use]
    pub fn first_run() -> Self {
        Self {
            theme: Some("gamer".to_owned()),
            color_scheme: Some("dark".to_owned()),
            ..Self::default()
        }
    }

    /// The settings a file's text means: a file that does not parse is
    /// treated as absent rather than as a file with every key missing, so a
    /// damaged file cannot turn an existing user's Default into Gamer.
    #[must_use]
    pub fn from_file(text: Option<&str>) -> Self {
        match text {
            Some(t) => toml::from_str(t).unwrap_or_else(|_| Self::first_run()),
            None => Self::first_run(),
        }
    }
}

/// Settings file path: `$XDG_CONFIG_HOME/bigame-mode/settings.toml`.
fn settings_path() -> PathBuf {
    bigame_core::paths::config_home()
        .join("bigame-mode")
        .join("settings.toml")
}

/// Load settings from disk. With no file — a first run — the first-run
/// defaults; with a file, its keys, the missing ones at their old meaning.
#[must_use]
pub fn load() -> Settings {
    let path = settings_path();
    Settings::from_file(std::fs::read_to_string(&path).ok().as_deref())
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::{Design, Scheme};

    fn theme_of(s: &Settings) -> (Design, Scheme) {
        (
            Design::from_setting(s.theme.as_deref()),
            Scheme::from_setting(s.color_scheme.as_deref()),
        )
    }

    #[test]
    fn a_new_installation_opens_in_gamer_dark() {
        let s = Settings::from_file(None);
        assert_eq!(theme_of(&s), (Design::Gamer, Scheme::Dark));
        assert_eq!(s.last_tab, "home");
    }

    #[test]
    fn an_older_file_without_the_keys_keeps_default_and_system() {
        // Written by a version that had no theme choice.
        let legacy = "window_width = 1024\nwindow_height = 700\nmaximized = false\n\
                      last_tab = \"dashboard\"\nnotifications_enabled = true\n\
                      ping_target = \"1.1.1.1\"\n";
        let s = Settings::from_file(Some(legacy));
        assert_eq!(theme_of(&s), (Design::Default, Scheme::System));
        assert_eq!((s.window_width, s.last_tab.as_str()), (1024, "dashboard"));
    }

    #[test]
    fn a_choice_survives_a_round_trip_through_the_file() {
        // Default: the absence of the key in a file that exists.
        let mut s = Settings::first_run();
        s.theme = Design::Default.to_setting();
        let text = toml::to_string_pretty(&s).unwrap();
        assert!(!text.contains("theme"), "{text}");
        assert_eq!(
            theme_of(&Settings::from_file(Some(&text))).0,
            Design::Default
        );

        // Light: written explicitly.
        s.color_scheme = Scheme::Light.to_setting();
        let text = toml::to_string_pretty(&s).unwrap();
        assert_eq!(theme_of(&Settings::from_file(Some(&text))).1, Scheme::Light);

        // Gamer + Dark from a first run, saved once, read back the same.
        let text = toml::to_string_pretty(&Settings::first_run()).unwrap();
        assert!(text.contains("theme = \"gamer\"") && text.contains("color_scheme = \"dark\""));
        assert_eq!(
            theme_of(&Settings::from_file(Some(&text))),
            (Design::Gamer, Scheme::Dark)
        );
    }

    #[test]
    fn a_file_that_does_not_parse_is_a_first_run_not_a_legacy_file() {
        // Neither meaning is certain; first-run defaults lose nothing that
        // could be read.
        assert_eq!(
            Settings::from_file(Some("not = [toml")),
            Settings::first_run()
        );
    }
}
