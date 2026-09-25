//! Persistence for global video-enhancement settings (upscaling + frame generation).
//!
//! Stored as TOML in `$XDG_CONFIG_HOME/bigame-mode/video.toml`.
//! These are the global defaults; a game's profile overrides the Gamescope part
//! (see `crate::launcher`).

use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::models::{FrameGenSettings, UpscalingSettings};

/// Combined video configuration stored as a single TOML file.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct VideoConfig {
    /// Spatial upscaling pipeline (Gamescope, Wine FSR, vkBasalt).
    pub upscaling: UpscalingSettings,
    /// Frame generation backend and parameters.
    pub frame_gen: FrameGenSettings,
}

fn config_path() -> PathBuf {
    config_dir().join("bigame-mode").join("video.toml")
}

fn config_dir() -> PathBuf {
    crate::paths::config_home()
}

/// Load video config from disk. Returns defaults on any error (missing file, parse fail).
#[must_use]
pub fn load() -> VideoConfig {
    load_from(&config_path())
}

/// Load video config from a specific file.
///
/// Exists so tests can supply their own path: `XDG_CONFIG_HOME` is
/// process-global, and mutating it while `cargo test` runs tests in parallel
/// threads races them and can write into the real user profile.
#[must_use]
pub fn load_from(path: &Path) -> VideoConfig {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| toml::from_str(&s).ok())
        .unwrap_or_default()
}

/// Persist video config to `$XDG_CONFIG_HOME/bigame-mode/video.toml`.
///
/// Also writes the corresponding systemd user environment.d snippet so the
/// computed env vars (Wine FSR, vkBasalt) reach game processes spawned
/// outside our launcher (notably Steam-launched games).
///
/// # Errors
/// Returns error if directory creation or file write fails.
pub fn save(cfg: &VideoConfig) -> Result<()> {
    save_to(cfg, &config_path())?;
    // Best-effort: keep environment.d in sync. Failure here must not block save.
    if let Err(e) = write_env_file(cfg) {
        tracing::warn!(error = %e, "failed to update environment.d snippet");
    }
    Ok(())
}

/// Persist video config to a specific file, without touching `environment.d`.
///
/// # Errors
/// Returns an error if directory creation or file write fails.
pub fn save_to(cfg: &VideoConfig, path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create config dir: {}", parent.display()))?;
    }
    let content = toml::to_string_pretty(cfg).context("serialize video config")?;
    std::fs::write(path, content)
        .with_context(|| format!("write video config: {}", path.display()))?;
    Ok(())
}

fn env_file_path() -> PathBuf {
    crate::paths::config_home()
        .join("environment.d")
        .join("bigame-mode.conf")
}

/// Every variable BiGame-mode puts in the session environment. One that is
/// not wanted any more is removed from the running session, not only from
/// the file: otherwise turning Wine FSR or vkBasalt off left it in force for
/// every game until the next login.
pub const SESSION_KEYS: &[&str] = &[
    "WINE_FULLSCREEN_FSR",
    "WINE_FULLSCREEN_FSR_MODE",
    "ENABLE_VKBASALT",
    "VKBASALT_CONFIG_FILE",
];

/// Write `~/.config/environment.d/bigame-mode.conf` with persistent video env
/// vars, and bring the running `systemd --user` manager to the same set.
///
/// environment.d is read at login; the running manager is what Steam, and
/// the games it starts after a Steam restart, inherit now. If `cfg` produces
/// no variables the file is removed and the variables are unset.
///
/// # Errors
/// Returns error if directory creation or file I/O fails.
pub fn write_env_file(cfg: &VideoConfig) -> Result<()> {
    let path = env_file_path();
    let env = crate::launcher::build_persistent_env(cfg);

    if env.is_empty() {
        if path.exists() {
            std::fs::remove_file(&path)
                .with_context(|| format!("remove env file: {}", path.display()))?;
        }
    } else {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("create env dir: {}", parent.display()))?;
        }
        let mut keys: Vec<&String> = env.keys().collect();
        keys.sort();
        let mut content = String::from("# Managed by BiGameMode. Do not edit manually.\n");
        for k in keys {
            // environment.d is KEY=VALUE per line, no quoting required for our values.
            let _ = writeln!(content, "{}={}", k, env[k]);
        }
        std::fs::write(&path, content)
            .with_context(|| format!("write env file: {}", path.display()))?;
    }

    let (unset, set) = session_change(&env);
    if let Err(e) = sync_session(&unset, &set) {
        tracing::warn!(error = %format!("{e:#}"), "could not update the running session's environment");
    }
    Ok(())
}

