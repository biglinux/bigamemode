//! One game's own launch settings, over the general ones (Tuning).
//!
//! Tuning's launch settings (`video.toml`) are what every game gets: the
//! Gamescope filter, sharpness and sizes, Wine FSR and vkBasalt. A game's
//! profile can set any of them for itself; every value it leaves unset
//! follows Tuning, so changing Tuning still reaches the game. They are kept
//! with Big Game Mode's other per-game settings (`crate::game_settings`), not
//! in falcond's profile: falcond does not read them, and they need no root.
//!
//! They reach a game the ways its other launch settings do: Big Game Mode's
//! own launch (`crate::launcher`), a Steam game's launch options
//! (`crate::steam_gamescope`), written with Steam closed, and a Heroic
//! game's settings in Heroic (`crate::heroic_launch`), written with Heroic
//! closed. A game Lutris or Flatpak starts gets them only when Big Game Mode
//! starts it.

use serde::{Deserialize, Serialize};

use crate::gamescope::{self, Mode};
use crate::models::{GamescopeFilter, UpscalingSettings, WineFsrMode};
use crate::text::N_;

/// A size in pixels: width, height. `(0, 0)` is "none": the game's own
/// size for the render size, the render size for the output.
pub type Size = (u32, u32);

/// A game's own launch settings. `None` follows Tuning.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct GameLaunch {
    /// Gamescope's render size (`-w`/`-h`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub render: Option<Size>,
    /// Gamescope's output size (`-W`/`-H`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<Size>,
    /// Gamescope's upscaling filter.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filter: Option<GamescopeFilter>,
    /// FSR sharpness, 0 (sharpest) to 20.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sharpness: Option<u8>,
    /// A frame limit through Gamescope (`-r`); 0 is none. Only a game has
    /// one: Tuning has no frame limit for every game.
    #[serde(skip_serializing_if = "is_zero")]
    pub frame_limit: u32,
    /// Wine FSR on or off.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wine_fsr: Option<bool>,
    /// Wine FSR's quality mode.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wine_fsr_mode: Option<WineFsrMode>,
    /// vkBasalt on or off.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vkbasalt: Option<bool>,
}

// serde's `skip_serializing_if` passes a reference.
#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_zero(n: &u32) -> bool {
    *n == 0
}

fn zero_is_none(size: Size) -> Size {
    if size.0 == 0 || size.1 == 0 {
        (0, 0)
    } else {
        size
    }
}

impl GameLaunch {
    /// Whether everything follows Tuning.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// Whether the game sets anything of Gamescope's for itself.
    #[must_use]
    pub fn sets_gamescope(&self) -> bool {
        self.render.is_some()
            || self.output.is_some()
            || self.filter.is_some()
            || self.sharpness.is_some()
            || self.frame_limit > 0
    }

    /// The settings a profile's `[gamescope]` table held, before they moved
    /// here: its sizes (none is the game's own size), its filter where
    /// Tuning has one, and its frame limit.
    #[must_use]
    pub fn from_legacy(cfg: &gamescope::Config) -> Self {
        Self {
            render: Some(zero_is_none((cfg.render_width, cfg.render_height))),
            output: (cfg.output_width > 0 && cfg.output_height > 0)
                .then_some((cfg.output_width, cfg.output_height)),
            filter: match cfg.filter {
                gamescope::Filter::Fsr => Some(GamescopeFilter::Fsr),
                gamescope::Filter::Nis => Some(GamescopeFilter::Nis),
                gamescope::Filter::Integer => Some(GamescopeFilter::Integer),
                _ => None,
            },
            sharpness: cfg.filter.uses_sharpness().then_some(cfg.sharpness.min(20)),
            frame_limit: match cfg.frame_limit {
                gamescope::FrameLimit::NestedRefresh(hz) => hz,
                gamescope::FrameLimit::None => 0,
            },
            ..Self::default()
        }
    }

