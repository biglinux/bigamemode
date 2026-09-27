//! One model for what Tuning, a game's profile and the profile wizard set.
//!
//! The three pages show the same settings; only the scope changes. Tuning is
//! the *general* configuration every game gets, a profile is one game's,
//! and the wizard builds exactly the object the profile editor edits. What
//! they share lives here, so a page never decides a rule on its own:
//!
//! * the fixed choices (scheduler modes, 3D V-Cache modes, falcond's profile
//!   sets), with the explanation each `(i)` shows;
//! * where an effective value comes from — falcond's own rules, read in its
//!   source: the global `scx_sched`/`vcache_mode` are loaded when falcond
//!   starts, and a profile's `none` keeps them (it does not mean "off");
//! * which technologies must not run together, answered by the one
//!   compatibility matrix, [`crate::graphics::rules`];
//! * [`GameOptimization`], one game's settings read from each owner and saved
//!   back to each owner in one call.

use anyhow::Result;

use crate::config::FalcondConfig;
use crate::graphics::rules::{self, Tech, Verdict};
use crate::models::{FrameGenBackend, UpscalingSettings};
use crate::profiles::GameProfile;
use crate::text::N_;
use crate::video_config::VideoConfig;

// ── Choices ─────────────────────────────────────────────────────────────────

/// One fixed value: what is written, its name and what it does. `label` and
/// `help` are marked for translation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Choice {
    /// The value in falcond's files.
    pub id: &'static str,
    /// Its name.
    pub label: &'static str,
    /// What choosing it does, in a sentence or two.
    pub help: &'static str,
}

const fn choice(id: &'static str, label: &'static str, help: &'static str) -> Choice {
    Choice { id, label, help }
}

/// `scx_sched_props`: the mode `scx_loader` starts a scheduler in. Each
/// scheduler maps a mode to its own options.
pub const SCHEDULER_MODES: &[Choice] = &[
    choice(
        "default",
        N_("Default"),
        N_("The scheduler as its authors tuned it. The safe choice when in doubt."),
    ),
    choice(
        "gaming",
        N_("Gaming"),
        N_(
            "Favours the game's threads and steady frame pacing over background work. The usual choice for playing.",
        ),
    ),
    choice(
        "power",
        N_("Power saving"),
        N_(
            "Keeps fewer cores busy so the others can sleep: for laptops on battery and light games. It can lower the frame rate.",
        ),
    ),
    choice(
        "latency",
        N_("Low latency"),
        N_(
            "Answers input and audio first, at some cost in throughput. For competitive games and streaming.",
        ),
    ),
    choice(
        "server",
        N_("Server"),
        N_("Throughput for long batch work, not for games. Listed because falcond accepts it."),
    ),
];

/// `vcache_mode`: which CCD of a dual-CCD X3D processor games prefer.
pub const VCACHE_MODES: &[Choice] = &[
    choice(
        "none",
        N_("Unchanged"),
        N_("BiGame-mode and falcond leave the driver's current preference alone."),
    ),
    choice(
        "cache",
        N_("Cache CCD"),
        N_(
            "Games prefer the CCD with the stacked 3D V-Cache. Best for most games: they gain more from cache than from clock speed.",
        ),
    ),
    choice(
        "freq",
        N_("Frequency CCD"),
        N_(
            "Games prefer the CCD without the extra cache, which clocks higher. For the few games that gain from clock speed more than from cache.",
        ),
    ),
];

/// `profile_mode`: which set of falcond's shipped profiles is used.
pub const PROFILE_SETS: &[Choice] = &[
    choice(
        "none",
        N_("Desktop"),
        N_("falcond's profiles for desktops and laptops. The right choice for a PC at a desk."),
    ),
    choice(
        "handheld",
        N_("Handheld"),
        N_(
            "Profiles for handheld PCs such as the Steam Deck or the ROG Ally: they favour battery life, and some switch performance mode off.",
        ),
    ),
    choice(
        "htpc",
        N_("Living room (HTPC)"),
        N_("Profiles for a PC connected to a TV and played from the couch."),
    ),
];