/// The managed keys to unset and the `KEY=VALUE` assignments to set so the
/// session holds exactly `env`.
fn session_change(env: &HashMap<String, String>) -> (Vec<String>, Vec<String>) {
    let unset = SESSION_KEYS
        .iter()
        .filter(|k| !env.contains_key(**k))
        .map(|k| (*k).to_owned())
        .collect();
    let mut set: Vec<String> = env.iter().map(|(k, v)| format!("{k}={v}")).collect();
    set.sort();
    (unset, set)
}

/// One `UnsetAndSetEnvironment` call on the user manager — no `systemctl`
/// process — then a read-back of its environment to confirm it holds.
fn sync_session(unset: &[String], set: &[String]) -> Result<()> {
    let conn = zbus::blocking::Connection::session().context("session bus")?;
    let manager = zbus::blocking::Proxy::new(
        &conn,
        "org.freedesktop.systemd1",
        "/org/freedesktop/systemd1",
        "org.freedesktop.systemd1.Manager",
    )
    .context("systemd user manager")?;
    manager
        .call_method("UnsetAndSetEnvironment", &(unset, set))
        .context("UnsetAndSetEnvironment")?;
    let now: Vec<String> = manager
        .get_property("Environment")
        .context("read the session environment back")?;
    let missing: Vec<&String> = set.iter().filter(|a| !now.contains(a)).collect();
    let left: Vec<&String> = unset
        .iter()
        .filter(|k| {
            now.iter()
                .any(|a| a.split_once('=').is_some_and(|(n, _)| n == k.as_str()))
        })
        .collect();
    if !missing.is_empty() || !left.is_empty() {
        anyhow::bail!(
            "session environment did not change: missing {missing:?}, still set {left:?}"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{FrameGenBackend, GamescopeFilter};

    #[test]
    fn turning_a_feature_off_unsets_its_variables_in_the_session() {
        let mut env = HashMap::new();
        env.insert("ENABLE_VKBASALT".to_owned(), "1".to_owned());
        let (unset, set) = session_change(&env);
        assert_eq!(set, ["ENABLE_VKBASALT=1"]);
        assert!(unset.contains(&"WINE_FULLSCREEN_FSR".to_owned()));
        assert!(unset.contains(&"VKBASALT_CONFIG_FILE".to_owned()));
        assert!(!unset.contains(&"ENABLE_VKBASALT".to_owned()));
        // Everything off: every managed key is removed, nothing is set.
        let (unset, set) = session_change(&HashMap::new());
        assert_eq!(unset.len(), SESSION_KEYS.len());
        assert!(set.is_empty());
    }

    #[test]
    fn test_video_config_defaults_stable() {
        let cfg = VideoConfig::default();
        assert!(!cfg.upscaling.gamescope_enabled);
        assert!(!cfg.frame_gen.enabled);
        assert_eq!(cfg.upscaling.gamescope_filter, GamescopeFilter::Fsr);
        assert_eq!(cfg.frame_gen.backend, FrameGenBackend::None);
    }

    /// A private config path per test.
    ///
    /// These tests deliberately do **not** touch `XDG_CONFIG_HOME`.
    /// Environment variables are process-global and `cargo test` runs tests in
    /// parallel threads, so mutating one races every other test in the binary
    /// and can leak files into the real user profile.
    fn temp_config(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "bigame_video_{tag}_{}_{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("video.toml")
    }

    #[test]
    fn test_video_config_save_load_round_trip() {
        let path = temp_config("roundtrip");

        let mut cfg = VideoConfig::default();
        cfg.upscaling.gamescope_enabled = true;
        cfg.upscaling.gamescope_sharpness = 7;
        cfg.frame_gen.enabled = true;

        save_to(&cfg, &path).expect("save should succeed");
        let loaded = load_from(&path);

        assert!(loaded.upscaling.gamescope_enabled);
        assert_eq!(loaded.upscaling.gamescope_sharpness, 7);
        assert!(loaded.frame_gen.enabled);

        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn a_missing_or_corrupt_config_falls_back_to_defaults() {
        let path = temp_config("corrupt");
        assert!(!load_from(&path).upscaling.gamescope_enabled);

        std::fs::write(&path, b"this is not toml {{{").unwrap();
        assert!(!load_from(&path).upscaling.gamescope_enabled);

        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }
}
