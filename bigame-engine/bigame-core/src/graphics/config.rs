//! What the user chose for one game's AI Graphics — the part of a profile
//! that travels.
//!
//! Only intent is stored: "FSR through `OptiScaler`, quality from the game",
//! never a path or a file list. Paths, the executable, the DLL slot and the
//! files to place are detected again wherever the profile is used, so a
//! profile exported from one machine means the same on another. Every field
//! has a default, so a profile written before AI Graphics existed — or by a
//! newer version with fields this one does not know — still loads.

use serde::{Deserialize, Serialize};

/// How AI Graphics is used for this game.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    /// Nothing is changed. The default: AI Graphics is opt-in per game.
    #[default]
    Off,
    /// BiGame-mode picks the smallest combination that works for this game
    /// and machine.
    Recommended,
    /// The user's own choices below.
    Advanced,
}

/// Which upscaler.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Upscaler {
    /// Best available for this game and GPU.
    #[default]
    Auto,
    /// The game's own DLSS (NVIDIA only).
    NativeDlss,
    /// DLSS at native resolution, anti-aliasing only (NVIDIA only).
    Dlaa,
    /// AMD FSR — the game's own, or through `OptiScaler`.
    Fsr,
    /// Intel `XeSS` — the game's own, or through `OptiScaler`.
    Xess,
    /// None: the game's own anti-aliasing.
    Off,
}

/// Render resolution preset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Quality {
    /// Whatever preset is selected in the game's menu. `OptiScaler` takes the
    /// game's preset along with its input, so this is the default.
    #[default]
    Game,
    /// Native resolution, anti-aliasing only.
    NativeAa,
    /// ~1.3× upscale.
    UltraQuality,
    /// ~1.5× upscale.
    Quality,
    /// ~1.7× upscale.
    Balanced,
    /// 2× upscale.
    Performance,
    /// 3× upscale.
    UltraPerformance,
}

impl Quality {}

/// How the upscaler reaches the game.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Layer {
    /// Native when the game has the chosen upscaler, `OptiScaler` otherwise.
    #[default]
    Auto,
    /// Only what the game ships; no files are placed.
    Native,
    /// `OptiScaler`.
    OptiScaler,
}

/// Frame generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FrameGeneration {
    /// Off unless it was measured to help here. Frame generation raises the
    /// presented frame rate, not the rendered one, and adds latency — it is
    /// never switched on by default.
    #[default]
    Auto,
    /// Off.
    Off,
    /// The game's own frame generation.
    Native,
    /// `OptiScaler`'s (`OptiFG`, FSR frame generation output). Experimental.
    OptiScaler,
}

/// HDR enhancement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Hdr {
    /// None.
    #[default]
    Off,
    /// A `RenoDX` mod for this game, when one exists — reported, never
    /// installed: it needs a `ReShade` add-on build, and `ReShade` binaries are
    /// distributed only by its own site, so they are not fetched.
    RenoDx,
}

/// Which `OptiScaler` release a profile uses.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "policy", content = "version")]
pub enum VersionPolicy {
    /// The release BiGame-mode was tested with.
    #[default]
    Recommended,
    /// Exactly this version, until the user changes it. A version known to
    /// work for a game stays when a newer one appears.
    Pinned(String),
}

/// A game's AI Graphics settings.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AiGraphicsConfig {
    /// Off, Recommended or Advanced.
    pub mode: Mode,
    /// Upscaler (Advanced).
    pub upscaler: Upscaler,
    /// Preset (Advanced).
    pub quality: Quality,
    /// Native or `OptiScaler` (Advanced).
    pub layer: Layer,
    /// Frame generation (Advanced).
    pub frame_generation: FrameGeneration,
    /// HDR enhancement (Advanced).
    pub hdr: Hdr,
    /// Allow combinations marked Experimental.
    pub experimental: bool,
    /// `OptiScaler` version.
    pub version: VersionPolicy,
}

impl AiGraphicsConfig {
    /// Whether `OptiScaler`'s frame generation is chosen: Advanced, with the
    /// experimental combinations allowed.
    #[must_use]
    pub fn optiscaler_frame_generation(&self) -> bool {
        self.mode == Mode::Advanced
            && self.frame_generation == FrameGeneration::OptiScaler
            && self.experimental
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_old_profile_without_ai_graphics_loads_as_off() {
        let c: AiGraphicsConfig = toml::from_str("").unwrap();
        assert_eq!(c, AiGraphicsConfig::default());
        assert_eq!(c.mode, Mode::Off);
    }

    #[test]
    fn unknown_fields_from_a_newer_version_are_ignored() {
        let c: AiGraphicsConfig =
            toml::from_str("mode = \"recommended\"\nfuture_thing = 3\n").unwrap();
        assert_eq!(c.mode, Mode::Recommended);
    }

    #[test]
    fn a_config_round_trips_without_any_path() {
        let c = AiGraphicsConfig {
            mode: Mode::Advanced,
            upscaler: Upscaler::Fsr,
            quality: Quality::Quality,
            layer: Layer::OptiScaler,
            frame_generation: FrameGeneration::Off,
            hdr: Hdr::Off,
            experimental: false,
            version: VersionPolicy::Pinned("0.9.4".into()),
        };
        let text = toml::to_string(&c).unwrap();
        assert!(!text.contains('/'), "portable: no paths\n{text}");
        assert_eq!(toml::from_str::<AiGraphicsConfig>(&text).unwrap(), c);
    }
}