/// The choice with `id`, if there is one.
#[must_use]
pub fn find(choices: &[Choice], id: &str) -> Option<Choice> {
    choices.iter().copied().find(|c| c.id == id)
}

/// Where `id` sits in `choices`; the first when it is not there.
#[must_use]
pub fn index_of(choices: &[Choice], id: &str) -> u32 {
    choices
        .iter()
        .position(|c| c.id == id)
        .and_then(|i| u32::try_from(i).ok())
        .unwrap_or(0)
}

// ── Where a value comes from ────────────────────────────────────────────────

/// Who decides an effective value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// The game's profile.
    Game,
    /// The general configuration (Tuning).
    General,
    /// Nobody: the system as it is (the kernel's scheduler, the driver's
    /// preference).
    System,
}

/// Whether a profile's scheduler or 3D V-Cache value keeps the general one:
/// falcond switches only on a value other than `none`.
#[must_use]
pub fn inherits(value: &str) -> bool {
    value.is_empty() || value == "none"
}

/// The scheduler a game runs with, its mode, and who chose it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Effective {
    /// The value (`none` for the kernel's own scheduler / no preference).
    pub value: String,
    /// The mode, for the scheduler; empty otherwise.
    pub mode: String,
    /// Who chose it.
    pub source: Source,
}

/// The scheduler falcond runs a game with: the profile's when it names one,
/// the general one otherwise (falcond loaded it at start-up and a profile's
/// `none` keeps it), the kernel's when neither does.
#[must_use]
pub fn effective_scheduler(profile: Option<&GameProfile>, general: &FalcondConfig) -> Effective {
    if let Some(p) = profile.filter(|p| !inherits(&p.scx_sched)) {
        return Effective {
            value: p.scx_sched.clone(),
            mode: p.scx_sched_props.clone(),
            source: Source::Game,
        };
    }
    if !inherits(&general.scx_sched) {
        return Effective {
            value: general.scx_sched.clone(),
            mode: general.scx_sched_props.clone(),
            source: Source::General,
        };
    }
    Effective {
        value: "none".to_owned(),
        mode: String::new(),
        source: Source::System,
    }
}

/// The 3D V-Cache preference a game runs with, and who chose it.
#[must_use]
pub fn effective_vcache(profile: Option<&GameProfile>, general: &FalcondConfig) -> Effective {
    if let Some(p) = profile.filter(|p| !inherits(&p.vcache_mode)) {
        return Effective {
            value: p.vcache_mode.clone(),
            mode: String::new(),
            source: Source::Game,
        };
    }
    if !inherits(&general.vcache_mode) {
        return Effective {
            value: general.vcache_mode.clone(),
            mode: String::new(),
            source: Source::General,
        };
    }
    Effective {
        value: "none".to_owned(),
        mode: String::new(),
        source: Source::System,
    }
}

/// Whether a game gets the performance power profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Performance {
    /// Yes.
    On,
    /// Its profile does not ask for it.
    Off,
    /// Its profile asks, but performance mode is off in the general
    /// configuration, which falcond reads as off for every game.
    BlockedByGeneral,
}

/// See [`Performance`].
#[must_use]
pub fn performance(profile: &GameProfile, general: &FalcondConfig) -> Performance {
    match (profile.performance_mode, general.enable_performance_mode) {
        (false, _) => Performance::Off,
        (true, true) => Performance::On,
        (true, false) => Performance::BlockedByGeneral,
    }
}

// ── Conflicts ───────────────────────────────────────────────────────────────

/// A technology a setting switches on, where two of them can collide.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Feature {
    /// Wine's fullscreen FSR (general).
    WineFsr,
    /// Gamescope rendering below its output size (general or per game).
    GamescopeUpscaling,
    /// `OptiScaler` upscaling (a game's AI Graphics).
    OptiScalerUpscaling,
    /// `OptiScaler` frame generation (a game's AI Graphics).
    OptiScalerFrameGen,
    /// lsfg-vk frame generation (a game's entry).
    LsfgVk,
}

