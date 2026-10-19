//! A pass over a library, against sources that answer as told.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use super::*;

const NOW: u64 = 1_792_380_000;

fn steam(id: u32, title: &str) -> GameRef {
    GameRef {
        steam: Some(id),
        title: title.into(),
    }
}

fn summary(tier: protondb::Tier) -> protondb::Rating {
    protondb::Rating::Summary(protondb::Summary {
        tier: Some(tier),
        trending: None,
        best: None,
        reports: 100,
        confidence: Some("strong".into()),
    })
}

/// A list as the anti-cheat project writes it, with Battlefield 6 at
/// `status`.
fn list(status: &str) -> String {
    format!(
        r#"[{{"slug":"battlefield-6","name":"Battlefield 6","status":"{status}","anticheats":["EA anticheat"],"notes":[],"updates":[],"storeIds":{{"steam":"2807960"}},"dateChanged":"2025-07-16T20:26:00Z"}}]"#
    )
}

/// Sources that answer from what the test sets, counting the requests.
struct Fake {
    calls: Arc<AtomicUsize>,
    offline: Arc<AtomicBool>,
    list: Arc<Mutex<String>>,
}

impl Fake {
    fn new() -> Self {
        Self {
            calls: Arc::new(AtomicUsize::new(0)),
            offline: Arc::new(AtomicBool::new(false)),
            list: Arc::new(Mutex::new(list("Denied"))),
        }
    }

