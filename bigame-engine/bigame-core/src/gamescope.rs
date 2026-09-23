//! Gamescope configuration and command-line construction.
//!
//! There is exactly one argument builder in this project, and it is
//! [`Config::to_args`]. The audit found two: this module emitted `--fsr`, a
//! flag Gamescope removed years ago, while `launcher` independently built the
//! correct `-F fsr`. Every launch through the first path failed with a parse
//! error, because `--fsr` collides with the `--fsr-sharpness` prefix.
//!
//! The lesson is baked into the signature: **the builder cannot be called
//! without capabilities**. Flags are emitted only when the Gamescope binary
//! actually on disk advertises them in `--help`, so a distribution patch, an
//! old build or a future removal degrades to "that option is not applied"
//! instead of "nothing launches".

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::capabilities::GamescopeCaps;

/// Upscaling filter, matching Gamescope's `-F/--filter` values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Filter {
    /// Bilinear — Gamescope's own default. Emitted as no filter argument.
    #[default]
    Linear,
    /// Nearest-neighbour.
    Nearest,
    /// AMD FidelityFX Super Resolution 1.0.
    Fsr,
    /// NVIDIA Image Scaling.
    Nis,
    /// Integer / pixel-exact scaling.
    Pixel,
}

impl Filter {
    /// The token Gamescope expects after `-F`.
    #[must_use]
    pub fn as_arg(self) -> &'static str {
        match self {
            Self::Linear => "linear",
            Self::Nearest => "nearest",
            Self::Fsr => "fsr",
            Self::Nis => "nis",
            Self::Pixel => "pixel",
        }
    }

    /// Whether this filter reads `--fsr-sharpness`.
    ///
    /// Despite the name the option applies to NIS as well; it is the shared
    /// sharpening stage.
    #[must_use]
    pub fn uses_sharpness(self) -> bool {
        matches!(self, Self::Fsr | Self::Nis)
    }
}

/// How the game's frame rate should be limited, if at all.
///
/// Gamescope offers two mechanisms with quite different meanings, and the audit
/// found the project conflating them: it stored a field called
/// `framerate_limit` and passed it to `-r`, which is `--nested-refresh` — the
/// refresh rate of the nested display. That does cap frames in nested mode, but
/// it is not the limiter, and the UI label promised something else.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FrameLimit {
    /// Let the game run unrestricted.
    #[default]
    None,
    /// Set the nested display's refresh rate (`-r`). In nested mode this is the
    /// mechanism that matches a target frame rate to a display cadence.
    NestedRefresh(u32),
}

/// Gamescope display and rendering configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Resolution the game renders at (`-w`/`-h`). Zero means "leave to the game".
    pub render_width: u32,
    /// See [`Config::render_width`].
    pub render_height: u32,
    /// Resolution Gamescope presents at (`-W`/`-H`). Zero means "same as render".
    pub output_width: u32,
    /// See [`Config::output_width`].
    pub output_height: u32,
    /// Upscaling filter.
    pub filter: Filter,
    /// Sharpness, 0 (max) to 20 (min), for FSR and NIS.
    pub sharpness: u8,
    /// Frame-rate handling.
    pub frame_limit: FrameLimit,
    /// Show the MangoHud overlay through `--mangoapp`.
    pub mangoapp: bool,
    /// Request variable refresh rate.
    pub adaptive_sync: bool,
    /// Request HDR output.
    pub hdr: bool,
    /// Run fullscreen rather than in a decorated window.
    ///
    /// Defaults to true: without it, nested Gamescope opens a small window,
    /// which is never what someone launching a game wants.
    pub fullscreen: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            render_width: 0,
            render_height: 0,
            output_width: 0,
            output_height: 0,
            filter: Filter::Linear,
            sharpness: 5,
            frame_limit: FrameLimit::None,
            mangoapp: false,
            adaptive_sync: false,
            hdr: false,
            fullscreen: true,
        }
    }
}

/// A flag that was requested but could not be used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unsupported {
    /// The flag, as Gamescope spells it.
    pub flag: String,
    /// What the user loses as a result.
    pub effect: String,
}

