//! Game information: what the world knows about a game on Linux, beside
//! what Big Game Mode does to it.
//!
//! Three questions, three sources, each the one that can answer it:
//!
//! - *Does it run well on Linux?* `ProtonDB`'s rating ([`protondb`]), and
//!   whether Steam lists a Linux build (appinfo, on this machine).
//! - *Does its anti-cheat let it be played online?* Are We Anti-Cheat
//!   Yet?'s list ([`anticheat`]), one file for the whole library.
//! - *How many play it?* Steam's own counts ([`players`]).
//!
//! A game another launcher installed (Heroic, Lutris, Faugus) is a Linux
//! build or not as the library found it; one that is not is looked for on
//! Steam's store by its title ([`steam_search`]), and what the three
//! sources say of its Steam version is said as that.
//!
//! Information only: nothing here changes a profile, a launch option, the
//! environment of a game or anything falcond reads, and nothing reads it but
//! the pages that show it.
//!
//! Everything is kept in one small file under `$XDG_CACHE_HOME/bigame-mode/
//! game-info/` (and the anti-cheat list beside it), written in one rename,
//! versioned, read back whole or not at all, and pruned of games that left
//! the library. The library shows what is kept at once; [`refresh`] asks
//! only for what is older than its source allows ([`PROTONDB_TTL`] and the
//! rest), a few at a time, never twice in one pass, and backs off a source
//! that fails or asks to wait (`Retry-After`), longer each time, with
//! jitter. The interface runs it only while the library is on screen, never
//! while a game runs, and stops it when either changes.

pub mod anticheat;
mod http;
pub mod players;
pub mod protondb;
pub mod steam_search;

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

pub use http::Failure;

/// `ProtonDB` ratings change slowly; its site rebuilds summaries daily.
pub const PROTONDB_TTL: u64 = 24 * 3600;
/// The anti-cheat list changes a few times a week.
pub const ANTICHEAT_TTL: u64 = 12 * 3600;
/// Steam caches its count for 15 minutes.
pub const PLAYERS_TTL: u64 = 15 * 60;
/// Steam's chart, as often as its counts.
pub const CHARTS_TTL: u64 = 15 * 60;
/// A game's Steam version found by its title: titles do not change.
pub const SEARCH_FOUND_TTL: u64 = 30 * 24 * 3600;
/// None found: asked again in a week, in case it comes to Steam.
pub const SEARCH_NONE_TTL: u64 = 7 * 24 * 3600;
/// At most this many titles looked for on Steam in one pass.
const MAX_SEARCHES_PER_PASS: usize = 20;
/// A game gone from the library is forgotten after this long.
const FORGET_AFTER: u64 = 30 * 24 * 3600;
/// Requests at once in one pass.
const WORKERS: usize = 3;
/// At most this many `ProtonDB` requests in one pass; the rest wait for the
/// next.
const MAX_RATINGS_PER_PASS: usize = 60;
/// The store's format.
const VERSION: u32 = 1;
/// No real store is larger.
const MAX_STORE: u64 = 16 * 1024 * 1024;

/// A game as the sources know it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct GameRef {
    /// Its Steam `AppID`, only for a game Steam installed: a Flatpak's id
    /// or another store's is not one.
    pub steam: Option<u32>,
    /// Its title, for the anti-cheat list's exact-title match when there is
    /// no Steam id, and to look for its Steam version.
    pub title: String,
    /// The copy installed is a Linux build ([`DetectedGame::runs_on_linux`]):
    /// it runs without Proton or Wine, whatever another store's version
    /// does.
    ///
    /// [`DetectedGame::runs_on_linux`]: crate::games::DetectedGame::runs_on_linux
    pub linux_build: bool,
}

impl GameRef {
    /// The reference of a detected game.
    #[must_use]
    pub fn of(game: &crate::games::DetectedGame) -> Self {
        Self {
            steam: (game.source == crate::games::Source::Steam)
                .then(|| game.app_id.as_deref()?.parse().ok())
                .flatten()
                .filter(|id| *id > 0),
            title: game.name.clone(),
            linux_build: game.runs_on_linux(),
        }
    }

