//! Details: what the machine is doing for the game, with the evidence.
//!
//! One rule for every optimization on this page: *what is it, is it
//! available, is it configured, is it active, how do we know, why is it not
//! working, how to fix it*. The visible line of each row answers the first
//! four; the row's body answers the rest.
//!
//! Every value comes from one reading of the system
//! ([`bigame_core::overview::Snapshot`]), taken off the main thread only
//! while the page is in front of someone ([`showing`]) — every 3 s with the
//! window focused, 6 s behind another window — and again at once when
//! falcond's status file changes or the running game changes. Left behind a
//! game being played, nothing is read: a reading looks into the game's
//! process and asks the GPU's power controller, which the game needs more.
//! The live charts are on their own page, Telemetry, on the same terms.

mod dns;
pub(crate) mod extras;
pub(crate) mod overview;
mod performance;
mod pipeline;
mod problems;

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use adw::prelude::*;
use gtk4::{gio, glib};
use libadwaita as adw;

use bigame_core::overview::Snapshot;

use crate::i18n::i18n;

/// Snapshot interval while the window has focus.
const SNAPSHOT_FOCUSED: Duration = Duration::from_secs(3);
/// Snapshot interval while the window is open but not focused — behind
/// other windows, or behind a game.
const SNAPSHOT_BACKGROUND: Duration = Duration::from_secs(6);
/// The health checks and the AI Graphics installs are slower (package
/// database, file hashes): on every visit, then every half minute.
const SLOW_INTERVAL: Duration = Duration::from_secs(30);

/// How long to wait before the next reading of the page `widget` is on.
pub(crate) fn next_poll(
    widget: &impl IsA<gtk4::Widget>,
    focused: Duration,
    background: Duration,
) -> Duration {
    let active = widget
        .root()
        .and_downcast::<gtk4::Window>()
        .is_some_and(|w| w.is_active());
    if active { focused } else { background }
}

/// Resolve once `widget` is on screen, at once if it already is.
///
/// The page's loops wait here while the window is hidden in the tray or
/// another page is shown, instead of waking on a timer to find out.
pub(crate) async fn mapped(widget: &impl IsA<gtk4::Widget>) {
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::task::{Poll, Waker};

    if widget.is_mapped() {
        return;
    }
    let widget = widget.clone().upcast::<gtk4::Widget>();
    let waker: Rc<RefCell<Option<Waker>>> = Rc::default();
    let on_map = {
        let waker = Rc::clone(&waker);
        widget.connect_map(move |_| {
            if let Some(w) = waker.borrow_mut().take() {
                w.wake();
            }
        })
    };
    std::future::poll_fn(|cx| {
        if widget.is_mapped() {
            Poll::Ready(())
        } else {
            *waker.borrow_mut() = Some(cx.waker().clone());
            Poll::Pending
        }
    })
    .await;
    widget.disconnect(on_map);
}

/// How often a page left behind a game looks whether it is in front again.
/// Only the window's state is asked: nothing is read.
const PAUSED_CHECK: Duration = Duration::from_secs(2);

/// Whether the page `widget` is in front of someone: on screen, its window
/// not minimised, and not left behind a game being played (the window is
/// not the active one while a game runs).
pub(crate) fn showing(widget: &impl IsA<gtk4::Widget>) -> bool {
    if !widget.is_mapped() {
        return false;
    }
    let Some(window) = widget.root().and_downcast::<gtk4::Window>() else {
        return true;
    };
    !window.is_suspended() && (window.is_active() || crate::game_watch::current().is_none())
}

/// Resolve once the page `widget` is in front of someone ([`showing`]): the
/// page's loops wait here, rather than reading for a window behind a game.
pub(crate) async fn in_view(widget: &impl IsA<gtk4::Widget>) {
    loop {
        mapped(widget).await;
        if showing(widget) {
            return;
        }
        glib::timeout_future(PAUSED_CHECK).await;
    }
}

