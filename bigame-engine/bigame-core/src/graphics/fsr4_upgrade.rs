//! FSR 4 through Proton for the game's own FSR: the one thing the Native
//! backend can *do*, and how it is verified.
//!
//! Proton ships AMD's FSR 4 provider (`contrib/amdxcffx64.dll`, copied into
//! every prefix's `system32`) and a Wine `amdxc64.dll` that hands a game's
//! `FidelityFX` API (FSR 3.1+) over to it — but only when the game runs with
//! `FSR4_UPGRADE=1` in its environment (`getenv` in that DLL; checked on the
//! reference desktop, Proton Experimental 11.0: without it, a game with FSR
//! 3.1 selected never mapped the provider). GE-Proton uses
//! `PROTON_FSR4_UPGRADE=1` and downloads its own copy of the provider.
//!
//! A game the Steam client starts gets its environment from its launch
//! options, so that is where the variable goes: written with Steam closed,
//! backed up and read back ([`crate::steam::set_launch_options`]), and only
//! the word this module adds is ever removed. A game Heroic starts gets it
//! in its settings there (`crate::heroic_launch`), both names, since Heroic
//! runs games with GE-Proton as often as with Valve's Proton: written with
//! Heroic closed, and only what Big Game Mode wrote is ever removed. It is
//! never applied by itself: the plan names it, Apply writes it.
//!
//! Verified, never assumed: the running game's environment holds the
//! variable, and it has the provider mapped ([`super::runtime::NativeRuntime`]).

use anyhow::Result;

/// The variable Valve's Proton and Proton-EM read.
pub const VARIABLE: &str = "FSR4_UPGRADE=1";
/// The variable GE-Proton reads (it also fetches the provider itself).
pub const GE_VARIABLE: &str = "PROTON_FSR4_UPGRADE=1";
/// The word Steam replaces with the game's command.
const COMMAND: &str = "%command%";

/// Whether `options` (Steam launch options) switch the upgrade on, for
/// either Proton flavour.
#[must_use]
pub fn enabled_in(options: &str) -> bool {
    options
        .split_whitespace()
        .any(|w| w == VARIABLE || w == GE_VARIABLE)
}

/// Steam launch options with the upgrade switched `on` or off.
///
/// Removes the variable wherever it was, then adds it in front when `on`.
/// Everything else — other variables, wrappers, the game's own arguments —
/// stays in place; arguments without `%command%` keep following it.
#[must_use]
pub fn launch_options(current: &str, on: bool) -> String {
    let mut words: Vec<&str> = current
        .split_whitespace()
        .filter(|w| *w != VARIABLE && *w != GE_VARIABLE)
        .collect();
    if !on {
        // A `%command%` that only ever stood in front of plain arguments
        // (put there by an earlier call) is not needed either.
        if words.first() == Some(&COMMAND) {
            words.remove(0);
        }
        return words.join(" ");
    }
    if !words.contains(&COMMAND) {
        words.insert(0, COMMAND);
    }
    words.insert(0, VARIABLE);
    words.join(" ")
}

/// Where the setting stands for a game.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Applied {
    /// Written to Steam's launch options, which now read as given.
    SteamLaunchOptions(String),
    /// Nothing changed: Steam is running and would discard the edit.
    SteamRunning,
    /// In the game's settings in Heroic: [`GE_VARIABLE`] and [`VARIABLE`],
    /// for whichever Proton Heroic runs it with.
    Heroic,
    /// Nothing changed: Heroic is open and would write its own copy of the
    /// game's settings back over the edit.
    HeroicRunning {
        /// The Heroic that is open.
        launcher: crate::launchers::Launcher,
        /// It runs a game now, so it must not be closed for this.
        game_running: bool,
    },
    /// Neither a Steam nor a Heroic game: Big Game Mode's own launch plan
    /// carries the variable.
    LaunchPlan,
}

