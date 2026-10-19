//! Say when a game closes with an error, and what was in it.
//!
//! A game's start and end come from the game watch ([`crate::game_watch`]).
//! When it starts, what its environment asks for is read once; a minute and
//! a half in, which layers it really loaded, once more. Nothing is read in
//! between: the game has the machine. When it is gone, its crash reporter
//! gets a few seconds to write, and then what a crash leaves behind is looked
//! for ([`bigame_core::game_crash::examine`]).
//!
//! A crash is said in the window's notice ([`crate::widgets::crash_notice`])
//! and, with the window hidden in the tray, by the desktop, with the same
//! choice: leave the extras out of that game from its next start. Nothing
//! changes until that is chosen.

use std::cell::{Cell, RefCell};
use std::os::fd::{AsFd as _, OwnedFd};
use std::time::SystemTime;

use adw::prelude::*;
use gtk4::{gio, glib};
use libadwaita as adw;

use bigame_core::game_crash::{self, Crash, Session};
use bigame_core::running::GameIdentity;
use bigame_core::text::Text;

use crate::i18n::{error_text, i18n, labelled, ni18n, tr};

/// The desktop notification's id, so a newer crash replaces it.
const NOTIFICATION_ID: &str = "game-crash";

/// How long into a game its layers are read: they are mapped once its
/// graphics start.
const LOOK_INSIDE_AFTER: u32 = 90;

/// How long a crash reporter gets to write once the game is gone.
const REPORTER_GRACE: u32 = 3;

