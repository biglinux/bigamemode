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
        N_("Big Game Mode and falcond leave the driver's current preference alone."),
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

/// What `OptiScaler` does in a game Big Game Mode installed it into: the
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
/// Big Game Mode's Gamescope choice), the game's own launch settings, lsfg-vk's
/// entry and `MangoHud`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameOptimization {
    /// The falcond profile. Its `fg_*` fields are a copy of
    /// [`Self::frame_generation`], kept for exports. Its `gamescope` table is
    /// an older version's, read into [`Self::launch`] and never written.
    pub profile: GameProfile,
    /// The game's own Gamescope, Wine FSR and vkBasalt values over Tuning's,
    /// kept in the game's settings (`crate::game_settings`).
    pub launch: crate::game_launch::GameLaunch,
    /// lsfg-vk, as its own file holds it.
    pub frame_generation: FrameGeneration,
    /// `MangoHud`.
    pub mangohud: crate::mangohud::Mode,
    /// `MangoHud` as saved, to know whether it changed.
    saved_mangohud: crate::mangohud::Mode,
    /// lsfg-vk's values as read, to know whether they changed: an entry the
    /// user keeps in lsfg-vk-ui is theirs until this page changes it.
    saved_frame_generation: FrameGeneration,
}

/// What a save did beyond the profile, which is saved first and whose
/// failure is the save's error.
#[derive(Debug, Default)]
pub struct SaveReport {
    /// The game's process name, which the profile is saved under.
    pub process: String,
    /// lsfg-vk's entry could not be written.
    pub frame_generation: Option<anyhow::Error>,
    /// The game's own launch settings could not be written.
    pub launch: Option<anyhow::Error>,
    /// falcond is not installed: the performance profile was not written,
    /// since nothing would read it; every other part was.
    pub falcond_missing: bool,
    /// `MangoHud` was written (or refused, or failed); `None` when it did
    /// not change.
    pub mangohud: Option<Result<crate::mangohud::Applied>>,
    /// Gamescope and the game's own variables in a Steam game's launch
    /// options.
    pub gamescope: Option<Result<crate::steam_gamescope::Applied>>,
    /// Gamescope, Wine FSR and the game's own variables in a Heroic game's
    /// settings in Heroic.
    pub heroic: Option<Result<crate::heroic_launch::Applied>>,
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
        me.saved_frame_generation = me.frame_generation;
        me.mangohud = crate::mangohud::mode_for(&me.profile.name);
        me.saved_mangohud = me.mangohud;
        me.launch = crate::game_settings::load(&me.profile.name)
            .map(|s| s.launch)
            .unwrap_or_default();
        me.take_legacy_gamescope();
        me.sync_copy();
        me
    }

    /// A profile's `[gamescope]` table, from an older version, becomes the
    /// game's own values when it has none yet; the table itself goes, so
    /// there is one place they live.
    fn take_legacy_gamescope(&mut self) {
        if let Some(old) = self.profile.gamescope.take()
            && !self.launch.sets_gamescope()
        {
            let moved = crate::game_launch::GameLaunch::from_legacy(&old);
            self.launch = crate::game_launch::GameLaunch {
                wine_fsr: self.launch.wine_fsr,
                wine_fsr_mode: self.launch.wine_fsr_mode,
                vkbasalt: self.launch.vkbasalt,
                ..moved
            };
        }
    }

    fn from_profile(profile: GameProfile) -> Self {
        let mut me = Self {
            profile,
            launch: crate::game_launch::GameLaunch::default(),
            frame_generation: FrameGeneration::default(),
            mangohud: crate::mangohud::Mode::Off,
            saved_mangohud: crate::mangohud::Mode::Off,
            saved_frame_generation: FrameGeneration::default(),
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
    /// Without falcond the profile is not written (nothing would read it, and
    /// its directory may not exist) and the rest is saved all the same.
    ///
    /// # Errors
    /// Returns an error when the profile itself is not saved; then nothing
    /// else is written.
    pub fn save(&mut self) -> Result<SaveReport> {
        self.save_with(crate::capabilities::which("falcond").is_some())
    }

    fn save_with(&mut self, falcond: bool) -> Result<SaveReport> {
        self.sync_copy();
        if falcond {
            crate::profiles::save_file(&self.profile)?;
        }
        let mut report = SaveReport {
            process: self.profile.name.clone(),
            falcond_missing: !falcond,
            ..SaveReport::default()
        };
        // lsfg-vk's file only when this page changed its values: saving a
        // profile for its scheduler must not take over, or set aside, an
        // entry the user keeps in lsfg-vk-ui.
        let f = self.frame_generation;
        let lsfg = if self.frame_generation_changed() {
            let global_on =
                crate::fg::global_state_allows_lsfg(&crate::video_config::load().frame_gen);
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
        if lsfg.is_ok() {
            self.saved_frame_generation = f;
        }
        report.frame_generation = lsfg.err();
        report.launch = self.save_launch().err();
        report.gamescope = Some(crate::steam_gamescope::apply(
            &self.profile.name,
            self.steam_wanted(),
        ));
        report.heroic = Some(self.apply_heroic());
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

    /// Whether lsfg-vk's values differ from those read from its file.
    fn frame_generation_changed(&self) -> bool {
        self.frame_generation != self.saved_frame_generation
    }

    /// Write the game's own launch settings into its settings file, keeping
    /// everything else there.
    fn save_launch(&self) -> Result<()> {
        let mut settings = crate::game_settings::load(&self.profile.name).unwrap_or_default();
        if settings.launch == self.launch {
            return Ok(());
        }
        settings.launch = self.launch;
        crate::game_settings::save(&self.profile.name, &settings)
    }
}

/// Bring every Steam game that has a Gamescope wrapper or variables from
/// Big Game Mode, or launch settings of its own, in line with Tuning: what a
/// game leaves to Tuning (the sizes and filter of Always, Wine FSR's mode)
/// must follow a change there into its launch options too. Other games are
/// left alone. Each result says what happened for that game.
#[must_use]
pub fn refresh_steam_gamescope() -> Vec<(String, anyhow::Result<crate::steam_gamescope::Applied>)> {
    let Ok(dir) = std::fs::read_dir(crate::game_settings::dir()) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut names: Vec<String> = dir
        .flatten()
        .filter_map(|e| {
            e.file_name()
                .to_str()
                .and_then(|n| n.strip_suffix(".toml"))
                .map(str::to_owned)
        })
        .collect();
    names.sort();
    // Read once: a game with launch settings of its own and nothing written
    // yet is looked at only if Steam starts it.
    let mut steam_games: Option<std::collections::HashSet<String>> = None;
    for name in names {
        let Ok(settings) = crate::game_settings::load(&name) else {
            continue;
        };
        let written = settings.steam_gamescope.is_some()
            || settings.steam_env.is_some()
            || settings.steam_wine_fsr_off;
        if !written {
            if settings.launch.is_empty() {
                continue;
            }
            let steam = steam_games.get_or_insert_with(|| {
                crate::games::detect_all()
                    .into_iter()
                    .filter(|g| g.source == crate::games::Source::Steam)
                    .map(|g| g.profile_key().to_owned())
                    .collect()
            });
            if !steam.contains(&name) {
                continue;
            }
        }
        let game = GameOptimization::load(&name);
        let wanted = game.steam_wanted();
        out.push((name.clone(), crate::steam_gamescope::apply(&name, wanted)));
    }
    out
}

/// What the Steam game whose process is `process` gets in its launch
/// options from Big Game Mode, as saved now; `wine_fsr_off` in place of the
/// saved choice to switch Wine FSR off for it, when given.
#[must_use]
pub fn steam_wanted(process: &str, wine_fsr_off: Option<bool>) -> crate::steam_gamescope::Wanted {
    let game = GameOptimization::load(process);
    let off = wine_fsr_off
        .unwrap_or_else(|| crate::game_settings::load(process).is_ok_and(|s| s.steam_wine_fsr_off));
    game.steam_wanted_with(off)
}

impl GameOptimization {
    /// What a Steam game's launch options get from Big Game Mode: the
    /// Gamescope wrapper and the variables for its own launch settings.
    fn steam_wanted(&self) -> crate::steam_gamescope::Wanted {
        let off =
            crate::game_settings::load(&self.profile.name).is_ok_and(|s| s.steam_wine_fsr_off);
        self.steam_wanted_with(off)
    }

    /// [`Self::steam_wanted`], with Wine FSR switched off for the game
    /// (`wine_fsr_off`) or not.
    fn steam_wanted_with(&self, wine_fsr_off: bool) -> crate::steam_gamescope::Wanted {
        let video = crate::video_config::load();
        let optiscaler =
            optiscaler_features(&self.profile.name).contains(&Feature::OptiScalerUpscaling);
        let gamescope = self.steam_gamescope_segment(&video, optiscaler);
        let upscales = !optiscaler
            && gamescope.is_some()
            && self
                .launch
                .upscales(&video.upscaling, self.profile.gamescope_mode, false);
        let env = self
            .launch
            .steam_env(&video.upscaling, upscales, optiscaler || wine_fsr_off)
            .join(" ");
        crate::steam_gamescope::Wanted {
            gamescope,
            env: (!env.is_empty()).then_some(env),
            wine_fsr_off,
            drop_their_wine_fsr_on: false,
        }
    }

    /// The Gamescope wrapper a Steam game's launch options get. Only what
    /// the game's own profile asks for: Always (with Tuning's settings for
    /// whatever the game leaves to them), or Automatic when the game's own
    /// values need Gamescope. The general switch is for games Big Game Mode
    /// starts itself and does not wrap every Steam game. Where `OptiScaler`
    /// already upscales, no render size. The Flatpak Steam runs Flathub's
    /// Gamescope, whose options are not the system's.
    fn steam_gamescope_segment(&self, video: &VideoConfig, optiscaler: bool) -> Option<String> {
        use crate::gamescope::Mode;
        let mode = self.profile.gamescope_mode;
        if mode == Mode::Disabled || (mode == Mode::Auto && !self.launch.sets_gamescope()) {
            return None;
        }
        let mut cfg = self
            .launch
            .config(&video.upscaling, mode)
            .with_screen_output(crate::screen::primary_size());
        if optiscaler {
            cfg.render_width = 0;
            cfg.render_height = 0;
        }
        let caps = if crate::steam::only_flatpak(&crate::steam::users(&crate::paths::home_dir())) {
            Some(crate::capabilities::GamescopeCaps::default())
        } else {
            crate::capabilities::gamescope_cached()
        };
        let segment = crate::steam_gamescope::segment(
            mode,
            &cfg,
            caps.as_ref(),
            crate::hardware::detect_session(),
        )?;
        Some(if self.vkbasalt_on(video) {
            crate::steam_gamescope::keeping_vkbasalt_in_the_game(&segment)
        } else {
            segment
        })
    }

    /// Whether vkBasalt is on for this game where its launcher starts it:
    /// its own choice, or the session's (a Turbo preset's over Tuning's).
    fn vkbasalt_on(&self, video: &VideoConfig) -> bool {
        crate::capabilities::vkbasalt_installed()
            && self.launch.vkbasalt.unwrap_or_else(|| {
                crate::turbo_preset::active_levers()
                    .vkbasalt
                    .unwrap_or(video.upscaling.vkbasalt_enabled)
            })
    }
}

// ── Heroic ──────────────────────────────────────────────────────────────────

/// Proton's FSR 4 upgrade, for both flavours a Heroic game may run: GE-Proton
/// reads the first (and then fetches AMD's provider itself), Valve's Proton
/// the second (`crate::graphics::fsr4_upgrade`).
const HEROIC_FSR4: [(&str, &str); 2] = [("PROTON_FSR4_UPGRADE", "1"), ("FSR4_UPGRADE", "1")];

impl GameOptimization {
    /// Write this game's own launch settings into its settings in Heroic
    /// (`crate::heroic_launch`), taking out what Big Game Mode wrote there
    /// before. A game Heroic does not start is left alone.
    ///
    /// # Errors
    /// Returns an error when a settings file cannot be read, is not JSON,
    /// or cannot be written.
    pub fn apply_heroic(&self) -> Result<crate::heroic_launch::Applied> {
        let video = crate::video_config::load();
        let optiscaler =
            optiscaler_features(&self.profile.name).contains(&Feature::OptiScalerUpscaling);
        let fsr4 =
            crate::game_settings::load(&self.profile.name).is_ok_and(|s| s.heroic_fsr4_upgrade);
        crate::heroic_launch::apply(&self.profile.name, |target| {
            self.heroic_wanted(&video, optiscaler, fsr4, target)
        })
    }

    /// What a Heroic game's settings get from Big Game Mode. Gamescope only
    /// as a Steam game gets it — Always, or Automatic with values of the
    /// game's own — and only where that Heroic can run it: its Flatpak
    /// needs Flathub's Gamescope extension, a native one `gamescope`.
    fn heroic_wanted(
        &self,
        video: &VideoConfig,
        optiscaler: bool,
        fsr4: bool,
        target: &crate::heroic_launch::Target,
    ) -> crate::heroic_launch::Wanted {
        use crate::gamescope::Mode;
        let mode = self.profile.gamescope_mode;
        let caps = if target.flatpak() {
            crate::heroic_launch::flatpak_gamescope_missing()
                .is_none()
                .then(crate::capabilities::GamescopeCaps::default)
        } else {
            crate::capabilities::gamescope_cached()
        };
        let asks = mode == Mode::Enabled || (mode == Mode::Auto && self.launch.sets_gamescope());
        let gamescope = asks
            .then(|| {
                let mut cfg = self.launch.config(&video.upscaling, mode);
                if optiscaler {
                    cfg.render_width = 0;
                    cfg.render_height = 0;
                }
                cfg
            })
            .filter(|cfg| {
                crate::gamescope::decide(
                    mode,
                    cfg,
                    caps.as_ref(),
                    crate::hardware::detect_session(),
                )
                .use_gamescope
            })
            // After the decision: the screen's size is what Gamescope shows,
            // not a reason to wrap the game.
            .map(|cfg| cfg.with_screen_output(crate::screen::primary_size()));
        let upscales = !optiscaler
            && gamescope.is_some()
            && self.launch.upscales(&video.upscaling, mode, false);
        // Heroic's Flatpak does not see the user's configuration folder.
        let vkbasalt_config = (!target.flatpak())
            .then_some(video.upscaling.vkbasalt_config_path.as_deref())
            .flatten();
        let (wine_fsr, mut env) =
            self.launch
                .heroic_env(&video.upscaling, upscales, optiscaler, vkbasalt_config);
        if fsr4 {
            env.extend(HEROIC_FSR4.map(|(k, v)| (k.to_owned(), v.to_owned())));
        }
        // MangoHud as Heroic has it: its own switch for Forced, the layer's
        // variable for On.
        if self.mangohud == crate::mangohud::Mode::On {
            env.push(("MANGOHUD".to_owned(), "1".to_owned()));
        }
        let mut wanted = crate::heroic_launch::Wanted {
            gamescope: gamescope
                .as_ref()
                .map(crate::heroic_launch::Gamescope::from_config),
            wine_fsr,
            wine_fsr_off_where_on: upscales || optiscaler,
            env,
            show_mangohud: self.mangohud == crate::mangohud::Mode::Forced,
            wrapper: None,
        };
        if wanted.gamescope.is_some() && self.vkbasalt_on(video) {
            wanted.keep_vkbasalt_in_the_game();
        }
        wanted
    }
}

/// Write the game whose process is `process` into its settings in Heroic,
/// from what is saved for it now.
///
/// # Errors
/// As [`GameOptimization::apply_heroic`].
pub fn apply_heroic(process: &str) -> Result<crate::heroic_launch::Applied> {
    GameOptimization::load(process).apply_heroic()
}

/// Proton's FSR 4 upgrade (AI Graphics' Native action) on or off for the
/// Heroic game whose process is `process`, written into its settings in
/// Heroic. The choice is kept only when it was written.
///
/// # Errors
/// Returns an error when the game's settings or Heroic's cannot be read
/// or written.
pub fn set_heroic_fsr4_upgrade(process: &str, on: bool) -> Result<crate::heroic_launch::Applied> {
    let mut settings = crate::game_settings::load(process)?;
    let before = settings.heroic_fsr4_upgrade;
    settings.heroic_fsr4_upgrade = on;
    crate::game_settings::save(process, &settings)?;
    let applied = apply_heroic(process);
    if !matches!(
        applied,
        Ok(crate::heroic_launch::Applied::Written | crate::heroic_launch::Applied::Unchanged)
    ) {
        // Not in effect: the saved choice goes back to what it was. The
        // record `apply_heroic` may have saved is reloaded, not overwritten.
        let mut settings = crate::game_settings::load(process)?;
        settings.heroic_fsr4_upgrade = before;
        crate::game_settings::save(process, &settings)?;
    }
    applied
}

/// Bring every Heroic game that has settings from Big Game Mode, or launch
/// settings of its own, in line with Tuning, as
/// [`refresh_steam_gamescope`] does for Steam games. Each result says what
/// happened for that game.
#[must_use]
pub fn refresh_heroic() -> Vec<(String, anyhow::Result<crate::heroic_launch::Applied>)> {
    let Ok(dir) = std::fs::read_dir(crate::game_settings::dir()) else {
        return Vec::new();
    };
    let mut names: Vec<String> = dir
        .flatten()
        .filter_map(|e| {
            e.file_name()
                .to_str()
                .and_then(|n| n.strip_suffix(".toml"))
                .map(str::to_owned)
        })
        .collect();
    names.sort();
    let mut heroic_games: Option<std::collections::HashSet<String>> = None;
    let mut out = Vec::new();
    for name in names {
        let Ok(settings) = crate::game_settings::load(&name) else {
            continue;
        };
        if settings.heroic.is_empty() {
            if settings.launch.is_empty() && !settings.heroic_fsr4_upgrade {
                continue;
            }
            let heroic = heroic_games.get_or_insert_with(|| {
                crate::games::detect_all()
                    .into_iter()
                    .filter(|g| {
                        matches!(g.launcher, Some(crate::games::LauncherRef::Heroic { .. }))
                    })
                    .map(|g| g.profile_key().to_owned())
                    .collect()
            });
            if !heroic.contains(&name) {
                continue;
            }
        }
        out.push((name.clone(), apply_heroic(&name)));
    }
    out
}

/// Whether the general frame-generation switch is on.
#[must_use]
pub fn lsfg_general_on(video: &VideoConfig) -> bool {
    video.frame_gen.enabled && video.frame_gen.backend == FrameGenBackend::LsfgVk
}

/// Which frames a Turbo preset's frame cap holds in a game that generates
/// frames: that depends on the frame generator, not on the cap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CappedFrames {
    /// The frames the game renders. lsfg-vk is a Vulkan layer under DXVK,
    /// VKD3D-Proton and `MangoHud`'s limiter, so it generates after the cap:
    /// the game shows `multiplier` times the cap.
    Rendered {
        /// lsfg-vk's multiplier for the game.
        multiplier: u32,
    },
    /// The frames shown, generated ones included. `OptiScaler`'s generation
    /// runs inside the game, above the cap: the game renders about half of
    /// it (Shadow of the Tomb Raider under 60: 30.0 rendered).
    Shown,
}

/// [`CappedFrames`] for the game whose process is `process`; `None` for a
/// game that generates no frames. `OptiScaler`'s generation switches
/// lsfg-vk off for the game, so it is the one that counts.
#[must_use]
pub fn capped_frames(process: &str) -> Option<CappedFrames> {
    if optiscaler_features(process).contains(&Feature::OptiScalerFrameGen) {
        return Some(CappedFrames::Shown);
    }
    let multiplier = crate::fg::read_profile_any(process).0;
    (multiplier > 1 && lsfg_general_on(&crate::video_config::load()))
        .then_some(CappedFrames::Rendered { multiplier })
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
    fn an_older_profiles_gamescope_table_becomes_the_games_own_values() {
        let mut g = GameOptimization::from_profile(GameProfile {
            name: "x".into(),
            gamescope: Some(crate::gamescope::Config {
                render_width: 1280,
                render_height: 720,
                ..crate::gamescope::Config::default()
            }),
            ..GameProfile::default()
        });
        g.launch.vkbasalt = Some(true);
        g.take_legacy_gamescope();
        assert_eq!(g.launch.render, Some((1280, 720)));
        assert_eq!(g.launch.vkbasalt, Some(true), "its other values stay");
        assert!(g.profile.gamescope.is_none(), "one place for them");
        // A game that already has its own values keeps them.
        let mut g = GameOptimization::from_profile(GameProfile {
            name: "x".into(),
            gamescope: Some(crate::gamescope::Config::default()),
            ..GameProfile::default()
        });
        g.launch.render = Some((1920, 1080));
        g.take_legacy_gamescope();
        assert_eq!(g.launch.render, Some((1920, 1080)));
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
    fn saving_a_profile_leaves_lsfg_vks_entry_alone_unless_it_changed() {
        // As loaded with a user's entry at x2: changing the scheduler alone
        // must not write lsfg-vk's file (with the general switch off that
        // would set the entry aside).
        let mut g = GameOptimization::new("Game.exe");
        g.frame_generation.multiplier = 2;
        g.saved_frame_generation = g.frame_generation;
        g.profile.scx_sched = "scx_lavd".into();
        assert!(!g.frame_generation_changed());
        g.frame_generation.flow_scale = 50;
        assert!(g.frame_generation_changed());
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