    /// Whether its Steam version is looked for by its title: not installed
    /// by Steam, and not a Linux build.
    #[must_use]
    pub fn searched(&self) -> bool {
        self.steam.is_none() && !self.linux_build
    }
}

/// Which sources may be asked.
// Independent switches; grouping them to please the lint would only add
// indirection.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Wants {
    /// `ProtonDB`.
    pub protondb: bool,
    /// The anti-cheat list.
    pub anticheat: bool,
    /// Steam's player counts.
    pub players: bool,
    /// Ask again whatever its age, as "Update" asks: a source resting
    /// after a failure or a `Retry-After` is still left alone.
    pub now: bool,
}

impl Wants {
    /// Whether anything at all may be asked.
    #[must_use]
    pub fn any(&self) -> bool {
        self.protondb || self.anticheat || self.players
    }
}

/// A value and when it was learnt (seconds since the epoch).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Cached<T> {
    /// The value.
    pub value: T,
    /// When it was fetched.
    pub at: u64,
}

impl<T> Cached<T> {
    /// Whether it is older than `ttl` at `now`.
    #[must_use]
    pub fn stale(&self, ttl: u64, now: u64) -> bool {
        now.saturating_sub(self.at) >= ttl
    }
}

/// A source's last failure, and until when it is left alone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Trouble {
    /// What went wrong, in the source's or curl's words.
    pub message: String,
    /// When.
    pub at: u64,
    /// Not asked again before this.
    pub until: u64,
    /// Failures in a row.
    pub failures: u32,
}

/// The sources, by name in the store.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    /// `ProtonDB`.
    Protondb,
    /// The anti-cheat list.
    Anticheat,
    /// Steam's counts and chart.
    Steam,
    /// Steam's store search, for the Steam version of another launcher's
    /// game.
    SteamSearch,
}

/// What is kept.
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Store {
    version: u32,
    /// `ProtonDB` ratings, by Steam `AppID`.
    pub protondb: BTreeMap<u32, Cached<protondb::Rating>>,
    /// The Steam apps Steam lists a Linux build for (appinfo).
    pub native: HashSet<u32>,
    /// Steam's current count; `None`: Steam counts no players for it.
    pub current: BTreeMap<u32, Cached<Option<u64>>>,
    /// The counts read, for the highest seen.
    pub samples: BTreeMap<u32, Vec<players::Sample>>,
    /// Steam's chart.
    pub charts: Option<Cached<players::Charts>>,
    /// When the anti-cheat list was last found current (fetched, or
    /// unchanged).
    pub anticheat_checked: Option<u64>,
    /// The status each library game had in the list, to tell a change.
    pub anticheat_seen: BTreeMap<u32, anticheat::Status>,
    /// Sources failing now.
    pub trouble: BTreeMap<Source, Trouble>,
    /// When each game was last in the library.
    pub last_seen: BTreeMap<u32, u64>,
    /// The Steam version of another launcher's game, by its title
    /// ([`anticheat::normalize`]d); `None`: there is none.
    pub steam_by_title: BTreeMap<String, Cached<Option<steam_search::Found>>>,
    /// When each such title was last in the library.
    pub titles_seen: BTreeMap<String, u64>,
}

/// Where the information is kept.
#[must_use]
pub fn dir() -> PathBuf {
    crate::paths::cache_home().join("bigame-mode/game-info")
}

fn store_path(dir: &Path) -> PathBuf {
    dir.join("store.json")
}

fn dataset_path(dir: &Path) -> PathBuf {
    dir.join("anticheat-games.json")
}

fn etag_path(dir: &Path) -> PathBuf {
    dir.join("anticheat-games.etag")
}