/// What the notice hears about.
pub enum Event<'a> {
    /// A game closed with an error; how many times it did this week.
    Crashed(&'a Crash, usize),
    /// The extras are being left out.
    Busy,
    /// Leaving them out is done (`ok`), or failed, as `text` says.
    LeftOut { ok: bool, text: &'a str },
}

type Listener = Box<dyn Fn(&Event) -> glib::ControlFlow>;

thread_local! {
    /// The game being watched, from the moment it is found.
    static WATCHED: Cell<Option<u32>> = const { Cell::new(None) };
    static SESSION: RefCell<Option<Session>> = const { RefCell::new(None) };
    /// The game's pidfd, held from the moment it is found: the kernel
    /// tells how the process ended to whoever holds one.
    static PIDFD: RefCell<Option<(u32, OwnedFd)>> = const { RefCell::new(None) };
    /// The last crash, until it is dismissed or dealt with.
    static LAST: RefCell<Option<(Crash, usize)>> = const { RefCell::new(None) };
    static LISTENERS: RefCell<Vec<Listener>> = const { RefCell::new(Vec::new()) };
    static BUSY: Cell<bool> = const { Cell::new(false) };
}

/// Register the notification's action and start watching games.
pub fn install(app: &adw::Application) {
    let action = gio::SimpleAction::new("crash-leave-out", Some(glib::VariantTy::STRING));
    action.connect_activate(|_, param| {
        if let Some(process) = param.and_then(glib::Variant::str) {
            leave_out(process);
        }
    });
    app.add_action(&action);
    crate::game_watch::subscribe(|game| {
        changed(game);
        glib::ControlFlow::Continue
    });
}

/// The last crash not yet dismissed, and how many times its game crashed
/// this week.
#[must_use]
pub fn last() -> Option<(Crash, usize)> {
    LAST.with(|l| l.borrow().clone())
}

/// Be told of crashes and of what leaving the extras out did, until the
/// listener returns [`glib::ControlFlow::Break`].
pub fn subscribe(listener: impl Fn(&Event) -> glib::ControlFlow + 'static) {
    LISTENERS.with(|l| l.borrow_mut().push(Box::new(listener)));
}

fn emit(event: &Event) {
    LISTENERS.with(|l| l.borrow_mut().retain(|f| f(event).is_continue()));
}

/// Forget the last crash: it is not shown again.
pub fn dismiss() {
    LAST.with(|l| l.borrow_mut().take());
    if let Some(app) = gio::Application::default() {
        app.withdraw_notification(NOTIFICATION_ID);
    }
}

/// The running game changed.
fn changed(game: Option<&GameIdentity>) {
    let pid = game.map(|g| g.pid);
    // The watch reports a game again when it learns its graphics.
    if WATCHED.with(Cell::get) == pid {
        return;
    }
    WATCHED.with(|w| w.set(pid));
    if let Some(old) = SESSION.with(|s| s.borrow_mut().take()) {
        finish(old);
    }
    let Some(game) = game.cloned() else {
        return;
    };
    if let Some(fd) = bigame_core::processes::exit_watch(game.pid) {
        PIDFD.with(|p| *p.borrow_mut() = Some((game.pid, fd)));
    }
    glib::spawn_future_local(async move {
        let found = game.clone();
        let Ok(session) = gio::spawn_blocking(move || Session::begin(&found)).await else {
            return;
        };
        if WATCHED.with(Cell::get) != Some(game.pid) {
            // Gone while its start was read.
            finish(session);
            return;
        }
        SESSION.with(|s| *s.borrow_mut() = Some(session));
        glib::timeout_future_seconds(LOOK_INSIDE_AFTER).await;
        let Some(mut copy) = SESSION.with(|s| s.borrow().clone().filter(|s| s.pid == game.pid))
        else {
            return;
        };
        let Some(current) = crate::game_watch::current().filter(|c| c.pid == game.pid) else {
            return;
        };
        let Ok(copy) = gio::spawn_blocking(move || {
            copy.look_inside(&current);
            copy
        })
        .await
        else {
            return;
        };
        SESSION.with(|s| {
            let mut s = s.borrow_mut();
            if s.as_ref().is_some_and(|kept| kept.pid == copy.pid) {
                *s = Some(copy);
            }
        });
    });
}

/// The game of `session` is gone: look for what a crash leaves.
fn finish(session: Session) {
    let ended = SystemTime::now();
    let pidfd = PIDFD
        .with(|p| p.borrow_mut().take_if(|(pid, _)| *pid == session.pid))
        .map(|(_, fd)| fd);
    glib::spawn_future_local(async move {
        glib::timeout_future_seconds(REPORTER_GRACE).await;
        let game = session.game.clone();
        let found = gio::spawn_blocking(move || {
            // Read once its parent has reaped it, which the wait above
            // nearly always covers.
            let how = pidfd.as_ref().and_then(|fd| {
                (0..10).find_map(|_| {
                    bigame_core::processes::ended(fd.as_fd()).or_else(|| {
                        std::thread::sleep(std::time::Duration::from_millis(200));
                        None
                    })
                })
            });
            tracing::debug!(game = %session.game, ?how, "the game's process ended");
            let crash = game_crash::examine(&session, ended, how)?;
            if let Err(e) = game_crash::record(&crash) {
                tracing::warn!(error = %format!("{e:#}"), "the crash could not be recorded");
            }
            let week = game_crash::crashes_of(&crash.process, 7, SystemTime::now());
            Some((crash, week))
        })
        .await
        .ok()
        .flatten();
        let Some((crash, week)) = found else {
            tracing::debug!(%game, "the game left no sign of a crash");
            return;
        };
        tracing::warn!(
            game = %crash.game,
            played_secs = crash.played_secs,
            evidence = %crash.evidence.english(),
            reason = ?crash.reason,
            report = ?crash.report,
            extras = ?crash.extras,
            "the game closed with an error"
        );
        LAST.with(|l| *l.borrow_mut() = Some((crash.clone(), week)));
        emit(&Event::Crashed(&crash, week));
        notify(&crash, week);
    });
}

/// Whether the window is on screen; hidden in the tray, only the desktop's
/// notifications reach the user.
fn window_shown() -> bool {
    gio::Application::default()
        .and_downcast::<gtk4::Application>()
        .and_then(|a| a.active_window())
        .is_some_and(|w| w.is_visible())
}

/// With the window hidden, the desktop says it, with the same choice.
fn notify(crash: &Crash, week: usize) {
    let Some(app) = gio::Application::default() else {
        return;
    };
    // "<game> closed" would contradict it.
    app.withdraw_notification("game-exit");
    if window_shown() || !crate::settings::load().notifications_enabled {
        return;
    }
    let n = gio::Notification::new(&title(crash));
    n.set_body(Some(&body(crash, week)));
    n.set_priority(gio::NotificationPriority::High);
    if !crash.removable().is_empty() {
        n.add_button_with_target_value(
            &i18n("Leave them out of this game"),
            "app.crash-leave-out",
            Some(&crash.process.to_variant()),
        );
    }
    app.send_notification(Some(NOTIFICATION_ID), &n);
}

/// The notice's and the notification's title.
#[must_use]
pub fn title(crash: &Crash) -> String {
    i18n("%s closed with an error").replace("%s", &crash.game)
}

/// How long `secs` of play were, as people say it.
fn played(secs: u64) -> String {
    let n = |v: u64| usize::try_from(v).unwrap_or(usize::MAX);
    match secs {
        0..60 => ni18n("%n second", "%n seconds", n(secs)),
        60..3600 => ni18n("%n minute", "%n minutes", n(secs / 60)),
        _ => Text::fill(
            &i18n("%s h %s min"),
            &[(secs / 3600).to_string(), (secs % 3600 / 60).to_string()],
        ),
    }
}

/// What the notice and the notification say under the title.
#[must_use]
pub fn body(crash: &Crash, week: usize) -> String {
    let mut lines = Vec::new();
    let evidence = tr(&crash.evidence);
    let mut first = i18n("After %s of play.").replace("%s", &played(crash.played_secs));
    first.push(' ');
    first.push_str(&match &crash.reason {
        Some(reason) => labelled(&evidence, reason),
        None => evidence,
    });
    lines.push(first);
    if week > 1 {
        lines.push(ni18n(
            "This game closed with an error %n time this week.",
            "This game closed with an error %n times this week.",
            week,
        ));
    }
    if crash.extras.is_empty() {
        lines.push(i18n(
            "Nothing was added to the game: the error is the game's own, or its drivers' or the system's.",
        ));
    } else {
        let names: Vec<String> = crash.extras.iter().map(|e| i18n(e.label())).collect();
        lines.push(labelled(&i18n("Also in the game"), &names.join(", ")));
        if crash.removable().is_empty() {
            lines.push(i18n(
                "Each is the game's own choice in Profiles or AI Graphics; changed there, it is left out from the next start.",
            ));
        } else {
            lines.push(i18n(
                "They may have nothing to do with it. Leaving them out of this game for a while tells whether they do.",
            ));
        }
    }
    lines.join("\n")
}

/// Leave the extras of the last crash out of the game `process`, off the
/// main thread, and say how it went where it was asked.
pub fn leave_out(process: &str) {
    let Some((crash, _)) = last().filter(|(c, _)| c.process == process) else {
        return;
    };
    if BUSY.with(|b| b.replace(true)) {
        return;
    }
    if let Some(app) = gio::Application::default() {
        app.withdraw_notification(NOTIFICATION_ID);
    }
    emit(&Event::Busy);
    glib::spawn_future_local(async move {
        let (process, extras) = (crash.process.clone(), crash.removable());
        let done = gio::spawn_blocking(move || game_crash::leave_out(&process, &extras)).await;
        BUSY.with(|b| b.set(false));
        let (ok, text) = match done {
            Ok(Ok(left)) => outcome(&crash, &left),
            Ok(Err(e)) => {
                tracing::warn!(game = %crash.process, error = %format!("{e:#}"), "the extras could not be left out");
                (false, error_text(&e))
            }
            Err(_) => (false, i18n("the worker thread stopped")),
        };
        if ok {
            LAST.with(|l| l.borrow_mut().take());
        }
        emit(&Event::LeftOut { ok, text: &text });
        if !window_shown()
            && let Some(app) = gio::Application::default()
        {
            let n = gio::Notification::new(&if ok {
                i18n("Left out of %s").replace("%s", &crash.game)
            } else {
                i18n("Not everything was left out of %s").replace("%s", &crash.game)
            });
            n.set_body(Some(&text));
            app.send_notification(Some(NOTIFICATION_ID), &n);
        }
    });
}

/// What [`game_crash::leave_out`] did, said; `false` when something still
/// stands in the way.
fn outcome(crash: &Crash, left: &game_crash::LeftOut) -> (bool, String) {
    use bigame_core::heroic_launch::Applied as Heroic;
    use bigame_core::steam_gamescope::Applied as Steam;
    let names: Vec<String> = left.off.iter().map(|e| i18n(e.label())).collect();
    let mut lines = vec![Text::fill(
        &i18n("From its next start, %s runs without: %s."),
        &[crash.game.clone(), names.join(", ")],
    )];
    let mut ok = true;
    match &left.steam {
        Some(Ok(Steam::Written(options) | Steam::TheirGamescope(options))) => {
            lines.push(labelled(&i18n("Steam launch options"), options));
        }
        Some(Ok(Steam::SteamRunning)) => {
            ok = false;
            lines.push(i18n(
                "Launch options: close Steam first: it keeps them in memory and would overwrite the change.",
            ));
        }
        Some(Err(e)) => {
            ok = false;
            lines.push(labelled(&i18n("Launch options"), &error_text(e)));
        }
        _ => {}
    }
    match &left.heroic {
        Some(Ok(Heroic::HeroicRunning { .. })) => {
            ok = false;
            lines.push(i18n(
                "Heroic is running a game: close the game first. Heroic keeps this game's settings in memory and would discard the change.",
            ));
        }
        Some(Err(e)) => {
            ok = false;
            lines.push(labelled("Heroic", &error_text(e)));
        }
        _ => {}
    }
    if !ok {
        lines.push(i18n(
            "It is kept in the game's settings and written once that is possible.",
        ));
    }
    lines.push(i18n(
        "To put them back: Profiles → the game → its launch settings.",
    ));
    (ok, lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_time_played_is_said_in_the_largest_whole_unit() {
        assert_eq!(played(42), "42 seconds");
        assert_eq!(played(1524), "25 minutes");
        assert_eq!(played(3600 + 5 * 60 + 7), "1 h 5 min");
    }
}