/// Switch the upgrade on or off for the game whose process is `process`:
/// the Steam game `app_id`, in every Steam account on this machine, or a
/// game Heroic starts, in its settings there.
///
/// # Errors
/// Returns an error if Steam's or Heroic's configuration cannot be written
/// or verified.
pub fn apply(process: &str, app_id: Option<&str>, on: bool) -> Result<Applied> {
    use crate::heroic_launch::Applied as H;
    let Some(app) = app_id else {
        if crate::heroic_launch::targets(process).is_empty() {
            return Ok(Applied::LaunchPlan);
        }
        return Ok(
            match crate::optimization::set_heroic_fsr4_upgrade(process, on)? {
                H::NotHeroic => Applied::LaunchPlan,
                H::Unchanged | H::Written => Applied::Heroic,
                H::HeroicRunning {
                    launcher,
                    game_running,
                } => Applied::HeroicRunning {
                    launcher,
                    game_running,
                },
            },
        );
    };
    if crate::steam::is_running() {
        return Ok(Applied::SteamRunning);
    }
    let mut last = String::new();
    for user in crate::steam::users(&crate::paths::home_dir()) {
        let current = crate::steam::launch_options(&user.config, app).unwrap_or_default();
        let wanted = launch_options(&current, on);
        if wanted != current {
            crate::steam::set_launch_options(&user.config, app, &wanted)?;
        }
        last = wanted;
    }
    // The rest of the user's launch options stays out of the journal, which
    // Logs and the support report export.
    tracing::info!(target: "graphics", app, on, "FSR 4 upgrade launch option written");
    Ok(Applied::SteamLaunchOptions(last))
}

/// Whether the Steam game `app_id` has the upgrade in its launch options in
/// any account, or the game whose process is `process` has it in its
/// settings in Heroic.
#[must_use]
pub fn is_enabled(process: &str, app_id: Option<&str>) -> bool {
    let Some(app) = app_id else {
        // Kept only once written (`crate::optimization::set_heroic_fsr4_upgrade`).
        return crate::game_settings::load(process).is_ok_and(|s| s.heroic_fsr4_upgrade);
    };
    crate::steam::users(&crate::paths::home_dir())
        .iter()
        .any(|u| crate::steam::launch_options(&u.config, app).is_some_and(|o| enabled_in(&o)))
}

/// Whether process `pid` runs with the variable, read from its environment.
#[must_use]
pub fn in_environment(pid: u32) -> Option<bool> {
    let env = std::fs::read(format!("/proc/{pid}/environ")).ok()?;
    Some(
        env.split(|b| *b == 0)
            .any(|e| e.starts_with(b"FSR4_UPGRADE=1") || e.starts_with(b"PROTON_FSR4_UPGRADE=")),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_variable_goes_in_front_and_comes_out_again_leaving_the_rest() {
        assert_eq!(launch_options("", true), "FSR4_UPGRADE=1 %command%");
        let mine = "MANGOHUD=1 gamemoderun %command% -dx12";
        let on = launch_options(mine, true);
        assert_eq!(on, "FSR4_UPGRADE=1 MANGOHUD=1 gamemoderun %command% -dx12");
        assert!(enabled_in(&on));
        assert_eq!(launch_options(&on, false), mine);
        assert_eq!(launch_options("FSR4_UPGRADE=1 %command%", false), "");
        // Plain arguments follow the command.
        assert_eq!(
            launch_options("-skipintro", true),
            "FSR4_UPGRADE=1 %command% -skipintro"
        );
        assert_eq!(
            launch_options("FSR4_UPGRADE=1 %command% -skipintro", false),
            "-skipintro"
        );
        // GE-Proton's spelling counts as on, and is replaced when switched.
        assert!(enabled_in("PROTON_FSR4_UPGRADE=1 %command%"));
        assert_eq!(launch_options("PROTON_FSR4_UPGRADE=1 %command%", false), "");
        assert!(!enabled_in("FSR4_UPGRADE=0 %command%"));
        assert_eq!(
            launch_options(&on, true),
            on,
            "applied twice is applied once"
        );
    }
}