    fn net(&self) -> Net {
        let (c1, c2, c3, c4) = (
            Arc::clone(&self.calls),
            Arc::clone(&self.calls),
            Arc::clone(&self.calls),
            Arc::clone(&self.calls),
        );
        let (o1, o2, o3, o4) = (
            Arc::clone(&self.offline),
            Arc::clone(&self.offline),
            Arc::clone(&self.offline),
            Arc::clone(&self.offline),
        );
        let text = Arc::clone(&self.list);
        Net {
            rating: Box::new(move |id| {
                c1.fetch_add(1, Ordering::SeqCst);
                if o1.load(Ordering::SeqCst) {
                    return Err(Failure::Offline);
                }
                Ok(match id {
                    1_091_500 => protondb::Fetched::Rating(summary(protondb::Tier::Gold)),
                    2_807_960 => protondb::Fetched::Rating(summary(protondb::Tier::Borked)),
                    429 => protondb::Fetched::Later(Some(std::time::Duration::from_secs(600))),
                    _ => protondb::Fetched::Rating(protondb::Rating::NoReports),
                })
            }),
            current: Box::new(move |id| {
                c2.fetch_add(1, Ordering::SeqCst);
                if o2.load(Ordering::SeqCst) {
                    return Err(Failure::Offline);
                }
                Ok(if id == 9 {
                    players::Fetched::NotCounted
                } else {
                    players::Fetched::Value(u64::from(id) % 1000)
                })
            }),
            charts: Box::new(move || {
                c3.fetch_add(1, Ordering::SeqCst);
                if o3.load(Ordering::SeqCst) {
                    return Err(Failure::Offline);
                }
                Ok(players::Fetched::Value(players::Charts {
                    updated: NOW,
                    games: [(1_091_500, (32_407, 67_873))].into_iter().collect(),
                }))
            }),
            dataset: Box::new(move |file, _etag| {
                c4.fetch_add(1, Ordering::SeqCst);
                if o4.load(Ordering::SeqCst) {
                    return Err(Failure::Offline);
                }
                write_bytes(file, text.lock().unwrap().as_bytes()).unwrap();
                Ok(anticheat::Fetched::Updated)
            }),
            linux: Box::new(|ids| ids.iter().copied().filter(|id| *id == 105_600).collect()),
        }
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

fn all() -> Wants {
    Wants {
        protondb: true,
        anticheat: true,
        players: true,
        now: false,
    }
}

fn library() -> Vec<GameRef> {
    vec![
        steam(1_091_500, "Cyberpunk 2077"),
        steam(2_807_960, "Battlefield 6"),
        steam(105_600, "Terraria"),
        GameRef {
            steam: None,
            title: "SuperTuxKart".into(),
        },
    ]
}

#[test]
fn a_pass_learns_each_answer_and_the_next_asks_only_what_aged() {
    let d = tempfile::tempdir().unwrap();
    let fake = Fake::new();
    let stop = AtomicBool::new(false);
    let (snap, done) = refresh_in(d.path(), &library(), all(), &stop, &fake.net(), NOW);
    assert!(done.changed);
    // A list, a chart, two ratings (Terraria is native), three counts.
    assert_eq!(fake.calls(), 1 + 1 + 2 + 3);
    let cp = snap.info(&steam(1_091_500, "Cyberpunk 2077"), NOW);
    assert!(matches!(cp.linux, Linux::Rating { stale: false, .. }));
    let p = cp.players.unwrap();
    assert_eq!(p.now, Some((500, NOW)));
    assert_eq!(
        p.peak,
        Peak::SteamToday {
            peak: 67_873,
            at: NOW
        }
    );
    let bf = snap.info(&steam(2_807_960, "Battlefield 6"), NOW);
    assert!(matches!(
        bf.anticheat,
        AntiCheat::Listed { ref game, .. } if game.status == anticheat::Status::Denied
    ));
    assert_eq!(
        snap.info(&steam(105_600, "Terraria"), NOW).linux,
        Linux::Native
    );
    let stk = snap.info(&library()[3], NOW);
    assert_eq!(stk.linux, Linux::NoSteamId);
    assert_eq!(stk.players, None);
    assert_eq!(stk.anticheat, AntiCheat::NotListed);

    // Ten minutes later: nothing is old enough to ask again.
    let before = fake.calls();
    let _ = refresh_in(d.path(), &library(), all(), &stop, &fake.net(), NOW + 600);
    assert_eq!(fake.calls(), before);
    // Twenty minutes: the counts and the chart, nothing else.
    let _ = refresh_in(d.path(), &library(), all(), &stop, &fake.net(), NOW + 1200);
    assert_eq!(fake.calls(), before + 1 + 3);
    // "Update": everything again, whatever its age.
    let before = fake.calls();
    let now = Wants { now: true, ..all() };
    let _ = refresh_in(d.path(), &library(), now, &stop, &fake.net(), NOW + 1201);
    assert_eq!(fake.calls(), before + 1 + 1 + 2 + 3);
}

#[test]
fn offline_keeps_what_is_known_and_leaves_the_sources_alone_for_a_while() {
    let d = tempfile::tempdir().unwrap();
    let fake = Fake::new();
    let stop = AtomicBool::new(false);
    let _ = refresh_in(d.path(), &library(), all(), &stop, &fake.net(), NOW);
    fake.offline.store(true, Ordering::SeqCst);
    let later = NOW + 2 * PROTONDB_TTL;
    let (snap, _) = refresh_in(d.path(), &library(), all(), &stop, &fake.net(), later);
    // What was known stays, said to be old.
    let cp = snap.info(&steam(1_091_500, "Cyberpunk 2077"), later);
    assert!(matches!(cp.linux, Linux::Rating { stale: true, .. }));
    assert!(snap.store.resting(Source::Protondb, later));
    // A game never fetched is unavailable, not borked.
    let new = snap.info(&steam(1, "New"), later);
    assert_eq!(new.linux, Linux::Unavailable);
    // Back online, but the pause is not over: nothing is asked.
    fake.offline.store(false, Ordering::SeqCst);
    let calls = fake.calls();
    let _ = refresh_in(d.path(), &library(), all(), &stop, &fake.net(), later + 1);
    assert_eq!(fake.calls(), calls);
}

#[test]
fn a_source_that_asks_to_wait_is_left_alone_as_long_as_it_asks() {
    let d = tempfile::tempdir().unwrap();
    let fake = Fake::new();
    let stop = AtomicBool::new(false);
    let games = vec![steam(429, "Busy")];
    let wants = Wants {
        protondb: true,
        ..Wants::default()
    };
    let (snap, _) = refresh_in(d.path(), &games, wants, &stop, &fake.net(), NOW);
    let t = &snap.store.trouble[&Source::Protondb];
    assert_eq!(t.until, NOW + 600);
    assert!(snap.store.resting(Source::Protondb, NOW + 599));
    assert!(!snap.store.resting(Source::Protondb, NOW + 600));
}

#[test]
fn a_change_of_anti_cheat_status_is_said_once_between_two_lists() {
    let d = tempfile::tempdir().unwrap();
    let fake = Fake::new();
    let stop = AtomicBool::new(false);
    let wants = Wants {
        anticheat: true,
        ..Wants::default()
    };
    let (_, done) = refresh_in(d.path(), &library(), wants, &stop, &fake.net(), NOW);
    assert!(done.changes.is_empty(), "the first list is no change");
    *fake.list.lock().unwrap() = list("Running");
    let at = NOW + ANTICHEAT_TTL;
    let (_, done) = refresh_in(d.path(), &library(), wants, &stop, &fake.net(), at);
    assert_eq!(
        done.changes,
        [Change {
            title: "Battlefield 6".into(),
            steam: 2_807_960,
            before: anticheat::Status::Denied,
            after: anticheat::Status::Running,
        }]
    );
    // Offline later: no list, no change, nothing said again.
    fake.offline.store(true, Ordering::SeqCst);
    let (_, done) = refresh_in(
        d.path(),
        &library(),
        wants,
        &stop,
        &fake.net(),
        at + ANTICHEAT_TTL,
    );
    assert!(done.changes.is_empty());
}

#[test]
fn nothing_is_asked_once_told_to_stop_or_with_every_source_off() {
    let d = tempfile::tempdir().unwrap();
    let fake = Fake::new();
    let _ = refresh_in(
        d.path(),
        &library(),
        all(),
        &AtomicBool::new(true),
        &fake.net(),
        NOW,
    );
    assert_eq!(fake.calls(), 0);
    let _ = refresh_in(
        d.path(),
        &library(),
        Wants::default(),
        &AtomicBool::new(false),
        &fake.net(),
        NOW,
    );
    assert_eq!(fake.calls(), 0);
}

#[test]
fn a_damaged_store_is_set_aside_and_started_again() {
    let d = tempfile::tempdir().unwrap();
    std::fs::write(store_path(d.path()), b"{ not json").unwrap();
    assert_eq!(Store::load(d.path()), Store::default());
    assert!(d.path().join("store.json.bad").is_file());
    // A store from a newer version is not read as this one.
    std::fs::write(store_path(d.path()), br#"{"version":99}"#).unwrap();
    assert_eq!(Store::load(d.path()), Store::default());
}

#[test]
fn a_game_gone_from_the_library_for_a_month_is_forgotten() {
    let d = tempfile::tempdir().unwrap();
    let fake = Fake::new();
    let stop = AtomicBool::new(false);
    let wants = Wants {
        protondb: true,
        ..Wants::default()
    };
    let _ = refresh_in(d.path(), &library(), wants, &stop, &fake.net(), NOW);
    let later = NOW + FORGET_AFTER + 1;
    let (snap, _) = refresh_in(d.path(), &[], wants, &stop, &fake.net(), later);
    assert!(snap.store.protondb.is_empty());
    assert!(snap.store.last_seen.is_empty());
}

#[test]
fn backing_off_doubles_up_to_six_hours() {
    assert!((300..360).contains(&backoff(1, NOW)));
    assert!((600..660).contains(&backoff(2, NOW)));
    assert!((6 * 3600..6 * 3600 + 60).contains(&backoff(30, NOW)));
}

#[test]
fn only_a_steam_installed_game_has_a_steam_id() {
    let mut g = crate::games::DetectedGame {
        name: "Cyberpunk 2077".into(),
        source: crate::games::Source::Steam,
        app_id: Some("1091500".into()),
        install_path: None,
        executables: Vec::new(),
        launch_file: None,
        cover: None,
        icon: None,
        launch_command: None,
        launcher: None,
    };
    assert_eq!(GameRef::of(&g).steam, Some(1_091_500));
    g.source = crate::games::Source::Flatpak;
    g.app_id = Some("com.example.Game".into());
    assert_eq!(GameRef::of(&g).steam, None);
    g.source = crate::games::Source::Heroic;
    g.app_id = Some("1091500".into());
    assert_eq!(
        GameRef::of(&g).steam,
        None,
        "another store's number is not a Steam id"
    );
}
