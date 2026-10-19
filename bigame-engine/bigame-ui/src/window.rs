//! Main application window with `AdwNavigationSplitView` sidebar navigation.
//!
//! Six pages, in the order a person uses them: Home (Turbo and the game
//! being played), Games (the library and each game's profile),
//! Optimization (what would help, the general configuration, what is in
//! force, the network and the programs beside the game), Telemetry (the
//! live charts), Records (the Turbo report, the games that crashed, the
//! logs) and Settings. Optimization and Records are pages of tabs; simple
//! mode hides their advanced tabs (Tuning, Details, Logs).
//!
//! A place is a page, or a page and one of its tabs: `optimization/tuning`.
//! `win.go` opens one, from anywhere in the window.

use adw::prelude::*;
use gtk4::{gio, glib};
use libadwaita as adw;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use crate::i18n::{error_text, i18n};
use crate::settings;
use crate::views;
use crate::widgets;

/// The pages, by stack name. A saved page name from an older version
/// maps to where its content went ([`migrate_tab`]).
pub const PAGES: [&str; 6] = [
    "home",
    "games",
    "optimization",
    "telemetry",
    "records",
    "settings",
];

/// Open a place (`page` or `page/tab`).
type Go = Rc<dyn Fn(&str)>;

/// Where a saved place (`page` or `page/tab`), from this version or an
/// earlier one, leads now: the page, and the tab when it names one.
#[must_use]
pub fn migrate_tab(name: &str) -> (&'static str, Option<&'static str>) {
    let (page, tab) = name.split_once('/').unwrap_or((name, ""));
    let tab_of = |tabs: &[&'static str]| tabs.iter().copied().find(|t| *t == tab);
    match page {
        // Before 3.0.0 each of these was a page of its own.
        "profiles" => ("games", None),
        "tuning" | "video" => ("optimization", Some("tuning")),
        "dashboard" | "benchmark" | "diagnostics" => ("optimization", Some("details")),
        "logs" => ("records", Some("logs")),
        "report" => ("records", Some("reports")),
        "optimization" => ("optimization", tab_of(&views::optimization::TABS)),
        "records" => ("records", tab_of(&views::records::TABS)),
        other => (
            PAGES
                .iter()
                .copied()
                .find(|p| *p == other)
                .unwrap_or("home"),
            None,
        ),
    }
}

/// The help for a place: its page's, or its tab's when the tab has its own.
#[must_use]
pub fn help_key(page: &str, tab: &str) -> String {
    match (page, tab) {
        ("optimization", "tuning" | "details") | ("records", "logs" | "crashes") => tab.to_owned(),
        ("records", _) => "report".to_owned(),
        _ => page.to_owned(),
    }
}

/// Build the main application window.
///
/// Layout: `AdwNavigationSplitView` with a persistent sidebar (navigation list)
/// + content area (`AdwViewStack` driven by sidebar selection).
///
/// On narrow screens the split view collapses to show one pane at a time.
#[allow(clippy::too_many_lines)]
// The error indicator is shared with the application's status timer, which
// runs on the GTK main loop. `Arc` rather than `Rc` because the tuple crosses
// an API boundary; GTK widgets are not `Send`, so this never leaves the main
// thread in practice.
#[allow(clippy::arc_with_non_send_sync)]
pub fn build(
    app: &adw::Application,
) -> (
    adw::ApplicationWindow,
    Arc<crate::widgets::error_indicator::ErrorIndicator>,
) {
    let error_indicator = Arc::new(crate::widgets::error_indicator::ErrorIndicator::new());
    // ── View stack (content driven by sidebar) ───────────────────────
    let view_stack = adw::ViewStack::new();

    let page_title = adw::WindowTitle::new(&i18n("Home"), "");

    // Records is built first: Home shows its Turbo report there.
    let records = views::records::Records::build();
    let optimization = views::optimization::Optimization::build();
    // A Turbo report is kept in Records, and shown there when asked.
    let show_report: views::home::ShowReport = {
        let stack = view_stack.clone();
        let records = Rc::clone(&records);
        Rc::new(move |report, open| {
            records.show_report(report, open);
            if open {
                stack.set_visible_child_name("records");
            }
        })
    };

    // A launcher opened before a Turbo change is asked to reopen from the
    // top of the window, whichever page is shown.
    let launcher_notice = widgets::launcher_notice::LauncherNotice::new(app);

    let home = views::home::build(app, Rc::clone(&show_report), Rc::clone(&launcher_notice));
    view_stack.add_named(&home, Some("home"));

    let games = views::profiles::build();
    view_stack.add_named(&games, Some("games"));

    view_stack.add_named(optimization.widget(), Some("optimization"));

    let telemetry = views::telemetry::build();
    view_stack.add_named(&telemetry, Some("telemetry"));

    view_stack.add_named(records.widget(), Some("records"));

    let settings_view = views::settings::build();
    view_stack.add_named(&settings_view, Some("settings"));

    // ── Content: header + view stack wrapped in toast overlay ────────
    let content_header = adw::HeaderBar::new();
    content_header.set_title_widget(Some(&page_title));

    // A game that closed with an error is said there too, under it.
    let crash_notice = widgets::crash_notice::CrashNotice::new();
    let notices = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    notices.set_valign(gtk4::Align::Start);
    notices.append(launcher_notice.widget());
    notices.append(crash_notice.widget());

    let pages = gtk4::Overlay::new();
    pages.set_child(Some(&view_stack));
    pages.add_overlay(&notices);

    let toast_overlay = adw::ToastOverlay::new();
    toast_overlay.set_child(Some(&pages));

    let content_view = adw::ToolbarView::new();
    content_view.add_top_bar(&content_header);
    content_view.set_content(Some(&toast_overlay));

    let content_page = adw::NavigationPage::builder()
        .title(crate::app::NAME)
        .child(&content_view)
        .build();

    // ── Sidebar: nav list (icon + label rows) ─────────────────────────
    let nav_items = [
        ("home", i18n("Home"), "go-home-symbolic"),
        ("games", i18n("Games"), "applications-games-symbolic"),
        ("optimization", i18n("Optimization"), "speedometer-symbolic"),
        ("telemetry", i18n("Telemetry"), "bigame-telemetry-symbolic"),
        ("records", i18n("Records"), "document-open-recent-symbolic"),
        ("settings", i18n("Settings"), "emblem-system-symbolic"),
    ];
    debug_assert_eq!(nav_items.len(), PAGES.len());

    let sidebar_list = gtk4::ListBox::new();
    sidebar_list.set_selection_mode(gtk4::SelectionMode::Single);
    sidebar_list.add_css_class("navigation-sidebar");

    for (id, label, icon) in &nav_items {
        let row = adw::ActionRow::new();
        row.set_activatable(true);
        row.set_widget_name(id);
        row.set_title(label.as_str());
        let img = gtk4::Image::from_icon_name(icon);
        img.set_pixel_size(16);
        row.add_prefix(&img);
        sidebar_list.append(&row);
    }

    // Select first row initially
    if let Some(row) = sidebar_list.row_at_index(0) {
        sidebar_list.select_row(Some(&row));
    }

    let sidebar_scroll = gtk4::ScrolledWindow::new();
    sidebar_scroll.set_child(Some(&sidebar_list));
    sidebar_scroll.set_vexpand(true);
    sidebar_scroll.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);

    let sidebar_header = adw::HeaderBar::new();
    let app_label = gtk4::Label::builder()
        .label(crate::app::NAME)
        .css_classes(["title"])
        .build();
    sidebar_header.set_title_widget(Some(&app_label));
    show_app_icon(&sidebar_header);

    let sidebar_view = adw::ToolbarView::new();
    sidebar_view.add_top_bar(&sidebar_header);
    sidebar_view.set_content(Some(&sidebar_scroll));

    let sidebar_page = adw::NavigationPage::builder()
        .title(crate::app::NAME)
        .child(&sidebar_view)
        .build();

    // ── Split view ────────────────────────────────────────────────────
    let nav_split = adw::NavigationSplitView::new();
    // Wide enough for the longest page name ("Configurações") on one line;
    // at libadwaita's 180 px default it broke with a hyphen in a narrow window.
    nav_split.set_min_sidebar_width(200.0);
    nav_split.set_sidebar(Some(&sidebar_page));
    nav_split.set_content(Some(&content_page));

    let saved = settings::load();

    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title(crate::app::NAME)
        .default_width(saved.window_width)
        .default_height(saved.window_height)
        .content(&nav_split)
        .build();

    // A notice's buttons act at once on another program (reopen Steam or
    // Heroic, rewrite a game's launch options). When the window comes to
    // the front, the focus is never left on one of them: the keys a player
    // was pressing in a game the moment the window took the focus would
    // otherwise press it (seen on the reference desktop: Heroic "reopened"
    // three times by keys meant for The Callisto Protocol). They stay a
    // click, or a Tab, away.
    {
        let notices = notices.clone();
        window.connect_is_active_notify(move |w| {
            if w.is_active()
                && gtk4::prelude::GtkWindowExt::focus(w).is_some_and(|f| f.is_ancestor(&notices))
            {
                gtk4::prelude::GtkWindowExt::set_focus(w, None::<&gtk4::Widget>);
            }
        });
    }

    // ── Sidebar row selection → switch content + update title ─────────
    {
        let stack = view_stack.clone();
        let title = page_title.clone();
        let split = nav_split.clone();
        sidebar_list.connect_row_selected(move |_, row| {
            let Some(row) = row else { return };
            let id = row.widget_name().to_string();
            stack.set_visible_child_name(&id);
            if let Some(ar) = row.downcast_ref::<adw::ActionRow>() {
                title.set_title(&ar.title());
            }
            // On narrow (collapsed) mode navigate to content pane
            if split.is_collapsed() {
                split.set_show_content(true);
            }
        });
    }

    // ── Places: a page, or a page and one of its tabs ─────────────────
    let go: Go = {
        let stack = view_stack.clone();
        let (optimization, records) = (Rc::clone(&optimization), Rc::clone(&records));
        Rc::new(move |place| {
            let (page, tab) = migrate_tab(place);
            if let Some(tab) = tab {
                match page {
                    "optimization" => {
                        optimization.tabs().show(tab);
                    }
                    "records" => {
                        records.tabs().show(tab);
                    }
                    _ => {}
                }
            }
            stack.set_visible_child_name(page);
        })
    };
    // The page on screen and its tab, as a place to come back to.
    let current_place: Rc<dyn Fn() -> String> = {
        let stack = view_stack.clone();
        let (optimization, records) = (Rc::clone(&optimization), Rc::clone(&records));
        Rc::new(move || {
            let page = stack
                .visible_child_name()
                .map(|n| n.to_string())
                .unwrap_or_default();
            match page.as_str() {
                "optimization" => format!("{page}/{}", optimization.tabs().current()),
                "records" => format!("{page}/{}", records.tabs().current()),
                _ => page,
            }
        })
    };

    // ── The sidebar and the title follow the page ─────────────────────
    {
        let stack = view_stack.clone();
        let title = page_title.clone();
        let list = sidebar_list.clone();
        let place = Rc::clone(&current_place);
        view_stack.connect_visible_child_name_notify(move |_| {
            let Some(name) = stack.visible_child_name() else {
                return;
            };
            let mut idx = 0i32;
            while let Some(row) = list.row_at_index(idx) {
                if row.widget_name() == name.as_str() {
                    if !row.is_selected() {
                        list.select_row(Some(&row));
                    }
                    if let Some(ar) = row.downcast_ref::<adw::ActionRow>() {
                        title.set_title(&ar.title());
                    }
                    break;
                }
                idx += 1;
            }
            let mut s = settings::load();
            s.last_tab = place();
            settings::save(&s);
        });
    }
    // A tab chosen is remembered too.
    for tabs in [optimization.tabs().clone(), records.tabs().clone()] {
        let place = Rc::clone(&current_place);
        let stack = view_stack.clone();
        let widget = tabs.widget().clone();
        tabs.connect_changed(move |_| {
            if stack.visible_child().as_ref() != Some(widget.upcast_ref()) {
                return;
            }
            let mut s = settings::load();
            s.last_tab = place();
            settings::save(&s);
        });
    }

    // ── Restore the last place ────────────────────────────────────────
    go(&saved.last_tab);

    // ── win.go: open a place from anywhere in the window ──────────────
    let go_action = gio::SimpleAction::new("go", Some(glib::VariantTy::STRING));
    {
        let go = Rc::clone(&go);
        let split = nav_split.clone();
        go_action.connect_activate(move |_, place| {
            if let Some(place) = place.and_then(glib::Variant::get::<String>) {
                go(&place);
                if split.is_collapsed() {
                    split.set_show_content(true);
                }
            }
        });
    }
    window.add_action(&go_action);

    // ── Persist window geometry on close ──────────────────────────────
    {
        let win = window.clone();
        window.connect_close_request(move |_| {
            let mut s = settings::load();
            s.maximized = win.is_maximized();
            if !win.is_maximized() {
                let w = win.width();
                let h = win.height();
                if w > 0 {
                    s.window_width = w;
                }
                if h > 0 {
                    s.window_height = h;
                }
            }
            settings::save(&s);
            glib::Propagation::Proceed
        });
    }

    if saved.maximized {
        window.maximize();
    }

    // ── Restore Defaults (win.restore-defaults) ───────────────────────
    let restore_action = gio::SimpleAction::new("restore-defaults", None);
    {
        let stack = view_stack.clone();
        let overlay = toast_overlay.clone();
        let win = window.clone();
        let opt = Rc::clone(&optimization);
        let go = Rc::clone(&go);
        restore_action.connect_activate(move |_, _| {
            let dialog = adw::AlertDialog::new(
                Some(&i18n("Restore Defaults")),
                Some(&i18n("Reset all Tuning and Gamescope settings to recommended defaults. This cannot be undone.")),
            );
            dialog.add_response("cancel", &i18n("Cancel"));
            dialog.add_response("restore", &i18n("Restore"));
            dialog.set_response_appearance("restore", adw::ResponseAppearance::Destructive);
            dialog.set_default_response(Some("cancel"));
            dialog.set_close_response("cancel");
            let stack2 = stack.clone();
            let overlay2 = overlay.clone();
            let opt2 = Rc::clone(&opt);
            let go2 = Rc::clone(&go);
            dialog.connect_response(None, move |_, response| {
                if response != "restore" {
                    return;
                }
                restore_and_rebuild(
                    &stack2,
                    &overlay2,
                    &opt2,
                    Some(Rc::clone(&go2)),
                    i18n("Default settings restored"),
                );
            });
            dialog.present(Some(&win));
        });
    }
    window.add_action(&restore_action);

    // ── Back to normal (win.back-to-normal) ───────────────────────────
    // The one way out for anyone: Turbo off, the choice back to Fastest, and
    // everything Restore Defaults resets. Games and their files stay.
    let normal_action = gio::SimpleAction::new("back-to-normal", None);
    {
        let stack = view_stack.clone();
        let overlay = toast_overlay.clone();
        let win = window.clone();
        let opt = Rc::clone(&optimization);
        let app = app.clone();
        normal_action.connect_activate(move |_, _| {
            let dialog = adw::AlertDialog::new(
                Some(&i18n("Back to normal?")),
                Some(&i18n(
                    "Turbo turns off, the choice goes back to Fastest, and every setting goes back to how it started. Your games are not touched.",
                )),
            );
            dialog.add_response("cancel", &i18n("Cancel"));
            dialog.add_response("normal", &i18n("Back to normal"));
            dialog.set_response_appearance("normal", adw::ResponseAppearance::Suggested);
            dialog.set_default_response(Some("cancel"));
            dialog.set_close_response("cancel");
            let (stack, overlay, opt, app) = (
                stack.clone(),
                overlay.clone(),
                Rc::clone(&opt),
                app.clone(),
            );
            dialog.connect_response(None, move |_, response| {
                if response != "normal" {
                    return;
                }
                turbo_off_and_fastest(&app);
                restore_and_rebuild(
                    &stack,
                    &overlay,
                    &opt,
                    None,
                    i18n("Everything is back to normal"),
                );
            });
            dialog.present(Some(&win));
        });
    }
    window.add_action(&normal_action);

    // ── Simple and advanced mode ──────────────────────────────────────
    // Tuning, Details and Logs only in advanced mode; a tab hidden while it
    // is shown gives way to its page's first.
    {
        let (optimization, records) = (Rc::clone(&optimization), Rc::clone(&records));
        settings::follow_advanced(move |advanced| {
            optimization.set_advanced(advanced);
            records.set_advanced(advanced);
        });
    }

    // ── Welcome screen (win.welcome) ──────────────────────────────────
    let welcome_action = gio::SimpleAction::new("welcome", None);
    {
        let win = window.clone();
        welcome_action.connect_activate(move |_, _| widgets::welcome::show(&win));
    }
    window.add_action(&welcome_action);
    // Once per run, the first time the window is on screen: a start hidden
    // in the tray shows it when the window is first opened from there.
    {
        let offered = std::cell::Cell::new(false);
        window.connect_map(move |win| {
            if offered.replace(true) || !settings::load().show_welcome {
                return;
            }
            // After the window has its size, so the dialog is laid out over
            // it rather than over a window still being mapped.
            let win = win.clone();
            glib::idle_add_local_once(move || widgets::welcome::show(&win));
        });
    }

    // ── Main menu button (content header, end) ────────────────────────
    let menu = adw::gio::Menu::new();
    let tools = adw::gio::Menu::new();
    tools.append(Some(&i18n("Back to normal")), Some("win.back-to-normal"));
    tools.append(
        Some(&i18n("Restore Defaults")),
        Some("win.restore-defaults"),
    );
    menu.append_section(None, &tools);
    let app_items = adw::gio::Menu::new();
    app_items.append(Some(&i18n("Welcome")), Some("win.welcome"));
    app_items.append(Some(&i18n("About")), Some("app.about"));
    app_items.append(Some(&i18n("Quit")), Some("app.quit"));
    menu.append_section(None, &app_items);

    let menu_btn = gtk4::MenuButton::builder()
        .icon_name("open-menu-symbolic")
        .menu_model(&menu)
        .primary(true)
        .tooltip_text(i18n("Main Menu"))
        .build();
    // ── Info button: context-aware tutorial ──────────────────────────
    let info_btn = gtk4::Button::builder()
        .icon_name("dialog-information-symbolic")
        .tooltip_text(i18n("Help & Tutorial"))
        .css_classes(["flat"])
        .build();
    {
        let place = Rc::clone(&current_place);
        info_btn.connect_clicked(move |btn| {
            let place = place();
            let (page, tab) = place.split_once('/').unwrap_or((place.as_str(), ""));
            widgets::tutorial::show(btn, &help_key(page, tab));
        });
    }

    content_header.pack_end(&menu_btn);
    content_header.pack_end(&info_btn);
    content_header.pack_end(error_indicator.widget());

    (window, error_indicator)
}

