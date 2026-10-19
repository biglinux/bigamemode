//! The games that closed with an error: the record the crash watch keeps
//! ([`bigame_core::game_crash::recorded`]), newest first, so a crash that
//! comes back with the same extras in the game shows as one.
//!
//! Read from the state directory when the page is built and again whenever
//! the crash watch finds a crash; nothing is read in between.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use gtk4::{gio, glib};
use libadwaita as adw;

use bigame_core::game_crash::Crash;

use crate::i18n::{i18n, labelled, tr};
use crate::widgets::layout;

/// How many crashes the group lists.
const SHOWN: usize = 10;

/// The group, hidden while no game has crashed.
pub fn group() -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::new();
    group.set_title(&i18n("Games that closed with an error"));
    layout::icon(&group, "dialog-error-symbolic");
    group.set_description(Some(&i18n(
        "What showed each crash, what the game said, and what was in it besides the game itself. The same extras in every crash of a game are worth leaving out of it.",
    )));
    group.set_visible(false);

    let rows: Rc<RefCell<Vec<adw::ActionRow>>> = Rc::new(RefCell::new(Vec::new()));
    let refresh: Rc<dyn Fn()> = {
        let (group, rows) = (group.clone(), Rc::clone(&rows));
        Rc::new(move || {
            let (group, rows) = (group.clone(), Rc::clone(&rows));
            glib::spawn_future_local(async move {
                // Whether each report is still there, read here: the game
                // library may be on a disk that has to spin up first.
                let crashes = gio::spawn_blocking(|| {
                    bigame_core::game_crash::recorded()
                        .into_iter()
                        .take(SHOWN)
                        .map(|c| {
                            let report = c.report.as_deref().is_some_and(std::path::Path::is_dir);
                            (c, report)
                        })
                        .collect::<Vec<_>>()
                })
                .await
                .unwrap_or_default();
                for row in rows.borrow_mut().drain(..) {
                    group.remove(&row);
                }
                group.set_visible(!crashes.is_empty());
                for (crash, report) in &crashes {
                    let row = row(crash, *report);
                    group.add(&row);
                    rows.borrow_mut().push(row);
                }
            });
        })
    };
    refresh();
    let weak = group.downgrade();
    crate::crash_watch::subscribe(move |event| {
        if weak.upgrade().is_none() {
            return glib::ControlFlow::Break;
        }
        if matches!(event, crate::crash_watch::Event::Crashed(..)) {
            refresh();
        }
        glib::ControlFlow::Continue
    });
    group
}

/// One crash: the game and when, then what showed it and what was in it;
/// `report`: the game's report is still there to open.
fn row(crash: &Crash, report: bool) -> adw::ActionRow {
    let when = i64::try_from(crash.at)
        .ok()
        .and_then(|t| glib::DateTime::from_unix_local(t).ok())
        .and_then(|t| t.format("%x %H:%M").ok())
        .map(|t| t.to_string())
        .unwrap_or_default();
    let mut lines = vec![crate::crash_watch::played_line(crash.played_secs)];
    let evidence = tr(&crash.evidence);
    lines.push(match &crash.reason {
        Some(reason) => labelled(&evidence, reason),
        None => evidence,
    });
    if !crash.extras.is_empty() {
        let names: Vec<String> = crash.extras.iter().map(|e| i18n(e.label())).collect();
        lines.push(labelled(&i18n("Also in the game"), &names.join(", ")));
    }
    let row = adw::ActionRow::builder()
        .title(format!("{} · {when}", crash.game))
        .subtitle(lines.join("\n"))
        .subtitle_lines(6)
        .use_markup(false)
        .build();
    if let Some(dir) = crash.report.clone().filter(|_| report) {
        let open = gtk4::Button::builder()
            .icon_name("folder-open-symbolic")
            .tooltip_text(i18n("Open the game's report"))
            .valign(gtk4::Align::Center)
            .css_classes(["flat"])
            .build();
        open.connect_clicked(move |button| {
            let window = button.root().and_downcast::<gtk4::Window>();
            gtk4::FileLauncher::new(Some(&gio::File::for_path(&dir))).launch(
                window.as_ref(),
                gio::Cancellable::NONE,
                |_| {},
            );
        });
        row.add_suffix(&open);
    }
    row
}
