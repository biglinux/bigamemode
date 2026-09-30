//! One reading of what Big Game Mode is doing for the machine and the running
//! game — the Details page's data, collected in one place.
//!
//! Every value here is read from the system that holds it (`turbo`,
//! `status`, `/proc`, sysfs, power-profiles-daemon, the launch settings),
//! never from what Big Game Mode last did. The page renders a [`Snapshot`];
//! the judgements it draws from one — *configured but not detected is a
//! problem*, *hardware that is not there is not an error* — are pure
//! functions with tests, so they cannot drift from the collection.
//!
//! Collecting reads `/proc`, sysfs, one D-Bus property and a few files: a few
//! milliseconds, off the main thread.

use std::path::PathBuf;

use crate::capabilities::{self, SchedExtCaps, Support};
use crate::running::{GameIdentity, InGame};
use crate::status::FalcondStatus;

/// The one state vocabulary for everything the pages show. Each state has a
/// meaning of its own; two states are never the same word for different
/// facts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// Verified in the running system: what was asked for is happening.
    Active,
    /// Asked for, and it will apply when the next game starts; nothing runs
    /// that could confirm it yet.
    Waiting,
    /// Asked for, but the running game shows no sign of it: attention.
    NotDetected,
    /// Asked for, and whether it took cannot be read from outside: neither
    /// confirmed nor denied.
    Configured,
    /// Present and not asked for: a choice, not a problem.
    Off,
    /// Asked for, and the software it needs is not installed.
    Missing,
    /// The hardware or kernel cannot do it; no action makes sense.
    Unsupported,
    /// Something failed.
    Error,
}

impl State {
    /// A problem worth attention, as opposed to a state.
    #[must_use]
    pub fn needs_attention(self) -> bool {
        matches!(self, Self::NotDetected | Self::Missing | Self::Error)
    }
}

/// The state of one presentation feature (Gamescope, Wine FSR, vkBasalt,
/// frame generation, `MangoHud`), from four facts:
///
/// - `configured`: the user asked for it (settings or the game's profile);
/// - `installed`: the software is there;
/// - `game`: a game is running;
/// - `detected`: what the running game shows — `Some(true)` seen in the
///   game, `Some(false)` looked for and not found, `None` not readable.
///
/// A feature seen in the game counts as active even when Big Game Mode did
/// not ask for it (Steam's launch options can add `MangoHud`): the page
/// reports what is, not what it did.
#[must_use]
pub fn feature_state(
    configured: bool,
    installed: bool,
    game: bool,
    detected: Option<bool>,
) -> State {
    match (configured, installed, game, detected) {
        (_, _, true, Some(true)) => State::Active,
        (true, false, _, _) => State::Missing,
        (false, _, _, _) => State::Off,
        (true, true, false, _) => State::Waiting,
        (true, true, true, Some(false)) => State::NotDetected,
        (true, true, true, None) => State::Configured,
    }
}

/// How the machine stands, in one line at the top of the page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Headline {
    /// Turbo is off: nothing is optimized.
    TurboOff,
    /// Turbo is on and no game runs.
    ReadyWaiting,
    /// Turbo is on and a game runs under falcond's profile.
    Optimizing,
    /// Turbo is on, a game runs, and falcond reports no profile for it.
    GameWithoutProfile,
    /// falcond's service is on but its status cannot be read.
    FalcondSilent,
    /// falcond's service failed.
    FalcondFailed,
    /// Turbo's state could not be read (systemd did not answer).
    TurboUnreadable,
    /// A game runs whose own profile falcond has, and falcond is not
    /// applying it ([`ProfileNotApplied`]).
    ProfileNotApplied,
}

/// The headline from Turbo, falcond and the game.
#[must_use]
pub fn headline(
    turbo_on: bool,
    unit_failed: bool,
    falcond: Option<&FalcondStatus>,
    game: bool,
) -> Headline {
    if unit_failed {
        return Headline::FalcondFailed;
    }
    if !turbo_on {
        return Headline::TurboOff;
    }
    let Some(st) = falcond else {
        return Headline::FalcondSilent;
    };
    if !game {
        return Headline::ReadyWaiting;
    }
    if st
        .active_profile
        .as_deref()
        .is_some_and(|p| !p.is_empty() && p != "None")
    {
        Headline::Optimizing
    } else {
        Headline::GameWithoutProfile
    }
}

/// falcond's active profile, explained: falcond's generic `Proton` profile
/// is what a Proton game without a profile of its own gets, and the name
/// alone reads as if it were the game's.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum AppliedProfile {
    /// No profile is active.
    #[default]
    None,
    /// A profile written for this process.
    Own {
        /// The profile's name (the process name).
        name: String,
        /// Where the file is, when it was found.
        path: Option<PathBuf>,
        /// The user's own, as opposed to one falcond ships.
        user: bool,
    },
    /// falcond's generic profile for Proton games.
    GenericProton,
    /// A profile falcond reports under a name no file explains.
    Other(String),
}

/// Classify falcond's `ACTIVE_PROFILE`.
#[must_use]
pub fn applied_profile(
    active: Option<&str>,
    matched: Option<&crate::running::ProfileMatch>,
) -> AppliedProfile {
    match active {
        None | Some("" | "None") => AppliedProfile::None,
        Some("Proton") => AppliedProfile::GenericProton,
        Some(name) => match matched {
            Some(m) if m.name == name => AppliedProfile::Own {
                name: name.to_owned(),
                path: Some(m.path.clone()),
                user: m.user,
            },
            _ => AppliedProfile::Other(name.to_owned()),
        },
    }
}

