//! Whether a game's anti-cheat lets it be played on Linux: the community
//! list of Are We Anti-Cheat Yet? (MIT), one file for every game.
//!
//! `https://raw.githubusercontent.com/AreWeAntiCheatYet/AreWeAntiCheatYet/master/games.json`,
//! checked on 2026-10-19: 1167 games in 470 KB, an `ETag`, a five-minute
//! cache. Each entry has a name, a slug, a status (`Supported`, `Running`,
//! `Planned`, `Broken`, `Denied`), the anti-cheats, notes and updates with
//! their references, `storeIds.steam` (a string) for 682 of them, and the
//! date it changed. The whole file is fetched — never one request per game
//! — when it is older than [`super::ANTICHEAT_TTL`], only if it changed
//! (`If-None-Match`), checked before it replaces the saved one, and indexed
//! once by Steam `AppID`.
//!
//! Five Steam ids are listed twice (a renamed game, a remaster, or a
//! mistake: Final Fantasy XIV under Predecessor's id). Such an id is matched
//! by the game's exact title, and failing that the newest entry is shown as
//! a match the list itself makes doubtful. A game with no Steam id is
//! matched by its exact title only when one entry has it; never by a
//! resemblance.

use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::http::{self, Failure};
use crate::text::N_;

/// The list's address.
pub const DATASET_URL: &str =
    "https://raw.githubusercontent.com/AreWeAntiCheatYet/AreWeAntiCheatYet/master/games.json";

/// Many times the real file: a bigger one is not read.
pub const MAX_DATASET: u64 = 8 * 1024 * 1024;

/// More entries than the list will have for years.
const MAX_GAMES: usize = 50_000;

/// The status the list gives a game.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    /// The anti-cheat is switched on for Linux by the game's maker.
    Supported,
    /// It runs, by players' reports, without the maker's support.
    Running,
    /// Support was announced.
    Planned,
    /// It does not run, or not well.
    Broken,
    /// The maker refuses Linux.
    Denied,
}

impl Status {
    /// The list's word.
    #[must_use]
    pub fn from_list(word: &str) -> Option<Self> {
        Some(match word.trim() {
            "Supported" => Self::Supported,
            "Running" => Self::Running,
            "Planned" => Self::Planned,
            "Broken" => Self::Broken,
            "Denied" => Self::Denied,
            _ => return None,
        })
    }

    /// Its name, as the list writes it: a proper name, like `ProtonDB`'s
    /// tiers, so what is shown is what the source says (its meaning is
    /// translated, [`Status::meaning`]).
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Supported => "Supported",
            Self::Running => "Running",
            Self::Planned => "Planned",
            Self::Broken => "Broken",
            Self::Denied => "Denied",
        }
    }

    /// What it means for the player, translatable. `Running` is not a
    /// promise: the anti-cheat may change, and an account may be at risk.
    #[must_use]
    pub fn meaning(self) -> &'static str {
        match self {
            Self::Supported => {
                N_("The game's maker switched its anti-cheat on for Linux and Proton.")
            }
            Self::Running => N_(
                "Players report it running, without the maker's support. It can stop working with an update, and it is no promise that an account is safe.",
            ),
            Self::Planned => N_("Linux support was announced, and is not there yet."),
            Self::Broken => {
                N_("The anti-cheat does not work on Linux, or not well enough to play online.")
            }
            Self::Denied => N_("The game's maker refuses Linux and Proton on purpose."),
        }
    }
}

/// One update in a game's history.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Update {
    /// What happened, as the list says it.
    pub name: String,
    /// When, as the list writes it.
    pub date: String,
    /// Where it was read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reference: Option<String>,
}

/// One game in the list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Game {
    /// Its name in the list.
    pub name: String,
    /// Its page on areweanticheatyet.com.
    pub slug: String,
    /// The status.
    pub status: Status,
    /// The anti-cheats it uses, as the list names them.
    pub anticheats: Vec<String>,
    /// Notes, each with the address it comes from when there is one.
    pub notes: Vec<(String, Option<String>)>,
    /// Its history, oldest first, as the list keeps it.
    pub updates: Vec<Update>,
    /// The source of the status, when the list gives one.
    pub reference: Option<String>,
    /// When the entry last changed, as the list writes it (ISO 8601).
    pub changed: Option<String>,
    /// Its Steam `AppID`.
    pub steam: Option<u32>,
}