impl Feature {
    /// The technology in the compatibility matrix.
    #[must_use]
    pub fn tech(self) -> Tech {
        match self {
            Self::WineFsr => Tech::WineFsr,
            Self::GamescopeUpscaling => Tech::GamescopeUpscaling,
            Self::OptiScalerUpscaling => Tech::OptiScalerUpscaler,
            Self::OptiScalerFrameGen => Tech::OptiScalerFrameGen,
            Self::LsfgVk => Tech::LsfgVk,
        }
    }

    /// A short name for a button ("Use …", "Keep …"), marked for
    /// translation.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::WineFsr => "Wine FSR",
            Self::GamescopeUpscaling => N_("Gamescope upscaling"),
            Self::OptiScalerUpscaling => N_("OptiScaler upscaling"),
            Self::OptiScalerFrameGen => N_("OptiScaler frame generation"),
            Self::LsfgVk => "lsfg-vk",
        }
    }

    /// Whether this is a frame generator (the other job is upscaling).
    #[must_use]
    pub fn generates_frames(self) -> bool {
        matches!(self, Self::OptiScalerFrameGen | Self::LsfgVk)
    }
}

/// Two technologies that must not both be on, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conflict {
    /// What the user is switching on.
    pub requested: Feature,
    /// What is already on.
    pub active: Feature,
    /// The matrix's reason, marked for translation.
    pub why: &'static str,
}

/// The conflict between switching on `requested` while `active` is on, as
/// the compatibility matrix rules it; `None` when they may run together.
#[must_use]
pub fn conflict(requested: Feature, active: Feature) -> Option<Conflict> {
    if requested == active {
        return None;
    }
    let rule = rules::check(requested.tech(), active.tech());
    (rule.verdict == Verdict::Conflict).then_some(Conflict {
        requested,
        active,
        why: rule.why,
    })
}

/// Whether the general Gamescope settings upscale: Gamescope on, with a
/// render size below the output.
#[must_use]
pub fn gamescope_upscales(u: &UpscalingSettings) -> bool {
    u.gamescope_enabled && u.base_width > 0 && u.base_height > 0
}

/// Whether a game's own Gamescope settings upscale.
#[must_use]
pub fn game_gamescope_upscales(profile: &GameProfile) -> bool {
    profile.gamescope_mode != crate::gamescope::Mode::Disabled
        && profile
            .gamescope
            .as_ref()
            .is_some_and(|g| g.render_width > 0 && g.render_height > 0)
}

/// The general features switched on in `video`.
#[must_use]
pub fn general_features(video: &VideoConfig) -> Vec<Feature> {
    let mut on = Vec::new();
    if video.upscaling.wine_fsr_enabled {
        on.push(Feature::WineFsr);
    }
    if gamescope_upscales(&video.upscaling) {
        on.push(Feature::GamescopeUpscaling);
    }
    on
}

/// Conflicts already in the general configuration — a file from an older
/// version, or edited by hand — to be named on the page with a way out.
#[must_use]
pub fn general_conflicts(video: &VideoConfig) -> Vec<Conflict> {
    let on = general_features(video);
    let mut out = Vec::new();
    for (i, &a) in on.iter().enumerate() {
        for &b in &on[i + 1..] {
            if let Some(c) = conflict(a, b) {
                out.push(c);
            }
        }
    }
    out
}

/// Switch a general feature off in `video`. Gamescope upscaling goes by
/// dropping the render size: Gamescope keeps running if it was on for
/// anything else (a frame limit, a stable fullscreen).
pub fn turn_off_general(video: &mut VideoConfig, feature: Feature) {
    match feature {
        Feature::WineFsr => video.upscaling.wine_fsr_enabled = false,
        Feature::GamescopeUpscaling => {
            video.upscaling.base_width = 0;
            video.upscaling.base_height = 0;
        }
        // Per-game features: nothing general to change.
        Feature::OptiScalerUpscaling | Feature::OptiScalerFrameGen | Feature::LsfgVk => {}
    }
}