    /// `general` with this game's values in place of Tuning's. A game set
    /// to Always gets Gamescope on, so Tuning's filter and sharpness apply
    /// to it as they do to every game when Gamescope is on there.
    #[must_use]
    pub fn over(&self, general: &UpscalingSettings, mode: Mode) -> UpscalingSettings {
        let mut u = general.clone();
        if mode == Mode::Enabled {
            u.gamescope_enabled = true;
        }
        if let Some((w, h)) = self.render.map(zero_is_none) {
            (u.base_width, u.base_height) = (w, h);
        }
        if let Some((w, h)) = self.output.map(zero_is_none) {
            (u.target_width, u.target_height) = (w, h);
        }
        if let Some(f) = self.filter {
            u.gamescope_filter = f;
        }
        if let Some(s) = self.sharpness {
            u.gamescope_sharpness = s.min(20);
        }
        if let Some(on) = self.wine_fsr {
            u.wine_fsr_enabled = on;
        }
        if let Some(m) = self.wine_fsr_mode {
            u.wine_fsr_mode = m;
        }
        if let Some(on) = self.vkbasalt {
            u.vkbasalt_enabled = on;
        }
        u
    }

    /// The per-game Gamescope configuration for the launch: this game's own
    /// sizes, filter and frame limit, and Tuning's sizes where the game has
    /// none and Gamescope is on there (`effective` is [`Self::over`]'s). It
    /// carries them when Gamescope is off in Tuning, where the launch reads
    /// the sizes and the filter from the game alone. `None` when the game
    /// sets nothing of Gamescope's.
    #[must_use]
    pub fn gamescope_override(&self, effective: &UpscalingSettings) -> Option<gamescope::Config> {
        if !self.sets_gamescope() {
            return None;
        }
        let on = effective.gamescope_enabled;
        let size = |own: Option<Size>, general: Size| match own {
            Some(s) => zero_is_none(s),
            None if on => general,
            None => (0, 0),
        };
        let render = size(self.render, (effective.base_width, effective.base_height));
        let output = size(
            self.output,
            (effective.target_width, effective.target_height),
        );
        Some(gamescope::Config {
            render_width: render.0,
            render_height: render.1,
            output_width: output.0,
            output_height: output.1,
            filter: match self.filter {
                Some(GamescopeFilter::Fsr) => gamescope::Filter::Fsr,
                Some(GamescopeFilter::Nis) => gamescope::Filter::Nis,
                Some(GamescopeFilter::Integer) => gamescope::Filter::Integer,
                None => gamescope::Filter::Linear,
            },
            sharpness: effective.clamped_sharpness(),
            frame_limit: match self.frame_limit {
                0 => gamescope::FrameLimit::None,
                hz => gamescope::FrameLimit::NestedRefresh(hz),
            },
            ..gamescope::Config::default()
        })
    }

    /// The Gamescope configuration the game gets: Tuning's with the game's
    /// own values in their place, merged as the launch merges them.
    #[must_use]
    pub fn config(&self, general: &UpscalingSettings, mode: Mode) -> gamescope::Config {
        let u = self.over(general, mode);
        let own = self.gamescope_override(&u);
        crate::launcher::LaunchPlan::merge_gamescope_config(&u, own.as_ref())
    }

    /// Whether Gamescope runs for this game, and why, as the launch decides
    /// it: `steam` for a game whose Steam launch options carry it, where
    /// Automatic runs Gamescope only for the game's own values.
    #[must_use]
    pub fn decide(
        &self,
        general: &UpscalingSettings,
        mode: Mode,
        steam: bool,
        caps: Option<&crate::capabilities::GamescopeCaps>,
        session: crate::hardware::Session,
    ) -> gamescope::Decision {
        if steam && mode == Mode::Auto && !self.sets_gamescope() {
            return gamescope::Decision {
                use_gamescope: false,
                reason: N_("this game sets nothing of its own that needs Gamescope").into(),
            };
        }
        let follows = !steam && self.over(general, mode).gamescope_enabled;
        let mode = match mode {
            Mode::Auto if follows => Mode::Enabled,
            m => m,
        };
        gamescope::decide(mode, &self.config(general, mode), caps, session)
    }

