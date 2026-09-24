//! Frame Generation (lsfg-vk) config management.
//!
//! lsfg-vk is a Vulkan implicit layer — NOT a kernel module.
//! It hot-reloads from `~/.config/lsfg-vk/conf.toml` via inotify/mtime.
//! bigame-mode writes/updates that file to apply per-game FG parameters.
//!
//! Type mapping: bigame stores `flow_scale` as `u32` 0–100 (percent);
//! lsfg-vk expects f32 0.0–1.0. Conversion happens here.

use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::models::{FrameGenBackend, FrameGenSettings};

/// Relative path from $HOME to the lsfg-vk config file.
const CONFIG_REL_PATH: &str = ".config/lsfg-vk/conf.toml";

// ── TOML structures mirroring lsfg-vk 1.x GameConf / GlobalConf ─────────────
// Fields sourced from lsfg-vk-common/include/lsfg-vk-common/configuration/config.hpp
// version must be 1 (as required by current lsfg-vk parser);
// multiplier must be > 1; flow_scale must be 0.25–1.0.

/// An lsfg-vk `[[profile]]` entry — matches `GameConf` exactly.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LsfgProfile {
    /// Display name shown in lsfg-vk-ui.
    pub name: String,
    /// Process names that activate this profile (proc comm or exe basename).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub active_in: Vec<String>,
    /// Optional GPU to use (by name) when multiple GPUs are present.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gpu: Option<String>,
    /// Frame generation multiplier (must be > 1; 2 = 2x FG, etc.).
    pub multiplier: u32,
    /// Optical flow quality scale (0.25–1.0 mapped from percent 25–100).
    pub flow_scale: f32,
    /// Enable performance (low-latency) mode.
    pub performance_mode: bool,
    /// Pacing method — only "none" is supported in lsfg-vk 1.x.
    pub pacing: String,
    /// HDR Mode
    #[serde(default)]
    pub hdr: bool,
    /// Present Mode (0=VSync/FIFO, 1=Recommended, 2=Mailbox, 3=Immediate)
    #[serde(default)]
    pub present_mode: u32,
}

/// lsfg-vk `[global]` section.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LsfgGlobal {
    /// Allow FP16 precision (default true — good for modern AMD).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allow_fp16: Option<bool>,
    /// Global DLL path.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dll: Option<String>,
}

impl LsfgGlobal {
    fn is_empty(&self) -> bool {
        self.allow_fp16.is_none() && self.dll.is_none()
    }
}

/// Full `~/.config/lsfg-vk/conf.toml` structure.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct LsfgConfig {
    /// Must be 1 for current lsfg-vk parser.
    #[serde(default = "default_version")]
    pub version: u32,
    /// `[global]` section — omitted if empty.
    #[serde(skip_serializing_if = "LsfgGlobal::is_empty", default)]
    pub global: LsfgGlobal,
    /// `[[profile]]` array — per-game settings.
    #[serde(rename = "profile", default)]
    pub profiles: Vec<LsfgProfile>,
}

/// lsfg-vk config format version — MUST be 1.
fn default_version() -> u32 {
    1
}

// ── Path resolution ─────────────────────────────────────────────────────────

/// Resolve `~/.config/lsfg-vk/conf.toml` via `$HOME`.
#[must_use]
pub fn config_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".into());
    PathBuf::from(home).join(CONFIG_REL_PATH)
}

// ── Internal read/write ─────────────────────────────────────────────────────

/// Read lsfg-vk config from disk, returning default if the file does not exist.
fn read_config() -> Result<LsfgConfig> {
    let path = config_path();
    if !path.exists() {
        return Ok(LsfgConfig {
            version: 1,
            ..Default::default()
        });
    }
    let content = std::fs::read_to_string(&path)
        .with_context(|| format!("read lsfg-vk config: {}", path.display()))?;
    toml::from_str(&content).context("parse lsfg-vk TOML")
}

/// Write lsfg-vk config to disk, creating parent directories as needed.
fn write_config(cfg: &LsfgConfig) -> Result<()> {
    let path = config_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create lsfg-vk config dir: {}", parent.display()))?;
    }
    let content = toml::to_string_pretty(cfg).context("serialize lsfg-vk config")?;
    std::fs::write(&path, content)
        .with_context(|| format!("write lsfg-vk config: {}", path.display()))
}

// ── Public API ──────────────────────────────────────────────────────────────

