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
//! Big Game Mode owns only the segment and the variables it wrote (the
//! segment kept in the game's settings, the variables by account in its
//! state directory); the user's own options are left alone, a variable
//! they set already included. Where they run
//! a Gamescope of the user's own, Big Game Mode's is not added: two nested
//! compositors would scale twice, or not start. vkBasalt, which Gamescope
//! would otherwise load for itself, stays in the game (as Big Game Mode's own
//! launch keeps it, `crate::launcher`). The Flatpak Steam finds `gamescope`
//! only in a Flatpak extension: without it nothing is written, and the
//! error names the command that installs it.

use anyhow::Result;

use crate::error::UserError;
use crate::gamescope::{Config, Mode};
use crate::steam::{COMMAND, Inserted, is_command, option_words};
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

/// Whether `word` sets a variable (`NAME=value`).
fn is_assignment(word: &str) -> bool {
    word.split_once('=').is_some_and(|(name, _)| {
        name.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
            && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    })
}

/// `current` launch options with the variables Big Game Mode wrote before
/// (`previous`) taken out and `wanted` put in front of everything, and the
/// variables it put in, to record. Each variable is taken out once,
/// wherever it is, so one that something else moved or removed does not
/// keep the others in. A variable the options already set in front — the
/// user's own `ENABLE_VKBASALT=1` — is neither set twice nor recorded: it
/// is not Big Game Mode's to take out later.
#[must_use]
pub fn env_options(
    current: &str,
    previous: Option<&str>,
    wanted: Option<&str>,
) -> (String, String) {
    let mut words = option_words(current);
    for old in option_words(previous.unwrap_or_default()) {
        if let Some(at) = words.iter().position(|w| *w == old) {
            words.remove(at);
        }
    }
    let set: Vec<&str> = words
        .iter()
        .take_while(|w| is_assignment(w))
        .copied()
        .collect();
    let added: Vec<&str> = option_words(wanted.unwrap_or_default())
        .into_iter()
        .filter(|w| !set.contains(w))
        .collect();
    if added.is_empty() {
        return (joined(&words), String::new());
    }
    if !words.iter().any(|w| is_command(w)) {
        // Plain arguments: they follow the command.
        words.insert(0, COMMAND);
    }
    let options = added
        .iter()
        .copied()
        .chain(words)
        .collect::<Vec<_>>()
        .join(" ");
    (options, added.join(" "))
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
    if wanted.gamescope.is_some()
        && crate::steam::any_flatpak(&users)
        && let Some(command) = crate::steam::flatpak_gamescope_missing()
    {
        anyhow::bail!(UserError::with(
            N_(
                "Steam's Flatpak finds Gamescope only in Flathub's Gamescope extension, which is not installed, and this game would not start with it. Nothing was written. Install it and restart Steam: %s"
            ),
            [command]
        ));
    }
    let record = env_record_path();
    let mut records: std::collections::BTreeMap<String, Inserted> =
        crate::mangohud::read_record(&record)?;
    let accounts: Vec<std::path::PathBuf> = users.into_iter().map(|u| u.config).collect();
    let plan = plan(
        &accounts,
        &apps,
        &|config, app| crate::steam::launch_options(config, app),
        &Previous {
            gamescope: settings.steam_gamescope.as_deref(),
            env: records.get(process),
            legacy_env: previous_env.as_deref(),
        },
        &wanted,
    );
    crate::steam::set_all(&plan.changes)?;
    if records.get(process) != Some(&plan.env) {
        records.insert(process.to_owned(), plan.env);
        crate::mangohud::write_record(&record, &records)?;
    }
    settings.steam_gamescope = wanted.gamescope.filter(|_| plan.segment_written);
    settings.steam_env = wanted.env;
    settings.steam_wine_fsr_off = wanted.wine_fsr_off;
    crate::game_settings::save(process, &settings)?;
    Ok(if plan.theirs {
        Applied::TheirGamescope(plan.last)
    } else {
        Applied::Written(plan.last)
    })
}

/// Where the variables put into each account's launch options are
/// recorded, by game.
fn env_record_path() -> std::path::PathBuf {
    crate::paths::state_home().join("bigame-mode/steam/env-added.toml")
}