/// What `OptiScaler` does in a game BiGame-mode installed it into: the
/// upscaler, and whether it generates frames (chosen on its page, or later
/// from its own overlay).
#[must_use]
pub fn optiscaler_features(process: &str) -> Vec<Feature> {
    let off = crate::graphics::launch_disables(
        &crate::graphics::state_dir(),
        &crate::game_settings::dir(),
        process,
    );
    if off.is_empty() {
        return Vec::new();
    }
    let mut on = vec![Feature::OptiScalerUpscaling];
    if off.contains(&Tech::LsfgVk) {
        on.push(Feature::OptiScalerFrameGen);
    }
    on
}

// ── One game's settings ─────────────────────────────────────────────────────

/// lsfg-vk's values for one game.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameGeneration {
    /// Frames shown per rendered frame; 1 is off.
    pub multiplier: u32,
    /// Motion estimation resolution, 25–100 %.
    pub flow_scale: u32,
    /// lsfg-vk's performance mode.
    pub performance: bool,
    /// HDR.
    pub hdr: bool,
    /// Present mode index (0 FIFO, 1 lsfg-vk's choice, 2 mailbox, 3 immediate).
    pub present_mode: u32,
}

impl Default for FrameGeneration {
    fn default() -> Self {
        Self {
            multiplier: 1,
            flow_scale: 100,
            performance: false,
            hdr: false,
            present_mode: 1,
        }
    }
}

impl FrameGeneration {
    /// Whether it generates frames.
    #[must_use]
    pub fn on(&self) -> bool {
        self.multiplier > 1
    }
}

/// One game's settings, whichever page made them: the falcond profile (with
/// BiGame-mode's Gamescope choice), lsfg-vk's entry and `MangoHud`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameOptimization {
    /// The falcond profile. Its `fg_*` fields are a copy of
    /// [`Self::frame_generation`], kept for exports.
    pub profile: GameProfile,
    /// lsfg-vk, as its own file holds it.
    pub frame_generation: FrameGeneration,
    /// `MangoHud`.
    pub mangohud: crate::mangohud::Mode,
    /// `MangoHud` as saved, to know whether it changed.
    saved_mangohud: crate::mangohud::Mode,
}

/// What a save did beyond the profile, which is saved first and whose
/// failure is the save's error.
#[derive(Debug, Default)]
pub struct SaveReport {
    /// lsfg-vk's entry could not be written.
    pub frame_generation: Option<anyhow::Error>,
    /// `MangoHud` was written (or refused, or failed); `None` when it did
    /// not change.
    pub mangohud: Option<Result<crate::mangohud::Applied>>,
    /// Gamescope in a Steam game's launch options; `None` without Gamescope.
    pub gamescope: Option<Result<crate::steam_gamescope::Applied>>,
}

impl GameOptimization {
    /// A new game's settings: the defaults every page starts from.
    #[must_use]
    pub fn new(process: &str) -> Self {
        Self::from_profile(GameProfile {
            name: process.to_owned(),
            ..GameProfile::default()
        })
    }

    /// The settings of the profile in file `stem`, each part read from its
    /// owner. A profile that does not read starts from the defaults.
    #[must_use]
    pub fn load(stem: &str) -> Self {
        let profile = crate::profiles::load(stem).unwrap_or_else(|_| GameProfile {
            name: stem.to_owned(),
            ..GameProfile::default()
        });
        let mut me = Self::from_profile(profile);
        let (multiplier, flow_scale, performance, hdr, present_mode) =
            crate::fg::read_profile_any(&me.profile.name);
        me.frame_generation = FrameGeneration {
            multiplier,
            flow_scale,
            performance,
            hdr,
            present_mode,
        };
        me.mangohud = crate::mangohud::mode_for(&me.profile.name);
        me.saved_mangohud = me.mangohud;
        me.sync_copy();
        me
    }

    fn from_profile(profile: GameProfile) -> Self {
        let mut me = Self {
            profile,
            frame_generation: FrameGeneration::default(),
            mangohud: crate::mangohud::Mode::Off,
            saved_mangohud: crate::mangohud::Mode::Off,
        };
        me.sync_copy();
        me
    }

