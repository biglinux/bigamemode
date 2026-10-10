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
//! Proton-tkg's `amdxc64.dll` reads the same `FSR4_UPGRADE` (checked in its
//! build 10.0-284605); Proton-CachyOS, a build of Valve's Proton, is
//! expected to, and has not been checked.
//!
//! A game the Steam client starts gets its environment from its launch
//! options, so that is where the variable goes: written with Steam closed,
//! backed up and read back ([`crate::steam::set_launch_options`]), and only
//! what this module recorded adding is ever removed ([`Added`]). A game Heroic starts gets it
//! in its settings there (`crate::heroic_launch`), both names, since Heroic
//! runs games with GE-Proton as often as with Valve's Proton: written with
//! Heroic closed, and only what Big Game Mode wrote is ever removed. It is
//! never applied by itself: the plan names it, Apply writes it.
//!
//! Verified, never assumed: the running game's environment holds the
//! variable, and it has the provider mapped ([`super::runtime::NativeRuntime`]).

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::steam::{COMMAND, is_command, option_words};

/// The variable Valve's Proton and Proton-EM read.
pub const VARIABLE: &str = "FSR4_UPGRADE=1";
/// The variable GE-Proton reads (it also fetches the provider itself).
pub const GE_VARIABLE: &str = "PROTON_FSR4_UPGRADE=1";

/// What Big Game Mode put into one Steam account's launch options for a
/// game, so that only that is taken out again.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Added {
    /// The account: its `localconfig.vdf`. A record from before 2.3.2 holds
    /// its `userdata` id, which stands for that id's account in every Steam
    /// install (the native and the Flatpak each have their own).
    pub account: String,
    /// `%command%` went in with the variable: the options had none.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub command: bool,
}

/// Whether `options` (Steam launch options) switch the upgrade on, for
/// either Proton flavour.
#[must_use]
pub fn enabled_in(options: &str) -> bool {
    option_words(options)
        .iter()
        .any(|w| *w == VARIABLE || *w == GE_VARIABLE)
}

/// `current` with [`VARIABLE`] in front, and `%command%` after it when there
/// was none (arguments without it keep following it), and whether
/// `%command%` was added; `None` when either spelling is there already.
#[must_use]
pub fn with_upgrade(current: &str) -> Option<(String, bool)> {
    if enabled_in(current) {
        return None;
    }
    let mut words = option_words(current);
    let command = !words.iter().any(|w| is_command(w));
    if command {
        words.insert(0, COMMAND);
    }
    words.insert(0, VARIABLE);
    Some((words.join(" "), command))
}

/// `current` without the [`VARIABLE`] this module put in, and without the
/// `%command%` it put in with it (`command`) while that still leads the
/// options: once the user put something in front of it, it is theirs.
/// GE-Proton's spelling is never taken out — only the user writes it.
#[must_use]
pub fn without_upgrade(current: &str, command: bool) -> String {
    let mut words = option_words(current);
    if let Some(at) = words.iter().position(|w| *w == VARIABLE) {
        words.remove(at);
    }
    if command && words.first() == Some(&COMMAND) {
        words.remove(0);
    }
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
    let accounts: Vec<(String, std::path::PathBuf)> =
        crate::steam::users(&crate::paths::home_dir())
            .into_iter()
            .map(|u| (u.id, u.config))
            .collect();
    let mut settings = crate::game_settings::load(process)?;
    let (last, record) = apply_to_accounts(
        &accounts,
        on,
        settings.steam_fsr4_upgrade.as_deref(),
        &|config| crate::steam::launch_options(config, app),
        &|config, value| crate::steam::set_launch_options(config, app, value),
    )?;
    if record != settings.steam_fsr4_upgrade {
        settings.steam_fsr4_upgrade = record;
        crate::game_settings::save(process, &settings)?;
    }
    // The rest of the user's launch options stays out of the journal, which
    // Logs and the support report export.
    tracing::info!(target: "graphics", app, on, "FSR 4 upgrade launch option written");
    Ok(Applied::SteamLaunchOptions(last))
}