impl Store {
    /// The store in `dir`; empty when there is none. One that does not read
    /// (damaged, or from a newer version) is set aside as `store.json.bad`
    /// and started again: the information comes back from its sources.
    #[must_use]
    pub fn load(dir: &Path) -> Self {
        let path = store_path(dir);
        let Ok(meta) = std::fs::metadata(&path) else {
            return Self::default();
        };
        let read = (meta.len() <= MAX_STORE)
            .then(|| std::fs::read(&path).ok())
            .flatten()
            .and_then(|b| serde_json::from_slice::<Self>(&b).ok())
            .filter(|s| s.version == VERSION);
        read.unwrap_or_else(|| {
            tracing::warn!(file = %path.display(), "the game information kept does not read; it is fetched again");
            let _ = std::fs::rename(&path, path.with_extension("json.bad"));
            Self::default()
        })
    }

    fn save(&mut self, dir: &Path) -> Result<()> {
        self.version = VERSION;
        write_bytes(&store_path(dir), &serde_json::to_vec(self)?)
    }

    /// Whether `source` is left alone at `now`.
    #[must_use]
    pub fn resting(&self, source: Source, now: u64) -> bool {
        self.trouble.get(&source).is_some_and(|t| t.until > now)
    }

    fn failed(
        &mut self,
        source: Source,
        message: String,
        wait: Option<std::time::Duration>,
        now: u64,
    ) {
        let failures = self.trouble.get(&source).map_or(0, |t| t.failures) + 1;
        let until = now + wait.map_or_else(|| backoff(failures, now), |d| d.as_secs().max(60));
        tracing::info!(target: "game_info", ?source, %message, failures, wait_secs = until - now, "a source failed; it is left alone for a while");
        self.trouble.insert(
            source,
            Trouble {
                message,
                at: now,
                until,
                failures,
            },
        );
    }

    fn worked(&mut self, source: Source) {
        self.trouble.remove(&source);
    }
}

/// Seconds to leave a source alone after `failures` failures in a row: five
/// minutes, doubled each time, at most six hours, plus up to a minute of
/// jitter so a pause shared by many clients does not end at one moment.
fn backoff(failures: u32, now: u64) -> u64 {
    let base = 300u64.saturating_mul(1 << failures.saturating_sub(1).min(7));
    let jitter = (now ^ u64::from(std::process::id())).wrapping_mul(0x9E37_79B9_7F4A_7C15) % 60;
    base.min(6 * 3600) + jitter
}

/// Write `bytes` to `path` in one rename, flushed: a reader sees the old file
/// or the new one, never half of one.
pub(crate) fn write_bytes(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write as _;
    let dir = path.parent().context("no parent folder")?;
    std::fs::create_dir_all(dir).with_context(|| format!("create {}", dir.display()))?;
    let tmp = dir.join(format!(
        ".{}.{}",
        path.file_name().unwrap_or_default().to_string_lossy(),
        std::process::id()
    ));
    let written = (|| {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        std::fs::rename(&tmp, path)
    })();
    if written.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    written.with_context(|| format!("write {}", path.display()))
}

/// Everything known, ready to read: the store and the indexed list.
#[derive(Debug, Default)]
pub struct Snapshot {
    /// What is kept.
    pub store: Store,
    /// The anti-cheat list.
    pub dataset: anticheat::Dataset,
}

impl Snapshot {
    /// What is kept in [`dir`], read from disk (off the main thread: the
    /// list is half a megabyte of JSON).
    #[must_use]
    pub fn load() -> Self {
        Self::load_from(&dir())
    }

    fn load_from(dir: &Path) -> Self {
        Self {
            store: Store::load(dir),
            dataset: anticheat::load(&dataset_path(dir)),
        }
    }

    /// The Steam version found for `game` by its title, if it was looked
    /// for and there is one.
    #[must_use]
    pub fn steam_version(&self, game: &GameRef) -> Option<&steam_search::Found> {
        game.searched()
            .then(|| {
                self.store
                    .steam_by_title
                    .get(&anticheat::normalize(&game.title))
            })
            .flatten()
            .and_then(|c| c.value.as_ref())
    }

