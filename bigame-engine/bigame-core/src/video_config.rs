//! Persistence for global video-enhancement settings (upscaling + frame generation).
//!
//! Stored as TOML in `$XDG_CONFIG_HOME/bigame-mode/video.toml`.
//! These are the global defaults; a game's profile overrides the Gamescope part
//! (see `crate::launcher`).

use std::collections::{BTreeMap, HashMap};
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
/// Returns `Ok(Some(error))` when the settings were saved but environment.d
/// or the running session could not be brought to them: games started now
/// do not get the change, and the caller says so.
///
/// # Errors
/// Returns error if directory creation or file write fails.
pub fn save(cfg: &VideoConfig) -> Result<Option<anyhow::Error>> {
    save_to(cfg, &config_path())?;
    // The settings are saved either way; the session part is reported apart.
    let unsynced = write_env_file(cfg).err();
    if let Some(e) = &unsynced {
        tracing::warn!(error = %format!("{e:#}"), "saved, but the session environment was not updated");
    }
    Ok(unsynced)
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

/// The session environment file: `~/.config/environment.d/bigame-mode.conf`.
pub(crate) fn env_file_path() -> PathBuf {
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
    // A Turbo preset's own, never in environment.d.
    "DXVK_CONFIG",
    "DXVK_FRAME_RATE",
    "VKD3D_FRAME_RATE",
    "FSR4_UPGRADE",
    "PROTON_FSR4_UPGRADE",
];

/// Write `~/.config/environment.d/bigame-mode.conf` with persistent video env
/// vars, and bring the running `systemd --user` manager to the same set.
///
/// environment.d is read at login; the running manager is what Steam, and
/// the games it starts after a Steam restart, inherit now. If `cfg` produces
/// no variables the file is removed and the variables are unset.
///
/// # Errors
/// Returns error if directory creation or file I/O fails, or the running
/// session's environment cannot be updated.
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

    sync_session_env(cfg).context("update the running session's environment")?;
    Ok(())
}

/// Bring the running session to `cfg`'s variables with the Turbo preset in
/// force laid over them ([`crate::turbo_preset`]), and return the
/// assignments it holds afterwards (read back).
///
/// The preset's part never goes to `environment.d`: only here.
///
/// # Errors
/// Returns an error when the session's environment cannot be set, or reads
/// back different.
pub fn sync_session_env(cfg: &VideoConfig) -> Result<Vec<String>> {
    sync_session_env_with(cfg, &crate::turbo_preset::layer())
}

/// [`sync_session_env`] with the preset `layer` rather than the one in
/// force: taking a preset away brings the session to Tuning's variables and
/// the user's own values while its record is still there.
///
/// # Errors
/// As [`sync_session_env`], and when the preset's own file cannot be written.
pub fn sync_session_env_with(
    cfg: &VideoConfig,
    layer: &crate::turbo_preset::Layer,
) -> Result<Vec<String>> {
    crate::turbo_preset::prepare(layer.levers)?;
    let env = session_env_with(cfg, layer);
    let (unset, set) = session_change(&env, layer.owns_preset_keys);
    sync_session(&unset, &set)?;
    Ok(set)
}

/// What [`sync_session_env`] brings the running session to: `cfg`'s
/// variables with the Turbo preset in force laid over them.
#[must_use]
pub fn session_env(cfg: &VideoConfig) -> HashMap<String, String> {
    session_env_with(cfg, &crate::turbo_preset::layer())
}

fn session_env_with(
    cfg: &VideoConfig,
    layer: &crate::turbo_preset::Layer,
) -> HashMap<String, String> {
    let mut env = crate::launcher::build_persistent_env(cfg);
    crate::turbo_preset::overlay_over(&mut env, layer.levers, &layer.before);
    env
}

/// The running `systemd --user` manager's environment.
///
/// # Errors
/// Returns an error when the session bus or the manager cannot be reached.
pub fn session_environment() -> Result<HashMap<String, String>> {
    let conn = zbus::blocking::Connection::session().context("session bus")?;
    let now: Vec<String> = user_manager(&conn)?
        .get_property("Environment")
        .context("read the session environment")?;
    Ok(now
        .iter()
        .filter_map(|a| a.split_once('='))
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .collect())
}

