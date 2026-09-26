//! One reading of what BiGame-mode is doing for the machine and the running
//! game — the Details page's data, collected in one place.
//!
//! Every value here is read from the system that holds it (`turbo`,
//! `status`, `/proc`, sysfs, power-profiles-daemon, the launch settings),
//! never from what BiGame-mode last did. The page renders a [`Snapshot`];
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
/// A feature seen in the game counts as active even when BiGame-mode did
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
    /// falcond's unit failed (systemd `failed`).
    pub unit_failed: bool,
    /// falcond is installed.
    pub falcond_installed: bool,
    /// falcond's status, when its file is there and trusted.
    pub falcond: Option<FalcondStatus>,
    /// The profile falcond applied, explained.
    pub profile: AppliedProfile,
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
    /// AI Graphics in the running game, when BiGame-mode installed it.
    pub ai_graphics: Option<crate::graphics::runtime::Status>,
    /// `OptiScaler`'s frame generation is on for the running game (chosen on
    /// its page, or switched on from `OptiScaler`'s overlay).
    pub ai_frame_generation: bool,
    /// Whether the running game's profile asks Gamescope never / always.
    pub gamescope_mode: crate::gamescope::Mode,
}

impl Snapshot {
    /// Read everything. Never fails: what cannot be read is `None`.
    #[must_use]
    pub fn collect(game: Option<GameIdentity>) -> Self {
        let systemd = crate::systemd::Reader::system();
        let unit = systemd
            .as_ref()
            .and_then(|r| r.unit_state(crate::turbo::BACKEND_UNIT));
        let turbo_on = unit.as_ref().map_or_else(
            || crate::turbo::state_blocking().is_ok_and(|s| s == crate::turbo::State::On),
            crate::systemd::UnitState::is_active,
        );
        let unit_failed = unit.as_ref().is_some_and(|u| u.active_state == "failed");
        let falcond = crate::status::read();
        // No `Capabilities::detect` here: it runs `gamescope --help` and
        // `systemctl`, too much for a reading taken every few seconds.
        let sched_caps = SchedExtCaps::detect();
        let config = crate::config::read().ok();
        let video = crate::video_config::load();

        let in_game = game.as_ref().map(crate::running::in_game);
        let matched = game.as_ref().and_then(|g| {
            crate::running::matching_profile(
                &g.process_name,
                falcond.as_ref().map_or("", |s| s.profile_mode.as_str()),
            )
        });
        let active = falcond.as_ref().and_then(|s| s.active_profile.as_deref());
        let profile = applied_profile(active, matched.as_ref());

        // What the active profile asks for; the global configuration
        // otherwise.
        let profile_file = match &profile {
            AppliedProfile::Own { name, .. } | AppliedProfile::Other(name) => {
                crate::profiles::load(name).ok()
            }
            AppliedProfile::GenericProton => crate::profiles::load("Proton").ok(),
            AppliedProfile::None => None,
        };
        let (requested_scx, requested_by) = match (&profile_file, &config) {
            (Some(p), _) => (p.scx_sched.clone(), RequestedBy::GameProfile),
            (None, Some(c)) if !c.scx_sched.is_empty() && c.scx_sched != "none" => {
                (c.scx_sched.clone(), RequestedBy::GlobalConfig)
            }
            _ => (String::new(), RequestedBy::Nobody),
        };
        let requested_vcache = match (&profile_file, &config) {
            (Some(p), _) => p.vcache_mode.clone(),
            (None, Some(c)) => c.vcache_mode.clone(),
            _ => String::new(),
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
            .map(|g| crate::processes::env_has_key(g.pid, "WINE_FULLSCREEN_FSR"));
        let gamescope_mode = profile_file
            .as_ref()
            .map_or(crate::gamescope::Mode::Auto, |p| p.gamescope_mode);

        Self {
            turbo_on,
            unit_failed,
            falcond_installed: capabilities::which("falcond").is_some(),
            profile,
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
            video,
            falcond,
            in_game,
            game,
        }
    }

    /// The headline.
    #[must_use]
    pub fn headline(&self) -> Headline {
        headline(
            self.turbo_on,
            self.unit_failed,
            self.falcond.as_ref(),
            self.game.is_some(),
        )
    }

    /// Gamescope: configured globally or by the game's profile, seen in the
    /// game's process tree.
    #[must_use]
    pub fn gamescope_state(&self) -> State {
        let configured = match self.gamescope_mode {
            crate::gamescope::Mode::Enabled => true,
            crate::gamescope::Mode::Disabled => false,
            crate::gamescope::Mode::Auto => self.video.upscaling.gamescope_enabled,
        };
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
        feature_state(
            self.video.upscaling.wine_fsr_enabled,
            true,
            self.game.is_some(),
            self.wine_fsr_in_game,
        )
    }

    /// vkBasalt: its layer mapped in the game.
    #[must_use]
    pub fn vkbasalt_state(&self) -> State {
        feature_state(
            self.video.upscaling.vkbasalt_enabled,
            self.vkbasalt_installed,
            self.game.is_some(),
            self.in_game.as_ref().map(|g| g.vkbasalt),
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
        if !self.turbo_on {
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
    /// BiGame-mode's own launches turn Wine FSR and Gamescope's scaling off
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
    /// Wine FSR, from outside BiGame-mode: Steam's launch options for the
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
        // Seen in the game although BiGame-mode did not ask (Steam's launch
        // options): what is, not what BiGame-mode did.
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