impl Added {
    /// Whether this record is of the account `id` whose configuration is
    /// `config`.
    fn names(&self, id: &str, config: &std::path::Path) -> bool {
        self.account == id || std::path::Path::new(&self.account) == config
    }
}

/// The launch option switched in every account it belongs in, as one
/// change: every account's new options are worked out before any is
/// written, and an account that cannot be written puts back the ones
/// written before it. Returns the last account's options and what to record
/// of what was added.
///
/// On goes to the accounts that have launch options for the game — the ones
/// that play it — or, when none has any yet, to every account (Steam keeps
/// nothing for a game played with its defaults, and which account plays it
/// cannot be told). Off takes out only what `added` records, from the
/// accounts it names; with nothing recorded (a game set up before this was
/// kept), the variable this module writes is taken out wherever it is.
fn apply_to_accounts(
    accounts: &[(String, std::path::PathBuf)],
    on: bool,
    added: Option<&[Added]>,
    read: &dyn Fn(&std::path::Path) -> Option<String>,
    write: &dyn Fn(&std::path::Path, &str) -> Result<()>,
) -> Result<(String, Option<Vec<Added>>)> {
    let playing: Vec<&(String, std::path::PathBuf)> =
        accounts.iter().filter(|(_, c)| read(c).is_some()).collect();
    let targets: Vec<&(String, std::path::PathBuf)> = match (on, added) {
        (true, _) if playing.is_empty() => accounts.iter().collect(),
        (true, _) => playing,
        (false, None) => accounts.iter().collect(),
        (false, Some(list)) => accounts
            .iter()
            .filter(|(id, config)| list.iter().any(|a| a.names(id, config)))
            .collect(),
    };
    // Every account's new options first.
    let mut changes: Vec<(&std::path::Path, String, String)> = Vec::new();
    let mut now_added: Vec<Added> = Vec::new();
    let mut last = String::new();
    for (id, config) in targets {
        let current = read(config).unwrap_or_default();
        let wanted = if on {
            match with_upgrade(&current) {
                Some((options, command)) => {
                    now_added.push(Added {
                        account: config.to_string_lossy().into_owned(),
                        command,
                    });
                    options
                }
                None => current.clone(),
            }
        } else {
            let command =
                added.is_none_or(|list| list.iter().any(|a| a.names(id, config) && a.command));
            without_upgrade(&current, command)
        };
        if wanted != current {
            changes.push((config, current, wanted.clone()));
        }
        last = wanted;
    }
    let mut written: Vec<(&std::path::Path, &str)> = Vec::new();
    for (config, before, wanted) in &changes {
        if let Err(e) = write(config, wanted) {
            for (done, before) in written.iter().rev() {
                if let Err(undo) = write(done, before) {
                    tracing::warn!(target: "graphics", error = %format!("{undo:#}"),
                        "a Steam account's launch options could not be put back");
                }
            }
            return Err(e);
        }
        written.push((config, before));
    }
    let record = if on {
        if now_added.is_empty() {
            added.map(<[Added]>::to_vec)
        } else {
            let mut all: Vec<Added> = added
                .unwrap_or_default()
                .iter()
                .filter(|a| !now_added.iter().any(|n| n.account == a.account))
                .cloned()
                .collect();
            all.extend(now_added);
            all.sort_by(|x, y| x.account.cmp(&y.account));
            Some(all)
        }
    } else {
        Some(Vec::new())
    };
    Ok((last, record))
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
    Some(upgrade_in(&env))
}

