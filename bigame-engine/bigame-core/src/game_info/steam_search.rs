//! The Steam version of a game another launcher installed (Heroic, Lutris,
//! Faugus), found by its title, so that `ProtonDB`'s rating, the anti-cheat
//! list's entry and Steam's counts can be said of it.
//!
//! Steam's store answers a search from its own pages at
//! `https://store.steampowered.com/api/storesearch/?term=<title>&l=english&cc=US`,
//! without an account; checked on 2026-10-19:
//!
//! ```json
//! {"total":1,"items":[{"type":"app","name":"CONTROL Ultimate Edition",
//!  "id":870780,"platforms":{"windows":true,"mac":true,"linux":false}}]}
//! ```
//!
//! Only an app whose name is the title, letter for letter once case, marks
//! such as ™ and ® and punctuation are set aside, is taken; or, failing
//! that, one that differs from it only by an edition's name ("Ultimate
//! Edition", "Game of the Year Edition"…). "CONTROL Resonant" is not
//! Control, a soundtrack or a DLC is not the game, and nothing found is
//! better than the wrong game: a title with no such match is "not on
//! Steam". Only the title is sent.

use serde::{Deserialize, Serialize};

use super::anticheat::normalize;
use super::http::{self, Failure};

/// A game's Steam version.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Found {
    /// Its `AppID`.
    pub id: u32,
    /// Its name on Steam, as Steam writes it.
    pub name: String,
}

/// What a search gave.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fetched {
    /// The answer: the game's Steam version, or none that is it.
    Found(Option<Found>),
    /// Asked to come back later (429, 503), for at least this long.
    Later(Option<std::time::Duration>),
    /// The server failed, or sent what is not a search.
    Failed(String),
}

/// No search answer is larger than a few kilobytes.
const MAX_BODY: u64 = 256 * 1024;

/// What an edition adds to a game's name, never what makes another game.
const EDITIONS: &[&str] = &[
    "ultimate edition",
    "definitive edition",
    "complete edition",
    // Before "deluxe edition", which it ends with.
    "digital deluxe edition",
    "deluxe edition",
    "standard edition",
    "enhanced edition",
    "game of the year edition",
    "goty edition",
    "goty",
];

/// The search's address for `title`.
#[must_use]
pub fn search_url(title: &str) -> String {
    let term: String = title
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                char::from(b).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect();
    format!("https://store.steampowered.com/api/storesearch/?term={term}&l=english&cc=US")
}

/// The game's page on Steam, for people.
#[must_use]
pub fn store_url(app_id: u32) -> String {
    format!("https://store.steampowered.com/app/{app_id}/")
}

/// Ask Steam's store for the game called `title`.
///
/// # Errors
/// Returns why no answer came at all (offline, a timeout).
pub fn fetch(title: &str) -> Result<Fetched, Failure> {
    let r = http::get(&search_url(title), MAX_BODY, None)?;
    Ok(interpret(r.status, &r.body, r.retry_after, title))
}

fn interpret(
    status: u16,
    body: &[u8],
    retry_after: Option<std::time::Duration>,
    title: &str,
) -> Fetched {
    match status {
        200 => match parse(body) {
            Ok(items) => Fetched::Found(best(&items, title)),
            Err(e) => Fetched::Failed(format!("not a search: {e}")),
        },
        429 | 503 => Fetched::Later(retry_after),
        other => Fetched::Failed(format!("HTTP {other}")),
    }
}

/// The apps of a search answer, in Steam's order.
///
/// # Errors
/// Returns an error when it is not a search answer.
pub fn parse(body: &[u8]) -> Result<Vec<Found>, String> {
    let v: serde_json::Value = serde_json::from_slice(body).map_err(|e| e.to_string())?;
    let items = v["items"].as_array().ok_or("no items")?;
    Ok(items
        .iter()
        .filter(|i| i["type"].as_str() == Some("app"))
        .filter_map(|i| {
            Some(Found {
                id: u32::try_from(i["id"].as_u64()?).ok().filter(|id| *id > 0)?,
                name: i["name"].as_str()?.chars().take(200).collect(),
            })
        })
        .collect())
}