    /// What is known of `game` at `now`.
    #[must_use]
    pub fn info(&self, game: &GameRef, now: u64) -> Info {
        let s = &self.store;
        let found = self.steam_version(game);
        let steam = game.steam.or(found.map(|f| f.id));
        let looked_for = game.searched()
            && s.steam_by_title
                .contains_key(&anticheat::normalize(&game.title));
        let linux = match steam {
            _ if game.linux_build => Linux::LinuxBuild,
            None if looked_for => Linux::NoSteamId,
            None if game.searched() && s.trouble.contains_key(&Source::SteamSearch) => {
                Linux::Unavailable
            }
            None if game.searched() => Linux::NotFetched,
            None => Linux::NoSteamId,
            // Steam's own data says it of the copy Steam installed; another
            // launcher's copy is the Windows build, rated by ProtonDB.
            Some(id) if game.steam.is_some() && s.native.contains(&id) => Linux::Native,
            Some(id) => match s.protondb.get(&id) {
                Some(c) => Linux::Rating {
                    rating: c.value.clone(),
                    at: c.at,
                    stale: c.stale(PROTONDB_TTL, now),
                },
                None if s.trouble.contains_key(&Source::Protondb) => Linux::Unavailable,
                None => Linux::NotFetched,
            },
        };
        // The native build is said, and ProtonDB's rating still shown in the
        // details: a native game can be played through Proton too.
        let rating = steam
            .and_then(|id| s.protondb.get(&id))
            .map(|c| (c.value.clone(), c.at));
        let anticheat = if self.dataset.is_empty() {
            AntiCheat::Unavailable
        } else {
            match self.dataset.find(steam, &game.title) {
                Some((g, how)) => AntiCheat::Listed {
                    game: Box::new(g.clone()),
                    how,
                },
                None => AntiCheat::NotListed,
            }
        };
        let anticheat_at = s.anticheat_checked;
        let players = steam.map(|id| {
            let current = s.current.get(&id);
            let chart = s
                .charts
                .as_ref()
                .filter(|c| !c.stale(24 * 3600, now))
                .and_then(|c| {
                    c.value
                        .games
                        .get(&id)
                        .map(|(_, peak)| (*peak, c.value.updated))
                });
            Players {
                now: current.and_then(|c| c.value.map(|n| (n, c.at))),
                not_counted: current.is_some_and(|c| c.value.is_none()),
                stale: current.is_none_or(|c| c.stale(PLAYERS_TTL, now)),
                peak: match chart {
                    Some((peak, at)) => Peak::SteamToday { peak, at },
                    None => match s.samples.get(&id).and_then(|v| players::observed(v, now)) {
                        Some(o) if o.enough() => Peak::Observed(o),
                        o => Peak::NotEnough(o),
                    },
                },
                samples: s.samples.get(&id).cloned().unwrap_or_default(),
            }
        });
        Info {
            steam,
            steam_version: found.cloned(),
            linux,
            rating,
            anticheat,
            anticheat_at,
            players,
        }
    }
}

/// How well a game runs on Linux, as far as is known.
#[derive(Debug, Clone, PartialEq)]
pub enum Linux {
    /// Steam lists a Linux build.
    Native,
    /// `ProtonDB`'s answer, when it was fetched, and whether that is older
    /// than its source allows.
    Rating {
        /// The answer.
        rating: protondb::Rating,
        /// When.
        at: u64,
        /// Older than [`PROTONDB_TTL`].
        stale: bool,
    },
    /// Not asked yet.
    NotFetched,
    /// `ProtonDB` could not be reached, and nothing is kept.
    Unavailable,
    /// No Steam version: not a Steam game, and none found on Steam's store
    /// by its title. `ProtonDB` rates Steam games by their id.
    NoSteamId,
    /// The copy installed is a Linux build.
    LinuxBuild,
}

/// What the anti-cheat list says of a game.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AntiCheat {
    /// Listed.
    Listed {
        /// Its entry.
        game: Box<anticheat::Game>,
        /// How it was found.
        how: anticheat::Match,
    },
    /// The list does not have it: not "no anti-cheat", only not listed.
    NotListed,
    /// No list yet (never fetched, or none kept).
    Unavailable,
}

