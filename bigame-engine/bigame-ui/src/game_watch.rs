//! Which game is running, for everything in the UI that cares.
//!
//! One watcher for the whole application, alive for as long as the
//! application is (it keeps running in the background with its window
//! closed). Home shows the game; the profile offer asks about it.
//!
//! Event first, poll second. falcond rewrites its status file whenever it
//! activates or drops a profile — and its generic Proton profile matches any
//! Windows game — so a file monitor on it catches most games the moment they
//! start. A slow poll catches the rest (native games falcond has no profile
//! for). Each check is one fork-free `/proc` walk, about 9 ms.

use std::cell::RefCell;
use std::rc::Rc;

use gtk4::gio;
use gtk4::glib;
use gtk4::prelude::*;

use bigame_core::running::GameIdentity;

/// How often to look when nothing has signalled a change.
const POLL: std::time::Duration = std::time::Duration::from_secs(5);

/// Returns [`glib::ControlFlow::Break`] to stop listening.
type Listener = Box<dyn Fn(Option<&GameIdentity>) -> glib::ControlFlow>;

#[derive(Default)]
struct Watch {
    current: RefCell<Option<GameIdentity>>,
    listeners: RefCell<Vec<Listener>>,
    checking: std::cell::Cell<bool>,
    // Kept alive: dropping the monitor stops the events.
    monitor: RefCell<Option<gio::FileMonitor>>,
}

thread_local! {
    static WATCH: Rc<Watch> = Rc::new(Watch::default());
}

/// Start watching. Idempotent; call from the main thread.
pub fn start() {
    WATCH.with(|watch| {
        if watch.monitor.borrow().is_some() {
            return;
        }
        let file = gio::File::for_path(bigame_core::status::status_path());
        if let Ok(monitor) = file.monitor_file(gio::FileMonitorFlags::NONE, gio::Cancellable::NONE)
        {
            monitor.connect_changed(|_, _, _, event| {
                if matches!(
                    event,
                    gio::FileMonitorEvent::ChangesDoneHint | gio::FileMonitorEvent::Created
                ) {
                    check();
                }
            });
            *watch.monitor.borrow_mut() = Some(monitor);
        }
        glib::timeout_add_local(POLL, || {
            check();
            glib::ControlFlow::Continue
        });
        glib::idle_add_local_once(check);
    });
}

/// Look now.
pub fn check() {
    WATCH.with(|watch| {
        if watch.checking.replace(true) {
            return;
        }
        let watch = Rc::clone(watch);
        glib::spawn_future_local(async move {
            let found = gio::spawn_blocking(bigame_core::running::detect)
                .await
                .ok()
                .flatten();
            watch.checking.set(false);
            let previous = watch.current.borrow().clone();
            let (found, changed) = merge(previous.as_ref(), found);
            if !changed {
                return;
            }
            match (&found, &previous) {
                (Some(g), Some(p)) if p.pid == g.pid => {
                    tracing::debug!(pid = g.pid, graphics = ?g.graphics, card = ?g.render_card, "game updated");
                }
                (Some(g), _) => {
                    tracing::info!(game = %g.display_name, process = %g.process_name, pid = g.pid, "game detected");
                }
                (None, _) => tracing::info!("game no longer running"),
            }
            *watch.current.borrow_mut() = found;
            let current = watch.current.borrow();
            watch
                .listeners
                .borrow_mut()
                .retain(|listener| listener(current.as_ref()).is_continue());
        });
    });
}

/// What the watch holds after a check found `found`, and whether that is a
/// change to pass on.
///
/// A game caught the moment it starts has not mapped its graphics DLLs nor
/// submitted GPU work yet; learning either later is a change. The card it
/// renders on, once learned, is kept when a later check of the same process
/// cannot tell.
fn merge(
    current: Option<&GameIdentity>,
    mut found: Option<GameIdentity>,
) -> (Option<GameIdentity>, bool) {
    if let (Some(cur), Some(new)) = (current, found.as_mut()) {
        if cur.pid == new.pid && new.render_card.is_none() {
            new.render_card.clone_from(&cur.render_card);
        }
    }
    let key = |g: Option<&GameIdentity>| g.map(|g| (g.pid, g.graphics, g.render_card.clone()));
    let changed = key(current) != key(found.as_ref());
    (found, changed)
}

/// The game running now, if any.
#[must_use]
pub fn current() -> Option<GameIdentity> {
    WATCH.with(|w| w.current.borrow().clone())
}

/// Be told whenever the running game changes, starting with the current one,
/// until the listener returns [`glib::ControlFlow::Break`].
pub fn subscribe(listener: impl Fn(Option<&GameIdentity>) -> glib::ControlFlow + 'static) {
    WATCH.with(|watch| {
        if listener(watch.current.borrow().as_ref()).is_continue() {
            watch.listeners.borrow_mut().push(Box::new(listener));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use bigame_core::running::{Graphics, Runtime};

    fn game(pid: u32, graphics: Graphics, card: Option<&str>) -> GameIdentity {
        GameIdentity {
            display_name: "Shadow of the Tomb Raider".into(),
            steam_app_id: Some("750920".into()),
            install_path: None,
            compatdata_path: None,
            pid,
            process_name: "SOTTR.exe".into(),
            executable: "SOTTR.exe".into(),
            runtime: Runtime::Proton("proton_10".into()),
            graphics,
            render_card: card.map(Into::into),
            tree: Vec::new(),
        }
    }

    #[test]
    fn the_render_card_learned_after_the_game_is_passed_on_and_kept() {
        // Caught at falcond's event: no DLLs mapped, no GPU work yet.
        let (first, changed) = merge(None, Some(game(7, Graphics::Unknown, None)));
        assert!(changed);
        let (graphics, changed) = merge(first.as_ref(), Some(game(7, Graphics::Dxvk, None)));
        assert!(changed);
        // The GPU work shows which card it renders on.
        let (card, changed) = merge(
            graphics.as_ref(),
            Some(game(7, Graphics::Dxvk, Some("card1"))),
        );
        assert!(changed, "a render card learned later is a change");
        assert_eq!(card.as_ref().unwrap().render_card.as_deref(), Some("card1"));
        // A later check that cannot tell keeps it, and is no change.
        let (kept, changed) = merge(card.as_ref(), Some(game(7, Graphics::Dxvk, None)));
        assert!(!changed);
        assert_eq!(kept.unwrap().render_card.as_deref(), Some("card1"));
        // Another process starts with nothing known.
        let (next, changed) = merge(card.as_ref(), Some(game(8, Graphics::Dxvk, None)));
        assert!(changed);
        assert_eq!(next.unwrap().render_card, None);
        assert!(merge(card.as_ref(), None).1);
    }
}
