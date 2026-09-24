//! Logs: everything involved in a game session, in one colour-coded list.
//!
//! Read from the journal in one call ([`bigame_core::logs`]), incrementally
//! by cursor, and only while this page is on screen. The previous page ran
//! four processes every five seconds whether or not anyone was looking.
//!
//! Only the severity label is coloured, so an error stands out without the
//! whole line shouting; the message itself stays in the normal text colour.

use std::cell::RefCell;
use std::fmt::Write as _;
use std::rc::Rc;
use std::time::Duration;

use adw::prelude::*;
use gtk4::{gio, glib};
use libadwaita as adw;

use bigame_core::logs::{Entry, Level, Source};

use crate::i18n::i18n;

/// Entries kept in memory; older ones scroll away.
const KEEP: usize = 3000;

/// What the filter shows.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Filter {
    All,
    Errors,
    Warnings,
    Success,
    Only(Source),
    KernelGpu,
}

impl Filter {
    fn all() -> Vec<(Self, String)> {
        vec![
            (Self::All, i18n("All")),
            (Self::Errors, i18n("Errors")),
            (Self::Warnings, i18n("Warnings and errors")),
            (Self::Success, i18n("Success")),
            (Self::Only(Source::Falcond), "falcond".into()),
            (Self::Only(Source::BiGame), "BiGame-mode".into()),
            (Self::Only(Source::Helper), i18n("BiGame-mode helper")),
            (Self::KernelGpu, i18n("Kernel and GPU")),
            (Self::Only(Source::Gamescope), "Gamescope".into()),
            (Self::Only(Source::Scheduler), "sched-ext".into()),
            (Self::Only(Source::PowerProfiles), i18n("Power profiles")),
        ]
    }

    fn accepts(self, entry: &Entry) -> bool {
        match self {
            Self::All => true,
            Self::Errors => entry.level == Level::Error,
            Self::Warnings => entry.level >= Level::Warning,
            Self::Success => entry.level == Level::Success,
            Self::Only(source) => entry.source == source,
            Self::KernelGpu => entry.source == Source::Kernel,
        }
    }
}

struct State {
    entries: Vec<Entry>,
    cursor: Option<String>,
    filter: Filter,
    search: String,
    loading: bool,
}