impl Game {
    /// Its page on areweanticheatyet.com.
    #[must_use]
    pub fn page_url(&self) -> String {
        format!("https://areweanticheatyet.com/game/{}", self.slug)
    }
}

/// How a game was found in the list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Match {
    /// By its Steam `AppID`, which one entry has.
    SteamId,
    /// By its Steam `AppID` and its exact title, among entries sharing it.
    SteamIdAndTitle,
    /// By its Steam `AppID`, which several entries share and none by this
    /// title: the newest is shown, and said to be doubtful.
    SteamIdAmbiguous,
    /// By its exact title, which one entry has (no Steam id to go by).
    Title,
}

/// The list, indexed.
#[derive(Debug, Default)]
pub struct Dataset {
    games: Vec<Game>,
    by_steam: HashMap<u32, Vec<usize>>,
    by_title: HashMap<String, Vec<usize>>,
}

impl Dataset {
    /// How many games it lists.
    #[must_use]
    pub fn len(&self) -> usize {
        self.games.len()
    }

    /// Whether it lists none.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.games.is_empty()
    }

    /// The entry for a game: by its Steam `AppID` when it has one (only by
    /// it: a Steam game the list does not have under its id is not listed),
    /// else by its exact title when one entry has it.
    #[must_use]
    pub fn find(&self, steam: Option<u32>, title: &str) -> Option<(&Game, Match)> {
        let wanted = normalize(title);
        if let Some(id) = steam {
            let ids = self.by_steam.get(&id)?;
            if let [one] = ids.as_slice() {
                return Some((&self.games[*one], Match::SteamId));
            }
            if let Some(i) = ids
                .iter()
                .find(|i| normalize(&self.games[**i].name) == wanted)
            {
                return Some((&self.games[*i], Match::SteamIdAndTitle));
            }
            let newest = ids
                .iter()
                .max_by(|a, b| self.games[**a].changed.cmp(&self.games[**b].changed))?;
            return Some((&self.games[*newest], Match::SteamIdAmbiguous));
        }
        match self.by_title.get(&wanted)?.as_slice() {
            [one] => Some((&self.games[*one], Match::Title)),
            _ => None,
        }
    }
}

/// A title for comparison: lower case, letters and digits only, marks
/// such as ™ and ® gone. "Tom Clancy's Rainbow Six® Siege" and "tom clancys
/// rainbow six siege" are the same; "Siege X" is not.
#[must_use]
pub fn normalize(title: &str) -> String {
    title
        .chars()
        .filter(|c| c.is_alphanumeric() || c.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// The list from its JSON. An entry that is not one the list writes (no
/// name, slug or known status) is left out rather than guessed; text is cut
/// to a length no real entry has.
///
/// # Errors
/// Returns an error when the file is not a JSON array, or holds no usable
/// entry: it would not replace the list kept.
pub fn parse(bytes: &[u8]) -> Result<Dataset, String> {
    let v: serde_json::Value = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    let entries = v.as_array().ok_or("not a list")?;
    let mut set = Dataset::default();
    for e in entries.iter().take(MAX_GAMES) {
        let Some(game) = entry(e) else { continue };
        let i = set.games.len();
        if let Some(id) = game.steam {
            set.by_steam.entry(id).or_default().push(i);
        }
        set.by_title
            .entry(normalize(&game.name))
            .or_default()
            .push(i);
        set.games.push(game);
    }
    if set.games.is_empty() {
        return Err("no usable entry".into());
    }
    Ok(set)
}

fn text(v: &serde_json::Value, max: usize) -> Option<String> {
    let s = v.as_str()?.trim();
    (!s.is_empty()).then(|| s.chars().take(max).collect())
}

/// An address the list gives, kept only when it is one a browser may open.
fn link(v: &serde_json::Value) -> Option<String> {
    text(v, 2048).filter(|u| u.starts_with("https://") || u.starts_with("http://"))
}

fn entry(e: &serde_json::Value) -> Option<Game> {
    let name = text(&e["name"], 200)?;
    let slug = text(&e["slug"], 200)?;
    if !slug
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return None;
    }
    let status = Status::from_list(e["status"].as_str()?)?;
    let steam = e["storeIds"]["steam"]
        .as_str()
        .and_then(|s| s.trim().parse::<u32>().ok())
        .or_else(|| {
            e["storeIds"]["steam"]
                .as_u64()
                .and_then(|n| u32::try_from(n).ok())
        })
        .filter(|id| *id > 0);
    let list = |key: &str| e[key].as_array().map(Vec::as_slice).unwrap_or_default();
    Some(Game {
        name,
        slug,
        status,
        anticheats: list("anticheats")
            .iter()
            .filter_map(|a| text(a, 100))
            .take(16)
            .collect(),
        notes: list("notes")
            .iter()
            .filter_map(|n| Some((text(&n[0], 1000)?, link(&n[1]))))
            .take(32)
            .collect(),
        updates: list("updates")
            .iter()
            .filter_map(|u| {
                Some(Update {
                    name: text(&u["name"], 300)?,
                    date: text(&u["date"], 64).unwrap_or_default(),
                    reference: link(&u["reference"]),
                })
            })
            .take(64)
            .collect(),
        reference: link(&e["reference"]),
        changed: text(&e["dateChanged"], 64),
        steam,
    })
}

/// What refreshing the list did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fetched {
    /// A new list, checked and saved.
    Updated,
    /// The saved list is still the list.
    Unchanged,
    /// Asked to come back later.
    Later(Option<std::time::Duration>),
    /// The server failed, or sent what is not the list: the saved one stays.
    Failed(String),
}