/// Whether an environment block (`/proc/<pid>/environ`) asks Proton for
/// the upgrade: either name set to a value other than empty or `0`.
fn upgrade_in(environ: &[u8]) -> bool {
    environ.split(|b| *b == 0).any(|e| {
        [b"FSR4_UPGRADE=".as_slice(), b"PROTON_FSR4_UPGRADE="]
            .iter()
            .find_map(|name| e.strip_prefix(*name))
            .is_some_and(|v| !v.is_empty() && v != b"0")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_variable_set_to_zero_is_no_upgrade() {
        assert!(upgrade_in(b"A=1\0FSR4_UPGRADE=1\0"));
        assert!(upgrade_in(b"PROTON_FSR4_UPGRADE=1\0"));
        assert!(!upgrade_in(b"PROTON_FSR4_UPGRADE=0\0FSR4_UPGRADE=\0"));
        assert!(!upgrade_in(b"NOT_FSR4_UPGRADE=1\0"));
    }

    #[test]
    fn the_variable_goes_in_front_and_comes_out_again_leaving_the_rest() {
        assert_eq!(
            with_upgrade(""),
            Some(("FSR4_UPGRADE=1 %command%".to_owned(), true))
        );
        let mine = "MANGOHUD=1 gamemoderun %command% -dx12";
        let (on, command) = with_upgrade(mine).unwrap();
        assert_eq!(on, "FSR4_UPGRADE=1 MANGOHUD=1 gamemoderun %command% -dx12");
        assert!(!command, "the user's own %command% stays the command");
        assert!(enabled_in(&on));
        assert_eq!(without_upgrade(&on, command), mine);
        // Plain arguments follow the command, which goes again with it.
        let (on, command) = with_upgrade("-skipintro").unwrap();
        assert_eq!(on, "FSR4_UPGRADE=1 %command% -skipintro");
        assert_eq!(without_upgrade(&on, command), "-skipintro");
        // Quoted words are words: a quoted command is the command, and a
        // quoted path with spaces stays one word.
        let quoted = r#"WINEDLLOVERRIDES="dxgi=n,b" "%command%" -path "C:\My Games""#;
        let (on, command) = with_upgrade(quoted).unwrap();
        assert!(!command);
        assert_eq!(on, format!("FSR4_UPGRADE=1 {quoted}"));
        assert_eq!(without_upgrade(&on, command), quoted);
        // Something the user put in front of the command since makes it
        // theirs.
        assert_eq!(
            without_upgrade("FSR4_UPGRADE=1 gamemoderun %command% -x", true),
            "gamemoderun %command% -x"
        );
        // GE-Proton's spelling counts as on; it is the user's, never removed.
        assert!(enabled_in("PROTON_FSR4_UPGRADE=1 %command%"));
        assert_eq!(with_upgrade("PROTON_FSR4_UPGRADE=1 %command%"), None);
        assert_eq!(
            without_upgrade("PROTON_FSR4_UPGRADE=1 %command%", true),
            "PROTON_FSR4_UPGRADE=1 %command%"
        );
        assert!(!enabled_in("FSR4_UPGRADE=0 %command%"));
        assert_eq!(with_upgrade(&on), None, "applied twice is applied once");
    }

    /// Launch options by account, as Steam's files would hold them.
    struct Accounts(std::cell::RefCell<std::collections::HashMap<std::path::PathBuf, String>>);

    impl Accounts {
        fn new(with: &[(&str, &str)]) -> Self {
            Self(std::cell::RefCell::new(
                with.iter()
                    .map(|(c, o)| (std::path::PathBuf::from(c), (*o).to_owned()))
                    .collect(),
            ))
        }
        fn get(&self, c: &str) -> Option<String> {
            self.0.borrow().get(std::path::Path::new(c)).cloned()
        }
    }

    fn ids(list: &[&str]) -> Vec<(String, std::path::PathBuf)> {
        list.iter()
            .map(|c| ((*c).to_owned(), (*c).into()))
            .collect()
    }

    #[test]
    fn the_option_goes_only_where_the_game_is_played_and_leaves_only_from_there() {
        // Account "a" plays the game; "b" has never set anything for it.
        let store = Accounts::new(&[("a", "-novid")]);
        let read = |c: &std::path::Path| store.0.borrow().get(c).cloned();
        let write = |c: &std::path::Path, v: &str| {
            store.0.borrow_mut().insert(c.to_path_buf(), v.to_owned());
            Ok(())
        };
        let (_, record) = apply_to_accounts(&ids(&["a", "b"]), true, None, &read, &write).unwrap();
        assert_eq!(
            store.get("a").as_deref(),
            Some("FSR4_UPGRADE=1 %command% -novid")
        );
        assert_eq!(store.get("b"), None, "not an account that plays it");
        assert_eq!(
            record,
            Some(vec![Added {
                account: "a".into(),
                command: true
            }])
        );
        // Off: out of "a" again, as it was.
        let (_, record) =
            apply_to_accounts(&ids(&["a", "b"]), false, record.as_deref(), &read, &write).unwrap();
        assert_eq!(store.get("a").as_deref(), Some("-novid"));
        assert_eq!(record, Some(Vec::new()));

        // The user's own variable in "b": on adds nothing there, and off
        // leaves it.
        let store = Accounts::new(&[("a", ""), ("b", "FSR4_UPGRADE=1 %command%")]);
        let read = |c: &std::path::Path| store.0.borrow().get(c).cloned();
        let write = |c: &std::path::Path, v: &str| {
            store.0.borrow_mut().insert(c.to_path_buf(), v.to_owned());
            Ok(())
        };
        let (_, record) =
            apply_to_accounts(&ids(&["a", "b"]), true, Some(&[]), &read, &write).unwrap();
        assert_eq!(record.as_ref().map(Vec::len), Some(1));
        apply_to_accounts(&ids(&["a", "b"]), false, record.as_deref(), &read, &write).unwrap();
        assert_eq!(store.get("a").as_deref(), Some(""));
        assert_eq!(store.get("b").as_deref(), Some("FSR4_UPGRADE=1 %command%"));
    }

    #[test]
    fn one_account_in_two_steam_installs_is_recorded_by_its_file() {
        // The same account id in the native Steam and in the Flatpak; the
        // user typed the variable in the Flatpak's options themselves.
        let store = Accounts::new(&[("/native", "-x"), ("/flatpak", "FSR4_UPGRADE=1 %command%")]);
        let read = |c: &std::path::Path| store.0.borrow().get(c).cloned();
        let write = |c: &std::path::Path, v: &str| {
            store.0.borrow_mut().insert(c.to_path_buf(), v.to_owned());
            Ok(())
        };
        let accounts = vec![
            ("1234".to_owned(), std::path::PathBuf::from("/native")),
            ("1234".to_owned(), std::path::PathBuf::from("/flatpak")),
        ];
        let (_, record) = apply_to_accounts(&accounts, true, None, &read, &write).unwrap();
        assert_eq!(
            record,
            Some(vec![Added {
                account: "/native".into(),
                command: true
            }])
        );
        apply_to_accounts(&accounts, false, record.as_deref(), &read, &write).unwrap();
        assert_eq!(store.get("/native").as_deref(), Some("-x"));
        assert_eq!(
            store.get("/flatpak").as_deref(),
            Some("FSR4_UPGRADE=1 %command%"),
            "the user's own, in the other install, stays"
        );

        // A record from before names the id: both installs' accounts.
        write(
            std::path::Path::new("/native"),
            "FSR4_UPGRADE=1 %command% -x",
        )
        .unwrap();
        let old = [Added {
            account: "1234".into(),
            command: false,
        }];
        apply_to_accounts(&accounts, false, Some(&old), &read, &write).unwrap();
        assert_eq!(store.get("/native").as_deref(), Some("%command% -x"));
        assert_eq!(store.get("/flatpak").as_deref(), Some("%command%"));
    }

    #[test]
    fn an_account_that_cannot_be_written_puts_back_the_ones_before_it() {
        let store = Accounts::new(&[("a", "-x"), ("b", "-y")]);
        let read = |c: &std::path::Path| store.0.borrow().get(c).cloned();
        let write = |c: &std::path::Path, v: &str| {
            anyhow::ensure!(
                c != std::path::Path::new("b") || !v.contains(VARIABLE),
                "read-only"
            );
            store.0.borrow_mut().insert(c.to_path_buf(), v.to_owned());
            Ok(())
        };
        assert!(apply_to_accounts(&ids(&["a", "b"]), true, None, &read, &write).is_err());
        assert_eq!(store.get("a").as_deref(), Some("-x"), "put back");
        assert_eq!(store.get("b").as_deref(), Some("-y"));
    }
}