/// Size of the application icon at the head of the sidebar.
const APP_ICON_SIZE: i32 = 24;

/// Show the application icon at the start of the sidebar's header bar, at
/// [`APP_ICON_SIZE`], where the desktop's title-bar layout asks for one.
///
/// The window controls draw the layout's `icon` at 16 px from a texture of
/// that size, so enlarging it with CSS only blurs it. The header bar is
/// given the layout without it, and an image of its own takes its place —
/// kept in step when the desktop's layout changes.
fn show_app_icon(header: &adw::HeaderBar) {
    let image = gtk4::Image::from_icon_name(crate::app::APP_ID);
    image.set_pixel_size(APP_ICON_SIZE);
    image.add_css_class("sidebar-app-icon");
    header.pack_start(&image);

    let Some(settings) = gtk4::Settings::default() else {
        return;
    };
    let apply = {
        let header = header.clone();
        let image = image.clone();
        move |settings: &gtk4::Settings| {
            let layout = settings.gtk_decoration_layout().unwrap_or_default();
            if layout.is_empty() {
                // No layout to adjust: GTK's own default, which has no icon.
                header.set_decoration_layout(None);
                image.set_visible(false);
                return;
            }
            let (layout, had_icon) = layout_without_icon(&layout);
            header.set_decoration_layout(Some(&layout));
            image.set_visible(had_icon);
        }
    };
    apply(&settings);
    settings.connect_gtk_decoration_layout_notify(apply);
}