fn user_manager(conn: &zbus::blocking::Connection) -> Result<zbus::blocking::Proxy<'static>> {
    zbus::blocking::Proxy::new(
        conn,
        "org.freedesktop.systemd1",
        "/org/freedesktop/systemd1",
        "org.freedesktop.systemd1.Manager",
    )
    .context("systemd user manager")
}

/// What of BiGame-mode's variables ([`SESSION_KEYS`]) an environment really
/// puts in force, `get` reading one variable: a switch counts only at `1`
/// (`0` and absent are both off), and a switch's detail only while its
/// switch is on. Two environments that give games the same thing compare
/// equal, however each came to say it.
pub fn in_force<'a>(get: impl Fn(&str) -> Option<&'a str>) -> BTreeMap<&'static str, &'a str> {
    let on = |switch: &str| get(switch) == Some("1");
    SESSION_KEYS
        .iter()
        .filter(|k| !SWITCHES.contains(k) || on(k))
        .filter(|k| {
            DETAILS
                .iter()
                .find(|(detail, _)| detail == *k)
                .is_none_or(|(_, switch)| on(switch))
        })
        .filter_map(|k| get(k).map(|v| (*k, v)))
        .collect()
}

/// The switches that turn a feature on only when set to `1`, and are turned
/// off by `0`.
const SWITCHES: &[&str] = &["WINE_FULLSCREEN_FSR", "ENABLE_VKBASALT"];

/// Variables read only while their switch is on: (detail, switch).
const DETAILS: &[(&str, &str)] = &[
    ("WINE_FULLSCREEN_FSR_MODE", "WINE_FULLSCREEN_FSR"),
    ("VKBASALT_CONFIG_FILE", "ENABLE_VKBASALT"),
];

/// The managed keys to unset and the `KEY=VALUE` assignments to set so the
/// session holds exactly `env`.
///
/// A switch that is no longer wanted is set to `0` rather than unset: what
/// environment.d put there at login comes from systemd's generator, and
/// `UnsetEnvironment` cannot remove it from the running manager (checked on
/// systemd 261: `unset-environment ENABLE_VKBASALT` left it at 1). vkBasalt's
/// layer and Wine enable on `1` only. The file no longer holds the variable,
/// so from the next login it is simply absent.
///
/// A Turbo preset's keys ([`crate::turbo_preset::PRESET_KEYS`]) are managed
/// only while a preset's record is there (`owns_preset_keys`): otherwise
/// they are the user's alone (a `DXVK_CONFIG` of their own, from their
/// `environment.d` or set by hand), and a Tuning save must not unset them.
fn session_change(
    env: &HashMap<String, String>,
    owns_preset_keys: bool,
) -> (Vec<String>, Vec<String>) {
    let managed = |k: &&&str| owns_preset_keys || !crate::turbo_preset::PRESET_KEYS.contains(*k);
    let absent = SESSION_KEYS
        .iter()
        .filter(managed)
        .filter(|k| !env.contains_key(**k));
    let unset = absent
        .clone()
        .filter(|k| !SWITCHES.contains(k))
        .map(|k| (*k).to_owned())
        .collect();
    let mut set: Vec<String> = env
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .chain(
            absent
                .filter(|k| SWITCHES.contains(k))
                .map(|k| format!("{k}=0")),
        )
        .collect();
    set.sort();
    (unset, set)
}

