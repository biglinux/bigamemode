//! Gamescope for a game the Steam client starts.
//!
//! Big Game Mode's own launch wraps a game in Gamescope itself, but a Steam
//! game is started by the Steam client in its own process tree, which
//! Big Game Mode cannot reach. What reaches it is the game's launch options, so
//! a profile's Gamescope choice is written there as a wrapper in front of
//! `%command%` — the same way `MangoHud` is (`crate::mangohud`): only while
//! Steam is closed (it keeps its configuration in memory), in every Steam
//! account of the user, and read back.
//!
//! The game's own Wine FSR and vkBasalt go the same way, as variables in
//! front of everything (`crate::game_launch::GameLaunch::steam_env`): a
//! variable after another wrapper (`gamemoderun VAR=1 %command%`) would be
//! taken for the program to run. `WINE_FULLSCREEN_FSR=0` has that one
//! owner too, whether Gamescope or `OptiScaler` upscales the game or AI
//! Graphics switched Wine FSR off for it.
//!
//! Big Game Mode owns only the segment and the variables it wrote (kept in the
//! game's settings); the user's own options are left alone. Where they run
//! a Gamescope of the user's own, Big Game Mode's is not added: two nested
//! compositors would scale twice, or not start. vkBasalt, which Gamescope
//! would otherwise load for itself, stays in the game (as Big Game Mode's own
//! launch keeps it, `crate::launcher`). The Flatpak Steam finds `gamescope`
//! only in a Flatpak extension: without it nothing is written, and the
//! error names the command that installs it.

use anyhow::Result;

use crate::error::UserError;
use crate::gamescope::{Config, Mode};
use crate::steam::{COMMAND, is_command, option_words};
use crate::text::N_;

/// `gamescope <args> --`, the wrapper for `cfg`, or `None` when Gamescope
/// should not run for this game (the same decision Big Game Mode's own launch
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

/// `segment` with vkBasalt kept out of Gamescope and in the game: Gamescope
/// would load it for itself, filter its own output after its FSR, and take
/// `ENABLE_VKBASALT` from the game. `env` in front, because a variable after
/// another wrapper would be taken for the program to run.
#[must_use]
pub fn keeping_vkbasalt_in_the_game(segment: &str) -> String {
    format!("env DISABLE_VKBASALT=1 {segment} env -u DISABLE_VKBASALT ENABLE_VKBASALT=1")
}

/// Whether `words` run a Gamescope before the game's command.
fn runs_gamescope(words: &[&str]) -> bool {
    words
        .iter()
        .take_while(|w| !is_command(w))
        .any(|w| *w == "gamescope" || w.ends_with("/gamescope"))
}

/// Take the first run of `seq` out of `words`; whether there was one.
fn take_out(words: &mut Vec<&str>, seq: &[&str]) -> bool {
    if seq.is_empty() {
        return false;
    }
    match words.windows(seq.len()).position(|w| w == seq) {
        Some(at) => {
            words.drain(at..at + seq.len());
            true
        }
        None => false,
    }
}

/// The words joined, nothing for a lone `%command%`.
fn joined(words: &[&str]) -> String {
    let rest = words.join(" ");
    if rest == COMMAND { String::new() } else { rest }
}

/// `current` launch options with the segment Big Game Mode wrote before
/// (`previous`) taken out and `wanted` put in front of `%command%` — before a
/// `mangohud` wrapper right in front of it, so the overlay stays inside
/// Gamescope and `MangoHud`'s own setting still finds its word, and before a
/// `prime-run` there, so render offload reaches the game and not Gamescope. A
/// segment already there is not added twice (a write that stopped half-way
/// leaves one), and none goes into options that run a Gamescope of the user's
/// own.
#[must_use]
pub fn launch_options(current: &str, previous: Option<&str>, wanted: Option<&str>) -> String {
    let mut words = option_words(current);
    if let Some(prev) = previous {
        take_out(&mut words, &option_words(prev));
    }
    let wanted = wanted.map(option_words).unwrap_or_default();
    while take_out(&mut words, &wanted) {}
    if wanted.is_empty() || runs_gamescope(&words) {
        return joined(&words);
    }
    if !words.iter().any(|w| is_command(w)) {
        // Plain arguments: they follow the command.
        words.insert(0, COMMAND);
    }
    let mut at = words.iter().position(|w| is_command(w)).unwrap_or(0);
    // `prime-run` too, which the hybrid health check suggests: in front of
    // Gamescope it hands Gamescope the offload variables, and Gamescope then
    // composites on the discrete GPU; on the GTX 1050 Ti laptop that
    // segfaulted 5 times in 5 with FSR upscaling. Inside, only the game gets
    // them.
    while at > 0 && matches!(words[at - 1], "mangohud" | "prime-run") {
        at -= 1;
    }
    for (k, w) in wanted.into_iter().enumerate() {
        words.insert(at + k, w);
    }
    words.join(" ")
}