/// A title-bar layout (`icon:minimize,maximize,close`) without the window
/// icon, and whether the icon was on its start side, before the colon.
fn layout_without_icon(layout: &str) -> (String, bool) {
    let strip = |side: &str| {
        side.split(',')
            .filter(|item| item.trim() != "icon")
            .collect::<Vec<_>>()
            .join(",")
    };
    match layout.split_once(':') {
        Some((start, end)) => (
            format!("{}:{}", strip(start), strip(end)),
            start.split(',').any(|item| item.trim() == "icon"),
        ),
        None => (
            strip(layout),
            layout.split(',').any(|item| item.trim() == "icon"),
        ),
    }
}

/// What Restore Defaults wrote, part by part.
struct Restored {
    falcond: anyhow::Result<()>,
    /// `Ok(Some(_))`: saved, but the running session was not updated.
    launch: anyhow::Result<Option<anyhow::Error>>,
    lsfg: anyhow::Result<bool>,
}

/// Put Tuning back to its defaults: falcond's settings (through the helper)
/// and the launch settings (`video.toml`, the session environment, and
/// lsfg-vk's entries, which follow its general switch).
/// Write the defaults off the main thread, say how it went (`done_text`
/// when every part was written), and rebuild Tuning so it reads back what
/// is in force; with `go`, Tuning is shown afterwards.
fn restore_and_rebuild(
    stack: &adw::ViewStack,
    overlay: &adw::ToastOverlay,
    optimization: &Rc<views::optimization::Optimization>,
    go: Option<Go>,
    done_text: String,
) {
    let (stack, overlay, optimization) = (stack.clone(), overlay.clone(), Rc::clone(optimization));
    // Off the main thread: falcond's goes through the helper, which may
    // wait on a password prompt, and the launch settings through the
    // session bus.
    glib::spawn_future_local(async move {
        let done = gio::spawn_blocking(restore_defaults).await;
        report_restore(&overlay, done.ok(), &done_text);
        // Rebuilt only once both are written, so the page reads back what
        // is now in force.
        optimization.rebuild_tuning();
        if let Some(go) = go {
            go("optimization/tuning");
        }
        views::tuning::launch_settings_changed(stack.upcast_ref());
    });
}