    /// Keep the profile's copy of lsfg-vk's values in step.
    fn sync_copy(&mut self) {
        let f = self.frame_generation;
        self.profile.fg_multiplier = f.multiplier;
        self.profile.fg_flow_scale = f.flow_scale;
        self.profile.fg_perf_mode = f.performance;
        self.profile.fg_hdr = f.hdr;
        self.profile.fg_present_mode = f.present_mode;
        // Never read by anything: lsfg-vk takes the DLL from its [global].
        self.profile.fg_dll_path = None;
        // Older wizards wrote `scx_sched = ` (empty), which falcond does not
        // parse; it meant "keep the general one", which is `none`.
        for (value, default) in [
            (&mut self.profile.scx_sched, "none"),
            (&mut self.profile.vcache_mode, "none"),
            (&mut self.profile.scx_sched_props, "default"),
        ] {
            if value.trim().is_empty() {
                default.clone_into(value);
            }
        }
    }

    /// Warnings a save should mention, and errors that stop it (marked for
    /// translation).
    #[must_use]
    pub fn problems(&self) -> (Vec<&'static str>, Vec<&'static str>) {
        let errors = crate::profiles::critical_errors(&self.profile);
        let warnings = crate::profiles::validate(&self.profile)
            .into_iter()
            .filter(|w| !errors.contains(w))
            .collect();
        (errors, warnings)
    }

    /// Save every part to its owner: the profile through the privileged
    /// helper (which reloads falcond), lsfg-vk's entry, then `MangoHud` if it
    /// changed. Blocking: it may wait on a Polkit prompt.
    ///
    /// # Errors
    /// Returns an error when the profile itself is not saved; then nothing
    /// else is written.
    pub fn save(&mut self) -> Result<SaveReport> {
        self.sync_copy();
        crate::profiles::save_file(&self.profile)?;
        let mut report = SaveReport::default();
        let f = self.frame_generation;
        let global_on = crate::fg::global_state_allows_lsfg(&crate::video_config::load().frame_gen);
        let lsfg = if f.on() || crate::fg::layer_installed() {
            crate::fg::save_for_game(
                &self.profile.name,
                f.multiplier,
                f.flow_scale,
                f.performance,
                f.hdr,
                f.present_mode,
                global_on,
            )
        } else {
            Ok(())
        };
        report.frame_generation = lsfg.err();
        if crate::capabilities::which("gamescope").is_some() {
            report.gamescope = Some(crate::steam_gamescope::apply(
                &self.profile.name,
                self.steam_gamescope_segment(),
            ));
        }
        if self.mangohud != self.saved_mangohud {
            let applied = crate::mangohud::apply(&self.profile.name, self.mangohud);
            if matches!(
                applied,
                Ok(crate::mangohud::Applied::Launcher { .. }
                    | crate::mangohud::Applied::LaunchPlan
                    | crate::mangohud::Applied::SteamLaunchOptions(_))
            ) {
                self.saved_mangohud = self.mangohud;
            }
            report.mangohud = Some(applied);
        }
        Ok(report)
    }
}

impl GameOptimization {
    /// The Gamescope wrapper a Steam game's launch options get. Only what
    /// the game's own profile asks for: Always (with the sizes from Tuning
    /// unless the game has its own), or Automatic with the game's own
    /// settings when they need Gamescope. The general switch is for games
    /// BiGame-mode starts itself and does not wrap every Steam game. Where
    /// `OptiScaler` already upscales, no render size.
    fn steam_gamescope_segment(&self) -> Option<String> {
        use crate::gamescope::Mode;
        let video = crate::video_config::load();
        let mut cfg = match self.profile.gamescope_mode {
            Mode::Disabled => return None,
            Mode::Enabled => crate::launcher::LaunchPlan::merge_gamescope_config(
                &video.upscaling,
                self.profile.gamescope.as_ref(),
            ),
            Mode::Auto => self.profile.gamescope.clone()?,
        };
        if optiscaler_features(&self.profile.name).contains(&Feature::OptiScalerUpscaling) {
            cfg.render_width = 0;
            cfg.render_height = 0;
        }
        crate::steam_gamescope::segment(
            self.profile.gamescope_mode,
            &cfg,
            crate::capabilities::gamescope_cached().as_ref(),
            crate::hardware::detect_session(),
        )
    }
}

