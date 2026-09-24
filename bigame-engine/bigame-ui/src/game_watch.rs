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

type Listener = Box<dyn Fn(Option<&GameIdentity>)>;

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
            // A game caught the moment it starts has not mapped its graphics
            // DLLs yet; learning them later is a change worth passing on.
            let key = |g: Option<&GameIdentity>| g.map(|g| (g.pid, g.graphics));
            let changed = key(watch.current.borrow().as_ref()) != key(found.as_ref());
            if !changed {
                return;
            }
            if let Some(g) = &found { tracing::info!(game = %g.display_name, process = %g.process_name, pid = g.pid, "game detected") } else { tracing::info!("game no longer running") }
            *watch.current.borrow_mut() = found;
            let current = watch.current.borrow();
            for listener in watch.listeners.borrow().iter() {
                listener(current.as_ref());
            }
        });
    });
}

/// The game running now, if any.
#[must_use]
pub fn current() -> Option<GameIdentity> {
    WATCH.with(|w| w.current.borrow().clone())
}

/// Be told whenever the running game changes, starting with the current one.
pub fn subscribe(listener: impl Fn(Option<&GameIdentity>) + 'static) {
    WATCH.with(|watch| {
        listener(watch.current.borrow().as_ref());
        watch.listeners.borrow_mut().push(Box::new(listener));
    });
}
