//! Gamescope for a game the Steam client starts.
//!
//! BiGame-mode's own launch wraps a game in Gamescope itself, but a Steam
//! game is started by the Steam client in its own process tree, which
//! BiGame-mode cannot reach. What reaches it is the game's launch options, so
//! a profile's Gamescope choice is written there as a wrapper in front of
//! `%command%` — the same way `MangoHud` is (`crate::mangohud`): only while
//! Steam is closed (it keeps its configuration in memory), in every Steam
//! account of the user, and read back.
//!
//! The game's own Wine FSR and vkBasalt go the same way, as variables in
//! front of everything (`crate::game_launch::GameLaunch::steam_env`): a
//! variable after another wrapper (`gamemoderun VAR=1 %command%`) would be
//! taken for the program to run.
//!
//! BiGame-mode owns only the segment and the variables it wrote (kept in the
//! game's settings); the user's own options and a Gamescope they typed
//! themselves are left alone.

use anyhow::Result;

use crate::gamescope::{Config, Mode};

/// The word Steam replaces with the game's command.
const COMMAND: &str = "%command%";

/// `gamescope <args> --`, the wrapper for `cfg`, or `None` when Gamescope
/// should not run for this game (the same decision BiGame-mode's own launch
/// makes).
#[must_use]
pub fn segment(
    mode: Mode,
    cfg: &Config,
    caps: Option<&crate::capabilities::GamescopeCaps>,
    session: crate::hardware::Session,
) -> Option<String> {
    let caps = caps?;
    if !crate::gamescope::decide(mode, cfg, Some(caps), session).use_gamescope {
        return None;
    }
    let args = cfg.to_args(caps).args;
    Some(format!("gamescope {} --", args.join(" ")).replace("  ", " "))
}

/// `current` launch options with the segment BiGame-mode wrote before
/// (`previous`) taken out and `wanted` put in front of `%command%` — before a
/// `mangohud` wrapper right in front of it, so the overlay stays inside
/// Gamescope and `MangoHud`'s own setting still finds its word, and before a
/// `prime-run` there, so render offload reaches the game and not Gamescope.
#[must_use]
pub fn launch_options(current: &str, previous: Option<&str>, wanted: Option<&str>) -> String {
    let mut words: Vec<String> = current.split_whitespace().map(str::to_owned).collect();
    if let Some(prev) = previous {
        let prev: Vec<&str> = prev.split_whitespace().collect();
        if !prev.is_empty() {
            if let Some(at) = words
                .windows(prev.len())
                .position(|w| w.iter().map(String::as_str).eq(prev.iter().copied()))
            {
                words.drain(at..at + prev.len());
            }
        }
    }
    let Some(wanted) = wanted else {
        let rest = words.join(" ");
        return if rest == COMMAND { String::new() } else { rest };
    };
    if !words.iter().any(|w| w == COMMAND) {
        // Plain arguments: they follow the command.
        words.insert(0, COMMAND.to_owned());
    }
    let mut at = words.iter().position(|w| w == COMMAND).unwrap_or(0);
    // `prime-run` too, which the hybrid health check suggests: in front of
    // Gamescope it hands Gamescope the offload variables, and Gamescope then
    // composites on the discrete GPU; on the GTX 1050 Ti laptop that
    // segfaulted 5 times in 5 with FSR upscaling. Inside, only the game gets
    // them.
    while at > 0 && matches!(words[at - 1].as_str(), "mangohud" | "prime-run") {
        at -= 1;
    }
    for (k, w) in wanted.split_whitespace().enumerate() {
        words.insert(at + k, w.to_owned());
    }
    words.join(" ")
}

/// What writing the wrapper did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Applied {
    /// Not a Steam game: BiGame-mode's own launch wraps it.
    NotSteam,
    /// Nothing to change.
    Unchanged,
    /// Steam is running and would overwrite the change; nothing written.
    SteamRunning,
    /// The launch options now read this (read back).
    Written(String),
}