/// The running game's own falcond profile, which falcond is not applying.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileNotApplied {
    /// The profile's name (the game's process name).
    pub name: String,
    /// What the game's main thread renamed itself to (`GameThread`), when
    /// the name no longer looks like the executable's.
    pub renamed_to: Option<String>,
    /// Whether the installed falcond is one that never looks at such a
    /// process: before 2.0.3 it inspects a process only when its thread
    /// name starts with `wine`, holds `.exe`, is cut at 15 characters or is
    /// a profile's name (`couldMatch`, fixed upstream in 2d9b455).
    pub falcond_misses_renamed: bool,
}

/// What a game's main thread is called, when that is no longer its
/// executable's name: the kernel keeps 15 characters of it
/// (`Cyberpunk2077.e`), and Wine names it after the program or the
/// preloader; a game's engine can rename it (`REDengine`: `GameThread`).
#[must_use]
pub fn renamed_main_thread(comm: &str, process_name: &str) -> Option<String> {
    let comm = comm.trim();
    let lower = comm.to_ascii_lowercase();
    let looks_like_exe = comm.is_empty()
        || process_name.to_ascii_lowercase().starts_with(&lower)
        || lower.contains(".exe")
        || lower.starts_with("wine");
    (!looks_like_exe).then(|| comm.to_owned())
}

/// Whether falcond `version` (`2.0.2-2`) predates the 2.0.3 fix that makes
/// it look at processes whose main thread renamed itself.
#[must_use]
pub fn falcond_misses_renamed(version: &str) -> bool {
    let numbers: Vec<u32> = version
        .split(['-', '+'])
        .next()
        .unwrap_or_default()
        .split('.')
        .map_while(|n| n.parse().ok())
        .collect();
    matches!(numbers.as_slice(), [2, 0, patch, ..] if *patch < 3)
        || numbers.first().is_some_and(|major| *major < 2)
}

/// The sched-ext scheduler: what can be switched, what was asked, what runs.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Scheduler {
    /// What the kernel and the installed tools allow.
    pub caps: SchedExtCaps,
    /// The scheduler the active profile asks for (`lavd`), or the global
    /// one; `none` or empty when nothing is asked.
    pub requested: String,
    /// Who asked: the game's profile, or falcond's global configuration.
    pub requested_by: RequestedBy,
    /// The scheduler the kernel reports loaded now (`scx_lavd` → `lavd`).
    pub loaded: Option<String>,
    /// What falcond reports as current, when it does.
    pub falcond_current: Option<String>,
}

/// Where a request came from.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum RequestedBy {
    /// Nothing asked for one.
    #[default]
    Nobody,
    /// The running game's profile.
    GameProfile,
    /// falcond's global configuration.
    GlobalConfig,
}

impl Scheduler {
    /// The scheduler's state.
    #[must_use]
    pub fn state(&self, game: bool) -> State {
        let asked = !self.requested.is_empty() && self.requested != "none";
        match self.caps.switchable() {
            Support::Unsupported(_) => return State::Unsupported,
            Support::NotInstalled(_) | Support::ServiceDown(_) if asked => return State::Missing,
            Support::NotInstalled(_) | Support::ServiceDown(_) => return State::Off,
            Support::Available => {}
        }
        let loaded = self
            .loaded
            .as_deref()
            .map(|s| s.strip_prefix("scx_").unwrap_or(s));
        match (asked, game, loaded) {
            // Loaded, and either what was asked or nobody asked: it runs.
            (_, _, Some(l)) if !asked || l == self.requested => State::Active,
            // Another scheduler than the one asked for, or none during the
            // game: the request did not take.
            (true, true, _) => State::NotDetected,
            // Asked, no game: it applies when one starts (a different one
            // loaded meanwhile is whatever else asked for it).
            (true, false, _) => State::Waiting,
            (false, _, _) => State::Off,
        }
    }
}

/// 3D V-Cache: the hardware, the request, and what is set now.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VCache {
    /// The CPU exposes the mode control.
    pub available: bool,
    /// What the profile or configuration asks for (`cache`, `freq`, `none`).
    pub requested: String,
    /// What the driver reports set now, when falcond reports it.
    pub current: Option<String>,
}

impl VCache {
    /// The V-Cache state: a CPU without one is *unsupported*, never a
    /// problem.
    #[must_use]
    pub fn state(&self, game: bool) -> State {
        if !self.available {
            return State::Unsupported;
        }
        let asked = !self.requested.is_empty() && self.requested != "none";
        match (asked, game, self.current.as_deref()) {
            (true, true, Some(c)) if c == self.requested => State::Active,
            (true, true, Some(_)) => State::NotDetected,
            (true, true, None) => State::Configured,
            (true, false, _) => State::Waiting,
            (false, _, _) => State::Off,
        }
    }
}

/// lsfg-vk: installed, ready, on globally, and for the running game.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Lsfg {
    /// The Vulkan layer is installed.
    pub installed: bool,
    /// A `Lossless.dll` is configured and exists.
    pub dll_ready: bool,
    /// The global switch (Tuning) is on.
    pub global_on: bool,
    /// The running game has an entry with a multiplier above 1.
    pub game_multiplier: Option<u32>,
}