/// Whether the general frame-generation switch is on.
#[must_use]
pub fn lsfg_general_on(video: &VideoConfig) -> bool {
    video.frame_gen.enabled && video.frame_gen.backend == FrameGenBackend::LsfgVk
}

#[cfg(test)]
mod tests {
    use super::*;

    fn general(sched: &str, mode: &str, vcache: &str) -> FalcondConfig {
        FalcondConfig {
            scx_sched: sched.into(),
            scx_sched_props: mode.into(),
            vcache_mode: vcache.into(),
            ..FalcondConfig::default()
        }
    }

    fn profile(sched: &str, mode: &str, vcache: &str) -> GameProfile {
        GameProfile {
            name: "game.exe".into(),
            scx_sched: sched.into(),
            scx_sched_props: mode.into(),
            vcache_mode: vcache.into(),
            ..GameProfile::default()
        }
    }

    #[test]
    fn a_profiles_none_keeps_the_general_scheduler_as_falcond_does() {
        let g = general("lavd", "gaming", "none");
        let e = effective_scheduler(Some(&profile("none", "default", "none")), &g);
        assert_eq!((e.value.as_str(), e.mode.as_str()), ("lavd", "gaming"));
        assert_eq!(e.source, Source::General);
        // The legacy empty value the wizard used to write reads the same.
        let e = effective_scheduler(Some(&profile("", "", "none")), &g);
        assert_eq!(e.source, Source::General);
    }

    #[test]
    fn a_profile_that_names_a_scheduler_wins() {
        let g = general("lavd", "gaming", "none");
        let e = effective_scheduler(Some(&profile("bpfland", "latency", "none")), &g);
        assert_eq!((e.value.as_str(), e.source), ("bpfland", Source::Game));
        assert_eq!(e.mode, "latency");
    }

    #[test]
    fn with_nothing_named_the_kernel_decides() {
        let g = general("none", "default", "none");
        let e = effective_scheduler(Some(&profile("none", "default", "none")), &g);
        assert_eq!((e.value.as_str(), e.source), ("none", Source::System));
        assert_eq!(effective_scheduler(None, &g).source, Source::System);
        assert_eq!(effective_vcache(None, &g).source, Source::System);
    }

    #[test]
    fn vcache_follows_the_same_rule() {
        let g = general("none", "default", "cache");
        let inherit = effective_vcache(Some(&profile("none", "default", "none")), &g);
        assert_eq!(
            (inherit.value.as_str(), inherit.source),
            ("cache", Source::General)
        );
        let own = effective_vcache(Some(&profile("none", "default", "freq")), &g);
        assert_eq!((own.value.as_str(), own.source), ("freq", Source::Game));
    }

    #[test]
    fn performance_mode_off_in_general_blocks_every_profile() {
        let mut g = FalcondConfig::default();
        let p = GameProfile::default();
        assert_eq!(performance(&p, &g), Performance::On);
        g.enable_performance_mode = false;
        assert_eq!(performance(&p, &g), Performance::BlockedByGeneral);
        let off = GameProfile {
            performance_mode: false,
            ..GameProfile::default()
        };
        assert_eq!(performance(&off, &g), Performance::Off);
    }

    #[test]
    fn the_required_pairs_are_conflicts_in_both_directions() {
        use Feature as F;
        for (a, b) in [
            (F::WineFsr, F::GamescopeUpscaling),
            (F::WineFsr, F::OptiScalerUpscaling),
            (F::GamescopeUpscaling, F::OptiScalerUpscaling),
            (F::LsfgVk, F::OptiScalerFrameGen),
        ] {
            assert!(conflict(a, b).is_some(), "{a:?} + {b:?}");
            assert!(conflict(b, a).is_some(), "{b:?} + {a:?}");
            assert!(!conflict(a, b).unwrap().why.is_empty());
        }
    }

    #[test]
    fn different_jobs_are_not_conflicts() {
        use Feature as F;
        assert!(conflict(F::OptiScalerUpscaling, F::LsfgVk).is_none());
        assert!(conflict(F::WineFsr, F::LsfgVk).is_none());
        assert!(conflict(F::OptiScalerUpscaling, F::OptiScalerFrameGen).is_none());
        assert!(conflict(F::WineFsr, F::WineFsr).is_none());
    }