/// What Big Game Mode wrote into a game's launch options before.
struct Previous<'a> {
    /// Its Gamescope segment.
    gamescope: Option<&'a str>,
    /// The variables it put into each account, since 2.3.2.
    env: Option<&'a Inserted>,
    /// With no record by account, the variables an older version wrote,
    /// taken for every account's.
    legacy_env: Option<&'a str>,
}

/// Every account's new launch options, worked out before any is written.
struct Plan {
    changes: Vec<crate::steam::Change>,
    /// The variables put in, to record.
    env: Inserted,
    /// Big Game Mode's Gamescope is in some account's options.
    segment_written: bool,
    /// Some account runs a Gamescope of the user's own instead.
    theirs: bool,
    /// The last account's options.
    last: String,
}

fn plan(
    accounts: &[std::path::PathBuf],
    apps: &[String],
    read: &dyn Fn(&std::path::Path, &str) -> Option<String>,
    previous: &Previous,
    wanted: &Wanted,
) -> Plan {
    let mut plan = Plan {
        changes: Vec::new(),
        env: Inserted::new(),
        segment_written: false,
        theirs: false,
        last: String::new(),
    };
    for config in accounts {
        let account = config.to_string_lossy().into_owned();
        for app in apps {
            let current = read(config, app).unwrap_or_default();
            let previous_env = match previous.env {
                Some(by) => by
                    .get(&account)
                    .and_then(|a| a.get(app))
                    .map(String::as_str),
                None => previous.legacy_env,
            };
            let (mut with_env, mut added) =
                env_options(&current, previous_env, wanted.env.as_deref());
            if wanted.drop_their_wine_fsr_on {
                let drop = |w: &&str| *w != "WINE_FULLSCREEN_FSR=1";
                let mut words = option_words(&with_env);
                words.retain(drop);
                with_env = joined(&words);
                let mut mine = option_words(&added);
                mine.retain(drop);
                added = mine.join(" ");
            }
            if !added.is_empty() {
                plan.env
                    .entry(account.clone())
                    .or_default()
                    .insert(app.clone(), added);
            }
            let next = launch_options(&with_env, previous.gamescope, wanted.gamescope.as_deref());
            if let Some(seg) = wanted.gamescope.as_deref() {
                if option_words(&next)
                    .windows(option_words(seg).len())
                    .any(|w| w == option_words(seg))
                {
                    plan.segment_written = true;
                } else {
                    plan.theirs = true;
                }
            }
            if next != current {
                plan.changes.push(crate::steam::Change {
                    config: config.clone(),
                    app: app.clone(),
                    before: current,
                    after: next.clone(),
                });
            }
            plan.last = next;
        }
    }
    plan
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

/// The app ids of the Steam games whose process is `process`.
pub(crate) fn steam_apps(process: &str) -> Vec<String> {
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

    /// The options [`env_options`] gives, without what it put in.
    fn env_only(current: &str, previous: Option<&str>, wanted: Option<&str>) -> String {
        env_options(current, previous, wanted).0
    }

    #[test]
    fn a_variable_the_user_set_is_not_recorded_and_stays() {
        for mine in [
            "ENABLE_VKBASALT=1 %command%",
            "WINE_FULLSCREEN_FSR=0 %command% -dx12",
            "WINE_FULLSCREEN_FSR=1 WINE_FULLSCREEN_FSR_MODE=quality %command%",
        ] {
            let (on, added) = env_options(mine, None, Some(mine.split(" %").next().unwrap()));
            assert_eq!((on.as_str(), added.as_str()), (mine, ""), "{mine}");
        }
        // Only the missing one goes in, and only it is recorded.
        let (on, added) = env_options(
            "ENABLE_VKBASALT=1 %command%",
            None,
            Some("WINE_FULLSCREEN_FSR=0 ENABLE_VKBASALT=1"),
        );
        assert_eq!(on, "WINE_FULLSCREEN_FSR=0 ENABLE_VKBASALT=1 %command%");
        assert_eq!(added, "WINE_FULLSCREEN_FSR=0");
        assert_eq!(
            env_only(&on, Some(&added), None),
            "ENABLE_VKBASALT=1 %command%"
        );
        // After a wrapper it is no variable of the game's: ours goes in front.
        let (on, added) = env_options(
            "gamemoderun ENABLE_VKBASALT=1",
            None,
            Some("ENABLE_VKBASALT=1"),
        );
        assert_eq!(
            on,
            "ENABLE_VKBASALT=1 %command% gamemoderun ENABLE_VKBASALT=1"
        );
        assert_eq!(added, "ENABLE_VKBASALT=1");
        assert!(is_assignment("_A1=x") && !is_assignment("1A=x") && !is_assignment("-x=1"));
    }

    #[test]
    fn each_account_takes_out_only_the_variables_put_into_it() {
        // One account in two Steam installs: the user typed vkBasalt's
        // variable in the Flatpak's options themselves.
        let store = std::cell::RefCell::new(std::collections::HashMap::from([
            ("/native".to_owned(), "-novid".to_owned()),
            (
                "/flatpak".to_owned(),
                "ENABLE_VKBASALT=1 %command%".to_owned(),
            ),
        ]));
        let read = |c: &std::path::Path, _: &str| {
            store.borrow().get(c.to_string_lossy().as_ref()).cloned()
        };
        let accounts = [std::path::PathBuf::from("/native"), "/flatpak".into()];
        let apps = ["10".to_owned()];
        let run = |previous: &Previous, wanted: &Wanted| {
            let plan = plan(&accounts, &apps, &read, previous, wanted);
            for c in &plan.changes {
                store
                    .borrow_mut()
                    .insert(c.config.to_string_lossy().into_owned(), c.after.clone());
            }
            plan.env
        };
        let on = Wanted {
            env: Some("ENABLE_VKBASALT=1".into()),
            ..Wanted::default()
        };
        let none = Previous {
            gamescope: None,
            env: None,
            legacy_env: None,
        };
        let record = run(&none, &on);
        assert_eq!(
            store.borrow()["/native"],
            "ENABLE_VKBASALT=1 %command% -novid"
        );
        assert_eq!(store.borrow()["/flatpak"], "ENABLE_VKBASALT=1 %command%");
        assert_eq!(record.len(), 1);
        assert_eq!(record["/native"]["10"], "ENABLE_VKBASALT=1");

        let record = run(
            &Previous {
                env: Some(&record),
                ..none
            },
            &Wanted::default(),
        );
        assert_eq!(store.borrow()["/native"], "%command% -novid");
        assert_eq!(
            store.borrow()["/flatpak"],
            "ENABLE_VKBASALT=1 %command%",
            "the user's own stays"
        );
        assert!(record.is_empty());

        // An older version's record stands for every account, as it did.
        store
            .borrow_mut()
            .insert("/native".into(), "ENABLE_VKBASALT=1 %command%".into());
        run(
            &Previous {
                legacy_env: Some("ENABLE_VKBASALT=1"),
                ..none
            },
            &Wanted::default(),
        );
        assert_eq!(store.borrow()["/native"], "");
        assert_eq!(store.borrow()["/flatpak"], "");
    }

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
        assert_eq!(env_only("", None, Some(env)), format!("{env} %command%"));
        let mine = "MANGOHUD=1 gamemoderun %command% -dx12";
        let on = env_only(mine, None, Some(env));
        assert_eq!(on, format!("{env} {mine}"));
        assert_eq!(env_only(&on, Some(env), None), mine);
        // Replaced, not added twice.
        assert_eq!(
            env_only(&on, Some(env), Some("ENABLE_VKBASALT=0")),
            format!("ENABLE_VKBASALT=0 {mine}")
        );
        assert_eq!(env_only(&format!("{env} %command%"), Some(env), None), "");
        // One variable removed by something else: the other still goes.
        let moved = on.replacen("WINE_FULLSCREEN_FSR=0 ", "", 1);
        assert_eq!(env_only(&moved, Some(env), None), mine);
        // Written already (a write that stopped half-way): not twice.
        assert_eq!(env_only(&on, None, Some(env)), on);
        // With the Gamescope wrapper, each in its place.
        let both = launch_options(&env_only(mine, None, Some(env)), None, Some(SEG));
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
            env_only(mine, None, Some("ENABLE_VKBASALT=1")),
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