/// What Details shows, read once.
// Independent facts about the machine, each read from its own source; a
// state machine would hide that they can disagree, which is the point.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    /// Turbo, as falcond's unit state says.
    pub turbo_on: bool,
    /// systemd could not be asked (no system bus): `turbo_on` is a guess and
    /// nothing may be said from it.
    pub turbo_unreadable: bool,
    /// falcond's unit failed (systemd `failed`).
    pub unit_failed: bool,
    /// falcond is installed.
    pub falcond_installed: bool,
    /// falcond's status, when its file is there and trusted.
    pub falcond: Option<FalcondStatus>,
    /// The profile falcond applied, explained.
    pub profile: AppliedProfile,
    /// The game's own profile, when falcond has one and is not applying it.
    pub profile_not_applied: Option<ProfileNotApplied>,
    /// power-profiles-daemon's active profile.
    pub power_profile: Option<String>,
    /// The scheduler.
    pub scheduler: Scheduler,
    /// 3D V-Cache.
    pub vcache: VCache,
    /// The running game.
    pub game: Option<GameIdentity>,
    /// What the running game really has.
    pub in_game: Option<InGame>,
    /// `WINE_FULLSCREEN_FSR` is in the running game's environment.
    pub wine_fsr_in_game: Option<bool>,
    /// The launch settings.
    pub video: crate::video_config::VideoConfig,
    /// Gamescope is installed.
    pub gamescope_installed: bool,
    /// vkBasalt's layer is installed.
    pub vkbasalt_installed: bool,
    /// `MangoHud` is installed.
    pub mangohud_installed: bool,
    /// `MangoHud` chosen for the running game.
    pub mangohud_for_game: crate::mangohud::Mode,
    /// lsfg-vk.
    pub lsfg: Lsfg,
    /// AI Graphics in the running game, when Big Game Mode installed it.
    pub ai_graphics: Option<crate::graphics::runtime::Status>,
    /// `OptiScaler`'s frame generation is on for the running game (chosen on
    /// its page, or switched on from `OptiScaler`'s overlay).
    pub ai_frame_generation: bool,
    /// Whether the running game's profile asks Gamescope never / always.
    pub gamescope_mode: crate::gamescope::Mode,
    /// The running game is started by the Steam client, and Big Game Mode's
    /// Gamescope reaches it only through its launch options: whether its
    /// profile put a wrapper there (`crate::steam_gamescope`).
    pub steam_launch: Option<bool>,
    /// Big Game Mode switched Wine FSR off for the running game in its Steam
    /// launch options.
    pub wine_fsr_off_for_game: bool,
}

/// For a running Steam game, whether its profile put a Gamescope wrapper in
/// its Steam launch options (`None` for a game Steam does not start), and
/// whether Big Game Mode switched Wine FSR off for it there.
/// The game's own profile, when falcond has one and, the game having
/// settled, still applies another or none.
fn profile_not_applied(
    game: Option<&GameIdentity>,
    matched: Option<&crate::running::ProfileMatch>,
    applied: &AppliedProfile,
) -> Option<ProfileNotApplied> {
    let (game, matched) = (game?, matched?);
    if matches!(applied, AppliedProfile::Own { name, .. } if *name == matched.name) {
        return None;
    }
    // falcond takes a few seconds to see a new game.
    if crate::running::running_for(game.pid).is_none_or(|secs| secs < 20) {
        return None;
    }
    let comm = std::fs::read_to_string(format!("/proc/{}/comm", game.pid)).unwrap_or_default();
    let renamed_to = renamed_main_thread(&comm, &game.process_name);
    let falcond_misses_renamed = renamed_to.is_some()
        && crate::health::package_version(
            std::path::Path::new(crate::health::PACMAN_DB),
            "falcond",
        )
        .is_some_and(|v| falcond_misses_renamed(&v));
    Some(ProfileNotApplied {
        name: matched.name.clone(),
        renamed_to,
        falcond_misses_renamed,
    })
}

/// The falcond profile for a running game: under its own process name, or,
/// for a game started by a launcher stub (Unreal's bootstrap) that runs as
/// another executable, under the name the library gives the game — the
/// stub's, which falcond sees running for as long as the game does.
#[must_use]
pub fn running_profile(game: &GameIdentity, mode: &str) -> Option<crate::running::ProfileMatch> {
    crate::running::matching_profile(&game.process_name, mode).or_else(|| {
        library_key(game)
            .filter(|key| !key.eq_ignore_ascii_case(&game.process_name))
            .and_then(|key| crate::running::matching_profile(&key, mode))
    })
}

/// The profile key the library gives the running game, read once per game
/// process: finding it walks every launcher's records, and a snapshot is
/// taken every few seconds.
fn library_key(game: &GameIdentity) -> Option<String> {
    static CACHE: std::sync::Mutex<Option<(u32, Option<String>)>> = std::sync::Mutex::new(None);
    let mut cache = CACHE.lock().ok()?;
    if let Some((pid, key)) = cache.as_ref() {
        if *pid == game.pid {
            return key.clone();
        }
    }
    let key =
        crate::games::installed_game_for_process(&game.process_name, game.install_path.as_deref())
            .map(|g| g.profile_key().to_owned());
    *cache = Some((game.pid, key.clone()));
    key
}