/// Turn Turbo off, through `app.turbo` as the tray does, and bring the
/// choice back to Fastest once the picker takes a choice again.
fn turbo_off_and_fastest(app: &adw::Application) {
    let fastest = bigame_core::turbo_preset::Preset::Fastest.id().to_variant();
    let on = app
        .action_state("turbo")
        .and_then(|v| v.get::<bool>())
        .unwrap_or(false);
    if !on {
        app.change_action_state("turbo-preset", &fastest);
        return;
    }
    let pending: Rc<RefCell<Option<glib::SignalHandlerId>>> = Rc::default();
    let handler = {
        let pending = Rc::clone(&pending);
        app.connect_action_enabled_changed(Some("turbo-preset"), move |app, _, enabled| {
            let off = app.action_state("turbo").and_then(|v| v.get::<bool>()) == Some(false);
            if enabled && off {
                app.change_action_state("turbo-preset", &fastest);
                if let Some(id) = pending.borrow_mut().take() {
                    app.disconnect(id);
                }
            }
        })
    };
    *pending.borrow_mut() = Some(handler);
    app.change_action_state("turbo", &false.to_variant());
}

fn restore_defaults() -> Restored {
    let video = bigame_core::video_config::VideoConfig::default();
    Restored {
        falcond: bigame_core::config::write_blocking(&bigame_core::config::FalcondConfig::default()),
        launch: bigame_core::video_config::save(&video),
        lsfg: bigame_core::fg::sync_global_enablement(&video.frame_gen),
    }
}

