//! The game information the library shows, and when it is asked for again.
//!
//! What is kept is read once, off the main thread, and shown at once; the
//! library never waits for a network. A pass ([`request`]) asks the sources
//! only for what aged past what each allows, in a worker thread, and only
//! when it may: the setting is on, the library is on screen (the page asks),
//! and no game is running. A game that starts stops a pass between two
//! requests ([`stop`]); nothing is asked while it runs.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use gtk4::{gio, glib};

use bigame_core::game_info::{Change, GameRef, Snapshot, Wants};

type Listener = Box<dyn Fn(&Rc<Snapshot>, &[Change]) -> glib::ControlFlow>;

thread_local! {
    static SNAPSHOT: RefCell<Rc<Snapshot>> = RefCell::new(Rc::new(Snapshot::default()));
    static LOADED: Cell<bool> = const { Cell::new(false) };
    static LISTENERS: RefCell<Vec<Listener>> = const { RefCell::new(Vec::new()) };
    /// The pass running, by its stop flag.
    static RUNNING: RefCell<Option<Arc<AtomicBool>>> = const { RefCell::new(None) };
    static WATCHING: Cell<bool> = const { Cell::new(false) };
    /// The library's games, as the library page last asked about them.
    static LIBRARY: RefCell<Vec<GameRef>> = const { RefCell::new(Vec::new()) };
}

/// What is known now (empty until the kept information is read).
#[must_use]
pub fn snapshot() -> Rc<Snapshot> {
    SNAPSHOT.with(|s| Rc::clone(&s.borrow()))
}

/// Be told whenever what is known changes, with the anti-cheat changes a
/// new list brought, until the listener returns
/// [`glib::ControlFlow::Break`].
pub fn subscribe(f: impl Fn(&Rc<Snapshot>, &[Change]) -> glib::ControlFlow + 'static) {
    LISTENERS.with(|l| l.borrow_mut().push(Box::new(f)));
}

fn publish(snap: Snapshot, changes: &[Change]) {
    let snap = Rc::new(snap);
    SNAPSHOT.with(|s| *s.borrow_mut() = Rc::clone(&snap));
    LISTENERS.with(|l| l.borrow_mut().retain(|f| f(&snap, changes).is_continue()));
}

/// Read what is kept, once, off the main thread.
pub fn load() {
    watch_games();
    if LOADED.with(|l| l.replace(true)) {
        return;
    }
    glib::spawn_future_local(async move {
        if let Ok(snap) = gio::spawn_blocking(Snapshot::load).await {
            publish(snap, &[]);
        }
    });
}

/// What the settings let a pass ask for.
#[must_use]
pub fn wants() -> Wants {
    let s = crate::settings::load();
    if !s.game_info_online {
        return Wants::default();
    }
    Wants {
        protondb: s.show_linux_rating,
        anticheat: s.show_anticheat,
        players: s.show_players,
        now: false,
    }
}

/// The library's games, as the library page last listed them.
pub fn set_library(games: Vec<GameRef>) {
    LIBRARY.with(|l| *l.borrow_mut() = games);
}

/// Ask about every game of the library again, whatever the age of what is
/// kept ("Update").
pub fn update_library() {
    let games = LIBRARY.with(|l| l.borrow().clone());
    request(games, true);
}

/// Ask the sources about `games`, if a pass may run now and none is;
/// `now`: whatever the age of what is kept.
pub fn request(games: Vec<GameRef>, now: bool) {
    let wants = Wants { now, ..wants() };
    if games.is_empty() || crate::game_watch::current().is_some() {
        return;
    }
    if RUNNING.with(|r| r.borrow().is_some()) {
        return;
    }
    let stop = Arc::new(AtomicBool::new(false));
    RUNNING.with(|r| *r.borrow_mut() = Some(Arc::clone(&stop)));
    glib::spawn_future_local(async move {
        let flag = Arc::clone(&stop);
        let pass =
            gio::spawn_blocking(move || bigame_core::game_info::refresh(&games, wants, &flag))
                .await;
        RUNNING.with(|r| r.borrow_mut().take());
        let Ok((snap, done)) = pass else {
            return;
        };
        for c in &done.changes {
            tracing::info!(target: "game_info", game = %c.title, before = ?c.before, after = ?c.after, "a game's anti-cheat status changed in the list");
        }
        // A pass stopped for a game still keeps what it learnt.
        publish(snap, &done.changes);
    });
}

/// Stop the pass running, between two requests.
pub fn stop() {
    RUNNING.with(|r| {
        if let Some(flag) = r.borrow().as_ref() {
            flag.store(true, Ordering::Relaxed);
        }
    });
}

/// Forget everything downloaded, and show what is left (nothing but what
/// this machine says).
pub fn clear() {
    stop();
    glib::spawn_future_local(async move {
        let cleared = gio::spawn_blocking(|| {
            let r = bigame_core::game_info::clear();
            (r, Snapshot::load())
        })
        .await;
        if let Ok((r, snap)) = cleared {
            if let Err(e) = r {
                tracing::warn!(target: "game_info", error = %format!("{e:#}"), "the game information could not be cleared");
            }
            publish(snap, &[]);
        }
    });
}

/// A game that starts stops a pass.
fn watch_games() {
    if WATCHING.with(|w| w.replace(true)) {
        return;
    }
    crate::game_watch::subscribe(|game| {
        if game.is_some() {
            stop();
        }
        glib::ControlFlow::Continue
    });
}