fn steam_launch_facts(game: Option<&GameIdentity>) -> (Option<bool>, bool) {
    let Some(g) = game else {
        return (None, false);
    };
    let settings = crate::game_settings::load(&g.process_name).unwrap_or_default();
    (
        g.steam_app_id
            .is_some()
            .then_some(settings.steam_gamescope.is_some()),
        settings.steam_wine_fsr_off,
    )
}

/// Turbo from falcond's unit: whether it is on, whether the unit failed,
/// and whether systemd could not be asked at all.
fn read_turbo() -> (bool, bool, bool) {
    let unit =
        crate::systemd::Reader::shared().and_then(|r| r.unit_state(crate::turbo::BACKEND_UNIT));
    if let Some(unit) = unit {
        return (unit.is_active(), unit.active_state == "failed", false);
    }
    match crate::turbo::state_blocking() {
        Ok(state) => (state == crate::turbo::State::On, false, false),
        Err(_) => (false, false, true),
    }
}

impl Snapshot {
    /// Read everything. Never fails: what cannot be read is `None`.
    #[must_use]
    pub fn collect(game: Option<GameIdentity>) -> Self {
        let (turbo_on, unit_failed, turbo_unreadable) = read_turbo();
        let falcond = crate::status::read();
        // No `Capabilities::detect` here: it runs `gamescope --help` and
        // `systemctl`, too much for a reading taken every few seconds.
        let sched_caps = SchedExtCaps::detect();
        let config = crate::config::read().ok();
        let video = crate::video_config::load();

        let in_game = game.as_ref().map(crate::running::in_game);
        let mode = falcond.as_ref().map_or("", |s| s.profile_mode.as_str());
        let matched = game.as_ref().and_then(|g| running_profile(g, mode));
        let active = falcond.as_ref().and_then(|s| s.active_profile.as_deref());
        let profile = applied_profile(active, matched.as_ref());
        let profile_not_applied = profile_not_applied(game.as_ref(), matched.as_ref(), &profile);

        // What the active profile asks for; the global configuration
        // otherwise.
        let profile_file = match &profile {
            AppliedProfile::Own { name, .. } | AppliedProfile::Other(name) => {
                crate::profiles::load(name).ok()
            }
            AppliedProfile::GenericProton => crate::profiles::load("Proton").ok(),
            AppliedProfile::None => None,
        };
        // falcond's rule: a profile's `none` keeps the general value it
        // loaded at start-up (`optimization::effective_scheduler`).
        let general = config.clone().unwrap_or_default();
        let scx = crate::optimization::effective_scheduler(profile_file.as_ref(), &general);
        let (requested_scx, requested_by) = match scx.source {
            crate::optimization::Source::Game => (scx.value, RequestedBy::GameProfile),
            crate::optimization::Source::General => (scx.value, RequestedBy::GlobalConfig),
            crate::optimization::Source::System => (String::new(), RequestedBy::Nobody),
        };
        let requested_vcache = if profile_file.is_none() && config.is_none() {
            String::new()
        } else {
            crate::optimization::effective_vcache(profile_file.as_ref(), &general).value
        };
        let non_empty = |s: &str| (!s.is_empty()).then(|| s.to_owned());

        let scheduler = Scheduler {
            loaded: crate::running::loaded_scheduler(),
            falcond_current: falcond.as_ref().and_then(|s| non_empty(&s.current_scx)),
            requested: requested_scx,
            requested_by,
            caps: sched_caps,
        };
        let vcache = VCache {
            available: crate::vcache::is_available(),
            requested: requested_vcache,
            current: falcond.as_ref().and_then(|s| non_empty(&s.current_vcache)),
        };
        let lsfg = Lsfg {
            installed: crate::fg::layer_installed(),
            dll_ready: crate::fg::is_lossless_dll_ready(),
            global_on: crate::fg::global_state_allows_lsfg(&video.frame_gen),
            game_multiplier: game
                .as_ref()
                .map(|g| crate::fg::read_profile(&g.process_name).0)
                .filter(|m| *m > 1),
        };
        let wine_fsr_in_game = game
            .as_ref()
            .map(|g| crate::processes::env_switch_on(g.pid, "WINE_FULLSCREEN_FSR"));
        let gamescope_mode = profile_file
            .as_ref()
            .map_or(crate::gamescope::Mode::Auto, |p| p.gamescope_mode);
        let (steam_launch, wine_fsr_off_for_game) = steam_launch_facts(game.as_ref());

        Self {
            turbo_on,
            turbo_unreadable,
            unit_failed,
            falcond_installed: capabilities::which("falcond").is_some(),
            profile,
            profile_not_applied,
            power_profile: crate::dbus::power_profile_get(),
            scheduler,
            vcache,
            wine_fsr_in_game,
            gamescope_installed: capabilities::which("gamescope").is_some(),
            vkbasalt_installed: capabilities::vkbasalt_installed(),
            mangohud_installed: capabilities::which("mangohud").is_some(),
            mangohud_for_game: game.as_ref().map_or(crate::mangohud::Mode::Off, |g| {
                crate::mangohud::mode_for(&g.process_name)
            }),
            lsfg,
            ai_graphics: game.as_ref().and_then(crate::graphics::status_running),
            ai_frame_generation: game.as_ref().is_some_and(|g| {
                crate::graphics::launch_disables(
                    &crate::graphics::state_dir(),
                    &crate::game_settings::dir(),
                    &g.process_name,
                )
                .contains(&crate::graphics::rules::Tech::LsfgVk)
            }),
            gamescope_mode,
            steam_launch,
            wine_fsr_off_for_game,
            video,
            falcond,
            in_game,
            game,
        }
    }

