//! Persistence for global video-enhancement settings (upscaling + frame generation).
//!
//! Stored as TOML in `$XDG_CONFIG_HOME/bigame-mode/video.toml`.
//! These are system-wide defaults; per-game profile overrides will extend them in
//! a later step.

use std::collections::HashMap;
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
    std::env::var("XDG_CONFIG_HOME").map_or_else(
        |_| PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| "/tmp".into())).join(".config"),
        PathBuf::from,
    )
}

/// Load video config from disk. Returns defaults on any error (missing file, parse fail).
#[must_use]
pub fn load() -> VideoConfig {
    load_from(&config_path())
}

/// Load video config from a specific file.
///
/// Exists so tests can supply their own path. Reaching for `XDG_CONFIG_HOME`
/// instead would mean mutating a process-global variable while `cargo test`
/// runs tests in parallel threads — which is what made the journal tests race
/// and leak state into the real user profile.
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
/// computed env vars (Wine FSR, vkBasalt, AFMF) reach game processes spawned
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
    let base = std::env::var("XDG_CONFIG_HOME").map_or_else(
        |_| PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| "/tmp".into())).join(".config"),
        PathBuf::from,
    );
    base.join("environment.d").join("bigame-mode.conf")
}

/// Write `~/.config/environment.d/bigame-mode.conf` with persistent video env vars.
///
/// systemd user manager imports these on next login so vars propagate to all
/// user-scope processes including Steam-launched games. If `cfg` produces an
/// empty env set the file is removed.
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
        return Ok(());
    }

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create env dir: {}", parent.display()))?;
    }

    let mut keys: Vec<&String> = env.keys().collect();
    keys.sort();
    let mut content = String::from("# Managed by BiGameMode. Do not edit manually.\n");
    for k in keys {
        // environment.d is KEY=VALUE per line, no quoting required for our values.
        content.push_str(&format!("{}={}\n", k, env[k]));
    }
    std::fs::write(&path, content)
        .with_context(|| format!("write env file: {}", path.display()))?;

    // Best-effort: push vars into the running systemd-user manager so newly
    // spawned processes (after Steam restart) inherit them without needing a
    // full session relogin.
    let _ = import_into_systemd_user(&env);
    Ok(())
}

/// Push the given env map into systemd --user manager environment.
/// Equivalent to `systemctl --user import-environment KEY1 KEY2 ...` but
/// sets values explicitly via `set-environment KEY=VALUE`.
fn import_into_systemd_user(env: &HashMap<String, String>) -> Result<()> {
    if env.is_empty() {
        return Ok(());
    }
    let mut args: Vec<String> = vec!["--user".into(), "set-environment".into()];
    for (k, v) in env {
        args.push(format!("{k}={v}"));
    }
    let status = std::process::Command::new("systemctl")
        .args(&args)
        .status()
        .context("invoke systemctl --user set-environment")?;
    if !status.success() {
        anyhow::bail!("systemctl --user set-environment exited {status}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{FrameGenBackend, GamescopeFilter};

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
    /// parallel threads, so mutating one races every other test in the binary —
    /// which is exactly how an earlier version of the journal tests leaked a
    /// file into the real user profile (audit T-01).
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