/// Fetch the list into `file` if it changed since the one saved there
/// (its `ETag` beside it, in `etag`). A new file replaces the old one only
/// once it reads as a list.
///
/// # Errors
/// Returns why no answer came at all.
pub fn fetch(file: &Path, etag: &Path) -> Result<Fetched, Failure> {
    // Without the list, an ETag would only ever bring a 304.
    if !file.is_file() {
        let _ = std::fs::remove_file(etag);
    }
    let r = http::get(DATASET_URL, MAX_DATASET, Some(etag))?;
    Ok(match r.status {
        304 => Fetched::Unchanged,
        200 => match parse(&r.body) {
            Ok(_) => match super::write_bytes(file, &r.body) {
                Ok(()) => Fetched::Updated,
                Err(e) => Fetched::Failed(format!("{e:#}")),
            },
            Err(e) => {
                // The ETag of a list not kept would hide the next good one.
                let _ = std::fs::remove_file(etag);
                Fetched::Failed(format!("not the list: {e}"))
            }
        },
        429 | 503 => Fetched::Later(r.retry_after),
        other => Fetched::Failed(format!("HTTP {other}")),
    })
}

/// The list saved in `file`; empty when there is none or it does not read.
#[must_use]
pub fn load(file: &Path) -> Dataset {
    std::fs::metadata(file)
        .ok()
        .filter(|m| m.len() <= MAX_DATASET)
        .and_then(|_| std::fs::read(file).ok())
        .and_then(|b| parse(&b).ok())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Entries as the list wrote them on 2026-10-19.
    const LIST: &str = r#"[
 {"url":"","slug":"battlefield-6","name":"Battlefield 6","logo":"","native":false,"status":"Denied","reference":"","anticheats":["EA anticheat"],"updates":[],"notes":[],"storeIds":{"steam":"2807960"},"dateChanged":"2025-07-16T20:26:00.0000000-04:00"},
 {"url":"https://deadbydaylight.com/","name":"Dead By Daylight","logo":"","native":false,"status":"Supported","reference":"","anticheats":["Easy Anti-Cheat"],"notes":[["Enabled on Steam and Epic Games Store.","https://store.steampowered.com/news/app/381210/view/3723958226061651097"],["Epic Games Store is broken on Linux. Steam Recommended.",null]],"updates":[{"name":"Plans to support Steam Deck","date":"Nov 9, 2021, 5:55 PM EST","reference":"https://www.theverge.com/x"}],"storeIds":{"steam":"381210"},"slug":"dead-by-daylight","dateChanged":"2024-01-19T04:40:56.000Z"},
 {"url":"","slug":"post-scriptum","name":"Post Scriptum","logo":"","native":false,"status":"Denied","reference":"","anticheats":["Easy Anti-Cheat"],"updates":[],"notes":[],"storeIds":{"steam":"736220"},"dateChanged":"2024-01-19T04:40:56.000Z"},
 {"url":"","slug":"squad44","name":"Squad 44","logo":"","native":false,"status":"Running","reference":"","anticheats":["Easy Anti-Cheat"],"updates":[],"notes":[],"storeIds":{"steam":"736220"},"dateChanged":"2025-04-05T19:12:01.0000000-04:00"},
 {"url":"","slug":"fortnite","name":"Fortnite","logo":"","native":false,"status":"Denied","reference":"","anticheats":["Easy Anti-Cheat","BattlEye"],"updates":[],"notes":[],"storeIds":{"epic":{"namespace":"fn","slug":"fortnite"}},"dateChanged":"2024-01-19T04:40:56.000Z"},
 {"url":"","slug":"../etc","name":"Bad","status":"Denied","anticheats":[],"notes":[],"updates":[],"storeIds":{}},
 {"url":"","slug":"later","name":"Later","status":"Maybe","anticheats":[],"notes":[],"updates":[],"storeIds":{}}
]"#;

    #[test]
    fn a_steam_game_is_found_by_its_id_with_what_the_list_says() {
        let d = parse(LIST.as_bytes()).unwrap();
        assert_eq!(
            d.len(),
            5,
            "the bad slug and the unknown status are left out"
        );
        let (bf, how) = d.find(Some(2_807_960), "Battlefield™ 6").unwrap();
        assert_eq!((bf.status, how), (Status::Denied, Match::SteamId));
        assert_eq!(bf.anticheats, ["EA anticheat"]);
        assert_eq!(
            bf.page_url(),
            "https://areweanticheatyet.com/game/battlefield-6"
        );
        let (dbd, _) = d.find(Some(381_210), "Dead by Daylight").unwrap();
        assert_eq!(dbd.notes.len(), 2);
        assert_eq!(dbd.notes[1].1, None);
        assert_eq!(dbd.updates[0].name, "Plans to support Steam Deck");
    }

    #[test]
    fn a_steam_game_is_never_matched_by_its_title_alone() {
        let d = parse(LIST.as_bytes()).unwrap();
        // Same title, other id: not this game.
        assert_eq!(d.find(Some(1), "Battlefield 6"), None);
        // No id to go by: the exact title, when one entry has it.
        let (f, how) = d.find(None, "FORTNITE").unwrap();
        assert_eq!((f.slug.as_str(), how), ("fortnite", Match::Title));
        assert_eq!(d.find(None, "Fortnite Festival"), None);
    }

    #[test]
    fn an_id_listed_twice_goes_by_the_title_then_says_it_is_doubtful() {
        let d = parse(LIST.as_bytes()).unwrap();
        let (g, how) = d.find(Some(736_220), "Post Scriptum").unwrap();
        assert_eq!((g.status, how), (Status::Denied, Match::SteamIdAndTitle));
        let (g, how) = d.find(Some(736_220), "Something Else").unwrap();
        assert_eq!(
            (g.slug.as_str(), how),
            ("squad44", Match::SteamIdAmbiguous),
            "the newest entry, said to be doubtful"
        );
    }

    #[test]
    fn what_is_not_the_list_never_replaces_it() {
        assert!(parse(b"{}").is_err());
        assert!(parse(b"[]").is_err());
        assert!(parse(b"[{\"name\":1}]").is_err());
        assert!(parse(b"not json").is_err());
    }

    #[test]
    fn titles_compare_without_marks_case_or_spacing() {
        assert_eq!(
            normalize("Tom Clancy's Rainbow Six® Siege"),
            normalize("tom clancys  rainbow six siege")
        );
        assert_ne!(
            normalize("Rainbow Six Siege X"),
            normalize("Rainbow Six Siege")
        );
    }
}