    #[test]
    fn an_old_file_with_both_upscalers_is_named_and_can_be_fixed() {
        let mut v = VideoConfig::default();
        v.upscaling.wine_fsr_enabled = true;
        v.upscaling.gamescope_enabled = true;
        v.upscaling.base_width = 1280;
        v.upscaling.base_height = 720;
        let found = general_conflicts(&v);
        assert_eq!(found.len(), 1);
        turn_off_general(&mut v, Feature::GamescopeUpscaling);
        assert!(general_conflicts(&v).is_empty());
        assert!(v.upscaling.gamescope_enabled, "Gamescope itself stays on");
        assert!(v.upscaling.wine_fsr_enabled);

        v.upscaling.base_width = 1280;
        v.upscaling.base_height = 720;
        turn_off_general(&mut v, Feature::WineFsr);
        assert!(general_conflicts(&v).is_empty());
        assert!(!v.upscaling.wine_fsr_enabled);
    }

    #[test]
    fn gamescope_without_a_render_size_does_not_upscale() {
        let mut u = UpscalingSettings {
            gamescope_enabled: true,
            ..UpscalingSettings::default()
        };
        assert!(!gamescope_upscales(&u));
        u.base_width = 1280;
        u.base_height = 720;
        assert!(gamescope_upscales(&u));
        u.gamescope_enabled = false;
        assert!(!gamescope_upscales(&u));
    }

    #[test]
    fn a_games_gamescope_upscales_only_with_a_render_size_and_not_never() {
        let mut p = GameProfile::default();
        assert!(!game_gamescope_upscales(&p));
        p.gamescope = Some(crate::gamescope::Config {
            render_width: 1280,
            render_height: 720,
            ..crate::gamescope::Config::default()
        });
        assert!(game_gamescope_upscales(&p));
        p.gamescope_mode = crate::gamescope::Mode::Disabled;
        assert!(!game_gamescope_upscales(&p));
    }

    #[test]
    fn choices_cover_what_falcond_accepts() {
        let ids = |c: &[Choice]| c.iter().map(|c| c.id).collect::<Vec<_>>();
        assert_eq!(
            ids(SCHEDULER_MODES),
            ["default", "gaming", "power", "latency", "server"]
        );
        assert_eq!(ids(VCACHE_MODES), ["none", "cache", "freq"]);
        assert_eq!(ids(PROFILE_SETS), ["none", "handheld", "htpc"]);
        for c in SCHEDULER_MODES
            .iter()
            .chain(VCACHE_MODES)
            .chain(PROFILE_SETS)
        {
            assert!(!c.label.is_empty() && !c.help.is_empty(), "{}", c.id);
        }
        assert_eq!(index_of(SCHEDULER_MODES, "latency"), 3);
        assert_eq!(index_of(SCHEDULER_MODES, "bogus"), 0);
        assert_eq!(find(VCACHE_MODES, "freq").map(|c| c.id), Some("freq"));
    }

    #[test]
    fn a_new_games_settings_are_the_same_whichever_page_starts_them() {
        let a = GameOptimization::new("SOTTR.exe");
        let b = GameOptimization::new("SOTTR.exe");
        assert_eq!(a, b);
        assert_eq!(a.profile.scx_sched, "none", "never the empty value");
        assert_eq!(a.profile.scx_sched_props, "default");
        assert_eq!(a.profile.fg_multiplier, 1);
        assert!(a.profile.fg_dll_path.is_none());
        assert!(!a.frame_generation.on());
    }

    #[test]
    fn the_empty_values_older_wizards_wrote_are_normalised() {
        let mut g = GameOptimization::from_profile(GameProfile {
            name: "x".into(),
            scx_sched: String::new(),
            scx_sched_props: String::new(),
            vcache_mode: " ".into(),
            ..GameProfile::default()
        });
        g.sync_copy();
        assert_eq!(g.profile.scx_sched, "none");
        assert_eq!(g.profile.scx_sched_props, "default");
        assert_eq!(g.profile.vcache_mode, "none");
    }
}