/// Build the Logs page.
#[must_use]
#[allow(clippy::too_many_lines)]
pub fn build() -> adw::PreferencesPage {
    let page = adw::PreferencesPage::new();
    let group = adw::PreferencesGroup::new();
    group.set_title(&i18n("Logs"));
    group.set_description(Some(&i18n(
        "falcond, BiGame-mode, the kernel's graphics drivers, Gamescope, sched-ext and power profiles, from the system journal.",
    )));

    // ── Controls ────────────────────────────────────────────────────────
    let filters = Filter::all();
    let names: Vec<&str> = filters.iter().map(|(_, n)| n.as_str()).collect();
    let dropdown = gtk4::DropDown::from_strings(&names);
    dropdown.set_tooltip_text(Some(&i18n("Show")));
    let search = gtk4::SearchEntry::builder()
        .placeholder_text(i18n("Search"))
        .hexpand(true)
        .build();
    let live = gtk4::ToggleButton::builder()
        .icon_name("media-playback-start-symbolic")
        .active(true)
        .tooltip_text(i18n("Follow new entries"))
        .css_classes(["flat"])
        .build();
    let refresh = gtk4::Button::builder()
        .icon_name("view-refresh-symbolic")
        .tooltip_text(i18n("Refresh"))
        .css_classes(["flat"])
        .build();
    let copy = gtk4::Button::builder()
        .icon_name("edit-copy-symbolic")
        .tooltip_text(i18n("Copy what is shown"))
        .css_classes(["flat"])
        .build();
    let export = gtk4::Button::builder()
        .icon_name("document-save-symbolic")
        .tooltip_text(i18n("Export, with personal data masked"))
        .css_classes(["flat"])
        .build();
    for (button, label) in [
        (
            live.upcast_ref::<gtk4::Widget>(),
            i18n("Follow new entries"),
        ),
        (refresh.upcast_ref(), i18n("Refresh")),
        (copy.upcast_ref(), i18n("Copy what is shown")),
        (
            export.upcast_ref(),
            i18n("Export, with personal data masked"),
        ),
    ] {
        button.update_property(&[gtk4::accessible::Property::Label(&label)]);
    }
    let bar = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    bar.append(&dropdown);
    bar.append(&search);
    bar.append(&live);
    bar.append(&refresh);
    bar.append(&copy);
    bar.append(&export);
    group.add(&bar);

    // ── The list ────────────────────────────────────────────────────────
    let view = gtk4::TextView::builder()
        .editable(false)
        .cursor_visible(false)
        .monospace(true)
        .wrap_mode(gtk4::WrapMode::WordChar)
        .top_margin(8)
        .bottom_margin(8)
        .left_margin(12)
        .right_margin(12)
        .build();
    view.add_css_class("card");
    install_tags(&view.buffer());
    let scroll = gtk4::ScrolledWindow::builder()
        .min_content_height(460)
        .vexpand(true)
        .child(&view)
        .build();
    scroll.set_margin_top(8);
    group.add(&scroll);
    let counts = gtk4::Label::new(None);
    counts.add_css_class("caption");
    counts.add_css_class("dim-label");
    counts.set_xalign(0.0);
    counts.set_margin_top(6);
    group.add(&counts);
    page.add(&group);

    let state = Rc::new(RefCell::new(State {
        entries: Vec::new(),
        cursor: None,
        filter: Filter::All,
        search: String::new(),
        loading: false,
    }));

    let render = {
        let state = Rc::clone(&state);
        let view = view.clone();
        let counts = counts.clone();
        Rc::new(move || render(&state.borrow(), &view, &counts))
    };

    let load = {
        let state = Rc::clone(&state);
        let render = Rc::clone(&render);
        let view = view.clone();
        Rc::new(move || {
            if state.borrow().loading {
                return;
            }
            state.borrow_mut().loading = true;
            let cursor = state.borrow().cursor.clone();
            let state = Rc::clone(&state);
            let render = Rc::clone(&render);
            let view = view.clone();
            glib::spawn_future_local(async move {
                let result =
                    gio::spawn_blocking(move || bigame_core::logs::read(600, cursor.as_deref()))
                        .await;
                let mut s = state.borrow_mut();
                s.loading = false;
                let Ok(Ok((new, cursor))) = result else {
                    return;
                };
                if cursor.is_some() {
                    s.cursor = cursor;
                }
                if new.is_empty() && !s.entries.is_empty() {
                    return;
                }
                s.entries.extend(new);
                let excess = s.entries.len().saturating_sub(KEEP);
                s.entries.drain(..excess);
                drop(s);
                render();
                // Keep the newest line in view.
                let buffer = view.buffer();
                let mut end = buffer.end_iter();
                view.scroll_to_iter(&mut end, 0.0, false, 0.0, 1.0);
            });
        })
    };

    {
        let state = Rc::clone(&state);
        let render = Rc::clone(&render);
        dropdown.connect_selected_notify(move |d| {
            let index = usize::try_from(d.selected()).unwrap_or(0);
            state.borrow_mut().filter = filters.get(index).map_or(Filter::All, |(f, _)| *f);
            render();
        });
    }
    {
        let state = Rc::clone(&state);
        let render = Rc::clone(&render);
        search.connect_search_changed(move |e| {
            state.borrow_mut().search = e.text().to_lowercase();
            render();
        });
    }
    {
        let load = Rc::clone(&load);
        refresh.connect_clicked(move |_| load());
    }
    {
        let view = view.clone();
        copy.connect_clicked(move |b| {
            let buffer = view.buffer();
            let text = buffer.text(&buffer.start_iter(), &buffer.end_iter(), false);
            b.clipboard().set_text(&text);
            crate::widgets::toast::show(b, &i18n("Copied"));
        });
    }
    {
        let state = Rc::clone(&state);
        export.connect_clicked(move |b| export_to_file(b, &state.borrow()));
    }

    // Load when first shown, then follow while shown. Nothing runs while the
    // page is not on screen.
    {
        let load = Rc::clone(&load);
        let first = std::cell::Cell::new(true);
        scroll.connect_map(move |_| {
            if first.replace(false) {
                load();
            }
        });
    }
    {
        let load = Rc::clone(&load);
        let scroll = scroll.clone();
        glib::timeout_add_local(Duration::from_secs(5), move || {
            if scroll.is_mapped() && live.is_active() {
                load();
            }
            glib::ControlFlow::Continue
        });
    }

    page
}

/// Severity colours, readable on light and dark backgrounds alike.
fn install_tags(buffer: &gtk4::TextBuffer) {
    let dark = adw::StyleManager::default().is_dark();
    let (red, orange, green) = if dark {
        ("#ff7b63", "#ffc057", "#8ff0a4")
    } else {
        ("#c01c28", "#9c6e03", "#1b7a3f")
    };
    let table = buffer.tag_table();
    for (name, colour) in [("error", red), ("warning", orange), ("success", green)] {
        let tag = gtk4::TextTag::builder()
            .name(name)
            .foreground(colour)
            .weight(700)
            .build();
        table.add(&tag);
    }
    let dim = gtk4::TextTag::builder()
        .name("dim")
        .foreground_rgba(&gtk4::gdk::RGBA::new(0.5, 0.5, 0.5, 1.0))
        .build();
    table.add(&dim);
}

