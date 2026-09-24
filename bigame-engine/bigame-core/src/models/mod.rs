//! Video feature models for upscaling and frame generation settings.

use serde::{Deserialize, Serialize};

/// Spatial upscaling pipeline options.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct UpscalingSettings {
    /// Enable gamescope-based upscaling pipeline.
    pub gamescope_enabled: bool,
    /// Upscaling filter used when gamescope is active.
    pub gamescope_filter: GamescopeFilter,
    /// Base input width used by gamescope (`-w`).
    pub base_width: u32,
    /// Base input height used by gamescope (`-h`).
    pub base_height: u32,
    /// Target output width used by gamescope (`-W`).
    pub target_width: u32,
    /// Target output height used by gamescope (`-H`).
    pub target_height: u32,
    /// FSR sharpness in gamescope (`--fsr-sharpness`, 0-20).
    pub gamescope_sharpness: u8,
    /// Enable Wine fullscreen FSR variables.
    pub wine_fsr_enabled: bool,
    /// Quality preset for `WINE_FULLSCREEN_FSR_MODE`.
    pub wine_fsr_mode: WineFsrMode,
    /// Enable vkBasalt shader injection for the game launch.
    pub vkbasalt_enabled: bool,
    /// Optional custom vkBasalt config path.
    pub vkbasalt_config_path: Option<String>,
}

impl UpscalingSettings {
    /// Validate gamescope sharpness and keep stable bounds for serialization.
    #[must_use]
    pub fn clamped_sharpness(&self) -> u8 {
        self.gamescope_sharpness.min(20)
    }
}

/// Gamescope upscaling filter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GamescopeFilter {
    /// AMD `FidelityFX` Super Resolution 1.0.
    #[default]
    Fsr,
    /// NVIDIA Image Scaling.
    Nis,
    /// Integer scaling.
    Integer,
}

/// Wine fullscreen FSR quality mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WineFsrMode {
    Performance,
    Balanced,
    #[default]
    Quality,
    Ultra,
}

/// Frame generation for every game: lsfg-vk, when it is installed.
///
/// Per-game upscaling and frame generation through `OptiScaler` are not here:
/// they are a game's AI Graphics settings (`crate::graphics`), planned,
/// installed with a backup and verified for that game. Files that still name
/// the retired `optiscaler` or `afmf` backends load, reading them as `none`;
/// unknown keys are ignored.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct FrameGenSettings {
    /// Frame generation is on.
    pub enabled: bool,
    /// Which.
    pub backend: FrameGenBackend,
}

/// Frame generation technology.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FrameGenBackend {
    /// None.
    #[default]
    #[serde(alias = "optiscaler", alias = "afmf")]
    None,
    /// lsfg-vk (Lossless Scaling frame generation, Vulkan layer).
    LsfgVk,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_upscaling_settings_defaults_stable() {
        let cfg = UpscalingSettings::default();
        assert!(!cfg.gamescope_enabled);
        assert_eq!(cfg.gamescope_filter, GamescopeFilter::Fsr);
        assert_eq!(cfg.base_width, 0);
        assert_eq!(cfg.base_height, 0);
        assert_eq!(cfg.target_width, 0);
        assert_eq!(cfg.target_height, 0);
        assert_eq!(cfg.gamescope_sharpness, 0);
        assert!(!cfg.wine_fsr_enabled);
        assert_eq!(cfg.wine_fsr_mode, WineFsrMode::Quality);
        assert!(!cfg.vkbasalt_enabled);
        assert_eq!(cfg.vkbasalt_config_path, None);
    }

    #[test]
    fn test_framegen_settings_defaults_stable() {
        let cfg = FrameGenSettings::default();
        assert!(!cfg.enabled);
        assert_eq!(cfg.backend, FrameGenBackend::None);
    }

    #[test]
    fn a_video_config_from_before_ai_graphics_still_loads() {
        // The OptiScaler/AFMF backends and their fields are gone; a file
        // that has them loads, with frame generation off.
        for backend in ["optiscaler", "afmf"] {
            let json = format!(
                r#"{{"enabled":true,"backend":"{backend}","mode":"xess","osd_enabled":true,
                    "optiscaler_enabled":true,"optiscaler_source_dir":"/opt/optiscaler",
                    "afmf_experimental_enabled":true,"afmf_env_override":"RADV_PERFTEST=afmf"}}"#
            );
            let fg: FrameGenSettings = serde_json::from_str(&json).unwrap();
            assert_eq!(fg.backend, FrameGenBackend::None, "{backend}");
        }
        let fg: FrameGenSettings =
            serde_json::from_str(r#"{"enabled":true,"backend":"lsfg_vk"}"#).unwrap();
        assert_eq!((fg.enabled, fg.backend), (true, FrameGenBackend::LsfgVk));
    }

    #[test]
    fn test_upscaling_sharpness_clamped_at_20() {
        let cfg = UpscalingSettings {
            gamescope_sharpness: 42,
            ..UpscalingSettings::default()
        };
        assert_eq!(cfg.clamped_sharpness(), 20);
    }

    #[test]
    fn test_models_json_round_trip() {
        let up = UpscalingSettings {
            gamescope_enabled: true,
            gamescope_filter: GamescopeFilter::Nis,
            base_width: 1280,
            base_height: 720,
            target_width: 1920,
            target_height: 1080,
            gamescope_sharpness: 10,
            wine_fsr_enabled: true,
            wine_fsr_mode: WineFsrMode::Ultra,
            vkbasalt_enabled: true,
            vkbasalt_config_path: Some("/etc/vkBasalt.conf".into()),
        };

        let fg = FrameGenSettings {
            enabled: true,
            backend: FrameGenBackend::LsfgVk,
        };

        let up_json = serde_json::to_string(&up).expect("serialize upscaling");
        let fg_json = serde_json::to_string(&fg).expect("serialize framegen");

        let up_back: UpscalingSettings =
            serde_json::from_str(&up_json).expect("deserialize upscaling");
        let fg_back: FrameGenSettings =
            serde_json::from_str(&fg_json).expect("deserialize framegen");

        assert_eq!(up_back, up);
        assert_eq!(fg_back, fg);
    }
}