/// One `UnsetAndSetEnvironment` call on the user manager — no `systemctl`
/// process — then a read-back of its environment to confirm it holds.
fn sync_session(unset: &[String], set: &[String]) -> Result<()> {
    let conn = zbus::blocking::Connection::session().context("session bus")?;
    let manager = user_manager(&conn)?;
    manager
        .call_method("UnsetAndSetEnvironment", &(unset, set))
        .context("UnsetAndSetEnvironment")?;
    let now: Vec<String> = manager
        .get_property("Environment")
        .context("read the session environment back")?;
    let missing: Vec<&String> = set.iter().filter(|a| !now.contains(a)).collect();
    // A detail of a switch that is off (Wine FSR's mode with
    // WINE_FULLSCREEN_FSR=0) may stay: the login put it there and systemd
    // keeps it, and with its switch at 0 nothing reads it.
    let harmless = |k: &str| {
        DETAILS
            .iter()
            .any(|(detail, switch)| *detail == k && set.iter().any(|a| *a == format!("{switch}=0")))
    };
    let left: Vec<&String> = unset
        .iter()
        .filter(|k| !harmless(k))
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
    fn turning_a_feature_off_takes_it_out_of_the_session() {
        let mut env = HashMap::new();
        env.insert("ENABLE_VKBASALT".to_owned(), "1".to_owned());
        let (unset, set) = session_change(&env, true);
        // Wine FSR off: its switch is set to 0, which overrides a value the
        // login put there; its mode and vkBasalt's file are just unset.
        assert_eq!(set, ["ENABLE_VKBASALT=1", "WINE_FULLSCREEN_FSR=0"]);
        assert!(unset.contains(&"WINE_FULLSCREEN_FSR_MODE".to_owned()));
        assert!(unset.contains(&"VKBASALT_CONFIG_FILE".to_owned()));
        assert!(!unset.contains(&"ENABLE_VKBASALT".to_owned()));
        // Everything off: both switches at 0, the other keys removed.
        let (unset, set) = session_change(&HashMap::new(), true);
        assert_eq!(set, ["ENABLE_VKBASALT=0", "WINE_FULLSCREEN_FSR=0"]);
        assert_eq!(unset.len(), SESSION_KEYS.len() - SWITCHES.len());
    }

    #[test]
    fn without_a_preset_the_users_own_preset_keys_are_left_alone() {
        // A Tuning save with no preset in force: a DXVK_CONFIG the user set
        // is theirs, and unsetting it failed the read-back when it came from
        // their environment.d.
        let (unset, set) = session_change(&HashMap::new(), false);
        for key in crate::turbo_preset::PRESET_KEYS {
            assert!(!unset.iter().any(|k| k == key), "{key} unset");
            assert!(
                !set.iter().any(|a| a.starts_with(&format!("{key}="))),
                "{key} set"
            );
        }
        // Tuning's own keys are still managed.
        assert!(unset.contains(&"VKBASALT_CONFIG_FILE".to_owned()));
        // Every preset key is one of the session's, so a preset can be
        // taken away again.
        for key in crate::turbo_preset::PRESET_KEYS {
            assert!(SESSION_KEYS.contains(key), "{key}");
        }
        // While a preset's record is there, its keys are BiGame-mode's.
        let (unset, _) = session_change(&HashMap::new(), true);
        assert!(unset.contains(&"DXVK_CONFIG".to_owned()));
    }

    #[test]
    fn taking_a_preset_away_puts_the_users_values_back() {
        use crate::turbo_preset::{Layer, Levers};
        let before = BTreeMap::from([
            (
                "DXVK_CONFIG".to_owned(),
                "dxgi.customVendorId = 10de".to_owned(),
            ),
            ("PROTON_FSR4_UPGRADE".to_owned(), "1".to_owned()),
        ]);
        let layer = Layer {
            levers: Levers::default(),
            before,
            owns_preset_keys: true,
        };
        let env = session_env_with(&VideoConfig::default(), &layer);
        let (unset, set) = session_change(&env, layer.owns_preset_keys);
        assert!(set.contains(&"DXVK_CONFIG=dxgi.customVendorId = 10de".to_owned()));
        assert!(set.contains(&"PROTON_FSR4_UPGRADE=1".to_owned()));
        // What the preset alone set goes.
        assert!(unset.contains(&"VKD3D_FRAME_RATE".to_owned()));
        assert!(unset.contains(&"FSR4_UPGRADE".to_owned()));
        assert!(!unset.contains(&"DXVK_CONFIG".to_owned()));
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
