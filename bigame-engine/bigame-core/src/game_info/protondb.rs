//! `ProtonDB`'s rating of a Steam game under Proton.
//!
//! `ProtonDB` publishes no versioned API. Its site reads a summary per game
//! from `https://www.protondb.com/api/v1/reports/summaries/<appid>.json`,
//! the same file other launchers' badges read; checked on 2026-10-19:
//!
//! ```json
//! {"bestReportedTier":"platinum","confidence":"strong","score":0.77,
//!  "tier":"gold","total":2740,"trendingTier":"platinum"}
//! ```
//!
//! A game nobody has reported on is a 404 (an HTML page), which is "no
//! reports", never Borked. A tier word this does not know (`pending`, for a
//! game with too few reports to rate, or one added later) is no tier rather
//! than a guess, and an unknown field is ignored, so a change to the file
//! degrades to less information, never to wrong information.

use serde::{Deserialize, Serialize};

use super::http::{self, Failure};

/// A rating, worst to best; [`Tier::Native`] is a Linux build, which
/// `ProtonDB`'s file does not say and Steam's store data does
/// ([`crate::games::steam_linux_builds`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Tier {
    /// It does not run, or barely.
    Borked,
    /// It runs, with significant problems.
    Bronze,
    /// It runs, with problems or limits.
    Silver,
    /// It runs very well, perhaps after small tweaks.
    Gold,
    /// It runs perfectly under Proton.
    Platinum,
    /// Steam lists a Linux build.
    Native,
}

impl Tier {
    /// `ProtonDB`'s word for it; `None` for a word it does not use for a
    /// rating.
    #[must_use]
    pub fn from_protondb(word: &str) -> Option<Self> {
        Some(match word.trim().to_ascii_lowercase().as_str() {
            "borked" => Self::Borked,
            "bronze" => Self::Bronze,
            "silver" => Self::Silver,
            "gold" => Self::Gold,
            "platinum" => Self::Platinum,
            "native" => Self::Native,
            _ => return None,
        })
    }

    /// Its name, as `ProtonDB` and Steam write it (a proper name, not
    /// translated).
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Borked => "Borked",
            Self::Bronze => "Bronze",
            Self::Silver => "Silver",
            Self::Gold => "Gold",
            Self::Platinum => "Platinum",
            Self::Native => "Native",
        }
    }

    /// What it means, translatable.
    #[must_use]
    pub fn meaning(self) -> &'static str {
        use crate::text::N_;
        match self {
            Self::Borked => N_("Does not run under Proton, or barely runs."),
            Self::Bronze => N_("Runs, with significant problems."),
            Self::Silver => N_("Runs, with some problems or limits."),
            Self::Gold => N_("Runs very well, perhaps after small tweaks."),
            Self::Platinum => N_("Runs perfectly under Proton, out of the box."),
            Self::Native => N_("Steam lists a Linux version of this game."),
        }
    }
}

/// One game's summary.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Summary {
    /// The rating `ProtonDB` gives it overall; `None` while it has too few
    /// reports to rate (`pending`).
    pub tier: Option<Tier>,
    /// The rating of its recent reports, when `ProtonDB` gives one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trending: Option<Tier>,
    /// The best rating any report gave.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub best: Option<Tier>,
    /// How many reports.
    pub reports: u32,
    /// `ProtonDB`'s confidence in the rating (`strong`, `good`, `low`…), as
    /// it writes it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<String>,
}

/// What `ProtonDB` says of a game.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum Rating {
    /// Rated, or reported on.
    Summary(Summary),
    /// Nobody has reported on it.
    NoReports,
}

/// What asking for a game's summary gave.
#[derive(Debug, Clone, PartialEq)]
pub enum Fetched {
    /// An answer to keep.
    Rating(Rating),
    /// Asked to come back later (429, 503), for at least this long.
    Later(Option<std::time::Duration>),
    /// The server failed (5xx), or sent what is not a summary.
    Failed(String),
}

/// The summary's address.
#[must_use]
pub fn summary_url(app_id: u32) -> String {
    format!("https://www.protondb.com/api/v1/reports/summaries/{app_id}.json")
}

/// The game's page on `ProtonDB`, for people.
#[must_use]
pub fn page_url(app_id: u32) -> String {
    format!("https://www.protondb.com/app/{app_id}")
}

/// No summary is larger than a few hundred bytes.
const MAX_BODY: u64 = 64 * 1024;