/// `title` without an edition's name at its end.
fn without_edition(title: &str) -> &str {
    EDITIONS
        .iter()
        .find_map(|e| {
            title
                .strip_suffix(e)
                .map(str::trim_end)
                .filter(|t| !t.is_empty())
        })
        .unwrap_or(title)
}

/// The app that is the game called `title`: one with that very name, else
/// the first that differs only by an edition.
#[must_use]
pub fn best(items: &[Found], title: &str) -> Option<Found> {
    let wanted = normalize(title);
    if wanted.is_empty() {
        return None;
    }
    items
        .iter()
        .find(|i| normalize(&i.name) == wanted)
        .or_else(|| {
            let bare = without_edition(&wanted);
            items
                .iter()
                .find(|i| without_edition(&normalize(&i.name)) == bare)
        })
        .cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(id: u32, name: &str) -> Found {
        Found {
            id,
            name: name.into(),
        }
    }

    #[test]
    fn the_game_is_found_by_its_title_and_never_another() {
        let control = [
            app(3_669_870, "CONTROL Resonant"),
            app(870_780, "CONTROL Ultimate Edition"),
            app(4_165_870, "Steam Controller"),
        ];
        assert_eq!(best(&control, "Control").map(|f| f.id), Some(870_780));
        let callisto = [
            app(1_544_020, "The Callisto Protocol™"),
            app(2_005_983, "The Callisto Protocol™ - Final Transmission"),
        ];
        assert_eq!(
            best(&callisto, "The Callisto Protocol").map(|f| f.id),
            Some(1_544_020)
        );
        let lego = [
            app(920_210, "LEGO® Star Wars™: The Skywalker Saga"),
            app(
                785_847,
                "LEGO® Star Wars™: The Skywalker Saga Galactic Edition",
            ),
        ];
        assert_eq!(
            best(&lego, "LEGO® Star Wars™: The Skywalker Saga").map(|f| f.id),
            Some(920_210)
        );
        // A sequel, a tool, a soundtrack: not the game.
        assert_eq!(best(&[app(1, "CONTROL Resonant")], "Control"), None);
        assert_eq!(best(&[app(2, "Hades II")], "Hades"), None);
        assert_eq!(best(&[app(3, "Cuphead - Soundtrack")], "Cuphead"), None);
        assert_eq!(best(&[], "Anything"), None);
        assert_eq!(best(&[app(4, "™")], "®"), None);
    }

    #[test]
    fn an_edition_of_the_title_counts_both_ways() {
        let goty = [app(5, "The Witcher 3: Wild Hunt")];
        assert_eq!(
            best(&goty, "The Witcher 3: Wild Hunt - Game of the Year Edition").map(|f| f.id),
            Some(5)
        );
        assert_eq!(
            best(&[app(6, "Alan Wake Remastered")], "Alan Wake"),
            None,
            "a remaster is not an edition"
        );
    }

    #[test]
    fn an_answer_is_read_as_steam_writes_it_and_anything_else_is_a_failure() {
        let body = br#"{"total":2,"items":[
            {"type":"app","name":"CONTROL Ultimate Edition","id":870780,"platforms":{"linux":false}},
            {"type":"sub","name":"A bundle","id":12},
            {"type":"app","name":"No id"}]}"#;
        assert_eq!(
            parse(body).unwrap(),
            vec![app(870_780, "CONTROL Ultimate Edition")]
        );
        assert_eq!(
            interpret(200, br#"{"total":0,"items":[]}"#, None, "Nothing"),
            Fetched::Found(None)
        );
        assert!(matches!(
            interpret(200, b"<html>", None, "x"),
            Fetched::Failed(_)
        ));
        assert!(matches!(interpret(500, b"", None, "x"), Fetched::Failed(_)));
        assert_eq!(interpret(429, b"", None, "x"), Fetched::Later(None));
    }

    #[test]
    fn the_title_is_sent_encoded_and_only_it() {
        assert_eq!(
            search_url("Shredder's Revenge & Co"),
            "https://store.steampowered.com/api/storesearch/?term=Shredder%27s%20Revenge%20%26%20Co&l=english&cc=US"
        );
        assert_eq!(
            search_url("LEGO®"),
            "https://store.steampowered.com/api/storesearch/?term=LEGO%C2%AE&l=english&cc=US"
        );
        assert_eq!(
            store_url(870_780),
            "https://store.steampowered.com/app/870780/"
        );
    }
}
