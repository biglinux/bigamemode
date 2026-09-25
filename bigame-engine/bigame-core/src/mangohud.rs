//! `MangoHud` per game: off, on, or forced.
//!
//! `MangoHud` reaches a game in one of two ways, and they cover different games:
//!
//! * **On** — its implicit Vulkan layer, switched on by `MANGOHUD=1`. That is
//!   every Vulkan game, and every Proton game (DXVK and `VKD3D-Proton` are
//!   Vulkan).
//! * **Forced** — the `mangohud` wrapper, which also preloads it into `OpenGL`
//!   games, where the Vulkan layer never loads.
//!
//! A game BiGame-mode starts gets it through its launch plan (inside Gamescope,
//! as Gamescope's own `--mangoapp`). A game the Steam client starts runs in
//! Steam's process tree, which BiGame-mode cannot reach: there the only way is
//! the game's launch options in Steam, edited with Steam closed, backed up and
//! read back (`steam::set_launch_options`). Options the user wrote stay; only
//! what this module adds is ever removed.

use anyhow::Result;
use serde::{Deserialize, Serialize};

/// A game's `MangoHud` setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    /// Not added by BiGame-mode (`MangoHud` may still come from elsewhere).
    #[default]
    Off,
    /// The Vulkan layer (`MANGOHUD=1`): Vulkan and Proton games.
    On,
    /// The `mangohud` wrapper: `OpenGL` games too.
    Forced,
}

/// The word Steam replaces with the game's command.
const COMMAND: &str = "%command%";
/// What [`Mode::On`] adds in front of Steam's launch options.
const LAYER: &str = "MANGOHUD=1";
/// What [`Mode::Forced`] puts in front of `%command%`.
const WRAPPER: &str = "mangohud";

/// Steam launch options with `mode` applied to `current`.
///
/// Removes what an earlier call added (a leading `MANGOHUD=1`, a `mangohud`
/// right before `%command%`), then adds what `mode` needs. Everything else —
/// other variables, other wrappers, the game's own arguments — is kept in
/// place. Launch options without `%command%` are the game's arguments, and
/// Steam appends them to the command; they are kept after it.
#[must_use]
pub fn launch_options(current: &str, mode: Mode) -> String {
    let mut words: Vec<&str> = current.split_whitespace().collect();
    words.retain(|w| *w != LAYER);
    if let Some(i) = words.iter().position(|w| *w == COMMAND) {
        if i > 0 && words[i - 1] == WRAPPER {
            words.remove(i - 1);
        }
    }
    let has_command = words.contains(&COMMAND);
    if mode == Mode::Off {
        let rest = words.join(" ");
        return if rest == COMMAND { String::new() } else { rest };
    }
    if !has_command {
        // Plain arguments: they follow the command.
        words.insert(0, COMMAND);
    }
    match mode {
        Mode::On => words.insert(0, LAYER),
        Mode::Forced => {
            let i = words.iter().position(|w| *w == COMMAND).unwrap_or(0);
            words.insert(i, WRAPPER);
        }
        Mode::Off => {}
    }
    words.join(" ")
}

/// Where a `MangoHud` setting took effect.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Applied {
    /// In the launch plan of games BiGame-mode starts; the game is not a
    /// Steam game.
    LaunchPlan,
    /// In Steam's launch options, which now read as given (read back).
    SteamLaunchOptions(String),
    /// Nothing changed: Steam is running and holds its configuration in
    /// memory, so the launch options could not be written.
    SteamRunning,
}

/// Save `mode` for the game whose process is `process`, and write it where
/// the game will see it.
///
/// # Errors
/// Returns an error if the setting cannot be saved or Steam's configuration
/// cannot be written or verified.
pub fn apply(process: &str, mode: Mode) -> Result<Applied> {
    let apps: Vec<String> = crate::games::detect_all()
        .into_iter()
        .filter(|g| g.source == crate::games::Source::Steam && g.profile_key() == process)
        .filter_map(|g| g.app_id)
        .collect();
    // A Steam game whose launch options cannot be written now keeps its old
    // setting too: a saved choice that is not in effect would be a lie.
    if !apps.is_empty() && crate::steam::is_running() {
        return Ok(Applied::SteamRunning);
    }
    let mut settings = crate::game_settings::load(process).unwrap_or_default();
    settings.mangohud = mode;
    crate::game_settings::save(process, &settings)?;
    if apps.is_empty() {
        return Ok(Applied::LaunchPlan);
    }
    let mut last = String::new();
    for user in crate::steam::users(&crate::paths::home_dir()) {
        for app in &apps {
            let current = crate::steam::launch_options(&user.config, app).unwrap_or_default();
            let wanted = launch_options(&current, mode);
            if wanted != current {
                crate::steam::set_launch_options(&user.config, app, &wanted)?;
            }
            last = wanted;
        }
    }
    Ok(Applied::SteamLaunchOptions(last))
}

/// The game's `MangoHud` setting, as saved.
#[must_use]
pub fn mode_for(process: &str) -> Mode {
    crate::game_settings::load(process).map_or(Mode::Off, |s| s.mangohud)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_launch_options_get_the_layer_or_the_wrapper() {
        assert_eq!(launch_options("", Mode::On), "MANGOHUD=1 %command%");
        assert_eq!(launch_options("", Mode::Forced), "mangohud %command%");
        assert_eq!(launch_options("", Mode::Off), "");
    }

    #[test]
    fn the_users_own_options_are_kept_in_place() {
        let mine = "PROTON_LOG=1 gamemoderun %command% -dx12";
        assert_eq!(
            launch_options(mine, Mode::On),
            "MANGOHUD=1 PROTON_LOG=1 gamemoderun %command% -dx12"
        );
        assert_eq!(
            launch_options(mine, Mode::Forced),
            "PROTON_LOG=1 gamemoderun mangohud %command% -dx12"
        );
        // Plain arguments follow the command.
        assert_eq!(
            launch_options("-novid", Mode::On),
            "MANGOHUD=1 %command% -novid"
        );
    }

    #[test]
    fn switching_mode_replaces_what_this_module_added_and_off_removes_it() {
        let on = launch_options("gamemoderun %command%", Mode::On);
        let forced = launch_options(&on, Mode::Forced);
        assert_eq!(forced, "gamemoderun mangohud %command%");
        assert_eq!(launch_options(&forced, Mode::Off), "gamemoderun %command%");
        assert_eq!(launch_options(&launch_options("", Mode::On), Mode::Off), "");
        // Applying twice changes nothing.
        assert_eq!(launch_options(&on, Mode::On), on);
    }
}