/// Say what Restore Defaults did: success only when every part was written.
fn report_restore(overlay: &adw::ToastOverlay, done: Option<Restored>, done_text: &str) {
    let Some(done) = done else {
        widgets::toast::error(
            overlay,
            &i18n("Could not restore falcond's settings"),
            &i18n("the worker thread stopped"),
        );
        return;
    };
    let mut ok = true;
    if let Err(e) = &done.falcond {
        ok = false;
        widgets::toast::error(
            overlay,
            &i18n("Could not restore falcond's settings"),
            &error_text(e),
        );
    }
    match &done.launch {
        Err(e) => {
            ok = false;
            widgets::toast::error(
                overlay,
                &i18n("Could not restore Gamescope's settings"),
                &error_text(e),
            );
        }
        Ok(Some(e)) => {
            ok = false;
            widgets::toast::error(
                overlay,
                &views::tuning::session_not_updated(),
                &error_text(e),
            );
        }
        Ok(None) => {}
    }
    if let Err(e) = &done.lsfg {
        ok = false;
        widgets::toast::error(
            overlay,
            &i18n("Could not update lsfg-vk's file"),
            &error_text(e),
        );
    }
    if ok {
        overlay.add_toast(adw::Toast::new(done_text));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_window_icon_leaves_the_layout_and_says_where_it_was() {
        assert_eq!(
            layout_without_icon("icon:minimize,maximize,close"),
            (":minimize,maximize,close".to_owned(), true)
        );
        assert_eq!(
            layout_without_icon("appmenu:close"),
            ("appmenu:close".to_owned(), false)
        );
        // On the end side it is removed too, but no icon goes at the start.
        assert_eq!(
            layout_without_icon("close:minimize,icon"),
            ("close:minimize".to_owned(), false)
        );
        assert_eq!(
            layout_without_icon("menu,icon:close"),
            ("menu:close".to_owned(), true)
        );
        assert_eq!(layout_without_icon(""), (String::new(), false));
    }

    #[test]
    fn the_pages_of_earlier_versions_lead_to_where_their_content_went() {
        assert_eq!(migrate_tab("profiles"), ("games", None));
        assert_eq!(migrate_tab("tuning"), ("optimization", Some("tuning")));
        assert_eq!(migrate_tab("video"), ("optimization", Some("tuning")));
        assert_eq!(migrate_tab("dashboard"), ("optimization", Some("details")));
        assert_eq!(migrate_tab("benchmark"), ("optimization", Some("details")));
        assert_eq!(
            migrate_tab("diagnostics"),
            ("optimization", Some("details"))
        );
        assert_eq!(migrate_tab("logs"), ("records", Some("logs")));
        assert_eq!(migrate_tab("report"), ("records", Some("reports")));
        assert_eq!(migrate_tab(""), ("home", None));
        assert_eq!(migrate_tab("nonsense"), ("home", None));
        for p in PAGES {
            assert_eq!(migrate_tab(p), (p, None));
        }
    }

    #[test]
    fn a_place_names_its_tab_and_an_unknown_tab_is_the_page() {
        assert_eq!(
            migrate_tab("optimization/around"),
            ("optimization", Some("around"))
        );
        assert_eq!(migrate_tab("records/crashes"), ("records", Some("crashes")));
        assert_eq!(migrate_tab("optimization/nonsense"), ("optimization", None));
        assert_eq!(migrate_tab("home/whatever"), ("home", None));
    }

    #[test]
    fn every_place_has_help() {
        assert_eq!(help_key("optimization", "tuning"), "tuning");
        assert_eq!(help_key("optimization", "overview"), "optimization");
        assert_eq!(help_key("records", "reports"), "report");
        assert_eq!(help_key("records", "logs"), "logs");
        assert_eq!(help_key("games", ""), "games");
    }
}
