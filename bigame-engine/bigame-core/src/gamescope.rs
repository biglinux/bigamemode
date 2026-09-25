//! Gamescope configuration and command-line construction.
//!
//! There is exactly one argument builder in this project, and it is
//! [`Config::to_args`]. Gamescope no longer has `--fsr` (the filter is
//! `-F fsr`), and because `--fsr` collides with the `--fsr-sharpness` prefix,
//! passing it is a parse error that stops the launch.
//!
//! So **the builder cannot be called without capabilities**: flags are emitted
//! only when the Gamescope binary actually on disk advertises them in `--help`,
//! so a distribution patch, an old build or a future removal degrades to "that
//! option is not applied" instead of "nothing launches".

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
    /// AMD `FidelityFX` Super Resolution 1.0.
    Fsr,
    /// NVIDIA Image Scaling.
    Nis,
    /// Gamescope's pixel-art filter (`-F pixel`): sharp edges, any factor.
    Pixel,
    /// Integer scaling (`-S integer`): whole-number factors only, the same
    /// pixel drawn as an exact block. A scaler, not a filter.
    Integer,
}

impl Filter {
    /// The token Gamescope expects after `-F`.
    #[must_use]
    pub fn as_arg(self) -> &'static str {
        match self {
            Self::Linear => "linear",
            // Integer is a scaler, not a filter: `build_argv` emits
            // `-S integer` for it and never asks for its filter token.
            Self::Nearest | Self::Integer => "nearest",
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
/// `-r` is `--nested-refresh`, the refresh rate of the nested display. It caps
/// frames in nested mode, but it is not a frame limiter, so the variant is
/// named for what it is.
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

/// Whether Gamescope should wrap a given game.
///
/// A global on/off switch is the wrong shape. Some titles are worse inside
/// Gamescope — overlay problems, input problems, HDR problems — and some simply
/// do not need it. Wrapping a game that gains nothing adds a compositor,
/// a copy and a frame of latency for no benefit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    /// Decide from the hardware, the session and what this profile asks for.
    #[default]
    Auto,
    /// Always wrap.
    Enabled,
    /// Never wrap.
    Disabled,
}

/// The outcome of deciding whether to wrap, with the reason.
///
/// The reason is carried rather than logged so the UI can show it. "Gamescope
/// is off" and "Gamescope is off because nothing in this profile needs it" are
/// different messages, and only the second lets someone act.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decision {
    /// Whether to wrap the game.
    pub use_gamescope: bool,
    /// Why, in one sentence, for the user.
    pub reason: String,
}

impl Decision {
    fn yes(reason: impl Into<String>) -> Self {
        Self {
            use_gamescope: true,
            reason: reason.into(),
        }
    }
    fn no(reason: impl Into<String>) -> Self {
        Self {
            use_gamescope: false,
            reason: reason.into(),
        }
    }
}

/// Decide whether to wrap this game in Gamescope.
///
/// `Auto` says yes only when the configuration asks for something Gamescope is
/// the right tool for: upscaling, a non-default filter, a frame-rate target,
/// HDR, VRR, or the `MangoHud` overlay through `--mangoapp`. Everything else is
/// left unwrapped, because a compositor that changes nothing is pure cost.
///
/// `Enabled` still checks that Gamescope is installed and that there is a
/// graphical session — an explicit choice cannot conjure a missing binary.
#[must_use]
pub fn decide(
    mode: Mode,
    config: &Config,
    caps: Option<&GamescopeCaps>,
    session: crate::hardware::Session,
) -> Decision {
    if mode == Mode::Disabled {
        return Decision::no("Gamescope is turned off for this game");
    }
    if caps.is_none() {
        return Decision::no("Gamescope is not installed");
    }
    if session == crate::hardware::Session::Tty {
        return Decision::no("no graphical session for Gamescope to nest in");
    }
    if mode == Mode::Enabled {
        return Decision::yes("Gamescope is turned on for this game");
    }

    // Auto.
    let scaling = config.render_width > 0
        && config.output_width > 0
        && (config.render_width, config.render_height)
            != (config.output_width, config.output_height);
    if scaling {
        return Decision::yes(format!(
            "rendering at {}×{} and presenting at {}×{}",
            config.render_width, config.render_height, config.output_width, config.output_height
        ));
    }
    if config.filter != Filter::Linear {
        return Decision::yes(format!("{} upscaling is selected", config.filter.as_arg()));
    }
    if matches!(config.frame_limit, FrameLimit::NestedRefresh(hz) if hz > 0) {
        return Decision::yes("a frame-rate target is set");
    }
    if config.hdr {
        return Decision::yes("HDR output is requested");
    }
    if config.adaptive_sync {
        return Decision::yes("variable refresh rate is requested");
    }
    if config.mangoapp {
        return Decision::yes("the MangoHud overlay is enabled");
    }

    Decision::no("nothing in this profile needs Gamescope, so the game runs directly")
}

