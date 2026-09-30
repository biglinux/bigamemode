//! The welcome screen: what Big Game Mode does, on one page.
//!
//! Shown the first time the window is on screen in a run — never while the
//! application starts hidden in the tray — until the switch at its foot is
//! turned off, and again from the main menu. Every feature it names is one
//! the application has; it is a map of the pages, not a promise.

use adw::prelude::*;
use gtk4::glib;
use libadwaita as adw;

use crate::i18n::i18n;
use crate::settings;

/// A feature as the welcome screen and the help pages lay it out: an icon,
/// its name in bold and what it does, dimmed, under it.
#[must_use]
pub fn feature_row(icon: &str, title: &str, text: &str, icon_size: i32) -> gtk4::Box {
    let image = gtk4::Image::from_icon_name(icon);
    image.set_pixel_size(icon_size);
    image.set_valign(gtk4::Align::Start);
    image.add_css_class("feature-icon");

    let heading = gtk4::Label::builder()
        .label(title)
        .wrap(true)
        .xalign(0.0)
        .css_classes(["heading"])
        .build();
    let body = gtk4::Label::builder()
        .label(text)
        .wrap(true)
        .xalign(0.0)
        .max_width_chars(44)
        .css_classes(["dim-label"])
        .build();

    let words = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
    words.set_hexpand(true);
    words.append(&heading);
    words.append(&body);

    let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 12);
    row.add_css_class("feature-row");
    row.append(&image);
    row.append(&words);
    row
}

