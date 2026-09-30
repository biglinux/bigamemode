//! Big Game Mode's own per-game settings, kept apart from falcond's profile.
//!
//! A falcond profile (`/usr/share/falcond/profiles/user/<process>.conf`) is
//! root-owned, written through the helper, and read by falcond, which uses
//! eight fields and ignores the rest. Big Game Mode's own per-game choices —
//! AI Graphics among them — are none of falcond's business, need no root to
//! change, and must not be mistaken for leftovers by the profile migration
//! (which drops fields falcond does not read). They live here, one small TOML
//! file per game in the user's configuration, keyed by the same process name
//! as the falcond profile.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::graphics::config::AiGraphicsConfig;

/// One game's Big Game Mode settings. Every field defaults, so a missing or
/// older file loads.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct GameSettings {
    /// AI Graphics.
    pub ai_graphics: AiGraphicsConfig,
    /// `MangoHud` for this game: off, on (Vulkan layer) or forced (wrapper).
    pub mangohud: crate::mangohud::Mode,
    /// The Gamescope wrapper Big Game Mode put into the game's Steam launch
    /// options, so it can replace or remove exactly that
    /// (`crate::steam_gamescope`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub steam_gamescope: Option<String>,
    /// Big Game Mode put `WINE_FULLSCREEN_FSR=0` into the game's Steam launch
    /// options: `OptiScaler` upscales it, and Wine FSR would be a second
    /// upscaler (`crate::steam_gamescope::set_wine_fsr_off`).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub steam_wine_fsr_off: bool,
    /// The variables Big Game Mode put in front of the game's Steam launch
    /// options for its own launch settings, so it can replace or remove
    /// exactly those (`crate::steam_gamescope`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub steam_env: Option<String>,
    /// The game's own launch settings (Gamescope, Wine FSR, vkBasalt), over
    /// Tuning's; what it leaves unset follows Tuning.
    #[serde(skip_serializing_if = "crate::game_launch::GameLaunch::is_empty")]
    pub launch: crate::game_launch::GameLaunch,
    /// Proton's FSR 4 upgrade for a game Heroic starts, from AI Graphics:
    /// its variables go into the game's settings in Heroic
    /// (`crate::heroic_launch`), as a Steam game's go into its launch
    /// options.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub heroic_fsr4_upgrade: bool,
    /// What Big Game Mode wrote into the game's settings in Heroic, one entry
    /// per settings file, so it can replace or remove exactly that
    /// (`crate::heroic_launch`).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub heroic: Vec<crate::heroic_launch::Written>,
}

/// The folder the per-game files are in.
#[must_use]
pub fn dir() -> PathBuf {
    crate::paths::config_home().join("bigame-mode/games")
}

/// Whether `name` can be a file name here: the same characters a profile name
/// may have, no path separators, not `.`/`..`.
fn valid(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && name != "."
        && name != ".."
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || " ._+-()".contains(c))
}

/// The file for the game whose process is `name`, in `folder`.
///
/// # Errors
/// Returns an error if `name` could name a path outside the folder.
pub fn path_in(folder: &Path, name: &str) -> Result<PathBuf> {
    anyhow::ensure!(valid(name), "not a usable game name: {name:?}");
    Ok(folder.join(format!("{name}.toml")))
}

/// The file for the game whose process is `name`.
///
/// # Errors
/// Returns an error if `name` could name a path outside the folder.
#[cfg(test)]
pub fn path(name: &str) -> Result<PathBuf> {
    path_in(&dir(), name)
}

/// Load the settings for `name`; defaults when there are none.
///
/// # Errors
/// Returns an error if the file exists but cannot be read or parsed — a
/// broken file is reported, not silently replaced with defaults.
pub fn load(name: &str) -> Result<GameSettings> {
    load_from(&dir(), name)
}

/// [`load`] from `folder`.
///
/// # Errors
/// As [`load`].
pub fn load_from(folder: &Path, name: &str) -> Result<GameSettings> {
    let p = path_in(folder, name)?;
    match std::fs::read_to_string(&p) {
        Ok(text) => toml::from_str(&text).with_context(|| format!("parse {}", p.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(GameSettings::default()),
        Err(e) => Err(e).with_context(|| format!("read {}", p.display())),
    }
}

/// Save the settings for `name`, atomically.
///
/// # Errors
/// Returns an error if the folder or file cannot be written.
pub fn save(name: &str, settings: &GameSettings) -> Result<()> {
    save_to(&dir(), name, settings)
}

/// [`save`] into `folder`.
///
/// # Errors
/// As [`save`].
pub fn save_to(folder: &Path, name: &str, settings: &GameSettings) -> Result<()> {
    use std::io::Write;
    let p = path_in(folder, name)?;
    let d = p.parent().context("no parent")?;
    std::fs::create_dir_all(d)?;
    let tmp = p.with_extension("toml.tmp");
    {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(toml::to_string_pretty(settings)?.as_bytes())?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, &p).with_context(|| format!("replace {}", p.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graphics::config::Mode;

    #[test]
    fn names_that_could_escape_the_folder_are_refused() {
        for bad in ["", ".", "..", "../x", "a/b", "a\\b", "x\0y"] {
            assert!(path(bad).is_err(), "{bad:?}");
        }
        for ok in ["SOTTR.exe", "Dead by Daylight", "PioneerGame.exe", "cs2"] {
            assert!(path(ok).is_ok(), "{ok:?}");
        }
    }

    #[test]
    fn settings_round_trip_and_a_missing_file_is_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        assert_eq!(load_from(d, "SOTTR.exe").unwrap(), GameSettings::default());
        let mut s = GameSettings::default();
        s.ai_graphics.mode = Mode::Recommended;
        save_to(d, "SOTTR.exe", &s).unwrap();
        assert_eq!(load_from(d, "SOTTR.exe").unwrap(), s);
        assert!(d.join("SOTTR.exe.toml").is_file());
        std::fs::write(d.join("bad.toml"), "ai_graphics = 3").unwrap();
        assert!(
            load_from(d, "bad").is_err(),
            "a broken file is reported, not replaced"
        );
    }

    #[test]
    fn a_file_from_before_a_games_own_launch_settings_still_loads() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        std::fs::write(
            d.join("SOTTR.exe.toml"),
            "mangohud = \"on\"\nsteam_gamescope = \"gamescope -f --\"\n\n[ai_graphics]\nmode = \"recommended\"\n",
        )
        .unwrap();
        let s = load_from(d, "SOTTR.exe").unwrap();
        assert_eq!(s.mangohud, crate::mangohud::Mode::On);
        assert_eq!(s.steam_gamescope.as_deref(), Some("gamescope -f --"));
        assert!(s.launch.is_empty() && s.steam_env.is_none());

        // Its own values round-trip, and none are written when there are none.
        let mut s = s;
        s.launch.render = Some((1280, 720));
        s.launch.vkbasalt = Some(true);
        save_to(d, "SOTTR.exe", &s).unwrap();
        assert_eq!(load_from(d, "SOTTR.exe").unwrap(), s);
        save_to(d, "plain", &GameSettings::default()).unwrap();
        let text = std::fs::read_to_string(d.join("plain.toml")).unwrap();
        assert!(!text.contains("launch"), "{text}");
    }
}
