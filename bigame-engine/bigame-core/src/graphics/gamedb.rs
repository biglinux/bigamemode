//! Facts about particular games that detection cannot read from their files.
//!
//! Detection is the main source and stays so: this list only adds to it. It
//! is short on purpose — an entry is there only with evidence behind it —
//! and carried inside the program, so AI Graphics never depends on a file
//! being installed. People add or override entries in
//! `$XDG_CONFIG_HOME/bigame-mode/graphics-games.toml` (same format); an entry
//! there replaces the carried one for the same game.
//!
//! What an entry can do is limited to what is safe to take on trust: name
//! the API a game renders with by default (a running game still says
//! otherwise), prefer the game's own upscaler, `OptiScaler` or nothing in
//! Recommended mode, record the `OptiScaler` version it was tested with, and
//! block injection. Nothing in it can *unblock* a game: anti-cheat is decided
//! by the game's files alone.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::optiscaler::Api;

/// The list carried in the program.
const CARRIED: &str = include_str!("../../data/graphics-games.toml");

/// What Recommended should use for a game.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Prefer {
    /// The game's own upscaler.
    Native,
    /// `OptiScaler`, where the game has an upscaler for it to take over.
    #[serde(rename = "optiscaler")]
    OptiScaler,
    /// Nothing: AI Graphics brings nothing to this game.
    Nothing,
}

/// Where an entry came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Origin {
    /// Carried in the program.
    #[default]
    Carried,
    /// The user's own list.
    User,
}

/// One game.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    /// Steam app id.
    #[serde(default)]
    pub steam_app_id: Option<String>,
    /// Executable name, for games outside Steam.
    #[serde(default)]
    pub exe: Option<String>,
    /// The API it renders with by default.
    #[serde(default)]
    pub api: Option<Api>,
    /// What Recommended uses.
    #[serde(default)]
    pub prefer: Option<Prefer>,
    /// Never inject into this game, for this reason.
    #[serde(default)]
    pub block: Option<String>,
    /// The `OptiScaler` version it was tested with.
    #[serde(default)]
    pub tested_optiscaler: Option<String>,
    /// Which list it came from (set on loading, not written in the file).
    #[serde(skip)]
    pub origin: Origin,
}

impl Entry {
    fn matches(&self, app_id: Option<&str>, exe: &str) -> bool {
        match (&self.steam_app_id, app_id, &self.exe) {
            (Some(a), Some(b), _) => a == b,
            (_, _, Some(e)) => e.eq_ignore_ascii_case(exe),
            _ => false,
        }
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    #[serde(default)]
    game: Vec<Entry>,
}

/// Both lists, the user's first.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GameDb {
    /// Entries, the user's before the carried ones.
    pub entries: Vec<Entry>,
}

fn parse(text: &str, origin: Origin) -> Result<Vec<Entry>, toml::de::Error> {
    let f: File = toml::from_str(text)?;
    Ok(f.game
        .into_iter()
        .filter(|e| e.steam_app_id.is_some() || e.exe.is_some())
        .map(|mut e| {
            e.origin = origin;
            e
        })
        .collect())
}

/// `$XDG_CONFIG_HOME/bigame-mode/graphics-games.toml`.
#[must_use]
pub fn user_path() -> PathBuf {
    crate::game_settings::dir()
        .parent()
        .map_or_else(crate::game_settings::dir, std::path::Path::to_path_buf)
        .join("graphics-games.toml")
}

impl GameDb {
    /// The carried list and `user` (the text of the user's list, if any). A
    /// user list that does not parse is ignored with a warning — a typo in
    /// it must not take the carried entries or a plan away.
    #[must_use]
    pub fn from_texts(user: Option<&str>) -> Self {
        let mut entries = user
            .and_then(|t| match parse(t, Origin::User) {
                Ok(v) => Some(v),
                Err(e) => {
                    tracing::warn!(target: "graphics", error = %e, "the user's graphics game list does not parse; ignored");
                    None
                }
            })
            .unwrap_or_default();
        entries.extend(parse(CARRIED, Origin::Carried).unwrap_or_default());
        Self { entries }
    }

    /// Both lists as they are on this machine.
    #[must_use]
    pub fn load() -> Self {
        Self::from_texts(std::fs::read_to_string(user_path()).ok().as_deref())
    }

    /// The entry for a game — the user's, when both lists have one.
    #[must_use]
    pub fn lookup(&self, app_id: Option<&str>, exe: &str) -> Option<&Entry> {
        self.entries.iter().find(|e| e.matches(app_id, exe))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_carried_list_parses_and_is_short() {
        let carried = parse(CARRIED, Origin::Carried).unwrap();
        assert!(!carried.is_empty() && carried.len() < 50);
        let db = GameDb::from_texts(None);
        let sottr = db.lookup(Some("750920"), "SOTTR.exe").unwrap();
        assert_eq!(sottr.api, Some(Api::Dx12));
        assert_eq!(sottr.tested_optiscaler.as_deref(), Some("0.9.4"));
        assert_eq!(sottr.origin, Origin::Carried);
        assert!(db.lookup(Some("1"), "x.exe").is_none());
    }

    #[test]
    fn the_users_entry_wins_and_non_steam_games_match_by_executable() {
        let user = r#"
[[game]]
steam_app_id = "750920"
prefer = "native"

[[game]]
exe = "Game.exe"
block = "crashes with any proxy DLL"
"#;
        let db = GameDb::from_texts(Some(user));
        let sottr = db.lookup(Some("750920"), "SOTTR.exe").unwrap();
        assert_eq!(
            (sottr.prefer, sottr.origin),
            (Some(Prefer::Native), Origin::User)
        );
        let g = db.lookup(None, "game.EXE").unwrap();
        assert_eq!(g.block.as_deref(), Some("crashes with any proxy DLL"));
    }

    #[test]
    fn a_broken_user_list_costs_only_itself() {
        for bad in [
            "[[game]]\nsteam_app_id = 750920\n",
            "[[game]]\nsteam_app_id = \"1\"\nunblock_anticheat = true\n",
            "not toml",
        ] {
            let db = GameDb::from_texts(Some(bad));
            assert!(
                db.entries.iter().all(|e| e.origin == Origin::Carried),
                "{bad}"
            );
            assert!(db.lookup(Some("750920"), "SOTTR.exe").is_some());
        }
        // An entry that names no game is dropped.
        let db = GameDb::from_texts(Some("[[game]]\nprefer = \"nothing\"\n"));
        assert!(db.entries.iter().all(|e| e.origin == Origin::Carried));
    }
}