/// The features, in pairs: one row of the grid each.
fn features() -> [(&'static str, String, String); 8] {
    [
        (
            "system-shutdown-symbolic",
            i18n("Turbo in one click"),
            i18n(
                "Switch it on and falcond gives each game its performance profile as it starts, then puts everything back when it closes.",
            ),
        ),
        (
            "power-profile-performance-symbolic",
            i18n("Presets"),
            i18n(
                "Standard, More FPS, Locked 60 FPS or Enhanced graphics: choose what games favour, from Home or the tray.",
            ),
        ),
        (
            "applications-games-symbolic",
            i18n("A profile for every game"),
            i18n(
                "Steam, Lutris, Heroic and menu games in one library. A wizard explains every option, and a new game can be offered a profile as it starts.",
            ),
        ),
        (
            "applications-science-symbolic",
            i18n("AI Graphics"),
            i18n(
                "Reads what a game really uses and recommends a plan, such as OptiScaler or FSR 4. Nothing is installed until you click Apply, always with a verified backup.",
            ),
        ),
        (
            "preferences-system-symbolic",
            i18n("One general configuration"),
            i18n(
                "Tuning holds what every game gets: Gamescope, Wine FSR, vkBasalt, lsfg-vk and MangoHud. Two technologies that collide are never left on together without asking you.",
            ),
        ),
        (
            "speedometer-symbolic",
            i18n("Details, with evidence"),
            i18n(
                "Each optimization says whether it is active, waiting, off, missing or not supported, how that is known, and how to fix it.",
            ),
        ),
        (
            "utilities-system-monitor-symbolic",
            i18n("Measure the difference"),
            i18n(
                "Runs a game with and without optimizations, several times, and calls a change an improvement only when it beats the variation.",
            ),
        ),
        (
            "utilities-terminal-symbolic",
            i18n("Always at hand"),
            i18n(
                "Closing the window leaves Big Game Mode in the tray, still watching for games. Logs gathers what falcond, Gamescope and the GPU drivers say.",
            ),
        ),
    ]
}

/// The icon, the greeting and what Big Game Mode is, in a line.
fn greeting() -> gtk4::Box {
    let header = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
    header.set_halign(gtk4::Align::Center);
    let icon = gtk4::Image::from_icon_name(crate::app::APP_ID);
    icon.set_pixel_size(64);
    header.append(&icon);
    header.append(
        &gtk4::Label::builder()
            .label(i18n("Welcome to Big Game Mode"))
            .wrap(true)
            .justify(gtk4::Justification::Center)
            .css_classes(["title-1"])
            .build(),
    );
    header.append(
        &gtk4::Label::builder()
            .label(i18n(
                "BigLinux's game mode: Turbo in one click, a profile for every game, and a page that shows, with evidence, what is really in force.",
            ))
            .wrap(true)
            .max_width_chars(70)
            .justify(gtk4::Justification::Center)
            .css_classes(["welcome-subtitle", "dim-label"])
            .build(),
    );
    header
}

/// Present the welcome screen over `parent`'s window.
pub fn show(parent: &impl IsA<gtk4::Widget>) {
    // A grid rather than two boxes, so each pair lines up across the columns
    // whatever the length of its texts.
    let grid = gtk4::Grid::builder()
        .row_spacing(16)
        .column_spacing(28)
        .column_homogeneous(true)
        .margin_top(14)
        .halign(gtk4::Align::Center)
        .hexpand(true)
        .build();
    for (i, (icon, title, text)) in (0i32..).zip(features()) {
        grid.attach(&feature_row(icon, &title, &text, 32), i % 2, i / 2, 1, 1);
    }

    let tip = gtk4::Label::builder()
        .label(i18n(
            "Tip: choose a preset, switch Turbo on and start a game; Home shows what it got. The (i) button at the top explains the page you are on.",
        ))
        .wrap(true)
        .justify(gtk4::Justification::Center)
        .margin_top(12)
        .css_classes(["caption", "dim-label"])
        .build();

    let content = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
    content.set_margin_start(20);
    content.set_margin_end(20);
    content.set_margin_top(20);
    content.set_margin_bottom(12);
    content.append(&greeting());
    content.append(&grid);
    content.append(&tip);

    let scrolled = gtk4::ScrolledWindow::builder()
        .hscrollbar_policy(gtk4::PolicyType::Never)
        .vexpand(true)
        .child(&content)
        .build();

    // The foot stays put while the features scroll.
    let label = i18n("Show dialog on startup");
    let switch = gtk4::Switch::builder()
        .valign(gtk4::Align::Center)
        .active(settings::load().show_welcome)
        .build();
    switch.update_property(&[gtk4::accessible::Property::Label(&label)]);
    // Saved as it is flipped: the screen can also be closed with Escape.
    switch.connect_active_notify(|s| {
        let mut saved = settings::load();
        saved.show_welcome = s.is_active();
        settings::save(&saved);
    });
    let switch_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    switch_box.set_hexpand(true);
    switch_box.append(&switch);
    switch_box.append(&gtk4::Label::builder().label(&label).xalign(0.0).build());

    let start = gtk4::Button::builder()
        .label(i18n("Let's Start"))
        .width_request(150)
        .css_classes(["suggested-action", "pill"])
        .build();

    let foot = gtk4::Box::new(gtk4::Orientation::Horizontal, 12);
    foot.set_margin_start(20);
    foot.set_margin_end(20);
    foot.set_margin_top(12);
    foot.set_margin_bottom(16);
    foot.append(&switch_box);
    foot.append(&start);

    let outer = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    outer.append(&scrolled);
    outer.append(&gtk4::Separator::new(gtk4::Orientation::Horizontal));
    outer.append(&foot);

    // With no header bar, the empty space is where the dialog is dragged.
    let handle = gtk4::WindowHandle::new();
    handle.set_child(Some(&outer));

    let dialog = adw::Dialog::builder()
        .title(i18n("Welcome to Big Game Mode"))
        .content_width(900)
        .content_height(650)
        .child(&handle)
        .build();
    dialog.add_css_class("welcome-dialog");
    start.connect_clicked(glib::clone!(
        #[weak]
        dialog,
        move |_| {
            dialog.close();
        }
    ));
    dialog.set_default_widget(Some(&start));
    dialog.present(Some(parent));
}