/// Build the Details page.
///
/// Filled the first time it is on screen: the hardware (sysfs, DRM, the
/// displays) is read off the main thread, and not at all for a window that
/// never shows the page — such as one started hidden at login.
#[must_use]
pub fn build() -> gtk4::Widget {
    let page = crate::widgets::layout::Page::new();
    let widget = page.widget().clone().upcast();
    glib::spawn_future_local(async move {
        mapped(page.widget()).await;
        if let Ok(hw) = gio::spawn_blocking(bigame_core::hardware::Hardware::detect).await {
            fill(&page, &Rc::new(hw));
        }
    });
    widget
}

#[allow(clippy::too_many_lines)]
fn fill(page: &crate::widgets::layout::Page, hw: &Rc<bigame_core::hardware::Hardware>) {
    let overview = overview::Overview::new(hw);
    page.add(overview.group());

    // What is in force, and what is wrong; then what feeds the game.
    let (left, right) = page.columns();
    let performance = performance::Performance::new(hw);
    left.append(performance.group());
    let problems = problems::Problems::new();
    left.append(problems.group());
    let pipeline = pipeline::Pipeline::new(hw);
    right.append(pipeline.header_group());
    right.append(pipeline.group());

    page.section(&i18n("Around the game"));
    let (left, right) = page.columns();
    left.append(&extras::network_group());
    left.append(&extras::steam_group());
    right.append(&extras::background_group());
    right.append(&extras::report_group());

    // ── The snapshot, and everything it feeds ───────────────────────────
    let root = page.widget().clone();
    let busy = Rc::new(Cell::new(false));
    let refresh: Rc<dyn Fn()> = {
        let (overview, performance, pipeline, problems) = (
            overview.clone(),
            performance.clone(),
            pipeline.clone(),
            problems.clone(),
        );
        let busy = Rc::clone(&busy);
        Rc::new(move || {
            // One reading at a time: a slow one is not stacked on by the
            // timer, the file watch and the game watch all firing at once.
            if busy.replace(true) {
                return;
            }
            let (overview, performance, pipeline, problems) = (
                overview.clone(),
                performance.clone(),
                pipeline.clone(),
                problems.clone(),
            );
            let busy = Rc::clone(&busy);
            glib::spawn_future_local(async move {
                let game = crate::game_watch::current();
                let snap = gio::spawn_blocking(move || Snapshot::collect(game))
                    .await
                    .unwrap_or_default();
                overview.show(&snap);
                performance.show(&snap);
                pipeline.show(&snap);
                problems.show_runtime(&snap);
                busy.set(false);
            });
        })
    };

    // The slow readings: on every visit, then every half minute.
    let slow: Rc<dyn Fn()> = {
        let (problems, pipeline) = (problems.clone(), pipeline.clone());
        Rc::new(move || {
            problems.refresh_health();
            pipeline.refresh_installs();
        })
    };

    {
        let refresh = Rc::clone(&refresh);
        let slow = Rc::clone(&slow);
        let root = root.clone();
        glib::spawn_future_local(async move {
            let mut since_slow = Duration::ZERO;
            loop {
                in_view(&root).await;
                refresh();
                if since_slow.is_zero() || since_slow >= SLOW_INTERVAL {
                    slow();
                    since_slow = Duration::from_millis(1);
                }
                let wait = next_poll(&root, SNAPSHOT_FOCUSED, SNAPSHOT_BACKGROUND);
                glib::timeout_future(wait).await;
                since_slow += wait;
            }
        });
    }

    // falcond's status file: a profile applied or restored shows at once.
    {
        let refresh = Rc::clone(&refresh);
        let root = root.clone();
        let file = gio::File::for_path(bigame_core::status::status_path());
        if let Ok(monitor) = file.monitor_file(gio::FileMonitorFlags::NONE, gio::Cancellable::NONE)
        {
            monitor.connect_changed(move |_, _, _, event| {
                if matches!(
                    event,
                    gio::FileMonitorEvent::Changed | gio::FileMonitorEvent::Created
                ) && showing(&root)
                {
                    refresh();
                }
            });
            // Kept for the life of the page, which is the life of the window.
            std::mem::forget(monitor);
        }
    }

    // A game starting or ending: the pipeline changes at once.
    {
        let refresh = Rc::clone(&refresh);
        let root = root.clone();
        crate::game_watch::subscribe(move |_| {
            if showing(&root) {
                refresh();
            }
            glib::ControlFlow::Continue
        });
    }
}