    /// Whether Gamescope runs this game at a render size of its own, which
    /// it enlarges: the job Wine FSR would do a second time. `follows` says
    /// whether Automatic follows Tuning's Gamescope switch (a game
    /// Big Game Mode starts) or runs only for the game's own values (a Steam
    /// game, whose launch options only the profile writes).
    #[must_use]
    pub fn upscales(&self, general: &UpscalingSettings, mode: Mode, follows: bool) -> bool {
        // Whether it runs, as the launch decides it where Gamescope is
        // installed and there is a session.
        let caps = crate::capabilities::GamescopeCaps::default();
        let session = crate::hardware::Session::Wayland;
        let runs = self
            .decide(general, mode, !follows, Some(&caps), session)
            .use_gamescope;
        let cfg = self.config(general, mode);
        runs && cfg.render_width > 0 && cfg.render_height > 0
    }

    /// The variables a Steam game's launch options get, in front of
    /// everything, for the values this game sets itself (a game that sets
    /// none gets Tuning's from the session). Where Gamescope or `OptiScaler`
    /// upscales the game, or Wine FSR is switched off for it
    /// (`wine_fsr_off`), Wine FSR is switched off whatever Tuning says: the
    /// session can hold it on from a Turbo preset, or from before Big Game Mode
    /// started, and two upscalers never run in series.
    #[must_use]
    pub fn steam_env(
        &self,
        general: &UpscalingSettings,
        gamescope_upscales: bool,
        wine_fsr_off: bool,
    ) -> Vec<String> {
        let mut words = Vec::new();
        let u = self.over(general, Mode::Auto);
        if gamescope_upscales || wine_fsr_off {
            words.push("WINE_FULLSCREEN_FSR=0".to_owned());
        } else {
            let mode = || {
                format!(
                    "WINE_FULLSCREEN_FSR_MODE={}",
                    wine_fsr_mode_word(u.wine_fsr_mode)
                )
            };
            match self.wine_fsr {
                Some(true) => {
                    words.push("WINE_FULLSCREEN_FSR=1".to_owned());
                    words.push(mode());
                }
                Some(false) => words.push("WINE_FULLSCREEN_FSR=0".to_owned()),
                None if self.wine_fsr_mode.is_some() && general.wine_fsr_enabled => {
                    words.push(mode());
                }
                None => {}
            }
        }
        match self.vkbasalt {
            Some(true) => words.push("ENABLE_VKBASALT=1".to_owned()),
            Some(false) => words.push("ENABLE_VKBASALT=0".to_owned()),
            None => {}
        }
        words
    }

    /// What a Heroic game's settings get for the values this game sets
    /// itself: Heroic's own Wine FSR switch (`enableFSR`: Heroic sets
    /// `WINE_FULLSCREEN_FSR` from it over the game's variables) and the
    /// variables. Where Gamescope or `OptiScaler` upscales the game, Wine
    /// FSR must be off, as Big Game Mode's own launch has it — never two
    /// upscalers: no switch and no mode here, and the caller switches
    /// Heroic's off where it is on. `vkbasalt_config` is Tuning's vkBasalt
    /// file, for a Heroic that can read it.
    #[must_use]
    pub fn heroic_env(
        &self,
        general: &UpscalingSettings,
        gamescope_upscales: bool,
        optiscaler_upscales: bool,
        vkbasalt_config: Option<&str>,
    ) -> (Option<bool>, Vec<(String, String)>) {
        let mut env = Vec::new();
        let u = self.over(general, Mode::Auto);
        let mode = || {
            (
                "WINE_FULLSCREEN_FSR_MODE".to_owned(),
                wine_fsr_mode_word(u.wine_fsr_mode).to_owned(),
            )
        };
        let wine_fsr = if optiscaler_upscales || gamescope_upscales {
            // Off, which Heroic's settings get as "off where it is on"
            // (`crate::heroic_launch::Wanted::wine_fsr_off_where_on`).
            None
        } else {
            match self.wine_fsr {
                Some(true) => env.push(mode()),
                // Over Heroic's own switch, when that is on.
                None if self.wine_fsr_mode.is_some() => env.push(mode()),
                _ => {}
            }
            self.wine_fsr
        };
        match self.vkbasalt {
            Some(true) => {
                env.push(("ENABLE_VKBASALT".to_owned(), "1".to_owned()));
                if let Some(path) = vkbasalt_config {
                    env.push(("VKBASALT_CONFIG_FILE".to_owned(), path.to_owned()));
                }
            }
            Some(false) => env.push(("ENABLE_VKBASALT".to_owned(), "0".to_owned())),
            None => {}
        }
        (wine_fsr, env)
    }
}

