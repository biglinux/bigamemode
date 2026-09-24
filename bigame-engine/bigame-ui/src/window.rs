//! Main application window with `AdwNavigationSplitView` sidebar navigation.

use adw::prelude::*;
use gtk4::{gio, glib};
use libadwaita as adw;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use crate::i18n::i18n;
use crate::settings;
use crate::views;
use crate::widgets;

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

    // Home is the landing page and carries the single Turbo control. The
    // report page is rebuilt on demand, so it always reflects the latest run
    // rather than a stale snapshot of an earlier one.
    let report_holder: Rc<RefCell<Option<gtk4::Widget>>> = Rc::new(RefCell::new(None));
    let page_title = adw::WindowTitle::new(&i18n("Home"), "");
    // A "Back to Home" button, shown only while the report is on screen.
    //
    // The report is not a sidebar destination — it has no row of its own — so
    // without this the sidebar and the window title would still say "Home"
    // while it is shown, with no obvious way out. Showing where you are, and
    // how to leave, is the minimum.
    let back_button = gtk4::Button::builder()
        .icon_name("go-previous-symbolic")
        .tooltip_text(i18n("Back to Home"))
        .visible(false)
        .build();
    back_button.update_property(&[gtk4::accessible::Property::Label(&i18n("Back to Home"))]);

    let show_report: Rc<dyn Fn(&bigame_core::turbo::Report)> = {
        let stack = view_stack.clone();
        let holder = Rc::clone(&report_holder);
        let title = page_title.clone();
        let back = back_button.clone();
        Rc::new(move |report| {
            if let Some(old) = holder.borrow_mut().take() {
                stack.remove(&old);
            }
            let page = views::report::build(report);
            stack.add_named(&page, Some("report"));
            *holder.borrow_mut() = Some(page);
            stack.set_visible_child_name("report");
            title.set_title(&i18n("Optimization Report"));
            back.set_visible(true);
        })
    };
    {
        let stack = view_stack.clone();
        back_button.connect_clicked(move |_| stack.set_visible_child_name("home"));
    }

    let home = views::home::build(Rc::clone(&show_report));
    view_stack.add_named(&home, Some("home"));

    let dashboard = views::dashboard::build();
    view_stack.add_named(&dashboard, Some("dashboard"));

    let profiles = views::profiles::build();
    view_stack.add_named(&profiles, Some("profiles"));

    // Wrap in Rc<RefCell> so the Restore Defaults action can swap it without a rebuild
    let tuning_holder = Rc::new(RefCell::new(views::tuning::build()));
    view_stack.add_named(&*tuning_holder.borrow(), Some("tuning"));

    let video_view = views::video::build();
    view_stack.add_named(&video_view, Some("video"));

    let benchmark = views::benchmark::build();
    view_stack.add_named(&benchmark, Some("benchmark"));

    let diagnostics = views::diagnostics::build();
    view_stack.add_named(&diagnostics, Some("diagnostics"));

    let logs = views::logs::build();
    view_stack.add_named(&logs, Some("logs"));

    let settings_view = views::settings::build();
    view_stack.add_named(&settings_view, Some("settings"));

    // ── Content: header + view stack wrapped in toast overlay ────────
    let content_header = adw::HeaderBar::new();
    content_header.set_title_widget(Some(&page_title));
    content_header.pack_start(&back_button);

    let toast_overlay = adw::ToastOverlay::new();
    toast_overlay.set_child(Some(&view_stack));

    let content_view = adw::ToolbarView::new();
    content_view.add_top_bar(&content_header);
    content_view.set_content(Some(&toast_overlay));

    let content_page = adw::NavigationPage::builder()
        .title("BiGame-mode")
        .child(&content_view)
        .build();

    // ── Sidebar: nav list (icon + label rows) ─────────────────────────
    let nav_items = [
        ("home", i18n("Home"), "go-home-symbolic"),
        ("dashboard", i18n("Details"), "speedometer-symbolic"),
        ("profiles", i18n("Profiles"), "applications-games-symbolic"),
        ("tuning", i18n("Tuning"), "preferences-system-symbolic"),
        ("video", i18n("Video"), "video-display-symbolic"),
        (
            "benchmark",
            i18n("Benchmark"),
            "applications-science-symbolic",
        ),
        (
            "diagnostics",
            i18n("Diagnostics"),
            "dialog-question-symbolic",
        ),
        ("logs", i18n("Logs"), "utilities-terminal-symbolic"),
        ("settings", i18n("Settings"), "emblem-system-symbolic"),
    ];

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
        .label("BiGame-mode")
        .css_classes(["title"])
        .build();
    sidebar_header.set_title_widget(Some(&app_label));

    let sidebar_view = adw::ToolbarView::new();
    sidebar_view.add_top_bar(&sidebar_header);
    sidebar_view.set_content(Some(&sidebar_scroll));

    let sidebar_page = adw::NavigationPage::builder()
        .title("BiGame-mode")
        .child(&sidebar_view)
        .build();

    // ── Split view ────────────────────────────────────────────────────
    let nav_split = adw::NavigationSplitView::new();
    nav_split.set_sidebar(Some(&sidebar_page));
    nav_split.set_content(Some(&content_page));

    let saved = settings::load();

    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title("BiGame-mode")
        .default_width(saved.window_width)
        .default_height(saved.window_height)
        .content(&nav_split)
        .build();

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

    // ── Restore last active tab ───────────────────────────────────────
    {
        let tab = saved.last_tab.clone();
        // "report" is rebuilt per run and does not exist at startup.
        let tab = if tab == "report" { String::new() } else { tab };
        if !tab.is_empty() {
            view_stack.set_visible_child_name(&tab);
            let mut idx = 0i32;
            while let Some(row) = sidebar_list.row_at_index(idx) {
                if row.widget_name() == tab.as_str() {
                    sidebar_list.select_row(Some(&row));
                    if let Some(ar) = row.downcast_ref::<adw::ActionRow>() {
                        page_title.set_title(&ar.title());
                    }
                    break;
                }
                idx += 1;
            }
        }
    }

    // ── Persist active tab on switch ──────────────────────────────────
    {
        let stack = view_stack.clone();
        let back = back_button.clone();
        let title = page_title.clone();
        let list = sidebar_list.clone();
        view_stack.connect_visible_child_name_notify(move |_| {
            let Some(name) = stack.visible_child_name() else {
                return;
            };
            let name = name.to_string();

            // Leaving the report by any route — the back button, the sidebar,
            // a keyboard shortcut — must put the header back.
            if name != "report" {
                back.set_visible(false);
                let mut idx = 0i32;
                while let Some(row) = list.row_at_index(idx) {
                    if row.widget_name() == name.as_str() {
                        if let Some(ar) = row.downcast_ref::<adw::ActionRow>() {
                            title.set_title(&ar.title());
                        }
                        break;
                    }
                    idx += 1;
                }
            }

            // The report is rebuilt per run and does not exist at startup, so
            // remembering it as the last tab would restore an empty page.
            if name == "report" {
                return;
            }
            let mut s = settings::load();
            s.last_tab = name;
            settings::save(&s);
        });
    }

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

    // ── Theme toggle (win.toggle-dark) ────────────────────────────────
    let style_mgr = adw::StyleManager::default();
    let is_dark = style_mgr.is_dark();
    let theme_action = gio::SimpleAction::new_stateful("toggle-dark", None, &is_dark.to_variant());
    theme_action.connect_activate(|action, _| {
        let mgr = adw::StyleManager::default();
        let dark = action
            .state()
            .and_then(|v| v.get::<bool>())
            .unwrap_or(false);
        let scheme = if dark {
            adw::ColorScheme::ForceLight
        } else {
            adw::ColorScheme::ForceDark
        };
        mgr.set_color_scheme(scheme);
        action.set_state(&(!dark).to_variant());
    });
    window.add_action(&theme_action);

    // ── Restore Defaults (win.restore-defaults) ───────────────────────
    let restore_action = gio::SimpleAction::new("restore-defaults", None);
    {
        let stack = view_stack.clone();
        let overlay = toast_overlay.clone();
        let win = window.clone();
        let th = Rc::clone(&tuning_holder);
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
            let th2 = Rc::clone(&th);
            dialog.connect_response(None, move |_, response| {
                if response != "restore" { return; }
                // Write default gamescope config (user-space), then falcond's
                // through the helper -- off the main thread, since it may wait
                // on a password prompt -- and say what actually happened.
                let gamescope = bigame_core::gamescope::save_global(&bigame_core::gamescope::Config::default());
                let overlay3 = overlay2.clone();
                glib::spawn_future_local(async move {
                    let falcond = gtk4::gio::spawn_blocking(|| {
                        bigame_core::config::write_blocking(&bigame_core::config::FalcondConfig::default())
                    })
                    .await;
                    let message = match (falcond, gamescope) {
                        (Ok(Ok(())), Ok(())) => i18n("Default settings restored"),
                        (Ok(Err(e)), _) => format!("{}: {e:#}", i18n("Could not restore falcond's settings")),
                        (Err(_), _) => i18n("Could not restore falcond's settings"),
                        (_, Err(e)) => format!("{}: {e:#}", i18n("Could not restore Gamescope's settings")),
                    };
                    overlay3.add_toast(adw::Toast::new(&message));
                });
                // Swap tuning page in the view stack
                let old = th2.borrow().clone();
                stack2.remove(&old);
                let new_tuning = views::tuning::build();
                stack2.add_named(&new_tuning, Some("tuning"));
                *th2.borrow_mut() = new_tuning;
                // Navigate to tuning so user sees the reset values
                stack2.set_visible_child_name("tuning");
            });
            dialog.present(Some(&win));
        });
    }
    window.add_action(&restore_action);

    // ── Main menu button (content header, end) ────────────────────────
    let menu = adw::gio::Menu::new();
    menu.append(Some(&i18n("Toggle Dark Mode")), Some("win.toggle-dark"));
    menu.append(
        Some(&i18n("Restore Defaults")),
        Some("win.restore-defaults"),
    );
    menu.append(Some(&i18n("About BiGame-mode")), Some("app.about"));
    menu.append(Some(&i18n("Quit")), Some("app.quit"));

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
        let stack = view_stack.clone();
        info_btn.connect_clicked(move |btn| {
            let tab = stack
                .visible_child_name()
                .map(|s| s.to_string())
                .unwrap_or_default();
            widgets::tutorial::show(btn, &tab);
        });
    }

    content_header.pack_end(&menu_btn);
    content_header.pack_end(&info_btn);
    content_header.pack_end(error_indicator.widget());

    (window, error_indicator)
}