/// Gamescope display and rendering configuration.
///
/// The booleans are independent feature requests rather than a state machine,
/// so grouping them would add nesting without removing a decision.
#[allow(clippy::struct_excessive_bools)]
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
    /// Show the `MangoHud` overlay through `--mangoapp`.
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

        if self.filter == Filter::Integer {
            if want("S", "integer scaling not applied", &mut unsupported) {
                args.extend(["-S".into(), "integer".into()]);
            }
        } else if self.filter != Filter::Linear
            && want("F", "upscaling filter not applied", &mut unsupported)
        {
            {
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
    crate::paths::config_home()
        .join("bigame-mode")
        .join("gamescope.toml")
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

    /// The flag set of a real Gamescope 3.16.28.
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

    use crate::hardware::Session;

    #[test]
    fn disabled_always_means_no() {
        let d = decide(
            Mode::Disabled,
            &Config::default(),
            Some(&modern()),
            Session::Wayland,
        );
        assert!(!d.use_gamescope);
        assert!(d.reason.contains("turned off"));
    }

    #[test]
    fn enabled_still_requires_gamescope_to_exist() {
        // An explicit choice cannot conjure a missing binary.
        let d = decide(Mode::Enabled, &Config::default(), None, Session::Wayland);
        assert!(!d.use_gamescope);
        assert!(d.reason.contains("not installed"));

        let d = decide(
            Mode::Enabled,
            &Config::default(),
            Some(&modern()),
            Session::Tty,
        );
        assert!(!d.use_gamescope);

        let d = decide(
            Mode::Enabled,
            &Config::default(),
            Some(&modern()),
            Session::Wayland,
        );
        assert!(d.use_gamescope);
    }

    #[test]
    fn auto_declines_when_nothing_needs_a_compositor() {
        // The default profile changes nothing, so wrapping it is pure cost.
        let d = decide(
            Mode::Auto,
            &Config::default(),
            Some(&modern()),
            Session::Wayland,
        );
        assert!(!d.use_gamescope);
        assert!(d.reason.contains("nothing in this profile needs Gamescope"));
    }

    #[test]
    fn auto_accepts_when_the_profile_asks_for_scaling() {
        let cfg = Config {
            render_width: 2560,
            render_height: 1080,
            output_width: 3440,
            output_height: 1440,
            ..Config::default()
        };
        let d = decide(Mode::Auto, &cfg, Some(&modern()), Session::Wayland);
        assert!(d.use_gamescope);
        assert!(d.reason.contains("2560×1080"));
    }

    #[test]
    fn auto_ignores_a_resolution_that_is_not_actually_scaling() {
        // Same in and out: Gamescope would copy the frame for nothing.
        let cfg = Config {
            render_width: 3440,
            render_height: 1440,
            output_width: 3440,
            output_height: 1440,
            ..Config::default()
        };
        assert!(!decide(Mode::Auto, &cfg, Some(&modern()), Session::Wayland).use_gamescope);
    }

    #[test]
    fn auto_accepts_each_feature_gamescope_is_the_right_tool_for() {
        let cases = [
            (
                Config {
                    filter: Filter::Fsr,
                    ..Config::default()
                },
                "fsr",
            ),
            (
                Config {
                    frame_limit: FrameLimit::NestedRefresh(144),
                    ..Config::default()
                },
                "frame-rate",
            ),
            (
                Config {
                    hdr: true,
                    ..Config::default()
                },
                "HDR",
            ),
            (
                Config {
                    adaptive_sync: true,
                    ..Config::default()
                },
                "variable refresh",
            ),
            (
                Config {
                    mangoapp: true,
                    ..Config::default()
                },
                "overlay",
            ),
        ];
        for (cfg, expected) in cases {
            let d = decide(Mode::Auto, &cfg, Some(&modern()), Session::Wayland);
            assert!(d.use_gamescope, "expected yes for {expected}: {}", d.reason);
            assert!(
                d.reason.contains(expected),
                "reason {:?} should mention {expected}",
                d.reason
            );
        }
    }

    #[test]
    fn every_decision_explains_itself() {
        for mode in [Mode::Auto, Mode::Enabled, Mode::Disabled] {
            for caps in [Some(modern()), None] {
                for session in [Session::Wayland, Session::X11, Session::Tty] {
                    let d = decide(mode, &Config::default(), caps.as_ref(), session);
                    assert!(!d.reason.is_empty(), "{mode:?} gave no reason");
                }
            }
        }
    }

    #[test]
    fn mode_round_trips_through_toml() {
        for mode in [Mode::Auto, Mode::Enabled, Mode::Disabled] {
            #[derive(serde::Serialize, serde::Deserialize, PartialEq, Debug)]
            struct W {
                mode: Mode,
            }
            let text = toml::to_string(&W { mode }).unwrap();
            assert_eq!(toml::from_str::<W>(&text).unwrap().mode, mode);
        }
        // Auto is the default, so an older profile with no field gets Auto.
        assert_eq!(Mode::default(), Mode::Auto);
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