/// The word `WINE_FULLSCREEN_FSR_MODE` takes for `mode`.
#[must_use]
pub fn wine_fsr_mode_word(mode: WineFsrMode) -> &'static str {
    match mode {
        WineFsrMode::Performance => "performance",
        WineFsrMode::Balanced => "balanced",
        WineFsrMode::Quality => "quality",
        WineFsrMode::Ultra => "ultra",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn general() -> UpscalingSettings {
        UpscalingSettings {
            gamescope_enabled: false,
            gamescope_filter: GamescopeFilter::Fsr,
            base_width: 1280,
            base_height: 720,
            target_width: 2560,
            target_height: 1440,
            gamescope_sharpness: 5,
            wine_fsr_enabled: true,
            wine_fsr_mode: WineFsrMode::Quality,
            vkbasalt_enabled: false,
            vkbasalt_config_path: None,
        }
    }

    #[test]
    fn nothing_set_is_tuning_itself() {
        let own = GameLaunch::default();
        assert!(own.is_empty() && !own.sets_gamescope());
        assert_eq!(own.over(&general(), Mode::Auto), general());
        assert_eq!(own.gamescope_override(&general()), None);
    }

    #[test]
    fn a_games_own_value_replaces_tunings_and_only_that_one() {
        let own = GameLaunch {
            render: Some((0, 0)),
            filter: Some(GamescopeFilter::Nis),
            wine_fsr: Some(false),
            vkbasalt: Some(true),
            ..GameLaunch::default()
        };
        let u = own.over(&general(), Mode::Auto);
        // The game's own size, over Tuning's 1280 × 720.
        assert_eq!((u.base_width, u.base_height), (0, 0));
        assert_eq!((u.target_width, u.target_height), (2560, 1440));
        assert_eq!(u.gamescope_filter, GamescopeFilter::Nis);
        assert_eq!(u.gamescope_sharpness, 5);
        assert!(!u.wine_fsr_enabled && u.vkbasalt_enabled);
        assert!(!u.gamescope_enabled, "Automatic keeps Tuning's switch");
        assert!(own.over(&general(), Mode::Enabled).gamescope_enabled);
    }

    #[test]
    fn the_override_carries_the_games_filter_and_frame_limit() {
        let own = GameLaunch {
            render: Some((1920, 1080)),
            frame_limit: 60,
            ..GameLaunch::default()
        };
        let u = own.over(&general(), Mode::Auto);
        let cfg = own.gamescope_override(&u).unwrap();
        assert_eq!((cfg.render_width, cfg.render_height), (1920, 1080));
        // Gamescope is off in Tuning: its output size is not the game's.
        assert_eq!((cfg.output_width, cfg.output_height), (0, 0));
        assert_eq!(cfg.filter, gamescope::Filter::Linear, "none of its own");
        assert_eq!(cfg.frame_limit, gamescope::FrameLimit::NestedRefresh(60));
        // On there, or with Always, the game's own size goes with Tuning's
        // output.
        let cfg = own.config(&general(), Mode::Enabled);
        assert_eq!((cfg.render_width, cfg.output_width), (1920, 2560));
    }

    #[test]
    fn a_profiles_old_gamescope_table_moves_over() {
        let old = gamescope::Config {
            render_width: 1600,
            render_height: 900,
            filter: gamescope::Filter::Fsr,
            sharpness: 3,
            frame_limit: gamescope::FrameLimit::NestedRefresh(72),
            ..gamescope::Config::default()
        };
        let own = GameLaunch::from_legacy(&old);
        assert_eq!(own.render, Some((1600, 900)));
        assert_eq!(own.output, None);
        assert_eq!(own.filter, Some(GamescopeFilter::Fsr));
        assert_eq!(own.sharpness, Some(3));
        assert_eq!(own.frame_limit, 72);
        // Linear was "no filter": Tuning's then.
        let plain = GameLaunch::from_legacy(&gamescope::Config::default());
        assert_eq!((plain.render, plain.filter), (Some((0, 0)), None));
    }

    #[test]
    fn upscaling_follows_the_switch_only_where_automatic_does() {
        let mut g = general();
        let own = GameLaunch::default();
        assert!(
            !own.upscales(&g, Mode::Auto, true),
            "Gamescope off in Tuning"
        );
        assert!(own.upscales(&g, Mode::Enabled, false));
        g.gamescope_enabled = true;
        assert!(own.upscales(&g, Mode::Auto, true));
        assert!(!own.upscales(&g, Mode::Auto, false), "a Steam game");
        assert!(!own.upscales(&g, Mode::Disabled, true));
        let native = GameLaunch {
            render: Some((0, 0)),
            ..GameLaunch::default()
        };
        assert!(!native.upscales(&g, Mode::Enabled, true));
        // A render size alone, where Automatic has nothing to enlarge it
        // to, does not run Gamescope at all.
        let alone = GameLaunch {
            render: Some((1280, 720)),
            ..GameLaunch::default()
        };
        assert!(!alone.upscales(&general(), Mode::Auto, false));
        let to = GameLaunch {
            output: Some((2560, 1440)),
            ..alone
        };
        assert!(to.upscales(&general(), Mode::Auto, false));
    }

    #[test]
    fn the_decision_is_the_launchs() {
        let caps = crate::capabilities::GamescopeCaps {
            version: None,
            flags: Vec::new(),
        };
        let wayland = crate::hardware::Session::Wayland;
        let mut g = general();
        let none = GameLaunch::default();
        let decide = |own: &GameLaunch, g: &UpscalingSettings, mode, steam| {
            own.decide(g, mode, steam, Some(&caps), wayland)
                .use_gamescope
        };
        // Automatic follows Tuning's switch for a game Big Game Mode starts…
        assert!(!decide(&none, &g, Mode::Auto, false));
        g.gamescope_enabled = true;
        assert!(decide(&none, &g, Mode::Auto, false));
        // …and not for a Steam game, which needs values of its own.
        assert!(!decide(&none, &g, Mode::Auto, true));
        let own = GameLaunch {
            render: Some((1920, 1080)),
            ..GameLaunch::default()
        };
        assert!(decide(&own, &g, Mode::Auto, true));
        assert!(!decide(&own, &g, Mode::Disabled, false));
        assert!(decide(&none, &general(), Mode::Enabled, true));
    }

    #[test]
    fn a_steam_game_gets_only_the_variables_it_sets() {
        let g = general();
        assert!(GameLaunch::default().steam_env(&g, false, false).is_empty());
        let on = GameLaunch {
            wine_fsr: Some(true),
            wine_fsr_mode: Some(WineFsrMode::Ultra),
            vkbasalt: Some(false),
            ..GameLaunch::default()
        };
        assert_eq!(
            on.steam_env(&g, false, false),
            [
                "WINE_FULLSCREEN_FSR=1",
                "WINE_FULLSCREEN_FSR_MODE=ultra",
                "ENABLE_VKBASALT=0"
            ]
        );
        // Gamescope upscales it: never two upscalers.
        assert_eq!(
            on.steam_env(&g, true, false),
            ["WINE_FULLSCREEN_FSR=0", "ENABLE_VKBASALT=0"]
        );
        // OptiScaler upscales it, or Wine FSR is switched off for it: the
        // same switch, from the same owner.
        assert_eq!(
            on.steam_env(&g, false, true),
            ["WINE_FULLSCREEN_FSR=0", "ENABLE_VKBASALT=0"]
        );
        // A quality mode alone, over Tuning's Wine FSR.
        let mode = GameLaunch {
            wine_fsr_mode: Some(WineFsrMode::Performance),
            ..GameLaunch::default()
        };
        assert_eq!(
            mode.steam_env(&g, false, false),
            ["WINE_FULLSCREEN_FSR_MODE=performance"]
        );
        let off = UpscalingSettings {
            wine_fsr_enabled: false,
            ..g
        };
        assert!(mode.steam_env(&off, false, false).is_empty());
        // Tuning's Wine FSR with this game's Gamescope upscaling.
        assert_eq!(
            GameLaunch::default().steam_env(&general(), true, false),
            ["WINE_FULLSCREEN_FSR=0"]
        );
        // Tuning's off, but the session may hold a Turbo preset's Wine FSR:
        // switched off all the same where something else upscales.
        for (gamescope, optiscaler) in [(true, false), (false, true)] {
            assert_eq!(
                GameLaunch::default().steam_env(&off, gamescope, optiscaler),
                ["WINE_FULLSCREEN_FSR=0"]
            );
        }
    }

    #[test]
    fn a_heroic_game_gets_heroics_switch_and_only_the_variables_it_sets() {
        let g = general();
        assert_eq!(
            GameLaunch::default().heroic_env(&g, false, false, None),
            (None, Vec::new())
        );
        let on = GameLaunch {
            wine_fsr: Some(true),
            wine_fsr_mode: Some(WineFsrMode::Ultra),
            vkbasalt: Some(true),
            ..GameLaunch::default()
        };
        let pair = |k: &str, v: &str| (k.to_owned(), v.to_owned());
        assert_eq!(
            on.heroic_env(&g, false, false, Some("/cfg/vk.conf")),
            (
                Some(true),
                vec![
                    pair("WINE_FULLSCREEN_FSR_MODE", "ultra"),
                    pair("ENABLE_VKBASALT", "1"),
                    pair("VKBASALT_CONFIG_FILE", "/cfg/vk.conf"),
                ]
            )
        );
        // Gamescope or OptiScaler upscales it: no Wine FSR of the game's own
        // (Heroic's goes off where it is on, `heroic_launch`).
        for (gamescope, optiscaler) in [(true, false), (false, true)] {
            let (fsr, env) = on.heroic_env(&g, gamescope, optiscaler, None);
            assert_eq!(fsr, None);
            assert_eq!(env, [pair("ENABLE_VKBASALT", "1")]);
        }
        // A mode alone goes over Heroic's own switch.
        let mode = GameLaunch {
            wine_fsr_mode: Some(WineFsrMode::Performance),
            ..GameLaunch::default()
        };
        assert_eq!(
            mode.heroic_env(&g, false, false, None),
            (None, vec![pair("WINE_FULLSCREEN_FSR_MODE", "performance")])
        );
    }

    #[test]
    fn it_reads_and_writes_as_a_short_table() {
        let own = GameLaunch {
            render: Some((1280, 720)),
            wine_fsr: Some(false),
            ..GameLaunch::default()
        };
        let text = toml::to_string(&own).unwrap();
        assert!(text.contains("render = [1280, 720]"), "{text}");
        assert!(
            !text.contains("output") && !text.contains("frame_limit"),
            "{text}"
        );
        assert_eq!(toml::from_str::<GameLaunch>(&text).unwrap(), own);
        assert_eq!(
            toml::from_str::<GameLaunch>("").unwrap(),
            GameLaunch::default()
        );
    }
}
