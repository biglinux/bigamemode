//! Details: what the machine is doing for the game, with the evidence.
//!
//! One rule for every optimization on this page: *what is it, is it
//! available, is it configured, is it active, how do we know, why is it not
//! working, how to fix it*. The visible line of each row answers the first
//! four; the row's body answers the rest.
//!
//! Every value comes from one reading of the system
//! ([`bigame_core::overview::Snapshot`]), taken off the main thread only
//! while the page is on screen — every 3 s with the window focused, 6 s
//! behind another window — and again at once when falcond's status file
//! changes or the running game changes. Telemetry has its own, faster
//! reading (1 s), also only while the page is on screen.

mod extras;
mod gpus;
mod overview;
mod performance;
mod pipeline;
mod problems;
mod telemetry;

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use adw::prelude::*;
use gtk4::{gio, glib};
use libadwaita as adw;

use bigame_core::overview::Snapshot;

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

/// Build the Details page.
#[must_use]
#[allow(clippy::too_many_lines)]
pub fn build() -> gtk4::Widget {
    let page = adw::PreferencesPage::new();
    let hw = Rc::new(bigame_core::hardware::Hardware::detect());

    let overview = overview::Overview::new(&hw);
    page.add(overview.group());

    let telemetry = telemetry::Telemetry::new();
    page.add(telemetry.group());
    telemetry.start();

    let gpus = gpus::Gpus::new(&hw);
    page.add(gpus.group());
    gpus.start(&hw, telemetry.gpu_targets());

    let performance = performance::Performance::new(&hw);
    page.add(performance.group());

    let pipeline = pipeline::Pipeline::new(&hw);
    page.add(pipeline.header_group());
    page.add(pipeline.group());

    let problems = problems::Problems::new();
    page.add(problems.group());

    page.add(&extras::network_group());
    page.add(&extras::background_group());
    page.add(&extras::steam_group());
    page.add(&extras::report_group());

    // ── The snapshot, and everything it feeds ───────────────────────────
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
        let root = page.clone();
        glib::spawn_future_local(async move {
            let mut since_slow = Duration::ZERO;
            loop {
                if root.is_mapped() {
                    refresh();
                    if since_slow.is_zero() || since_slow >= SLOW_INTERVAL {
                        slow();
                        since_slow = Duration::from_millis(1);
                    }
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
        let root = page.clone();
        let file = gio::File::for_path(bigame_core::status::status_path());
        if let Ok(monitor) = file.monitor_file(gio::FileMonitorFlags::NONE, gio::Cancellable::NONE)
        {
            monitor.connect_changed(move |_, _, _, event| {
                if matches!(
                    event,
                    gio::FileMonitorEvent::Changed | gio::FileMonitorEvent::Created
                ) && root.is_mapped()
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
        let root = page.clone();
        crate::game_watch::subscribe(move |_| {
            if root.is_mapped() {
                refresh();
            }
        });
    }

    page.upcast()
}