/// `current` launch options with the variables BiGame-mode wrote before
/// (`previous`) taken out and `wanted` put in front of everything. Each
/// variable is taken out once, wherever it is, so one that something else
/// moved or removed does not keep the others in.
#[must_use]
pub fn env_options(current: &str, previous: Option<&str>, wanted: Option<&str>) -> String {
    let mut words: Vec<&str> = current.split_whitespace().collect();
    for old in previous.unwrap_or_default().split_whitespace() {
        if let Some(at) = words.iter().position(|w| *w == old) {
            words.remove(at);
        }
    }
    let wanted: Vec<&str> = wanted.unwrap_or_default().split_whitespace().collect();
    if wanted.is_empty() {
        let rest = words.join(" ");
        return if rest == COMMAND { String::new() } else { rest };
    }
    if !words.contains(&COMMAND) {
        // Plain arguments: they follow the command.
        words.insert(0, COMMAND);
    }
    wanted
        .into_iter()
        .chain(words)
        .collect::<Vec<_>>()
        .join(" ")
}

/// What a game's launch options should hold from BiGame-mode: its
/// Gamescope wrapper and its variables, each `None` for nothing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Wanted {
    /// `gamescope <args> --`, in front of `%command%`.
    pub gamescope: Option<String>,
    /// `NAME=value …`, in front of everything.
    pub env: Option<String>,
}

/// Put the Gamescope wrapper and the variables for the game whose process
/// is `process` into its Steam launch options, or take BiGame-mode's out
/// (`None`).
///
/// # Errors
/// Returns an error when Steam's configuration cannot be written or read
/// back, or the game's settings cannot be saved.
pub fn apply(process: &str, wanted: Wanted) -> Result<Applied> {
    let apps: Vec<String> = crate::games::detect_all()
        .into_iter()
        .filter(|g| g.profile_key() == process && g.source == crate::games::Source::Steam)
        .filter_map(|g| g.app_id)
        .collect();
    if apps.is_empty() {
        return Ok(Applied::NotSteam);
    }
    let mut settings = crate::game_settings::load(process).unwrap_or_default();
    if settings.steam_gamescope == wanted.gamescope && settings.steam_env == wanted.env {
        return Ok(Applied::Unchanged);
    }
    if crate::steam::is_running() {
        return Ok(Applied::SteamRunning);
    }
    let mut last = String::new();
    for user in crate::steam::users(&crate::paths::home_dir()) {
        for app in &apps {
            let current = crate::steam::launch_options(&user.config, app).unwrap_or_default();
            let next = launch_options(
                &env_options(
                    &current,
                    settings.steam_env.as_deref(),
                    wanted.env.as_deref(),
                ),
                settings.steam_gamescope.as_deref(),
                wanted.gamescope.as_deref(),
            );
            if next != current {
                crate::steam::set_launch_options(&user.config, app, &next)?;
            }
            last = next;
        }
    }
    settings.steam_gamescope = wanted.gamescope;
    settings.steam_env = wanted.env;
    crate::game_settings::save(process, &settings)?;
    Ok(Applied::Written(last))
}

// ── Wine FSR off for one game ───────────────────────────────────────────────

/// The switch that turns Wine FSR off for one game, whatever the session
/// environment says.
const WINE_FSR_OFF: &str = "WINE_FULLSCREEN_FSR=0";

/// `current` with Wine FSR turned off (`off`) or BiGame-mode's switch taken
/// out again. Off also replaces a `WINE_FULLSCREEN_FSR=1` already there (the
/// user asked, with the button that says so); taking it out leaves Wine FSR
/// to the session, as Tuning sets it.
#[must_use]
pub fn wine_fsr_options(current: &str, off: bool) -> String {
    let mut words: Vec<&str> = current
        .split_whitespace()
        .filter(|w| *w != WINE_FSR_OFF && !(off && *w == "WINE_FULLSCREEN_FSR=1"))
        .collect();
    if off {
        if !words.contains(&COMMAND) {
            words.insert(0, COMMAND);
        }
        words.insert(0, WINE_FSR_OFF);
    }
    let rest = words.join(" ");
    if rest == COMMAND { String::new() } else { rest }
}