    /// The headline.
    #[must_use]
    pub fn headline(&self) -> Headline {
        if self.turbo_unreadable {
            return Headline::TurboUnreadable;
        }
        match headline(
            self.turbo_on,
            self.unit_failed,
            self.falcond.as_ref(),
            self.game.is_some(),
        ) {
            Headline::Optimizing | Headline::GameWithoutProfile
                if self.profile_not_applied.is_some() =>
            {
                Headline::ProfileNotApplied
            }
            other => other,
        }
    }

    /// Gamescope: configured globally or by the game's profile, seen in the
    /// game's process tree.
    #[must_use]
    pub fn gamescope_state(&self) -> State {
        // A Steam game gets Gamescope only from its launch options: the
        // general switch is for Big Game Mode's own launches.
        let configured = self.steam_launch.unwrap_or(match self.gamescope_mode {
            crate::gamescope::Mode::Enabled => true,
            crate::gamescope::Mode::Disabled => false,
            crate::gamescope::Mode::Auto => self.video.upscaling.gamescope_enabled,
        });
        feature_state(
            configured,
            self.gamescope_installed,
            self.game.is_some(),
            self.in_game.as_ref().map(|g| g.gamescope),
        )
    }

    /// Wine FSR: the variable in the game's environment.
    #[must_use]
    pub fn wine_fsr_state(&self) -> State {
        // Not asked for a game Big Game Mode switched it off for: one with AI
        // Graphics files (its own launch leaves it out) and one whose Steam
        // launch options carry WINE_FULLSCREEN_FSR=0 from Big Game Mode.
        let asked = self.video.upscaling.wine_fsr_enabled
            && !(self.game.is_some() && (self.ai_graphics.is_some() || self.wine_fsr_off_for_game));
        match feature_state(asked, true, self.game.is_some(), self.wine_fsr_in_game) {
            // The variable in the game says Wine FSR is ready, not that it
            // scales: it does only when the game picks a fullscreen mode below
            // the display's, which cannot be read from outside. Tomb Raider
            // (2013) at the display's 3440×1440 had it and scaled nothing.
            State::Active => State::Configured,
            state => state,
        }
    }

    /// vkBasalt: its layer mapped in the game.
    #[must_use]
    pub fn vkbasalt_state(&self) -> State {
        feature_state(
            self.video.upscaling.vkbasalt_enabled,
            self.vkbasalt_installed,
            self.game.is_some(),
            self.in_game
                .as_ref()
                .map(|g| g.vkbasalt || g.vkbasalt_in_gamescope),
        )
    }

    /// Frame generation through lsfg-vk: asked for globally and for the
    /// game, the layer mapped and generating.
    #[must_use]
    pub fn frame_generation_state(&self) -> State {
        // Asked for: the global switch on, and for a running game an entry
        // of its own; with no game the switch alone is the request.
        let configured =
            self.lsfg.global_on && (self.lsfg.game_multiplier.is_some() || self.game.is_none());
        if configured && !self.lsfg.dll_ready {
            return State::Missing;
        }
        feature_state(
            configured,
            self.lsfg.installed,
            self.game.is_some(),
            self.in_game.as_ref().map(|g| g.frame_generation.is_some()),
        )
    }

    /// `MangoHud`: chosen for the game, its library mapped.
    #[must_use]
    pub fn mangohud_state(&self) -> State {
        feature_state(
            self.mangohud_for_game != crate::mangohud::Mode::Off,
            self.mangohud_installed,
            self.game.is_some(),
            self.in_game.as_ref().map(|g| g.mangohud),
        )
    }

    /// Turbo.
    #[must_use]
    pub fn turbo_state(&self) -> State {
        if self.unit_failed {
            State::Error
        } else if self.turbo_unreadable {
            State::NotDetected
        } else if !self.falcond_installed {
            State::Missing
        } else if self.turbo_on {
            State::Active
        } else {
            State::Off
        }
    }

    /// falcond as a service.
    #[must_use]
    pub fn falcond_state(&self) -> State {
        match (self.turbo_state(), &self.falcond) {
            (State::Error, _) => State::Error,
            (State::Missing, _) => State::Missing,
            (State::Off, _) => State::Off,
            (_, Some(_)) => State::Active,
            (_, None) => State::NotDetected,
        }
    }

    /// The power profile as an optimization: `performance` while a game
    /// runs under Turbo is what falcond does.
    #[must_use]
    pub fn power_state(&self) -> State {
        let Some(p) = self.power_profile.as_deref() else {
            return State::Missing;
        };
        if !self.turbo_on && !self.turbo_unreadable {
            return State::Off;
        }
        match (self.game.is_some(), p) {
            (true, "performance") => State::Active,
            (true, _) => State::NotDetected,
            (false, _) => State::Waiting,
        }
    }

