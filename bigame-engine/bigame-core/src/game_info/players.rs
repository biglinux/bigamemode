//! How many people play a game on Steam, from Steam itself.
//!
//! - **Now**: `ISteamUserStats/GetNumberOfCurrentPlayers` (no key; checked
//!   2026-10-19: `{"response":{"player_count":31901,"result":1}}`, a game
//!   Steam does not count answers `result: 42` with a 404). Steam caches it
//!   for 15 minutes (`max-age=900`), so it is not asked more often. It
//!   counts players connected to Steam in the game, not other stores.
//! - **Today's peak**: `ISteamChartsService/GetGamesByConcurrentPlayers`,
//!   what Steam's "Most Played" chart shows, one request for its top 100
//!   (`peak_in_game`, with the time of its `last_update`). Steam's own
//!   number for the day, for those games only.
//! - **Highest seen**: for every other game, the highest of the counts Big
//!   Game Mode itself read in the last 24 hours, said to be that, with how
//!   many readings and over how long — never as a peak of record. Under
//!   [`MIN_SPAN`] of readings it is "not enough history".
//!
//! `SteamDB` publishes no API and forbids automated collection, so it is not
//! read; `SteamSpy`'s day peak read 0 for Battlefield 6 with 36,000 playing,
//! so it is not either.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use super::http::{self, Failure};

/// The current-players address.
#[must_use]
pub fn current_url(app_id: u32) -> String {
    format!(
        "https://api.steampowered.com/ISteamUserStats/GetNumberOfCurrentPlayers/v1/?appid={app_id}"
    )
}

/// The "Most Played" chart's address.
pub const CHARTS_URL: &str =
    "https://api.steampowered.com/ISteamChartsService/GetGamesByConcurrentPlayers/v1/";

/// The game's page on `SteamDB`, for people (a link, never read).
#[must_use]
pub fn steamdb_url(app_id: u32) -> String {
    format!("https://steamdb.info/app/{app_id}/charts/")
}

/// What asking for a count gave.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fetched<T> {
    /// The answer.
    Value(T),
    /// Steam counts no players for this app.
    NotCounted,
    /// Asked to come back later.
    Later(Option<std::time::Duration>),
    /// The server failed, or sent what is not a count.
    Failed(String),
}

/// Ask Steam how many play `app_id` now.
///
/// # Errors
/// Returns why no answer came at all.
pub fn fetch_current(app_id: u32) -> Result<Fetched<u64>, Failure> {
    let r = http::get(&current_url(app_id), 16 * 1024, None)?;
    Ok(interpret_current(r.status, &r.body, r.retry_after))
}

fn interpret_current(
    status: u16,
    body: &[u8],
    retry_after: Option<std::time::Duration>,
) -> Fetched<u64> {
    match status {
        429 | 503 => return Fetched::Later(retry_after),
        200 | 404 => {}
        other => return Fetched::Failed(format!("HTTP {other}")),
    }
    let Ok(v) = serde_json::from_slice::<serde_json::Value>(body) else {
        return Fetched::Failed("not JSON".into());
    };
    let r = &v["response"];
    match (r["result"].as_u64(), r["player_count"].as_u64()) {
        (Some(1), Some(count)) => Fetched::Value(count),
        // 42: no match. Steam keeps no count for this app.
        (Some(_), None) => Fetched::NotCounted,
        _ => Fetched::Failed("not a player count".into()),
    }
}

/// Steam's "Most Played" chart.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Charts {
    /// When Steam last updated it, seconds since the epoch.
    pub updated: u64,
    /// By `AppID`: the players in the game now, and the day's peak.
    pub games: HashMap<u32, (u64, u64)>,
}

/// Ask Steam for its chart.
///
/// # Errors
/// Returns why no answer came at all.
pub fn fetch_charts() -> Result<Fetched<Charts>, Failure> {
    let r = http::get(CHARTS_URL, 512 * 1024, None)?;
    Ok(match r.status {
        200 => parse_charts(&r.body).map_or_else(Fetched::Failed, Fetched::Value),
        429 | 503 => Fetched::Later(r.retry_after),
        other => Fetched::Failed(format!("HTTP {other}")),
    })
}

fn parse_charts(body: &[u8]) -> Result<Charts, String> {
    let v: serde_json::Value = serde_json::from_slice(body).map_err(|e| e.to_string())?;
    let r = &v["response"];
    let ranks = r["ranks"].as_array().ok_or("no ranks")?;
    let mut games = HashMap::new();
    for rank in ranks.iter().take(1000) {
        let (Some(id), Some(now), Some(peak)) = (
            rank["appid"].as_u64().and_then(|n| u32::try_from(n).ok()),
            rank["concurrent_in_game"].as_u64(),
            rank["peak_in_game"].as_u64(),
        ) else {
            continue;
        };
        games.insert(id, (now, peak));
    }
    if games.is_empty() {
        return Err("no game in the chart".into());
    }
    Ok(Charts {
        updated: r["last_update"].as_u64().unwrap_or_default(),
        games,
    })
}

/// One count Big Game Mode read: when, how many.
pub type Sample = (u64, u64);