/// Whether the game's Steam launch options switch Wine FSR on themselves.
#[must_use]
pub fn wine_fsr_in_options(process: &str) -> bool {
    steam_apps(process).iter().any(|app| {
        crate::steam::users(&crate::paths::home_dir())
            .iter()
            .filter_map(|u| crate::steam::launch_options(&u.config, app))
            .any(|o| o.split_whitespace().any(|w| w == "WINE_FULLSCREEN_FSR=1"))
    })
}

/// The game's Steam launch options as they read now, from the first
/// account that has any; `None` for a game that is not a Steam game or has
/// none.
#[must_use]
pub fn current_options(process: &str) -> Option<String> {
    let users = crate::steam::users(&crate::paths::home_dir());
    steam_apps(process).iter().find_map(|app| {
        users
            .iter()
            .filter_map(|u| crate::steam::launch_options(&u.config, app))
            .find(|o| !o.trim().is_empty())
    })
}

fn steam_apps(process: &str) -> Vec<String> {
    crate::games::detect_all()
        .into_iter()
        .filter(|g| g.profile_key() == process && g.source == crate::games::Source::Steam)
        .filter_map(|g| g.app_id)
        .collect()
}

/// Turn Wine FSR off for the game whose process is `process` in its Steam
/// launch options (`off`), or take BiGame-mode's switch out again.
///
/// # Errors
/// Returns an error when Steam's configuration cannot be written or the
/// game's settings cannot be saved.
pub fn set_wine_fsr_off(process: &str, off: bool) -> Result<Applied> {
    let apps = steam_apps(process);
    if apps.is_empty() {
        return Ok(Applied::NotSteam);
    }
    let mut settings = crate::game_settings::load(process).unwrap_or_default();
    if crate::steam::is_running() {
        return Ok(Applied::SteamRunning);
    }
    let mut last = String::new();
    for user in crate::steam::users(&crate::paths::home_dir()) {
        for app in &apps {
            let current = crate::steam::launch_options(&user.config, app).unwrap_or_default();
            let next = wine_fsr_options(&current, off);
            if next != current {
                crate::steam::set_launch_options(&user.config, app, &next)?;
            }
            last = next;
        }
    }
    settings.steam_wine_fsr_off = off;
    crate::game_settings::save(process, &settings)?;
    Ok(Applied::Written(last))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEG: &str = "gamescope -w 1280 -h 720 -F fsr -f --";

    #[test]
    fn the_wrapper_goes_in_front_of_the_command_and_the_users_options_stay() {
        assert_eq!(
            launch_options("", None, Some(SEG)),
            format!("{SEG} %command%")
        );
        let mine = "PROTON_LOG=1 %command% -dx12";
        assert_eq!(
            launch_options(mine, None, Some(SEG)),
            format!("PROTON_LOG=1 {SEG} %command% -dx12")
        );
        // Arguments without %command% follow the command.
        assert_eq!(
            launch_options("-novid", None, Some(SEG)),
            format!("{SEG} %command% -novid")
        );
    }

    #[test]
    fn wine_fsr_is_switched_off_for_one_game_and_back() {
        assert_eq!(
            wine_fsr_options("", true),
            "WINE_FULLSCREEN_FSR=0 %command%"
        );
        // The user's own =1 goes with the button; everything else stays.
        let mine = "MANGOHUD=1 WINE_FULLSCREEN_FSR=1 ENABLE_VKBASALT=1 %command% -dx12";
        let off = wine_fsr_options(mine, true);
        assert_eq!(
            off,
            "WINE_FULLSCREEN_FSR=0 MANGOHUD=1 ENABLE_VKBASALT=1 %command% -dx12"
        );
        assert_eq!(wine_fsr_options(&off, true), off, "idempotent");
        assert_eq!(
            wine_fsr_options(&off, false),
            "MANGOHUD=1 ENABLE_VKBASALT=1 %command% -dx12"
        );
        assert_eq!(
            wine_fsr_options("WINE_FULLSCREEN_FSR=0 %command%", false),
            ""
        );
    }

    #[test]
    fn a_games_variables_go_in_front_of_everything_and_come_out_again() {
        let env = "WINE_FULLSCREEN_FSR=0 ENABLE_VKBASALT=1";
        assert_eq!(env_options("", None, Some(env)), format!("{env} %command%"));
        let mine = "MANGOHUD=1 gamemoderun %command% -dx12";
        let on = env_options(mine, None, Some(env));
        assert_eq!(on, format!("{env} {mine}"));
        assert_eq!(env_options(&on, Some(env), None), mine);
        // Replaced, not added twice.
        assert_eq!(
            env_options(&on, Some(env), Some("ENABLE_VKBASALT=0")),
            format!("ENABLE_VKBASALT=0 {mine}")
        );
        assert_eq!(
            env_options(&format!("{env} %command%"), Some(env), None),
            ""
        );
        // One variable removed by something else: the other still goes.
        let moved = wine_fsr_options(&on, false);
        assert_eq!(env_options(&moved, Some(env), None), mine);
        // With the Gamescope wrapper, each in its place.
        let both = launch_options(&env_options(mine, None, Some(env)), None, Some(SEG));
        assert_eq!(
            both,
            format!("{env} MANGOHUD=1 gamemoderun {SEG} %command% -dx12")
        );
    }

    #[test]
    fn mangohud_stays_inside_gamescope() {
        assert_eq!(
            launch_options("mangohud %command%", None, Some(SEG)),
            format!("{SEG} mangohud %command%")
        );
        // …and MangoHud's own rule still finds its wrapper.
        let both = launch_options("mangohud %command%", None, Some(SEG));
        assert_eq!(
            crate::mangohud::launch_options(&both, crate::mangohud::Mode::Off),
            format!("{SEG} %command%")
        );
    }

    #[test]
    fn prime_run_stays_inside_gamescope() {
        // The health check's advice for a hybrid laptop, then a Gamescope
        // choice: the offload reaches the game, not Gamescope.
        assert_eq!(
            launch_options("prime-run %command% -dx12", None, Some(SEG)),
            format!("{SEG} prime-run %command% -dx12")
        );
        assert_eq!(
            launch_options("prime-run mangohud %command%", None, Some(SEG)),
            format!("{SEG} prime-run mangohud %command%")
        );
        // Taken out again, the user's own words are as they were.
        let written = launch_options("MANGOHUD=1 prime-run %command%", None, Some(SEG));
        assert_eq!(written, format!("MANGOHUD=1 {SEG} prime-run %command%"));
        assert_eq!(
            launch_options(&written, Some(SEG), None),
            "MANGOHUD=1 prime-run %command%"
        );
    }

    #[test]
    fn only_the_segment_bigame_mode_wrote_is_replaced_or_removed() {
        let written = launch_options("MANGOHUD=1 %command%", None, Some(SEG));
        let other = "gamescope -w 1920 -h 1080 -f --";
        assert_eq!(
            launch_options(&written, Some(SEG), Some(other)),
            format!("MANGOHUD=1 {other} %command%")
        );
        assert_eq!(
            launch_options(&written, Some(SEG), None),
            "MANGOHUD=1 %command%"
        );
        assert_eq!(
            launch_options(&format!("{SEG} %command%"), Some(SEG), None),
            ""
        );
        // A Gamescope the user typed is not ours to remove.
        let theirs = "gamescope -W 3440 -H 1440 -- %command%";
        assert_eq!(launch_options(theirs, Some(SEG), None), theirs);
    }

    #[test]
    fn the_segment_follows_the_same_decision_as_bigame_modes_own_launch() {
        let caps = crate::capabilities::GamescopeCaps {
            version: None,
            flags: ["w", "h", "W", "H", "F", "f", "r", "S"]
                .iter()
                .map(|s| (*s).to_owned())
                .collect(),
        };
        let wayland = crate::hardware::Session::Wayland;
        let cfg = Config {
            render_width: 1280,
            render_height: 720,
            ..Config::default()
        };
        assert_eq!(segment(Mode::Disabled, &cfg, Some(&caps), wayland), None);
        assert_eq!(segment(Mode::Enabled, &cfg, None, wayland), None);
        let seg = segment(Mode::Enabled, &cfg, Some(&caps), wayland).unwrap();
        assert!(
            seg.starts_with("gamescope ") && seg.ends_with(" --"),
            "{seg}"
        );
        assert!(seg.contains("-w 1280") && seg.contains("-h 720"), "{seg}");
        assert!(!seg.contains("  "));
    }
}