    /// A second upscaler seen in the running game next to `OptiScaler`'s.
    ///
    /// Big Game Mode's own launches turn Wine FSR and Gamescope's scaling off
    /// for a game with AI Graphics, but a game started by Steam gets Steam's
    /// launch options and the session environment: `WINE_FULLSCREEN_FSR=1`
    /// there puts Wine's upscaler in series with `OptiScaler`'s.
    #[must_use]
    pub fn upscaler_conflict(&self) -> Option<UpscalerConflict> {
        let ai_active = matches!(
            self.ai_graphics,
            Some(crate::graphics::runtime::Status::Active { .. })
        );
        if !ai_active {
            return None;
        }
        if self.wine_fsr_in_game == Some(true) {
            return Some(if self.video.upscaling.wine_fsr_enabled {
                UpscalerConflict::WineFsrFromTuning
            } else {
                UpscalerConflict::WineFsrFromElsewhere
            });
        }
        let gamescope_scales = self.in_game.as_ref().is_some_and(|g| g.gamescope)
            && self.video.upscaling.base_width > 0;
        gamescope_scales.then_some(UpscalerConflict::GamescopeScaling)
    }

    /// Every state that needs attention, for the overview's count.
    #[must_use]
    pub fn attention_count(&self) -> usize {
        [
            self.turbo_state(),
            self.falcond_state(),
            self.power_state(),
            self.scheduler.state(self.game.is_some()),
            self.vcache.state(self.game.is_some()),
            self.gamescope_state(),
            self.wine_fsr_state(),
            self.vkbasalt_state(),
            self.frame_generation_state(),
            self.mangohud_state(),
        ]
        .iter()
        .filter(|s| s.needs_attention())
        .count()
            + usize::from(self.upscaler_conflict().is_some())
    }
}