/// A game's day peak.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Peak {
    /// Steam's own, from its chart (the day so far), as of `at`.
    SteamToday {
        /// The peak.
        peak: u64,
        /// When Steam updated its chart.
        at: u64,
    },
    /// The highest count Big Game Mode read in 24 hours, enough of them.
    Observed(players::Observed),
    /// Not enough readings to say, with what there is.
    NotEnough(Option<players::Observed>),
}

/// How many play a Steam game.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Players {
    /// Steam's count now, and when it was read.
    pub now: Option<(u64, u64)>,
    /// Steam keeps no count for this app.
    pub not_counted: bool,
    /// The count is older than [`PLAYERS_TTL`], or missing.
    pub stale: bool,
    /// The day's peak.
    pub peak: Peak,
    /// The counts read, oldest first, for a chart of what was seen.
    pub samples: Vec<players::Sample>,
}

/// Everything known of one game.
#[derive(Debug, Clone, PartialEq)]
pub struct Info {
    /// Its Steam `AppID`: Steam's own, or its Steam version's.
    pub steam: Option<u32>,
    /// The Steam version found by its title, for a game another launcher
    /// installed: what is said of `steam` is said of that version.
    pub steam_version: Option<steam_search::Found>,
    /// How well it runs.
    pub linux: Linux,
    /// `ProtonDB`'s answer, even for a native game.
    pub rating: Option<(protondb::Rating, u64)>,
    /// Its anti-cheat.
    pub anticheat: AntiCheat,
    /// When the anti-cheat list was last found current.
    pub anticheat_at: Option<u64>,
    /// Its players, for a Steam game.
    pub players: Option<Players>,
}

/// A library game whose anti-cheat status changed in the list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    /// The game's title.
    pub title: String,
    /// Its Steam `AppID`.
    pub steam: u32,
    /// The status before.
    pub before: anticheat::Status,
    /// The status now.
    pub after: anticheat::Status,
}

/// What a pass did.
#[derive(Debug, Default)]
pub struct Refreshed {
    /// Anything was learnt.
    pub changed: bool,
    /// Library games whose anti-cheat status changed in a new list.
    pub changes: Vec<Change>,
}

/// One pass: ask the sources `wants` allows for what `games` need and is
/// older than their source allows, keep it, and return what is known. Stops
/// between requests once `stop` is set. Blocking; never on the main thread.
#[must_use]
pub fn refresh(games: &[GameRef], wants: Wants, stop: &AtomicBool) -> (Snapshot, Refreshed) {
    refresh_in(&dir(), games, wants, stop, &Net::real(), crate::unix_now())
}

type RatingFn = dyn Fn(u32) -> Result<protondb::Fetched, Failure> + Sync;
type CurrentFn = dyn Fn(u32) -> Result<players::Fetched<u64>, Failure> + Sync;
type ChartsFn = dyn Fn() -> Result<players::Fetched<players::Charts>, Failure> + Sync;
type DatasetFn = dyn Fn(&Path, &Path) -> Result<anticheat::Fetched, Failure> + Sync;
type LinuxFn = dyn Fn(&HashSet<u32>) -> HashSet<u32> + Sync;
type SearchFn = dyn Fn(&str) -> Result<steam_search::Fetched, Failure> + Sync;

/// The sources, as functions, so a pass can be tested without a network.
struct Net {
    rating: Box<RatingFn>,
    current: Box<CurrentFn>,
    charts: Box<ChartsFn>,
    dataset: Box<DatasetFn>,
    linux: Box<LinuxFn>,
    search: Box<SearchFn>,
}

/// One request a pass makes for one game.
#[derive(Clone, Copy)]
enum Job {
    Rating(u32),
    Current(u32),
}

/// Its answer.
enum Answer {
    Rating(u32, Result<protondb::Fetched, Failure>),
    Current(u32, Result<players::Fetched<u64>, Failure>),
}

impl Net {
    fn real() -> Self {
        Self {
            rating: Box::new(protondb::fetch),
            current: Box::new(players::fetch_current),
            charts: Box::new(players::fetch_charts),
            dataset: Box::new(anticheat::fetch),
            linux: Box::new(|ids| crate::games::steam_linux_builds(&crate::paths::home_dir(), ids)),
            search: Box::new(steam_search::fetch),
        }
    }
}