/// How far back readings are kept.
pub const HISTORY: u64 = 48 * 3600;
/// At most this many readings per game.
pub const MAX_SAMPLES: usize = 400;
/// The window a highest-seen count is taken over.
pub const WINDOW: u64 = 24 * 3600;
/// Readings must span this much of the window to say anything.
pub const MIN_SPAN: u64 = 12 * 3600;
/// And there must be this many of them.
pub const MIN_SAMPLES: usize = 6;

/// Add a reading, keeping only [`HISTORY`] and [`MAX_SAMPLES`].
pub fn record(samples: &mut Vec<Sample>, at: u64, count: u64) {
    samples.retain(|(t, _)| at.saturating_sub(*t) <= HISTORY && *t <= at);
    if samples.last().is_some_and(|(t, _)| *t == at) {
        samples.pop();
    }
    samples.push((at, count));
    if samples.len() > MAX_SAMPLES {
        let drop = samples.len() - MAX_SAMPLES;
        samples.drain(..drop);
    }
}

/// The highest count read in the last [`WINDOW`] before `now`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Observed {
    /// The highest count.
    pub peak: u64,
    /// How many readings.
    pub samples: usize,
    /// From the first reading in the window to the last, in seconds.
    pub span: u64,
}

impl Observed {
    /// Whether the readings cover enough of the window to be shown as the
    /// highest seen ([`MIN_SPAN`], [`MIN_SAMPLES`]).
    #[must_use]
    pub fn enough(&self) -> bool {
        self.span >= MIN_SPAN && self.samples >= MIN_SAMPLES
    }
}

/// [`Observed`] over `samples`; `None` without any in the window.
#[must_use]
pub fn observed(samples: &[Sample], now: u64) -> Option<Observed> {
    let window: Vec<&Sample> = samples
        .iter()
        .filter(|(t, _)| *t <= now && now - t <= WINDOW)
        .collect();
    let first = window.iter().map(|(t, _)| *t).min()?;
    let last = window.iter().map(|(t, _)| *t).max()?;
    Some(Observed {
        peak: window.iter().map(|(_, c)| *c).max()?,
        samples: window.len(),
        span: last - first,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steams_count_is_read_and_an_app_it_does_not_count_is_said_so() {
        assert_eq!(
            interpret_current(
                200,
                br#"{"response":{"player_count":31901,"result":1}}"#,
                None
            ),
            Fetched::Value(31_901)
        );
        assert_eq!(
            interpret_current(404, br#"{"response":{"result":42}}"#, None),
            Fetched::NotCounted
        );
        assert!(matches!(
            interpret_current(500, b"", None),
            Fetched::Failed(_)
        ));
        assert!(matches!(
            interpret_current(200, b"<html>", None),
            Fetched::Failed(_)
        ));
        assert_eq!(interpret_current(429, b"", None), Fetched::Later(None));
    }

    /// The chart as Steam answered on 2026-10-19, cut to a few games.
    const CHARTS: &str = r#"{"response":{"last_update":1792379931,"ranks":[
      {"rank":1,"appid":570,"concurrent_in_game":494555,"peak_in_game":1009131},
      {"rank":30,"appid":2807960,"concurrent_in_game":36703,"peak_in_game":76774},
      {"rank":36,"appid":1091500,"concurrent_in_game":32407,"peak_in_game":67873},
      {"rank":99,"appid":"x"}]}}"#;

    #[test]
    fn the_chart_gives_the_days_peak_of_its_games() {
        let c = parse_charts(CHARTS.as_bytes()).unwrap();
        assert_eq!(c.updated, 1_792_379_931);
        assert_eq!(c.games[&1_091_500], (32_407, 67_873));
        assert_eq!(c.games.len(), 3, "a malformed rank is left out");
        assert!(parse_charts(br#"{"response":{"ranks":[]}}"#).is_err());
    }

    #[test]
    fn the_highest_seen_needs_readings_over_half_a_day() {
        let now = 1_000_000;
        // An hour of readings: a highest seen, not enough to say.
        let mut s = Vec::new();
        for i in 0..4 {
            record(&mut s, now - 3600 + i * 900, 100 + i);
        }
        let o = observed(&s, now).unwrap();
        assert_eq!((o.peak, o.samples), (103, 4));
        assert!(!o.enough());
        // Readings since 13 hours ago, in the order they are read.
        let mut s = Vec::new();
        for i in 0..8 {
            record(&mut s, now - 13 * 3600 + i * 600, 500);
        }
        for i in 0..4 {
            record(&mut s, now - 3600 + i * 900, 100 + i);
        }
        let o = observed(&s, now).unwrap();
        assert_eq!((o.peak, o.samples), (500, 12));
        assert!(o.enough());
        // Older than the window: not counted.
        assert_eq!(observed(&s, now + 2 * WINDOW), None);
    }

    #[test]
    fn readings_are_bounded_in_age_and_number() {
        let mut s = Vec::new();
        for i in 0..(MAX_SAMPLES as u64 + 50) {
            record(&mut s, 10_000 + i, i);
        }
        assert_eq!(s.len(), MAX_SAMPLES);
        record(&mut s, 10_000 + HISTORY + 1_000, 1);
        assert!(
            s.iter().all(|(t, _)| *t >= 1_000 + 10_000),
            "older than HISTORY dropped"
        );
        // The same second twice keeps the newer count.
        record(&mut s, 999_999, 1);
        record(&mut s, 999_999, 2);
        assert_eq!(s.last(), Some(&(999_999, 2)));
    }
}
