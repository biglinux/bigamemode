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

/// Artificial frame generation settings and integration mode.
///
/// The booleans are independent feature toggles rather than a state machine;
/// grouping them would add nesting without removing a decision.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct FrameGenSettings {
    /// Enables frame generation layer orchestration.
    pub enabled: bool,
    /// Backend/technology used for frame generation.
    pub backend: FrameGenBackend,
    /// Desired frame generation mode (FSR3, `XeSS`, etc.).
    pub mode: FrameGenMode,
    /// Shows frame generation on-screen status indicator.
    pub osd_enabled: bool,
    /// Enables `OptiScaler` file staging into game prefix.
    pub optiscaler_enabled: bool,
    /// Optional source directory containing `OptiScaler` DLL payload.
    pub optiscaler_source_dir: Option<String>,
    /// Enable experimental AFMF variables for advanced users.
    pub afmf_experimental_enabled: bool,
    /// Optional custom AFMF environment override (e.g. `RADV_PERFTEST=afmf`).
    pub afmf_env_override: Option<String>,
}

/// Frame generation technology backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FrameGenBackend {
    /// No external frame generation backend selected.
    #[default]
    None,
    /// `OptiScaler` + dlssg-to-fsr3 path.
    OptiScaler,
    /// AMD Fluid Motion Frames path.
    Afmf,
    /// Existing lsfg-vk path for compatibility with current stack.
    LsfgVk,
}

/// Frame generation quality/method mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FrameGenMode {
    #[default]
    Fsr3,
    Xess,
    Dlss,
    Native,
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
        assert_eq!(cfg.mode, FrameGenMode::Fsr3);
        assert!(!cfg.osd_enabled);
        assert!(!cfg.optiscaler_enabled);
        assert_eq!(cfg.optiscaler_source_dir, None);
        assert!(!cfg.afmf_experimental_enabled);
        assert_eq!(cfg.afmf_env_override, None);
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
            backend: FrameGenBackend::OptiScaler,
            mode: FrameGenMode::Xess,
            osd_enabled: true,
            optiscaler_enabled: true,
            optiscaler_source_dir: Some("/opt/optiscaler".into()),
            afmf_experimental_enabled: false,
            afmf_env_override: None,
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