#[allow(clippy::too_many_lines)]
fn refresh_in(
    dir: &Path,
    games: &[GameRef],
    wants: Wants,
    stop: &AtomicBool,
    net: &Net,
    now: u64,
) -> (Snapshot, Refreshed) {
    let mut snap = Snapshot::load_from(dir);
    let mut done = Refreshed::default();
    let store = &mut snap.store;
    // Aged past `ttl`, or asked for now.
    let due = |at: u64, ttl: u64| wants.now || now.saturating_sub(at) >= ttl;

    // Another launcher's games: their Steam version, by title, first, so
    // what is asked below is asked for it too.
    search_titles(store, games, wants, stop, net, now, &mut done);
    let steam_of = |store: &Store, g: &GameRef| {
        g.steam.or_else(|| {
            g.searched()
                .then(|| store.steam_by_title.get(&anticheat::normalize(&g.title)))
                .flatten()
                .and_then(|c| c.value.as_ref().map(|f| f.id))
        })
    };
    let ids: Vec<u32> = {
        let mut ids: Vec<u32> = games.iter().filter_map(|g| steam_of(store, g)).collect();
        ids.sort_unstable();
        ids.dedup();
        ids
    };
    for id in &ids {
        store.last_seen.insert(*id, now);
    }
    // Local, read on every pass: a game updated with a Linux build says
    // so. Only for the copies Steam installed.
    let installed: HashSet<u32> = games.iter().filter_map(|g| g.steam).collect();
    let native = (net.linux)(&installed);
    if native
        != store
            .native
            .iter()
            .copied()
            .filter(|i| installed.contains(i))
            .collect()
    {
        done.changed = true;
    }
    store.native.retain(|i| !installed.contains(i));
    store.native.extend(native);

    // The anti-cheat list: one file for the whole library.
    if wants.anticheat
        && !stop.load(Ordering::Relaxed)
        && store
            .anticheat_checked
            .is_none_or(|at| due(at, ANTICHEAT_TTL))
        && !store.resting(Source::Anticheat, now)
    {
        match (net.dataset)(&dataset_path(dir), &etag_path(dir)) {
            Ok(anticheat::Fetched::Updated) => {
                store.worked(Source::Anticheat);
                store.anticheat_checked = Some(now);
                snap.dataset = anticheat::load(&dataset_path(dir));
                done.changed = true;
            }
            Ok(anticheat::Fetched::Unchanged) => {
                store.worked(Source::Anticheat);
                store.anticheat_checked = Some(now);
            }
            Ok(anticheat::Fetched::Later(wait)) => {
                store.failed(Source::Anticheat, "asked to wait".into(), wait, now);
            }
            Ok(anticheat::Fetched::Failed(e)) => store.failed(Source::Anticheat, e, None, now),
            Err(e) => store.failed(Source::Anticheat, e.to_string(), None, now),
        }
    }
    // A change of status is said only between two lists that both have the
    // game, so a failed fetch or a game just added is never "a change".
    if !snap.dataset.is_empty() {
        for g in games {
            let Some(id) = steam_of(store, g) else {
                continue;
            };
            let Some((entry, _)) = snap.dataset.find(Some(id), &g.title) else {
                continue;
            };
            match store.anticheat_seen.insert(id, entry.status) {
                Some(before) if before != entry.status => done.changes.push(Change {
                    title: g.title.clone(),
                    steam: id,
                    before,
                    after: entry.status,
                }),
                _ => {}
            }
        }
    }

    // Steam's chart: one request for its top 100.
    if wants.players
        && !ids.is_empty()
        && !stop.load(Ordering::Relaxed)
        && store.charts.as_ref().is_none_or(|c| due(c.at, CHARTS_TTL))
        && !store.resting(Source::Steam, now)
    {
        match (net.charts)() {
            Ok(players::Fetched::Value(charts)) => {
                store.worked(Source::Steam);
                store.charts = Some(Cached {
                    value: charts,
                    at: now,
                });
                done.changed = true;
            }
            Ok(players::Fetched::Later(wait)) => {
                store.failed(Source::Steam, "asked to wait".into(), wait, now);
            }
            Ok(players::Fetched::Failed(e)) => store.failed(Source::Steam, e, None, now),
            Ok(players::Fetched::NotCounted) => {}
            Err(e) => store.failed(Source::Steam, e.to_string(), None, now),
        }
    }

    // Per game: what is older than its source allows.
    let mut jobs: Vec<Job> = Vec::new();
    if wants.protondb && !store.resting(Source::Protondb, now) {
        jobs.extend(
            ids.iter()
                .filter(|id| !store.native.contains(id))
                .filter(|id| {
                    store
                        .protondb
                        .get(id)
                        .is_none_or(|c| due(c.at, PROTONDB_TTL))
                })
                .take(MAX_RATINGS_PER_PASS)
                .map(|id| Job::Rating(*id)),
        );
    }
    if wants.players && !store.resting(Source::Steam, now) {
        jobs.extend(
            ids.iter()
                .filter(|id| store.current.get(id).is_none_or(|c| due(c.at, PLAYERS_TTL)))
                .map(|id| Job::Current(*id)),
        );
    }
    let queue = std::sync::Mutex::new(jobs.into_iter());
    // A source that failed in this pass is not asked again in it.
    let halted: [AtomicBool; 2] = [AtomicBool::new(false), AtomicBool::new(false)];
    let answers = std::sync::Mutex::new(Vec::new());
    std::thread::scope(|scope| {
        for _ in 0..WORKERS {
            scope.spawn(|| {
                loop {
                    if stop.load(Ordering::Relaxed) {
                        return;
                    }
                    let Some(job) = queue.lock().ok().and_then(|mut q| q.next()) else {
                        return;
                    };
                    let answer = match job {
                        Job::Rating(id) if !halted[0].load(Ordering::Relaxed) => {
                            let r = (net.rating)(id);
                            if !matches!(r, Ok(protondb::Fetched::Rating(_))) {
                                halted[0].store(true, Ordering::Relaxed);
                            }
                            Answer::Rating(id, r)
                        }
                        Job::Current(id) if !halted[1].load(Ordering::Relaxed) => {
                            let r = (net.current)(id);
                            if !matches!(
                                r,
                                Ok(players::Fetched::Value(_) | players::Fetched::NotCounted)
                            ) {
                                halted[1].store(true, Ordering::Relaxed);
                            }
                            Answer::Current(id, r)
                        }
                        _ => continue,
                    };
                    if let Ok(mut a) = answers.lock() {
                        a.push(answer);
                    }
                }
            });
        }
    });
    for answer in answers.into_inner().unwrap_or_default() {
        match answer {
            Answer::Rating(id, Ok(protondb::Fetched::Rating(rating))) => {
                store.worked(Source::Protondb);
                store.protondb.insert(
                    id,
                    Cached {
                        value: rating,
                        at: now,
                    },
                );
                done.changed = true;
            }
            Answer::Rating(_, Ok(protondb::Fetched::Later(wait))) => {
                store.failed(Source::Protondb, "asked to wait".into(), wait, now);
            }
            Answer::Rating(_, Ok(protondb::Fetched::Failed(e))) => {
                store.failed(Source::Protondb, e, None, now);
            }
            Answer::Rating(_, Err(e)) => store.failed(Source::Protondb, e.to_string(), None, now),
            Answer::Current(id, Ok(players::Fetched::Value(count))) => {
                store.worked(Source::Steam);
                store.current.insert(
                    id,
                    Cached {
                        value: Some(count),
                        at: now,
                    },
                );
                players::record(store.samples.entry(id).or_default(), now, count);
                done.changed = true;
            }
            Answer::Current(id, Ok(players::Fetched::NotCounted)) => {
                store.current.insert(
                    id,
                    Cached {
                        value: None,
                        at: now,
                    },
                );
                done.changed = true;
            }
            Answer::Current(_, Ok(players::Fetched::Later(wait))) => {
                store.failed(Source::Steam, "asked to wait".into(), wait, now);
            }
            Answer::Current(_, Ok(players::Fetched::Failed(e))) => {
                store.failed(Source::Steam, e, None, now);
            }
            Answer::Current(_, Err(e)) => store.failed(Source::Steam, e.to_string(), None, now),
        }
    }

    // Games gone from the library for a month are forgotten.
    let gone: Vec<u32> = store
        .last_seen
        .iter()
        .filter(|(_, at)| now.saturating_sub(**at) > FORGET_AFTER)
        .map(|(id, _)| *id)
        .collect();
    for id in gone {
        store.last_seen.remove(&id);
        store.protondb.remove(&id);
        store.current.remove(&id);
        store.samples.remove(&id);
        store.anticheat_seen.remove(&id);
        store.native.remove(&id);
        done.changed = true;
    }
    let gone: Vec<String> = store
        .titles_seen
        .iter()
        .filter(|(_, at)| now.saturating_sub(**at) > FORGET_AFTER)
        .map(|(t, _)| t.clone())
        .collect();
    for t in gone {
        store.titles_seen.remove(&t);
        store.steam_by_title.remove(&t);
        done.changed = true;
    }
    if let Err(e) = store.save(dir) {
        tracing::warn!(target: "game_info", error = %format!("{e:#}"), "the game information could not be kept");
    }
    (snap, done)
}