/// Ask `ProtonDB` for `app_id`'s summary.
///
/// # Errors
/// Returns why no answer came at all (offline, a timeout).
pub fn fetch(app_id: u32) -> Result<Fetched, Failure> {
    let r = http::get(&summary_url(app_id), MAX_BODY, None)?;
    Ok(interpret(r.status, &r.body, r.retry_after))
}

/// An answer as [`Fetched`].
fn interpret(status: u16, body: &[u8], retry_after: Option<std::time::Duration>) -> Fetched {
    match status {
        200 => parse(body).map_or_else(
            |e| Fetched::Failed(format!("not a summary: {e}")),
            |s| Fetched::Rating(Rating::Summary(s)),
        ),
        404 => Fetched::Rating(Rating::NoReports),
        429 | 503 => Fetched::Later(retry_after),
        other => Fetched::Failed(format!("HTTP {other}")),
    }
}

/// A summary from `ProtonDB`'s JSON.
///
/// # Errors
/// Returns an error when it is not a JSON object with a report count.
pub fn parse(body: &[u8]) -> Result<Summary, String> {
    let v: serde_json::Value = serde_json::from_slice(body).map_err(|e| e.to_string())?;
    let obj = v.as_object().ok_or("not an object")?;
    let tier = |key: &str| {
        obj.get(key)
            .and_then(|t| t.as_str())
            .and_then(Tier::from_protondb)
    };
    let reports = obj
        .get("total")
        .and_then(serde_json::Value::as_u64)
        .ok_or("no report count")?;
    Ok(Summary {
        // ProtonDB never calls a game native in this file; a "native" here
        // would be someone else's claim, not ProtonDB's.
        tier: tier("tier").filter(|t| *t != Tier::Native),
        trending: tier("trendingTier").filter(|t| *t != Tier::Native),
        best: tier("bestReportedTier").filter(|t| *t != Tier::Native),
        reports: u32::try_from(reports).unwrap_or(u32::MAX),
        confidence: obj
            .get("confidence")
            .and_then(|c| c.as_str())
            .map(|c| c.chars().take(32).collect()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Cyberpunk 2077's summary, 2026-10-19.
    const CYBERPUNK: &str = r#"{
  "bestReportedTier": "platinum",
  "confidence": "strong",
  "score": 0.77,
  "tier": "gold",
  "total": 2740,
  "trendingTier": "platinum"
}"#;

    #[test]
    fn a_summary_is_read_as_protondb_wrote_it() {
        let s = parse(CYBERPUNK.as_bytes()).unwrap();
        assert_eq!(s.tier, Some(Tier::Gold));
        assert_eq!(s.trending, Some(Tier::Platinum));
        assert_eq!(s.best, Some(Tier::Platinum));
        assert_eq!(s.reports, 2740);
        assert_eq!(s.confidence.as_deref(), Some("strong"));
    }

    #[test]
    fn no_reports_is_never_borked() {
        assert_eq!(
            interpret(404, b"<!DOCTYPE html>", None),
            Fetched::Rating(Rating::NoReports)
        );
        // Too few reports to rate: no tier, the reports still counted.
        let pending = parse(br#"{"tier":"pending","total":3,"confidence":"low"}"#).unwrap();
        assert_eq!(pending.tier, None);
        assert_eq!(pending.reports, 3);
    }

    #[test]
    fn a_changed_or_broken_answer_is_less_information_never_wrong() {
        // A new field, and a new word, are ignored.
        let s = parse(br#"{"tier":"diamond","total":10,"newThing":[1]}"#).unwrap();
        assert_eq!(s.tier, None);
        assert!(parse(b"[]").is_err());
        assert!(
            parse(br#"{"tier":"gold"}"#).is_err(),
            "a rating needs its reports"
        );
        assert!(matches!(
            interpret(200, b"<html>", None),
            Fetched::Failed(_)
        ));
        assert!(matches!(interpret(500, b"", None), Fetched::Failed(_)));
        assert_eq!(
            interpret(429, b"", Some(std::time::Duration::from_secs(60))),
            Fetched::Later(Some(std::time::Duration::from_secs(60)))
        );
    }

    #[test]
    fn tiers_sort_worst_to_best_and_read_protondbs_words() {
        assert!(Tier::Borked < Tier::Bronze && Tier::Gold < Tier::Platinum);
        assert_eq!(Tier::from_protondb(" Platinum "), Some(Tier::Platinum));
        assert_eq!(Tier::from_protondb("pending"), None);
        assert_eq!(page_url(1_091_500), "https://www.protondb.com/app/1091500");
    }
}