/// What writing the wrapper did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Applied {
    /// Not a Steam game: Big Game Mode's own launch wraps it.
    NotSteam,
    /// Nothing to change.
    Unchanged,
    /// Steam is running and would overwrite the change; nothing written.
    SteamRunning,
    /// The launch options now read this (read back).
    Written(String),
    /// The launch options now read this (read back), without Big Game Mode's
    /// Gamescope: they already run a Gamescope of the user's own.
    TheirGamescope(String),
}

/// `current` launch options with the variables Big Game Mode wrote before
/// (`previous`) taken out and `wanted` put in front of everything. Each
/// variable is taken out once, wherever it is, so one that something else
/// moved or removed does not keep the others in; variables already in front
/// are not put there twice.
#[must_use]
pub fn env_options(current: &str, previous: Option<&str>, wanted: Option<&str>) -> String {
    let mut words = option_words(current);
    for old in option_words(previous.unwrap_or_default()) {
        if let Some(at) = words.iter().position(|w| *w == old) {
            words.remove(at);
        }
    }
    let wanted = option_words(wanted.unwrap_or_default());
    if wanted.is_empty() {
        return joined(&words);
    }
    if words.starts_with(&wanted) {
        return words.join(" ");
    }
    if !words.iter().any(|w| is_command(w)) {
        // Plain arguments: they follow the command.
        words.insert(0, COMMAND);
    }
    wanted
        .into_iter()
        .chain(words)
        .collect::<Vec<_>>()
        .join(" ")
}

/// What a game's launch options should hold from Big Game Mode: its
/// Gamescope wrapper and its variables, each `None` for nothing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Wanted {
    /// `gamescope <args> --`, in front of `%command%`.
    pub gamescope: Option<String>,
    /// `NAME=value …`, in front of everything.
    pub env: Option<String>,
    /// Wine FSR is switched off for this game (AI Graphics), kept in its
    /// settings; `env` already carries the switch.
    pub wine_fsr_off: bool,
    /// A `WINE_FULLSCREEN_FSR=1` of the user's own goes too: the user asked
    /// for Wine FSR off, with the button that says so.
    pub drop_their_wine_fsr_on: bool,
}

/// The switch that turns Wine FSR off for one game, whatever the session
/// environment says.
const WINE_FSR_OFF: &str = "WINE_FULLSCREEN_FSR=0";

/// Put the Gamescope wrapper and the variables for the game whose process
/// is `process` into its Steam launch options, or take Big Game Mode's out
/// (`None`). Every account's new options are worked out before any is
/// written.
///
/// # Errors
/// Returns an error when Steam's configuration cannot be written or read
/// back, when the game's settings cannot be read or saved, or when the
/// Flatpak Steam has no Gamescope for a wrapper that needs one.
pub fn apply(process: &str, wanted: Wanted) -> Result<Applied> {
    let apps = steam_apps(process);
    if apps.is_empty() {
        return Ok(Applied::NotSteam);
    }
    let mut settings = crate::game_settings::load(process)?;
    // Older versions wrote Wine FSR's switch apart, with no record of it in
    // `steam_env`: it goes out once, and comes back under the record if it
    // is still wanted.
    let recorded = settings.steam_env.clone().unwrap_or_default();
    let legacy_off =
        settings.steam_wine_fsr_off && !option_words(&recorded).contains(&WINE_FSR_OFF);
    let previous_env = if legacy_off {
        Some(format!("{recorded} {WINE_FSR_OFF}").trim().to_owned())
    } else {
        settings.steam_env.clone()
    };
    if !legacy_off
        && !wanted.drop_their_wine_fsr_on
        && settings.steam_gamescope == wanted.gamescope
        && settings.steam_env == wanted.env
        && settings.steam_wine_fsr_off == wanted.wine_fsr_off
    {
        return Ok(Applied::Unchanged);
    }
    if crate::steam::is_running() {
        return Ok(Applied::SteamRunning);
    }
    let users = crate::steam::users(&crate::paths::home_dir());
    if wanted.gamescope.is_some() && crate::steam::any_flatpak(&users) {
        if let Some(command) = crate::steam::flatpak_gamescope_missing() {
            anyhow::bail!(UserError::with(
                N_(
                    "Steam's Flatpak finds Gamescope only in Flathub's Gamescope extension, which is not installed, and this game would not start with it. Nothing was written. Install it and restart Steam: %s"
                ),
                [command]
            ));
        }
    }
    let mut changes = Vec::new();
    let (mut segment_written, mut theirs) = (false, false);
    let mut last = String::new();
    for user in &users {
        for app in &apps {
            let current = crate::steam::launch_options(&user.config, app).unwrap_or_default();
            let mut with_env =
                env_options(&current, previous_env.as_deref(), wanted.env.as_deref());
            if wanted.drop_their_wine_fsr_on {
                let mut words = option_words(&with_env);
                words.retain(|w| *w != "WINE_FULLSCREEN_FSR=1");
                with_env = joined(&words);
            }
            let next = launch_options(
                &with_env,
                settings.steam_gamescope.as_deref(),
                wanted.gamescope.as_deref(),
            );
            if let Some(seg) = wanted.gamescope.as_deref() {
                if option_words(&next)
                    .windows(option_words(seg).len())
                    .any(|w| w == option_words(seg))
                {
                    segment_written = true;
                } else {
                    theirs = true;
                }
            }
            if next != current {
                changes.push((user.config.clone(), app.clone(), next.clone()));
            }
            last = next;
        }
    }
    for (config, app, next) in &changes {
        crate::steam::set_launch_options(config, app, next)?;
    }
    settings.steam_gamescope = wanted.gamescope.filter(|_| segment_written);
    settings.steam_env = wanted.env;
    settings.steam_wine_fsr_off = wanted.wine_fsr_off;
    crate::game_settings::save(process, &settings)?;
    Ok(if theirs {
        Applied::TheirGamescope(last)
    } else {
        Applied::Written(last)
    })
}

