//! The ⓘ button: an explanation one click away instead of on the page.
//!
//! Technical rows keep a short subtitle and put the rest here — what the
//! setting is, what it changes, who controls it, and what was measured — so
//! the page stays readable for someone who only wants to play.

use gtk4::glib;
use gtk4::prelude::*;

use crate::i18n::i18n;

/// An info button whose popover shows `heading` and `text`.
#[must_use]
pub fn button(heading: &str, text: &str) -> gtk4::MenuButton {
    let title = gtk4::Label::new(Some(heading));
    title.add_css_class("heading");
    title.set_xalign(0.0);
    title.set_wrap(true);

    let body = gtk4::Label::new(Some(text));
    body.set_xalign(0.0);
    body.set_wrap(true);
    body.set_max_width_chars(48);
    body.set_selectable(true);

    let content = gtk4::Box::new(gtk4::Orientation::Vertical, 6);
    content.set_margin_top(12);
    content.set_margin_bottom(12);
    content.set_margin_start(12);
    content.set_margin_end(12);
    content.append(&title);
    content.append(&body);

    let popover = gtk4::Popover::new();
    popover.set_child(Some(&content));
    // The popover hands its focus to the text, and a selectable label
    // takes keyboard focus by selecting everything: the explanation opened
    // painted as one blue block. It stays selectable, starting with none.
    popover.connect_show(move |_| {
        let body = body.clone();
        glib::idle_add_local_once(move || body.select_region(0, 0));
    });

    let button = gtk4::MenuButton::builder()
        .icon_name("help-about-symbolic")
        .popover(&popover)
        .valign(gtk4::Align::Center)
        .css_classes(["flat", "circular"])
        .tooltip_text(i18n("More information"))
        .build();
    button.update_property(&[gtk4::accessible::Property::Label(&format!(
        "{} {heading}",
        i18n("More information about")
    ))]);
    button
}

/// One entry of an information dialog: a name and what it means.
pub struct Entry {
    /// The name, in bold.
    pub title: String,
    /// The explanation.
    pub body: String,
}

/// The `(i)` that opens an information dialog — the same button and the
/// same dialog for every topic (schedulers, their modes, 3D V-Cache,
/// falcond's profile sets). `content` is built when the dialog opens.
#[must_use]
pub fn dialog_button(
    tooltip: &str,
    content: impl Fn() -> (String, String, Vec<Entry>) + 'static,
) -> gtk4::Button {
    let button = gtk4::Button::builder()
        .icon_name("dialog-information-symbolic")
        .valign(gtk4::Align::Center)
        .css_classes(["flat", "circular"])
        .tooltip_text(tooltip)
        .build();
    button.update_property(&[gtk4::accessible::Property::Label(tooltip)]);
    button.connect_clicked(move |b| {
        let (title, intro, entries) = content();
        show_dialog(b, &title, &intro, &entries);
    });
    button
}

/// Show an information dialog over `anchor`: a title, an introduction and
/// a list of entries, scrollable, sized for reading.
pub fn show_dialog(anchor: &impl IsA<gtk4::Widget>, title: &str, intro: &str, entries: &[Entry]) {
    use libadwaita as adw;
    use libadwaita::prelude::*;

    let content = gtk4::Box::new(gtk4::Orientation::Vertical, 16);
    content.set_margin_top(12);
    content.set_margin_bottom(24);
    content.set_margin_start(24);
    content.set_margin_end(24);
    if !intro.is_empty() {
        let label = gtk4::Label::builder()
            .label(intro)
            .wrap(true)
            .xalign(0.0)
            .build();
        content.append(&label);
    }
    for e in entries {
        let group = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
        let name = gtk4::Label::builder()
            .label(&e.title)
            .wrap(true)
            .xalign(0.0)
            .css_classes(["heading"])
            .build();
        let body = gtk4::Label::builder()
            .label(&e.body)
            .wrap(true)
            .xalign(0.0)
            .build();
        group.append(&name);
        group.append(&body);
        content.append(&group);
    }
    let scroll = gtk4::ScrolledWindow::builder()
        .hscrollbar_policy(gtk4::PolicyType::Never)
        .propagate_natural_height(true)
        .vexpand(true)
        .child(&content)
        .build();
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&adw::HeaderBar::new());
    toolbar.set_content(Some(&scroll));
    let dialog = adw::Dialog::builder()
        .title(title)
        .content_width(560)
        .content_height(520)
        .child(&toolbar)
        .build();
    dialog.add_css_class("info-dialog");
    dialog.present(Some(anchor));
}

/// Entries for a list of fixed choices.
#[must_use]
pub fn choice_entries(choices: &[bigame_core::optimization::Choice]) -> Vec<Entry> {
    choices
        .iter()
        .map(|c| Entry {
            title: format!("{} ({})", i18n(c.label), c.id),
            body: i18n(c.help),
        })
        .collect()
}

/// The `(i)` for the scheduler's mode.
#[must_use]
pub fn scheduler_modes_button() -> gtk4::Button {
    dialog_button(&i18n("About scheduler modes"), || {
        let mut entries = choice_entries(bigame_core::optimization::SCHEDULER_MODES);
        entries.push(Entry {
            title: i18n("What was measured"),
            body: i18n(
                "On the reference desktop no scheduler or mode made Shadow of the Tomb Raider faster than the kernel's own scheduler. Try one when a game stutters, and keep it only if it helps.",
            ),
        });
        (
            i18n("Scheduler mode"),
            i18n(
                "The mode is handed to scx_loader when falcond starts a sched-ext scheduler. Each scheduler turns it into its own options; a scheduler without a setting for a mode runs as in Default. With no scheduler chosen, the mode does nothing.",
            ),
            entries,
        )
    })
}

/// The `(i)` for 3D V-Cache.
#[must_use]
pub fn vcache_button() -> gtk4::Button {
    dialog_button(&i18n("About 3D V-Cache"), || {
        let mut entries = choice_entries(bigame_core::optimization::VCACHE_MODES);
        entries.push(Entry {
            title: i18n("When it does nothing"),
            body: i18n(
                "On processors with a single CCD (such as the Ryzen 7 7800X3D or 9800X3D) every core already has the cache, and processors without 3D V-Cache have nothing to choose. There the driver is absent, and BiGame-mode shows the option as not supported instead of a control that would do nothing.",
            ),
        });
        (
            i18n("3D V-Cache"),
            i18n(
                "AMD processors with 3D V-Cache (X3D) have extra L3 cache stacked on one CCD, a group of cores. On models with two CCDs, such as the Ryzen 9 7950X3D or 9950X3D, only one has the extra cache and the other clocks higher. The amd_x3d_vcache driver tells the system which CCD to prefer; falcond sets it while a game runs and puts it back when the game closes.",
            ),
            entries,
        )
    })
}

/// The `(i)` for falcond's profile set.
#[must_use]
pub fn profile_sets_button() -> gtk4::Button {
    dialog_button(&i18n("About profile sets"), || {
        (
            i18n("Profile set"),
            i18n(
                "falcond ships ready-made profiles for well-known games, in three sets. The set decides which of them falcond uses; the profiles you make here apply in every set. falcond reloads its profiles after a change.",
            ),
            choice_entries(bigame_core::optimization::PROFILE_SETS),
        )
    })
}