/// Look for the Steam version of the games of `games` another launcher
/// installed and that are not Linux builds, by title: those never looked
/// for, and those whose answer aged (a month when found, a week when not),
/// a few in a pass, one after the other; a failure stops the rest.
fn search_titles(
    store: &mut Store,
    games: &[GameRef],
    wants: Wants,
    stop: &AtomicBool,
    net: &Net,
    now: u64,
    done: &mut Refreshed,
) {
    let mut titles: Vec<(String, &str)> = games
        .iter()
        .filter(|g| g.searched())
        .map(|g| (anticheat::normalize(&g.title), g.title.as_str()))
        .filter(|(key, _)| !key.is_empty())
        .collect();
    titles.sort_by(|a, b| a.0.cmp(&b.0));
    titles.dedup_by(|a, b| a.0 == b.0);
    for (key, _) in &titles {
        store.titles_seen.insert(key.clone(), now);
    }
    if !wants.any() || store.resting(Source::SteamSearch, now) {
        return;
    }
    let due: Vec<(String, &str)> = titles
        .into_iter()
        .filter(|(key, _)| {
            store.steam_by_title.get(key).is_none_or(|c| {
                let ttl = if c.value.is_some() {
                    SEARCH_FOUND_TTL
                } else {
                    SEARCH_NONE_TTL
                };
                wants.now || c.stale(ttl, now)
            })
        })
        .take(MAX_SEARCHES_PER_PASS)
        .collect();
    for (key, title) in due {
        if stop.load(Ordering::Relaxed) {
            return;
        }
        match (net.search)(title) {
            Ok(steam_search::Fetched::Found(found)) => {
                store.worked(Source::SteamSearch);
                store.steam_by_title.insert(
                    key,
                    Cached {
                        value: found,
                        at: now,
                    },
                );
                done.changed = true;
            }
            Ok(steam_search::Fetched::Later(wait)) => {
                store.failed(Source::SteamSearch, "asked to wait".into(), wait, now);
                return;
            }
            Ok(steam_search::Fetched::Failed(e)) => {
                store.failed(Source::SteamSearch, e, None, now);
                return;
            }
            Err(e) => {
                store.failed(Source::SteamSearch, e.to_string(), None, now);
                return;
            }
        }
    }
}

/// Forget everything downloaded: the next pass fetches it again.
///
/// # Errors
/// Returns an error when the folder cannot be removed.
pub fn clear() -> Result<()> {
    match std::fs::remove_dir_all(dir()) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
            Err(e).with_context(|| format!("remove {}", dir().display()))
        }
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests;