/// Write or update FG parameters for a game profile in the lsfg-vk TOML.
///
/// `flow_scale_pct` is 25–100 (percent). Stored as 0.25–1.0 float in lsfg-vk.
/// Pacing is always "none" — lsfg-vk 1.x only supports None.
/// Matches on `active_in` containing `name`; appends a new entry if not found.
///
/// # Errors
/// Returns error if config read/write fails.
pub fn write_profile(
    name: &str,
    multiplier: u32,
    flow_scale_pct: u32,
    perf_mode: bool,
    hdr: bool,
    present_mode: u32,
) -> Result<()> {
    // Multiplier 1 means "disabled" for this profile.
    if multiplier <= 1 {
        return disable_for_game(name);
    }

    // lsfg-vk requires multiplier > 1; clamp upper range only.
    let multiplier = multiplier.min(20);

    anyhow::ensure!(
        is_lossless_dll_ready(),
        "Lossless.dll not found in configured LSFG path"
    );

    anyhow::ensure!(
        (25..=100).contains(&flow_scale_pct),
        "flow_scale_pct must be 25–100"
    );

    let mut cfg = read_config()?;
    // Force parser-compatible version.
    cfg.version = 1;

    // flow_scale_pct is validated; integers ≤ 100 are exact in f32.
    #[allow(clippy::cast_precision_loss)]
    let flow_scale = flow_scale_pct as f32 / 100.0;

    // Update existing entry or push a new one.
    if let Some(entry) = cfg
        .profiles
        .iter_mut()
        .find(|p| p.active_in.contains(&name.to_owned()))
    {
        entry.multiplier = multiplier;
        entry.flow_scale = flow_scale;
        entry.performance_mode = perf_mode;
        entry.pacing = "none".to_string();
        entry.hdr = hdr;
        entry.present_mode = present_mode;
    } else {
        cfg.profiles.push(LsfgProfile {
            name: name.to_owned(),
            active_in: vec![name.to_owned()],
            gpu: None,
            multiplier,
            flow_scale,
            performance_mode: perf_mode,
            pacing: "none".to_string(),
            hdr,
            present_mode,
        });
    }

    write_config(&cfg)
}

/// Remove the FG profile entry for a game (call when deleting a game profile).
///
/// No-op if no matching entry exists.
///
/// # Errors
/// Returns error if config read/write fails.
pub fn delete_profile(name: &str) -> Result<()> {
    let mut cfg = read_config()?;
    let before = cfg.profiles.len();
    cfg.profiles
        .retain(|p| !p.active_in.contains(&name.to_owned()));
    if cfg.profiles.len() != before {
        write_config(&cfg)?;
    }
    Ok(())
}

/// Read current FG parameters for a game from the lsfg-vk TOML.
///
/// Returns `(multiplier, flow_scale_pct, perf_mode)`.
/// Falls back to defaults `(2, 100, false)` if no matching profile exists.
#[must_use]
pub fn read_profile(name: &str) -> (u32, u32, bool, bool, u32) {
    let Ok(cfg) = read_config() else {
        return (2, 100, false, false, 0);
    };
    let Some(entry) = cfg
        .profiles
        .iter()
        .find(|p| p.active_in.contains(&name.to_owned()))
    else {
        return (2, 100, false, false, 0);
    };
    // ensure multiplier is valid per lsfg-vk constraints
    let multiplier = entry.multiplier.max(1);
    // `f` is 0.25–1.0 from lsfg-vk; after round() the value fits in u32.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let flow_scale_pct = ((entry.flow_scale * 100.0).round() as u32).clamp(25, 100);

    (
        multiplier,
        flow_scale_pct,
        entry.performance_mode,
        entry.hdr,
        entry.present_mode,
    )
}

/// Read `[global].dll` from the lsfg-vk config.
///
/// Returns `None` if not set or config unavailable.
#[must_use]
pub fn read_global_dll() -> Option<String> {
    read_config().ok()?.global.dll
}