/// Two upscalers in series in the running game, and where the second came
/// from — which says where to turn it off.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpscalerConflict {
    /// Wine FSR, switched on in Tuning (the session environment).
    WineFsrFromTuning,
    /// Wine FSR, from outside Big Game Mode: Steam's launch options for the
    /// game, most often.
    WineFsrFromElsewhere,
    /// Gamescope rendering below its output.
    GamescopeScaling,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_feature_is_active_only_when_seen_in_the_game() {
        // Configured, installed, a game runs: what the game shows decides.
        assert_eq!(feature_state(true, true, true, Some(true)), State::Active);
        assert_eq!(
            feature_state(true, true, true, Some(false)),
            State::NotDetected
        );
        assert_eq!(feature_state(true, true, true, None), State::Configured);
        // No game: it waits, whatever was configured.
        assert_eq!(feature_state(true, true, false, None), State::Waiting);
        assert_eq!(
            feature_state(true, true, false, Some(false)),
            State::Waiting
        );
        // Not asked for: off, even with the software there.
        assert_eq!(feature_state(false, true, false, None), State::Off);
        assert_eq!(feature_state(false, true, true, Some(false)), State::Off);
        // Asked for without the software: missing, before anything else.
        assert_eq!(
            feature_state(true, false, true, Some(false)),
            State::Missing
        );
        assert_eq!(feature_state(true, false, false, None), State::Missing);
        // Seen in the game although Big Game Mode did not ask (Steam's launch
        // options): what is, not what Big Game Mode did.
        assert_eq!(feature_state(false, true, true, Some(true)), State::Active);
    }

    #[test]
    fn only_problems_need_attention() {
        for s in [
            State::Active,
            State::Waiting,
            State::Configured,
            State::Off,
            State::Unsupported,
        ] {
            assert!(!s.needs_attention(), "{s:?}");
        }
        for s in [State::NotDetected, State::Missing, State::Error] {
            assert!(s.needs_attention(), "{s:?}");
        }
    }

    fn status(active: Option<&str>) -> FalcondStatus {
        FalcondStatus {
            active_profile: active.map(str::to_owned),
            ..FalcondStatus::default()
        }
    }

    #[test]
    fn the_headline_follows_turbo_falcond_and_the_game() {
        assert_eq!(headline(false, false, None, false), Headline::TurboOff);
        assert_eq!(headline(false, false, None, true), Headline::TurboOff);
        assert_eq!(headline(true, true, None, false), Headline::FalcondFailed);
        assert_eq!(headline(true, false, None, false), Headline::FalcondSilent);
        assert_eq!(
            headline(true, false, Some(&status(None)), false),
            Headline::ReadyWaiting
        );
        assert_eq!(
            headline(true, false, Some(&status(Some("Proton"))), true),
            Headline::Optimizing
        );
        assert_eq!(
            headline(true, false, Some(&status(None)), true),
            Headline::GameWithoutProfile
        );
    }

    #[test]
    fn a_main_thread_that_renamed_itself_is_told_apart() {
        // Cyberpunk 2077 under Proton: REDengine renames its main thread.
        assert_eq!(
            renamed_main_thread("GameThread\n", "Cyberpunk2077.exe").as_deref(),
            Some("GameThread")
        );
        // The kernel's 15 characters, Wine's own names: not renamed.
        assert_eq!(
            renamed_main_thread("Cyberpunk2077.e", "Cyberpunk2077.exe"),
            None
        );
        assert_eq!(renamed_main_thread("SOTTR.exe", "SOTTR.exe"), None);
        assert_eq!(renamed_main_thread("wine64-preloade", "Game.exe"), None);
        assert_eq!(renamed_main_thread("", "Game.exe"), None);
    }

    #[test]
    fn falcond_before_2_0_3_misses_a_renamed_game() {
        assert!(falcond_misses_renamed("2.0.2-2"));
        assert!(falcond_misses_renamed("1.9.0-1"));
        assert!(!falcond_misses_renamed("2.0.3-1"));
        assert!(!falcond_misses_renamed("2.0.14-1"));
        assert!(!falcond_misses_renamed("2.1.0-1"));
        assert!(!falcond_misses_renamed("3.0.0-1"));
        assert!(
            !falcond_misses_renamed("unknown"),
            "no claim without a version"
        );
    }

    #[test]
    fn the_games_own_profile_not_applied_changes_the_headline() {
        let mut s = Snapshot {
            turbo_on: true,
            falcond: Some(status(None)),
            game: Some(crate::running::GameIdentity {
                display_name: "Cyberpunk 2077".into(),
                steam_app_id: Some("1091500".into()),
                install_path: None,
                compatdata_path: None,
                pid: 1,
                process_name: "Cyberpunk2077.exe".into(),
                executable: "S:\\x\\Cyberpunk2077.exe".into(),
                runtime: crate::running::Runtime::Proton(String::new()),
                graphics: crate::running::Graphics::Vkd3dProton,
                render_card: None,
                tree: vec![],
            }),
            ..Snapshot::default()
        };
        assert_eq!(s.headline(), Headline::GameWithoutProfile);
        s.profile_not_applied = Some(ProfileNotApplied {
            name: "Cyberpunk2077.exe".into(),
            renamed_to: Some("GameThread".into()),
            falcond_misses_renamed: true,
        });
        assert_eq!(s.headline(), Headline::ProfileNotApplied);
        // falcond's Proton profile in its place is not the game's either.
        s.falcond = Some(status(Some("Proton")));
        assert_eq!(s.headline(), Headline::ProfileNotApplied);
    }

    #[test]
    fn falconds_proton_profile_is_explained_not_shown_as_the_games() {
        assert_eq!(applied_profile(None, None), AppliedProfile::None);
        assert_eq!(applied_profile(Some("None"), None), AppliedProfile::None);
        assert_eq!(
            applied_profile(Some("Proton"), None),
            AppliedProfile::GenericProton
        );
        let m = crate::running::ProfileMatch {
            name: "SOTTR.exe".into(),
            path: "/usr/share/falcond/profiles/user/SOTTR.exe.conf".into(),
            user: true,
        };
        assert_eq!(
            applied_profile(Some("SOTTR.exe"), Some(&m)),
            AppliedProfile::Own {
                name: "SOTTR.exe".into(),
                path: Some(m.path.clone()),
                user: true
            }
        );
        assert_eq!(
            applied_profile(Some("Elsewhere"), Some(&m)),
            AppliedProfile::Other("Elsewhere".into())
        );
    }

    fn switchable() -> SchedExtCaps {
        SchedExtCaps {
            kernel_support: true,
            state: Some("enabled".into()),
            installed: vec!["lavd".into(), "bpfland".into()],
            scxctl: true,
            loader_installed: true,
            loader_service: true,
        }
    }

    #[test]
    fn the_scheduler_state_compares_what_was_asked_with_what_runs() {
        let mut s = Scheduler {
            caps: switchable(),
            requested: "lavd".into(),
            requested_by: RequestedBy::GameProfile,
            loaded: Some("scx_lavd".into()),
            falcond_current: None,
        };
        assert_eq!(s.state(true), State::Active);
        s.loaded = Some("scx_bpfland".into());
        assert_eq!(s.state(true), State::NotDetected, "another scheduler runs");
        s.loaded = None;
        assert_eq!(
            s.state(true),
            State::NotDetected,
            "none runs during the game"
        );
        assert_eq!(s.state(false), State::Waiting);
        s.requested = "none".into();
        assert_eq!(s.state(false), State::Off);
        s.loaded = Some("scx_lavd".into());
        assert_eq!(
            s.state(true),
            State::Active,
            "loaded by someone else: it runs"
        );
    }

    #[test]
    fn a_scheduler_that_cannot_be_switched_is_missing_only_when_asked_for() {
        let mut caps = switchable();
        caps.installed.clear();
        let s = Scheduler {
            caps,
            requested: "lavd".into(),
            ..Scheduler::default()
        };
        assert_eq!(s.state(true), State::Missing);
        let s = Scheduler {
            requested: "none".into(),
            ..s
        };
        assert_eq!(s.state(true), State::Off);
        let s = Scheduler {
            caps: SchedExtCaps::default(),
            requested: "lavd".into(),
            ..Scheduler::default()
        };
        assert_eq!(s.state(true), State::Unsupported, "no kernel support");
    }

    #[test]
    fn a_cpu_without_vcache_is_unsupported_never_a_problem() {
        let v = VCache {
            available: false,
            requested: "cache".into(),
            current: None,
        };
        assert_eq!(v.state(true), State::Unsupported);
        assert!(!v.state(true).needs_attention());
        let v = VCache {
            available: true,
            requested: "cache".into(),
            current: Some("cache".into()),
        };
        assert_eq!(v.state(true), State::Active);
        assert_eq!(v.state(false), State::Waiting);
        let v = VCache {
            current: Some("frequency".into()),
            ..v
        };
        assert_eq!(v.state(true), State::NotDetected);
        let v = VCache {
            requested: "none".into(),
            ..v
        };
        assert_eq!(v.state(true), State::Off);
    }

    #[test]
    fn frame_generation_without_the_dll_is_missing_and_off_when_not_asked() {
        let mut s = Snapshot {
            lsfg: Lsfg {
                installed: true,
                dll_ready: false,
                global_on: true,
                game_multiplier: None,
            },
            ..Snapshot::default()
        };
        assert_eq!(s.frame_generation_state(), State::Missing);
        s.lsfg.dll_ready = true;
        assert_eq!(s.frame_generation_state(), State::Waiting);
        s.lsfg.global_on = false;
        assert_eq!(s.frame_generation_state(), State::Off);
    }

    #[test]
    fn turbo_and_falcond_states_are_read_from_the_unit() {
        let s = Snapshot {
            turbo_on: true,
            falcond_installed: true,
            falcond: Some(status(None)),
            ..Snapshot::default()
        };
        assert_eq!(s.turbo_state(), State::Active);
        assert_eq!(s.falcond_state(), State::Active);
        let s = Snapshot { falcond: None, ..s };
        assert_eq!(s.falcond_state(), State::NotDetected, "running, but silent");
        let s = Snapshot {
            unit_failed: true,
            ..s
        };
        assert_eq!(s.turbo_state(), State::Error);
        let s = Snapshot {
            falcond_installed: false,
            unit_failed: false,
            ..s
        };
        assert_eq!(s.turbo_state(), State::Missing);
        let s = Snapshot {
            turbo_on: false,
            falcond_installed: true,
            ..s
        };
        assert_eq!(s.turbo_state(), State::Off);
        assert_eq!(s.falcond_state(), State::Off);
    }

    #[test]
    fn wine_fsr_next_to_optiscaler_is_a_conflict_and_says_where_it_came_from() {
        let active = crate::graphics::runtime::Status::Active {
            upscaler: "fsr31".into(),
            version: None,
            fsr4: None,
            fsr_generation: None,
        };
        let mut s = Snapshot {
            ai_graphics: Some(active.clone()),
            wine_fsr_in_game: Some(true),
            ..Snapshot::default()
        };
        // Seen on the reference desktop: Steam's launch options for Shadow
        // of the Tomb Raider carry WINE_FULLSCREEN_FSR=1.
        assert_eq!(
            s.upscaler_conflict(),
            Some(UpscalerConflict::WineFsrFromElsewhere)
        );
        s.video.upscaling.wine_fsr_enabled = true;
        assert_eq!(
            s.upscaler_conflict(),
            Some(UpscalerConflict::WineFsrFromTuning)
        );
        let with = s.attention_count();
        s.wine_fsr_in_game = Some(false);
        assert_eq!(s.upscaler_conflict(), None);
        assert_eq!(with - s.attention_count(), 1, "the conflict counts once");
        // No OptiScaler upscaling: Wine FSR alone is not a conflict.
        s.wine_fsr_in_game = Some(true);
        s.ai_graphics = Some(crate::graphics::runtime::Status::Configured);
        assert_eq!(s.upscaler_conflict(), None);
    }

    #[test]
    fn wine_fsr_in_the_game_is_ready_not_proven_scaling() {
        let mut s = Snapshot {
            game: Some(crate::running::GameIdentity {
                display_name: "Tomb Raider".into(),
                steam_app_id: Some("203160".into()),
                install_path: None,
                compatdata_path: None,
                pid: 1,
                process_name: "TombRaider.exe".into(),
                executable: "S:\\x\\TombRaider.exe".into(),
                runtime: crate::running::Runtime::Proton("Proton - Experimental".into()),
                graphics: crate::running::Graphics::Dxvk,
                render_card: None,
                tree: vec![],
            }),
            wine_fsr_in_game: Some(true),
            ..Snapshot::default()
        };
        s.video.upscaling.wine_fsr_enabled = true;
        assert_eq!(s.wine_fsr_state(), State::Configured);
        s.wine_fsr_in_game = Some(false);
        assert_eq!(s.wine_fsr_state(), State::NotDetected);
    }

    #[test]
    fn an_unreadable_turbo_is_not_reported_as_off() {
        // No system bus: systemd cannot be asked. That is not Turbo off.
        let s = Snapshot {
            turbo_unreadable: true,
            falcond_installed: true,
            power_profile: Some("performance".into()),
            ..Snapshot::default()
        };
        assert_eq!(s.headline(), Headline::TurboUnreadable);
        assert_eq!(s.turbo_state(), State::NotDetected);
        assert_ne!(s.power_state(), State::Off);
    }

    #[test]
    fn the_power_profile_is_judged_only_while_a_game_runs_under_turbo() {
        let s = Snapshot {
            turbo_on: true,
            power_profile: Some("balanced".into()),
            ..Snapshot::default()
        };
        assert_eq!(s.power_state(), State::Waiting);
        let s = Snapshot {
            turbo_on: false,
            ..s
        };
        assert_eq!(s.power_state(), State::Off);
        let s = Snapshot {
            power_profile: None,
            ..s
        };
        assert_eq!(s.power_state(), State::Missing);
    }
}
