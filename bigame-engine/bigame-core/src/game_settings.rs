//! BiGame-mode's own per-game settings, kept apart from falcond's profile.
//!
//! A falcond profile (`/usr/share/falcond/profiles/user/<process>.conf`) is
//! root-owned, written through the helper, and read by falcond, which uses
//! eight fields and ignores the rest. BiGame-mode's own per-game choices —
//! AI Graphics among them — are none of falcond's business, need no root to
//! change, and must not be mistaken for leftovers by the profile migration
//! (which drops fields falcond does not read). They live here, one small TOML
//! file per game in the user's configuration, keyed by the same process name
//! as the falcond profile.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::graphics::config::AiGraphicsConfig;

/// One game's BiGame-mode settings. Every field defaults, so a missing or
/// older file loads.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct GameSettings {
    /// AI Graphics.
    pub ai_graphics: AiGraphicsConfig,
}

/// The folder the per-game files are in.
#[must_use]
pub fn dir() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_else(|| "/tmp".into())).join(".config")
        })
        .join("bigame-mode/games")
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
}