/// Whether the lsfg-vk Vulkan layer is installed (system-wide or in
/// `/usr/local`).
#[must_use]
pub fn layer_installed() -> bool {
    const LAYERS: &[&str] = &[
        "/etc/vulkan/implicit_layer.d/VkLayer_LS_frame_generation.json",
        "/etc/vulkan/implicit_layer.d/VkLayer_LSFGVK_frame_generation.json",
        "/usr/share/vulkan/implicit_layer.d/VkLayer_LSFGVK_frame_generation.json",
        "/usr/share/vulkan/implicit_layer.d/VkLayer_LS_frame_generation.json",
        "/usr/local/share/vulkan/implicit_layer.d/VkLayer_LSFGVK_frame_generation.json",
        "/usr/local/share/vulkan/implicit_layer.d/VkLayer_LS_frame_generation.json",
    ];
    LAYERS.iter().any(|p| std::path::Path::new(p).exists())
        || std::path::Path::new("/usr/lib/liblsfg-vk.so").exists()
}

/// Returns `true` when a valid Lossless.dll path is configured and exists.
#[must_use]
pub fn is_lossless_dll_ready() -> bool {
    let Some(path) = read_global_dll() else {
        return false;
    };
    let p = PathBuf::from(path);
    p.is_file()
}

/// Write `[global].dll` to the lsfg-vk config.
///
/// Pass `None` to remove the global DLL override.
///
/// # Errors
/// Returns error if config read/write fails.
pub fn write_global_dll(dll: Option<String>) -> Result<()> {
    let mut cfg = read_config()?;
    cfg.global.dll = dll;
    write_config(&cfg)
}

// ── Conflict detection helpers ───────────────────────────────────────────────

/// Returns `true` if ANY lsfg-vk profile has `multiplier > 1`.
///
/// Used to decide whether to show a conflict warning when `OptiScaler` or AFMF
/// is also enabled (both generate frames — running both simultaneously causes
/// visual artifacts).
#[must_use]
pub fn has_any_active_profile() -> bool {
    if !is_lossless_dll_ready() {
        return false;
    }
    read_config()
        .ok()
        .is_some_and(|cfg| cfg.profiles.iter().any(|p| p.multiplier > 1))
}

/// Returns `true` when global video settings still allow lsfg-vk profiles.
#[must_use]
pub fn global_state_allows_lsfg(frame_gen: &FrameGenSettings) -> bool {
    frame_gen.enabled && frame_gen.backend == FrameGenBackend::LsfgVk
}

/// Neutralize persisted lsfg-vk profiles when global frame generation disables them.
///
/// This prevents Steam-native launches from continuing to load the lsfg-vk
/// implicit layer after the user has disabled frame generation globally.
///
/// # Errors
/// Returns error if the lsfg-vk config cannot be read or written.
pub fn sync_global_enablement(frame_gen: &FrameGenSettings) -> Result<bool> {
    if global_state_allows_lsfg(frame_gen) {
        return Ok(false);
    }

    disable_all_profiles()
}

/// Returns `true` if the specific game has an lsfg-vk profile with `multiplier > 1`.
///
/// A game is "active" for lsfg-vk FG when its profile multiplier is ≥ 2.
#[must_use]
pub fn is_active_for_game(name: &str) -> bool {
    if !is_lossless_dll_ready() {
        return false;
    }
    read_config().ok().is_some_and(|cfg| {
        cfg.profiles
            .iter()
            .any(|p| p.active_in.contains(&name.to_owned()) && p.multiplier > 1)
    })
}

/// Disable LSFG for all profiles by forcing multipliers to 1.
///
/// Returns `Ok(true)` if at least one profile was changed.
///
/// # Errors
/// Returns an error if the lsfg-vk config cannot be read or written.
pub fn disable_all_profiles() -> Result<bool> {
    let mut cfg = read_config()?;
    let mut changed = false;
    for profile in &mut cfg.profiles {
        if profile.multiplier > 1 {
            profile.multiplier = 1;
            changed = true;
        }
    }
    if changed {
        write_config(&cfg)?;
    }
    Ok(changed)
}

/// Disable lsfg-vk FG for a specific game by setting its multiplier to 1.
///
/// Intended to resolve frame generation conflicts when the user switches to
/// `OptiScaler` or AFMF as the primary backend. Does nothing if no profile exists.
///
/// # Errors
/// Returns error if config read/write fails.
pub fn disable_for_game(name: &str) -> Result<()> {
    let mut cfg = read_config()?;
    let mut changed = false;
    for profile in &mut cfg.profiles {
        if profile.active_in.contains(&name.to_owned()) && profile.multiplier > 1 {
            profile.multiplier = 1;
            changed = true;
        }
    }
    if changed {
        write_config(&cfg)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::global_state_allows_lsfg;
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
}