/// Arguments plus the list of requests this Gamescope build could not honour.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Args {
    /// Arguments to pass before the `--` separator.
    pub args: Vec<String>,
    /// Requested options this build does not support. Surface these instead of
    /// letting them vanish — a silently dropped setting is how users conclude
    /// the application does nothing.
    pub unsupported: Vec<Unsupported>,
}

impl Config {
    /// Clamp sharpness to the range Gamescope documents.
    #[must_use]
    pub fn clamped_sharpness(&self) -> u8 {
        self.sharpness.min(20)
    }

    /// Build the Gamescope argument list for a specific installed build.
    ///
    /// Only flags present in that build's `--help` are emitted; anything else
    /// is reported through [`Args::unsupported`] rather than passed and hoped
    /// for.
    #[must_use]
    pub fn to_args(&self, caps: &GamescopeCaps) -> Args {
        let mut args: Vec<String> = Vec::new();
        let mut unsupported: Vec<Unsupported> = Vec::new();

        let want = |flag: &str, effect: &str, unsupported: &mut Vec<Unsupported>| -> bool {
            if caps.has_flag(flag) {
                true
            } else {
                unsupported.push(Unsupported {
                    flag: flag.to_owned(),
                    effect: effect.to_owned(),
                });
                false
            }
        };

        if self.render_width > 0 && self.render_height > 0 {
            args.extend([
                "-w".into(),
                self.render_width.to_string(),
                "-h".into(),
                self.render_height.to_string(),
            ]);
        }
        if self.output_width > 0 && self.output_height > 0 {
            args.extend([
                "-W".into(),
                self.output_width.to_string(),
                "-H".into(),
                self.output_height.to_string(),
            ]);
        }

        if self.filter != Filter::Linear {
            if want("F", "upscaling filter not applied", &mut unsupported) {
                args.extend(["-F".into(), self.filter.as_arg().into()]);
                if self.filter.uses_sharpness()
                    && want(
                        "fsr-sharpness",
                        "sharpness left at default",
                        &mut unsupported,
                    )
                {
                    args.extend([
                        "--fsr-sharpness".into(),
                        self.clamped_sharpness().to_string(),
                    ]);
                }
            }
        }

        if let FrameLimit::NestedRefresh(hz) = self.frame_limit {
            if hz > 0 && want("r", "frame rate not limited", &mut unsupported) {
                args.extend(["-r".into(), hz.to_string()]);
            }
        }

        if self.mangoapp && want("mangoapp", "overlay not shown", &mut unsupported) {
            args.push("--mangoapp".into());
        }
        if self.adaptive_sync && want("adaptive-sync", "VRR not requested", &mut unsupported) {
            args.push("--adaptive-sync".into());
        }
        if self.hdr && want("hdr-enabled", "HDR not requested", &mut unsupported) {
            args.push("--hdr-enabled".into());
        }
        if self.fullscreen && want("f", "window will not be fullscreen", &mut unsupported) {
            args.push("-f".into());
        }

        Args { args, unsupported }
    }

    /// Build a full command line: `gamescope <args> -- <command> <command_args>`.
    #[must_use]
    pub fn build_argv(
        &self,
        caps: &GamescopeCaps,
        command: &str,
        command_args: &[String],
    ) -> (Vec<String>, Vec<Unsupported>) {
        let built = self.to_args(caps);
        let mut argv = built.args;
        argv.push("--".into());
        argv.push(command.to_owned());
        argv.extend(command_args.iter().cloned());
        (argv, built.unsupported)
    }
}

/// Global default Gamescope config path.
fn global_config_path() -> std::path::PathBuf {
    let config = std::env::var("XDG_CONFIG_HOME").map_or_else(
        |_| {
            std::path::PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| "/tmp".into()))
                .join(".config")
        },
        std::path::PathBuf::from,
    );
    config.join("bigame-mode").join("gamescope.toml")
}

/// Load the global default Gamescope config, falling back to defaults.
#[must_use]
pub fn load_global() -> Config {
    std::fs::read_to_string(global_config_path())
        .ok()
        .and_then(|s| toml::from_str(&s).ok())
        .unwrap_or_default()
}