fn level_tag(level: Level) -> Option<&'static str> {
    match level {
        Level::Error => Some("error"),
        Level::Warning => Some("warning"),
        Level::Success => Some("success"),
        Level::Debug => Some("dim"),
        Level::Info => None,
    }
}

fn time_label(us: u64) -> String {
    let secs = i64::try_from(us / 1_000_000).unwrap_or(0);
    glib::DateTime::from_unix_local(secs)
        .and_then(|t| t.format("%H:%M:%S"))
        .map(|s| s.to_string())
        .unwrap_or_default()
}

fn visible(state: &State) -> impl Iterator<Item = &Entry> {
    state.entries.iter().filter(move |e| {
        state.filter.accepts(e)
            && (state.search.is_empty() || e.message.to_lowercase().contains(&state.search))
    })
}

fn render(state: &State, view: &gtk4::TextView, counts: &gtk4::Label) {
    let buffer = view.buffer();
    buffer.set_text("");
    let mut shown = 0usize;
    let mut end = buffer.end_iter();
    for entry in visible(state) {
        shown += 1;
        buffer.insert_with_tags_by_name(&mut end, &time_label(entry.time_us), &["dim"]);
        buffer.insert(&mut end, "  ");
        match level_tag(entry.level) {
            Some(tag) => buffer.insert_with_tags_by_name(&mut end, entry.level.label(), &[tag]),
            None => buffer.insert(&mut end, entry.level.label()),
        }
        buffer.insert_with_tags_by_name(
            &mut end,
            &format!("  {:<9}", entry.source.label()),
            &["dim"],
        );
        buffer.insert(&mut end, &entry.message);
        buffer.insert(&mut end, "\n");
    }
    if shown == 0 {
        buffer.set_text(&i18n("Nothing matches."));
    }
    let errors = state
        .entries
        .iter()
        .filter(|e| e.level == Level::Error)
        .count();
    let warnings = state
        .entries
        .iter()
        .filter(|e| e.level == Level::Warning)
        .count();
    counts.set_label(&format!(
        "{shown} {} · {errors} {} · {warnings} {}",
        i18n("shown"),
        i18n("errors"),
        i18n("warnings")
    ));
}

fn export_to_file(anchor: &gtk4::Button, state: &State) {
    let home = std::env::var("HOME").unwrap_or_default();
    let user = std::env::var("USER").unwrap_or_default();
    let host = std::fs::read_to_string("/etc/hostname")
        .unwrap_or_default()
        .trim()
        .to_owned();
    let mut text = String::new();
    for e in visible(state) {
        let _ = writeln!(
            text,
            "{}  {}  {:<9}{}",
            time_label(e.time_us),
            e.level.label(),
            e.source.label(),
            e.message
        );
    }
    let text = bigame_core::logs::redact(&text, &home, &user, &host);
    let dialog = gtk4::FileDialog::builder()
        .title(i18n("Export log"))
        .initial_name("bigamemode-log.txt")
        .build();
    let window = anchor.root().and_downcast::<gtk4::Window>();
    let anchor = anchor.clone();
    dialog.save(window.as_ref(), gio::Cancellable::NONE, move |result| {
        let Ok(file) = result else { return };
        let Some(path) = file.path() else { return };
        match std::fs::write(&path, text) {
            Ok(()) => crate::widgets::toast::show(&anchor, &i18n("Log exported")),
            Err(e) => {
                crate::widgets::toast::show(&anchor, &format!("{}: {e}", i18n("Could not export")));
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(source: Source, level: Level, message: &str) -> Entry {
        Entry {
            time_us: 0,
            source,
            level,
            message: message.into(),
        }
    }

    #[test]
    fn filters_select_by_severity_and_source() {
        let e = entry(
            Source::Falcond,
            Level::Error,
            "failed to switch scx scheduler",
        );
        let w = entry(Source::Kernel, Level::Warning, "amdgpu ring timeout");
        let i = entry(Source::BiGame, Level::Info, "game detected");
        assert!(Filter::Errors.accepts(&e) && !Filter::Errors.accepts(&w));
        assert!(
            Filter::Warnings.accepts(&e)
                && Filter::Warnings.accepts(&w)
                && !Filter::Warnings.accepts(&i)
        );
        assert!(Filter::KernelGpu.accepts(&w) && !Filter::KernelGpu.accepts(&e));
        assert!(Filter::Only(Source::BiGame).accepts(&i));
    }

    #[test]
    fn only_notable_levels_are_coloured() {
        assert_eq!(level_tag(Level::Info), None);
        assert_eq!(level_tag(Level::Error), Some("error"));
    }
}