// ── Wine FSR off for one game ───────────────────────────────────────────────

/// Whether the game's Steam launch options switch Wine FSR on themselves.
#[must_use]
pub fn wine_fsr_in_options(process: &str) -> bool {
    steam_apps(process).iter().any(|app| {
        crate::steam::users(&crate::paths::home_dir())
            .iter()
            .filter_map(|u| crate::steam::launch_options(&u.config, app))
            .any(|o| option_words(&o).contains(&"WINE_FULLSCREEN_FSR=1"))
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
/// launch options (`off`, which also takes out a `WINE_FULLSCREEN_FSR=1` the
/// user typed), or take Big Game Mode's switch out again. The switch is
/// written by the one owner of the game's variables
/// ([`crate::game_launch::GameLaunch::steam_env`]), so it stays wherever
/// Gamescope or `OptiScaler` still upscales the game.
///
/// # Errors
/// As [`apply`].
pub fn set_wine_fsr_off(process: &str, off: bool) -> Result<Applied> {
    if steam_apps(process).is_empty() {
        return Ok(Applied::NotSteam);
    }
    let mut wanted = crate::optimization::steam_wanted(process, Some(off));
    wanted.drop_their_wine_fsr_on = off;
    apply(process, wanted)
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
        let moved = on.replacen("WINE_FULLSCREEN_FSR=0 ", "", 1);
        assert_eq!(env_options(&moved, Some(env), None), mine);
        // Written already (a write that stopped half-way): not twice.
        assert_eq!(env_options(&on, None, Some(env)), on);
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
            crate::mangohud::launch_options(&both, Some("mangohud"), crate::mangohud::Mode::Off).0,
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

    #[test]
    fn a_gamescope_the_user_typed_gets_none_of_bigame_modes_inside_it() {
        let theirs = "gamescope -W 3440 -H 1440 -f -- %command%";
        assert_eq!(launch_options(theirs, None, Some(SEG)), theirs);
        let path = "/usr/bin/gamescope -f -- mangohud %command% -dx12";
        assert_eq!(launch_options(path, None, Some(SEG)), path);
        // A Gamescope only in the game's arguments is not a wrapper.
        assert_eq!(
            launch_options("%command% -gamescope", None, Some(SEG)),
            format!("{SEG} %command% -gamescope")
        );
    }

    #[test]
    fn the_segment_is_never_added_twice() {
        // An account written before the next one failed keeps its segment
        // with no record of it: the next save must not nest a second one.
        let once = launch_options("PROTON_LOG=1 %command%", None, Some(SEG));
        assert_eq!(launch_options(&once, None, Some(SEG)), once);
        let twice = format!("{SEG} {SEG} %command%");
        assert_eq!(
            launch_options(&twice, None, Some(SEG)),
            format!("{SEG} %command%")
        );
        assert_eq!(
            launch_options(&once, None, None),
            once,
            "not ours to remove"
        );
    }

    #[test]
    fn a_quoted_command_is_the_command() {
        let mine = "gamemoderun \"%command%\" -name 'A  B'";
        assert_eq!(
            env_options(mine, None, Some("ENABLE_VKBASALT=1")),
            format!("ENABLE_VKBASALT=1 {mine}")
        );
        assert_eq!(
            launch_options(mine, None, Some(SEG)),
            format!("gamemoderun {SEG} \"%command%\" -name 'A  B'")
        );
        // And taken out again, the user's spacing inside quotes intact.
        let on = launch_options(mine, None, Some(SEG));
        assert_eq!(launch_options(&on, Some(SEG), None), mine);
    }

    #[test]
    fn vkbasalt_is_kept_out_of_steams_gamescope() {
        let seg = keeping_vkbasalt_in_the_game(SEG);
        assert_eq!(
            seg,
            format!("env DISABLE_VKBASALT=1 {SEG} env -u DISABLE_VKBASALT ENABLE_VKBASALT=1")
        );
        // After another wrapper, `env` keeps the variable a variable.
        let on = launch_options("gamemoderun %command%", None, Some(&seg));
        assert_eq!(on, format!("gamemoderun {seg} %command%"));
        assert_eq!(
            launch_options(&on, Some(&seg), None),
            "gamemoderun %command%"
        );
    }
}