/// Persist the global default Gamescope config.
///
/// # Errors
/// Returns an error if the config directory or file cannot be written.
pub fn save_global(config: &Config) -> Result<()> {
    let path = global_config_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create config dir: {}", parent.display()))?;
    }
    let content = toml::to_string_pretty(config).context("serialize gamescope config")?;
    std::fs::write(&path, content)
        .with_context(|| format!("write gamescope config: {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The flag set of the real Gamescope 3.16.28 on the bench.
    fn modern() -> GamescopeCaps {
        GamescopeCaps {
            version: None,
            flags: [
                "F",
                "f",
                "b",
                "w",
                "h",
                "W",
                "H",
                "r",
                "filter",
                "fsr-sharpness",
                "sharpness",
                "mangoapp",
                "adaptive-sync",
                "hdr-enabled",
                "framerate-limit",
                "backend",
                "output-width",
                "nested-width",
            ]
            .iter()
            .map(|s| (*s).to_owned())
            .collect(),
        }
    }

    /// A hypothetical older or heavily patched build.
    fn minimal() -> GamescopeCaps {
        GamescopeCaps {
            version: None,
            flags: ["w", "h", "f"].iter().map(|s| (*s).to_owned()).collect(),
        }
    }

    #[test]
    fn never_emits_the_flag_that_broke_every_launch() {
        // `--fsr` was removed from Gamescope and collides with the
        // `--fsr-sharpness` prefix, so passing it aborts the launch.
        let cfg = Config {
            filter: Filter::Fsr,
            ..Config::default()
        };
        let built = cfg.to_args(&modern());
        assert!(
            !built.args.iter().any(|a| a == "--fsr"),
            "must never emit --fsr: {:?}",
            built.args
        );
        // The correct form instead:
        let i = built.args.iter().position(|a| a == "-F").unwrap();
        assert_eq!(built.args[i + 1], "fsr");
    }

    #[test]
    fn emits_sharpness_for_fsr_and_nis_only() {
        for filter in [Filter::Fsr, Filter::Nis] {
            let cfg = Config {
                filter,
                sharpness: 3,
                ..Config::default()
            };
            let built = cfg.to_args(&modern());
            let i = built
                .args
                .iter()
                .position(|a| a == "--fsr-sharpness")
                .unwrap();
            assert_eq!(built.args[i + 1], "3");
        }
        let cfg = Config {
            filter: Filter::Pixel,
            ..Config::default()
        };
        assert!(
            !cfg.to_args(&modern())
                .args
                .iter()
                .any(|a| a == "--fsr-sharpness")
        );
    }

    #[test]
    fn sharpness_is_clamped_to_the_documented_range() {
        let cfg = Config {
            filter: Filter::Fsr,
            sharpness: 200,
            ..Config::default()
        };
        assert_eq!(cfg.clamped_sharpness(), 20);
        let built = cfg.to_args(&modern());
        let i = built
            .args
            .iter()
            .position(|a| a == "--fsr-sharpness")
            .unwrap();
        assert_eq!(built.args[i + 1], "20");
    }

    #[test]
    fn default_filter_adds_no_argument() {
        // Linear is Gamescope's own default; passing it would be noise.
        let built = Config::default().to_args(&modern());
        assert!(!built.args.iter().any(|a| a == "-F"));
    }

    #[test]
    fn unsupported_flags_are_reported_not_passed() {
        let cfg = Config {
            filter: Filter::Fsr,
            adaptive_sync: true,
            hdr: true,
            mangoapp: true,
            ..Config::default()
        };
        let built = cfg.to_args(&minimal());

        // None of the unsupported options reach the command line...
        for flag in ["-F", "--adaptive-sync", "--hdr-enabled", "--mangoapp"] {
            assert!(!built.args.iter().any(|a| a == flag), "leaked {flag}");
        }
        // ...and every one of them is reported, so the UI can say why.
        let reported: Vec<&str> = built.unsupported.iter().map(|u| u.flag.as_str()).collect();
        assert!(reported.contains(&"F"));
        assert!(reported.contains(&"adaptive-sync"));
        assert!(reported.contains(&"hdr-enabled"));
        assert!(reported.contains(&"mangoapp"));
        assert!(built.unsupported.iter().all(|u| !u.effect.is_empty()));
    }

    #[test]
    fn a_supported_build_reports_nothing_unsupported() {
        let cfg = Config {
            filter: Filter::Fsr,
            adaptive_sync: true,
            hdr: true,
            mangoapp: true,
            frame_limit: FrameLimit::NestedRefresh(144),
            ..Config::default()
        };
        assert!(cfg.to_args(&modern()).unsupported.is_empty());
    }

    #[test]
    fn fullscreen_is_on_by_default() {
        // Without -f, nested Gamescope opens a small decorated window.
        assert!(Config::default().fullscreen);
        assert!(
            Config::default()
                .to_args(&modern())
                .args
                .iter()
                .any(|a| a == "-f")
        );
    }

    #[test]
    fn render_and_output_resolutions_use_distinct_flags() {
        let cfg = Config {
            render_width: 1280,
            render_height: 720,
            output_width: 3440,
            output_height: 1440,
            ..Config::default()
        };
        let a = cfg.to_args(&modern()).args;
        let w = a.iter().position(|x| x == "-w").unwrap();
        assert_eq!(a[w + 1], "1280");
        let big_w = a.iter().position(|x| x == "-W").unwrap();
        assert_eq!(a[big_w + 1], "3440");
    }

    #[test]
    fn partial_resolutions_are_ignored() {
        // A width with no height would make Gamescope infer a wrong aspect.
        let cfg = Config {
            render_width: 1920,
            render_height: 0,
            ..Config::default()
        };
        assert!(!cfg.to_args(&modern()).args.iter().any(|a| a == "-w"));
    }

    #[test]
    fn frame_limit_none_emits_nothing() {
        let built = Config::default().to_args(&modern());
        assert!(!built.args.iter().any(|a| a == "-r"));
    }

    #[test]
    fn zero_refresh_is_treated_as_no_limit() {
        let cfg = Config {
            frame_limit: FrameLimit::NestedRefresh(0),
            ..Config::default()
        };
        let built = cfg.to_args(&modern());
        assert!(!built.args.iter().any(|a| a == "-r"));
        assert!(built.unsupported.is_empty());
    }

    #[test]
    fn argv_places_the_separator_before_the_game() {
        let cfg = Config::default();
        let (argv, _) = cfg.build_argv(&modern(), "steam", &["-gamepadui".to_owned()]);
        let sep = argv.iter().position(|a| a == "--").unwrap();
        assert_eq!(argv[sep + 1], "steam");
        assert_eq!(argv[sep + 2], "-gamepadui");
        // Everything before the separator is a gamescope option.
        assert!(
            argv[..sep]
                .iter()
                .all(|a| a.starts_with('-') || a.parse::<u32>().is_ok())
        );
    }

    #[test]
    fn config_survives_a_toml_round_trip() {
        let cfg = Config {
            render_width: 2560,
            render_height: 1440,
            output_width: 3440,
            output_height: 1440,
            filter: Filter::Nis,
            sharpness: 12,
            frame_limit: FrameLimit::NestedRefresh(120),
            mangoapp: true,
            adaptive_sync: true,
            hdr: false,
            fullscreen: true,
        };
        let text = toml::to_string_pretty(&cfg).unwrap();
        assert_eq!(toml::from_str::<Config>(&text).unwrap(), cfg);
    }

    #[test]
    fn filter_tokens_match_gamescope_spelling() {
        assert_eq!(Filter::Fsr.as_arg(), "fsr");
        assert_eq!(Filter::Nis.as_arg(), "nis");
        assert_eq!(Filter::Pixel.as_arg(), "pixel");
        assert_eq!(Filter::Nearest.as_arg(), "nearest");
        assert_eq!(Filter::Linear.as_arg(), "linear");
    }
}
