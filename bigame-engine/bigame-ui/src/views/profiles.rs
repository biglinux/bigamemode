//! Profiles view: game profile list with navigation to detail/editor.
//!
//! Uses `bigame_core::profiles` for CRUD operations.
//! Saves via pkexec to user profiles directory.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use gtk4::{gio, glib};
use libadwaita as adw;

use bigame_core::profiles::GameProfile;

use crate::i18n::i18n;
use crate::widgets::game_card;
use crate::widgets::toast;

/// Build the Profiles view with navigation stack.
#[must_use]
pub fn build() -> adw::NavigationView {
    let nav_view = adw::NavigationView::new();
    let list_page = build_list_page(&nav_view);
    nav_view.add(&list_page);
    nav_view
}

/// Build the profile list page.
#[allow(clippy::too_many_lines)]
fn build_list_page(nav_view: &adw::NavigationView) -> adw::NavigationPage {
    let page = adw::PreferencesPage::new();

    let group = adw::PreferencesGroup::new();
    group.set_title(&i18n("Game Library"));
    group.set_description(Some(&i18n(
        "Games found on this system, and the profiles that tune them.",
    )));

    // A poster grid rather than a list. The reference design this follows is a
    // library of cover art, and a library reads far faster as pictures than as
    // rows of text — especially when most entries are titles the user
    // recognises by their box art.
    let list_box = gtk4::FlowBox::builder()
        .selection_mode(gtk4::SelectionMode::None)
        .homogeneous(true)
        .column_spacing(18)
        .row_spacing(18)
        .min_children_per_line(2)
        .max_children_per_line(10)
        .valign(gtk4::Align::Start)
        .build();

    // Action buttons
    let add_btn = gtk4::Button::builder()
        .icon_name("list-add-symbolic")
        .tooltip_text(i18n("New Profile"))
        .css_classes(["circular", "flat"])
        .build();
    let import_btn = gtk4::Button::builder()
        .icon_name("document-open-symbolic")
        .tooltip_text(i18n("Import Profile"))
        .css_classes(["circular", "flat"])
        .build();
    let refresh_btn = gtk4::Button::builder()
        .icon_name("view-refresh-symbolic")
        .tooltip_text(i18n("Rescan game library"))
        .css_classes(["circular", "flat"])
        .build();
    let hdr = gtk4::Box::builder().spacing(4).build();
    hdr.append(&refresh_btn);
    hdr.append(&import_btn);
    hdr.append(&add_btn);
    group.set_header_suffix(Some(&hdr));

    group.add(&list_box);

    // Filled whenever the list comes on screen: the first time the page is
    // shown, on return from a detail or create page, and after a profile was
    // made elsewhere — from Home or the notification offer, which on the lab
    // VM left the new profile missing here until the rescan button.
    {
        let lb = list_box.clone();
        let nav_ref = nav_view.clone();
        list_box.connect_map(move |_| refresh_profile_list(&lb, &nav_ref));
    }

    // Refreshing is driven by navigation and by the explicit button above,
    // not by a timer. The previous two-second poll rebuilt every row forever,
    // which with cover art would mean re-reading the whole library twice a
    // second in a window the user may not even be looking at.
    {
        let lb = list_box.clone();
        let nav_ref = nav_view.clone();
        refresh_btn.connect_clicked(move |_| refresh_profile_list(&lb, &nav_ref));
    }

    page.add(&group);

    // "New Profile" → empty detail page
    {
        let nav = nav_view.clone();
        add_btn.connect_clicked(move |_| {
            nav.push(&build_detail_page_for(&GameProfile::default()));
        });
    }

    // Import profile from file
    {
        let nav = nav_view.clone();
        let lb = list_box.clone();
        import_btn.connect_clicked(move |btn| {
            let dialog = gtk4::FileDialog::builder()
                .title(i18n("Import Profile"))
                .build();
            let filter = gtk4::FileFilter::new();
            filter.add_pattern("*.conf");
            filter.add_pattern("*.toml");
            filter.set_name(Some("Profile files (*.conf, *.toml)"));
            let filters = gio::ListStore::new::<gtk4::FileFilter>();
            filters.append(&filter);
            dialog.set_filters(Some(&filters));

            let btn_ref = btn.clone();
            let nav_ref = nav.clone();
            let lb_ref = lb.clone();
            let win = btn.root().and_downcast::<gtk4::Window>();
            dialog.open(win.as_ref(), gio::Cancellable::NONE, move |result| {
                if let Ok(file) = result {
                    if let Some(path) = file.path() {
                        gtk4::glib::spawn_future_local(async move {
                            match bigame_core::profiles::import(&path) {
                                Ok(name) => {
                                    toast::show(&btn_ref, &i18n("Profile imported"));
                                    refresh_profile_list(&lb_ref, &nav_ref);
                                    nav_ref.push(&build_detail_page(&name));
                                }
                                Err(e) => {
                                    toast::show(
                                        &btn_ref,
                                        &i18n("Import failed: %s").replace("%s", &e.to_string()),
                                    );
                                }
                            }
                        });
                    }
                }
            });
        });
    }

    // Drag-and-drop import
    {
        let nav = nav_view.clone();
        let list_box = list_box.clone();
        let drop_target =
            gtk4::DropTarget::new(gio::File::static_type(), gtk4::gdk::DragAction::COPY);
        drop_target.connect_drop(move |target, value, _x, _y| {
            let Some(file) = value.get::<gio::File>().ok() else {
                return false;
            };
            let Some(path) = file.path() else {
                return false;
            };
            let list_box_ref = list_box.clone();
            let nav_ref = nav.clone();
            let target_ref = target.clone();
            gtk4::glib::spawn_future_local(async move {
                if let Ok(name) = bigame_core::profiles::import(&path) {
                    crate::views::profiles::refresh_profile_list(&list_box_ref, &nav_ref);
                    if let Some(widget) = target_ref.widget() {
                        toast::show(&widget, &i18n("Profile imported via drag-and-drop"));
                    }
                    nav_ref.push(&build_detail_page(&name));
                }
            });
            true
        });
        page.add_controller(drop_target);
    }

    // AdwPreferencesPage clamps its content to form width, which is right for
    // settings and wrong for a poster grid — it held the library to three
    // columns on a 1250 px window. The page keeps its structure and margins,
    // but the clamp is widened so the grid can use the space it has.
    if let Some(clamp) = find_clamp(page.upcast_ref::<gtk4::Widget>()) {
        clamp.set_maximum_size(1500);
        clamp.set_tightening_threshold(1200);
    }

    adw::NavigationPage::builder()
        .title(i18n("Game Library"))
        .child(&page)
        .build()
}

/// Locate the `AdwClamp` that `AdwPreferencesPage` builds internally.
///
/// There is no public API for this, so the widget tree is walked. Returning
/// `None` simply leaves the default clamp in place, which is a narrower grid
/// rather than a broken one.
fn find_clamp(widget: &gtk4::Widget) -> Option<adw::Clamp> {
    if let Ok(clamp) = widget.clone().downcast::<adw::Clamp>() {
        return Some(clamp);
    }
    let mut child = widget.first_child();
    while let Some(c) = child {
        if let Some(found) = find_clamp(&c) {
            return Some(found);
        }
        child = c.next_sibling();
    }
    None
}

/// Build detail page loading profile from disk by name.
fn build_detail_page(profile_name: &str) -> adw::NavigationPage {
    let profile = bigame_core::profiles::load(profile_name).unwrap_or_else(|_| GameProfile {
        name: profile_name.to_owned(),
        ..GameProfile::default()
    });
    build_detail_page_for(&profile)
}

/// Find index of `needle` in a `StringList`.
fn find_index(model: &gtk4::StringList, needle: &str) -> u32 {
    for i in 0..model.n_items() {
        if model.string(i).as_deref() == Some(needle) {
            return i;
        }
    }
    0
}

/// Build the performance widgets for the detail page.
#[allow(clippy::too_many_lines)]
fn build_perf_widgets(page: &adw::PreferencesPage, profile: &GameProfile) -> PerfWidgets {
    // Performance group
    let perf = adw::PreferencesGroup::new();
    perf.set_title(&i18n("Performance Settings"));

    let perf_mode = adw::SwitchRow::builder()
        .title(i18n("Performance Mode"))
        .subtitle(i18n("Enable system-wide optimizations"))
        .active(profile.performance_mode)
        .build();
    perf.add(&perf_mode);

    let idle_inhibit = adw::SwitchRow::builder()
        .title(i18n("Idle Inhibit"))
        .subtitle(i18n("Prevent screensaver while running"))
        .active(profile.idle_inhibit)
        .build();
    perf.add(&idle_inhibit);

    // No per-game CPU governor: falcond has no such field, so the control
    // this replaced saved a value that nothing ever applied.
    let installed = bigame_core::sched::detect_installed();
    let installed_refs: Vec<&str> = installed.iter().map(String::as_str).collect();
    let sched_model = gtk4::StringList::new(&installed_refs);
    let sched_row = adw::ComboRow::builder()
        .title(i18n("Scheduler"))
        .model(&sched_model)
        .build();
    sched_row.set_selected(find_index(&sched_model, &profile.scx_sched));

    let info_btn2 = gtk4::Button::builder()
        .icon_name("dialog-information-symbolic")
        .valign(gtk4::Align::Center)
        .css_classes(["flat", "circular"])
        .tooltip_text(i18n("Learn about Schedulers"))
        .build();
    info_btn2.connect_clicked(|btn| {
        if let Some(win) = btn.root().and_downcast::<gtk4::Window>() {
            crate::widgets::scheduler_info::show(&win);
        }
    });
    sched_row.add_suffix(&info_btn2);

    perf.add(&sched_row);

    let mode_model = gtk4::StringList::new(&["default", "gaming", "power", "latency", "server"]);
    let mode_row = adw::ComboRow::builder()
        .title(i18n("Scheduler Mode"))
        .model(&mode_model)
        .build();
    mode_row.set_selected(find_index(&mode_model, &profile.scx_sched_props));
    perf.add(&mode_row);

    // No custom scheduler flags either: falcond 2.0.2 does not read them.
    let vcache_model = gtk4::StringList::new(&["none", "cache", "freq"]);
    let vcache_row = adw::ComboRow::builder()
        .title(i18n("VCache Mode"))
        .model(&vcache_model)
        .build();
    vcache_row.set_selected(find_index(&vcache_model, &profile.vcache_mode));
    perf.add(&vcache_row);
    page.add(&perf);

    // Scripts group
    // No start/stop script fields. falcond runs them as root through /bin/sh,
    // so the helper refuses any profile that sets one; a field whose only
    // possible outcome is a failed save is not a feature.

    // Gamescope per-game overrides
    let gs_group = adw::PreferencesGroup::new();
    gs_group.set_title(&i18n("Gamescope"));
    gs_group.set_description(Some(&i18n(
        "Per-game overrides (leave disabled for global defaults)",
    )));

    // Pre-populate with the saved global defaults if no per-game override exists.
    let gs_cfg = profile
        .gamescope
        .clone()
        .unwrap_or_else(bigame_core::gamescope::load_global);

    // When Gamescope runs: automatically, always, or never.
    //
    // A plain on/off switch was the wrong shape. Some titles are worse inside
    // Gamescope — overlay, input and HDR problems — and some simply do not
    // need it; wrapping a game that gains nothing adds a compositor, a copy and
    // a frame of latency for no benefit.
    let gs_mode_model =
        gtk4::StringList::new(&[&i18n("Automatic"), &i18n("Always"), &i18n("Never")]);
    let gs_mode = adw::ComboRow::builder()
        .title(i18n("Use Gamescope"))
        .model(&gs_mode_model)
        .selected(match profile.gamescope_mode {
            bigame_core::gamescope::Mode::Auto => 0,
            bigame_core::gamescope::Mode::Enabled => 1,
            bigame_core::gamescope::Mode::Disabled => 2,
        })
        .build();
    gs_group.add(&gs_mode);

    // What Automatic would decide, given the settings below — shown so the
    // choice is not a mystery, and updated as those settings change.
    let gs_explain = adw::ActionRow::builder()
        .title(i18n("What Automatic does here"))
        .subtitle(i18n("Checking…"))
        .build();
    gs_explain.add_prefix(&gtk4::Image::from_icon_name("dialog-information-symbolic"));
    gs_group.add(&gs_explain);

    let gs_enable = adw::SwitchRow::builder()
        .title(i18n("Override Gamescope settings for this game"))
        .subtitle(i18n("Leave off to use the global defaults"))
        .active(profile.gamescope.is_some())
        .build();
    gs_group.add(&gs_enable);

    let gs_width = adw::SpinRow::new(
        Some(&gtk4::Adjustment::new(
            f64::from(gs_cfg.render_width),
            640.0,
            7680.0,
            1.0,
            10.0,
            0.0,
        )),
        1.0,
        0,
    );
    gs_width.set_title(&i18n("Width"));
    gs_width.set_sensitive(profile.gamescope.is_some());
    gs_group.add(&gs_width);

    let gs_height = adw::SpinRow::new(
        Some(&gtk4::Adjustment::new(
            f64::from(gs_cfg.render_height),
            480.0,
            4320.0,
            1.0,
            10.0,
            0.0,
        )),
        1.0,
        0,
    );
    gs_height.set_title(&i18n("Height"));
    gs_height.set_sensitive(profile.gamescope.is_some());
    gs_group.add(&gs_height);

    let gs_fsr = adw::SwitchRow::builder()
        .title(i18n("FSR"))
        .active(gs_cfg.filter == bigame_core::gamescope::Filter::Fsr)
        .sensitive(profile.gamescope.is_some())
        .build();
    gs_group.add(&gs_fsr);

    let gs_fps = adw::SpinRow::new(
        Some(&gtk4::Adjustment::new(
            f64::from(match gs_cfg.frame_limit {
                bigame_core::gamescope::FrameLimit::NestedRefresh(hz) => hz,
                bigame_core::gamescope::FrameLimit::None => 0,
            }),
            0.0,
            500.0,
            1.0,
            10.0,
            0.0,
        )),
        1.0,
        0,
    );
    gs_fps.set_title(&i18n("Framerate Limit"));
    gs_fps.set_sensitive(profile.gamescope.is_some());
    gs_group.add(&gs_fps);

    // Toggle sensitivity of gamescope fields
    let w_ref = gs_width.clone();
    let h_ref = gs_height.clone();
    let fsr_ref = gs_fsr.clone();
    let fps_ref = gs_fps.clone();
    gs_enable.connect_active_notify(move |sw| {
        let on = sw.is_active();
        w_ref.set_sensitive(on);
        h_ref.set_sensitive(on);
        fsr_ref.set_sensitive(on);
        fps_ref.set_sensitive(on);
    });

    // Keep the explanation honest as the controls change.
    {
        let explain = gs_explain.clone();
        let mode = gs_mode.clone();
        let width = gs_width.clone();
        let height = gs_height.clone();
        let fsr = gs_fsr.clone();
        let fps = gs_fps.clone();
        let refresh = Rc::new(move || {
            #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
            let cfg = bigame_core::gamescope::Config {
                render_width: width.value() as u32,
                render_height: height.value() as u32,
                filter: if fsr.is_active() {
                    bigame_core::gamescope::Filter::Fsr
                } else {
                    bigame_core::gamescope::Filter::Linear
                },
                frame_limit: {
                    let hz = fps.value() as u32;
                    if hz > 0 {
                        bigame_core::gamescope::FrameLimit::NestedRefresh(hz)
                    } else {
                        bigame_core::gamescope::FrameLimit::None
                    }
                },
                ..bigame_core::gamescope::Config::default()
            };
            let caps = bigame_core::capabilities::Capabilities::detect().gamescope;
            let session = bigame_core::hardware::Hardware::detect().session;
            let decision = bigame_core::gamescope::decide(
                bigame_core::gamescope::Mode::Auto,
                &cfg,
                caps.as_ref(),
                session,
            );
            explain.set_subtitle(&format!(
                "{} — {}",
                if decision.use_gamescope {
                    i18n("Gamescope would run")
                } else {
                    i18n("Gamescope would not run")
                },
                decision.reason
            ));
            // The explanation only describes Automatic.
            explain.set_visible(mode.selected() == 0);
        });
        refresh();
        for widget in [&gs_width, &gs_height] {
            let refresh = Rc::clone(&refresh);
            widget.connect_value_notify(move |_| refresh());
        }
        {
            let refresh = Rc::clone(&refresh);
            gs_fps.connect_value_notify(move |_| refresh());
        }
        {
            let refresh = Rc::clone(&refresh);
            gs_fsr.connect_active_notify(move |_| refresh());
        }
        {
            let refresh = Rc::clone(&refresh);
            gs_mode.connect_selected_notify(move |_| refresh());
        }
    }

    page.add(&gs_group);

    // Frame Generation group
    let fg_group = adw::PreferencesGroup::new();
    fg_group.set_title(&i18n("Frame Generation (LSFG-VK)"));

    let fg_dll_path = adw::EntryRow::builder()
        .title(i18n("Path to Lossless.dll"))
        .text(profile.fg_dll_path.as_deref().unwrap_or(""))
        .build();

    let file_btn = gtk4::Button::builder()
        .icon_name("folder-open-symbolic")
        .valign(gtk4::Align::Center)
        .css_classes(["flat"])
        .build();
    fg_dll_path.add_suffix(&file_btn);

    let info_btn = gtk4::Button::builder()
        .icon_name("dialog-information-symbolic")
        .tooltip_text(i18n(
            "Lossless Scaling is proprietary.
Click to visit losslessscaling.com",
        ))
        .valign(gtk4::Align::Center)
        .css_classes(["flat", "circular"])
        .build();
    info_btn.connect_clicked(|btn| {
        if let Some(win) = btn.root().and_downcast::<gtk4::Window>() {
            let dialog = adw::AlertDialog::builder()
                .heading(i18n("Lossless Scaling Required"))
                .body(i18n("This feature uses LSFG-VK which requires the proprietary Lossless.dll to function.

You must legally acquire Lossless Scaling on Steam or other platforms to obtain this file."))
                .body_use_markup(true)
                .build();

            dialog.add_response("cancel", &i18n("Close"));
            dialog.add_response("web", &i18n("Visit Website"));
            dialog.set_response_appearance("web", adw::ResponseAppearance::Suggested);

            let win_clone = win.clone();
            dialog.choose(&win, gtk4::gio::Cancellable::NONE, move |response| {
                if response == "web" {
                    let launcher = gtk4::UriLauncher::new("https://losslessscaling.com/");
                    launcher.launch(Some(&win_clone), gtk4::gio::Cancellable::NONE, |_| {});
                }
            });
        }
    });
    fg_dll_path.add_suffix(&info_btn);

    let r_clone = fg_dll_path.clone();
    file_btn.connect_clicked(move |btn| {
        let dialog = gtk4::FileDialog::builder()
            .title(i18n("Select Lossless.dll"))
            .modal(true)
            .build();
        let f = gtk4::FileFilter::new();
        f.set_name(Some("DLL files (*.dll)"));
        f.add_pattern("*.dll");
        let filters = gio::ListStore::new::<gtk4::FileFilter>();
        filters.append(&f);
        dialog.set_filters(Some(&filters));

        let r = r_clone.clone();
        if let Some(win) = btn.root().and_downcast::<gtk4::Window>() {
            dialog.open(Some(&win), gio::Cancellable::NONE, move |res| {
                if let Ok(file) = res {
                    if let Some(path) = file.path() {
                        r.set_text(&path.to_string_lossy());
                    }
                }
            });
        }
    });
    fg_group.add(&fg_dll_path);

    let fg_multiplier = adw::SpinRow::new(
        Some(&gtk4::Adjustment::new(
            f64::from(profile.fg_multiplier).clamp(1.0, 20.0),
            1.0,
            20.0,
            1.0,
            1.0,
            0.0,
        )),
        1.0,
        0,
    );
    fg_multiplier.set_title(&i18n("Multiplier (1-20x)"));
    fg_group.add(&fg_multiplier);

    let fg_flow_scale = adw::SpinRow::new(
        Some(&gtk4::Adjustment::new(
            f64::from(profile.fg_flow_scale).clamp(25.0, 100.0),
            25.0,
            100.0,
            1.0,
            10.0,
            0.0,
        )),
        1.0,
        0,
    );
    fg_flow_scale.set_title(&i18n("Flow Scale (25-100%)"));
    fg_group.add(&fg_flow_scale);

    let fg_perf_mode = adw::SwitchRow::builder()
        .title(i18n("Performance Mode"))
        .active(profile.fg_perf_mode)
        .build();
    fg_group.add(&fg_perf_mode);

    let fg_hdr = adw::SwitchRow::builder()
        .title(i18n("HDR Mode"))
        .active(profile.fg_hdr)
        .build();
    fg_group.add(&fg_hdr);

    let fg_present_model = gtk4::StringList::new(&[
        &i18n("VSync/FIFO (default)"),
        &i18n("Recommended"),
        &i18n("Mailbox"),
        &i18n("Immediate"),
    ]);
    let fg_present_mode = adw::ComboRow::new();
    fg_present_mode.set_title(&i18n("Present Mode"));
    fg_present_mode.set_model(Some(&fg_present_model));
    fg_present_mode.set_selected(profile.fg_present_mode);
    fg_group.add(&fg_present_mode);

    page.add(&fg_group);

    PerfWidgets {
        perf_mode,
        idle_inhibit,
        sched_model,
        sched_row,
        mode_model,
        mode_row,
        vcache_model,
        vcache_row,
        gs_enable,
        gs_mode,
        gs_width,
        gs_height,
        gs_fsr,
        gs_fps,
        fg_dll_path,
        fg_multiplier,
        fg_flow_scale,
        fg_perf_mode,
        fg_hdr,
        fg_present_model,
        fg_present_mode,
    }
}

/// Intermediate struct holding references to detail page widgets.
struct PerfWidgets {
    perf_mode: adw::SwitchRow,
    idle_inhibit: adw::SwitchRow,
    sched_model: gtk4::StringList,
    sched_row: adw::ComboRow,
    mode_model: gtk4::StringList,
    mode_row: adw::ComboRow,
    vcache_model: gtk4::StringList,
    vcache_row: adw::ComboRow,
    gs_enable: adw::SwitchRow,
    gs_mode: adw::ComboRow,
    gs_width: adw::SpinRow,
    gs_height: adw::SpinRow,
    gs_fsr: adw::SwitchRow,
    gs_fps: adw::SpinRow,
    fg_dll_path: adw::EntryRow,
    fg_multiplier: adw::SpinRow,
    fg_flow_scale: adw::SpinRow,
    fg_perf_mode: adw::SwitchRow,
    fg_hdr: adw::SwitchRow,
    /// Held to maintain `GObject` lifetime of the `ComboRow` model.
    #[allow(dead_code)]
    fg_present_model: gtk4::StringList,
    fg_present_mode: adw::ComboRow,
}

/// Build a profile detail/editor page with save button.
#[allow(clippy::too_many_lines)]
fn build_detail_page_for(profile: &GameProfile) -> adw::NavigationPage {
    let page = adw::PreferencesPage::new();
    let shared = Rc::new(RefCell::new(profile.clone()));

    // Identity group
    let identity = adw::PreferencesGroup::new();
    identity.set_title(&i18n("Profile"));

    let name_row = adw::EntryRow::builder()
        .title(i18n("Process Name"))
        .text(&profile.name)
        .build();
    identity.add(&name_row);
    page.add(&identity);

    let w = build_perf_widgets(&page, profile);

    // Save button
    let save_btn = gtk4::Button::builder()
        .label(i18n("Save Profile"))
        .css_classes(["suggested-action", "pill"])
        .halign(gtk4::Align::Center)
        .margin_top(12)
        .build();
    let save_group = adw::PreferencesGroup::new();
    save_group.add(&save_btn);
    page.add(&save_group);

    // Collect widget values on save
    let cfg = Rc::clone(&shared);
    save_btn.connect_clicked(move |btn| {
        {
            let mut p = cfg.borrow_mut();
            p.name = name_row.text().to_string();
            p.performance_mode = w.perf_mode.is_active();
            p.idle_inhibit = w.idle_inhibit.is_active();
            if let Some(v) = w.sched_model.string(w.sched_row.selected()) {
                p.scx_sched = v.to_string();
            }
            if let Some(v) = w.mode_model.string(w.mode_row.selected()) {
                p.scx_sched_props = v.to_string();
            }
            if let Some(v) = w.vcache_model.string(w.vcache_row.selected()) {
                p.vcache_mode = v.to_string();
            }

            p.gamescope_mode = match w.gs_mode.selected() {
                1 => bigame_core::gamescope::Mode::Enabled,
                2 => bigame_core::gamescope::Mode::Disabled,
                _ => bigame_core::gamescope::Mode::Auto,
            };

            // Per-game Gamescope overrides
            p.gamescope = if w.gs_enable.is_active() {
                #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
                {
                    let fps = w.gs_fps.value() as u32;
                    Some(bigame_core::gamescope::Config {
                        render_width: w.gs_width.value() as u32,
                        render_height: w.gs_height.value() as u32,
                        filter: if w.gs_fsr.is_active() {
                            bigame_core::gamescope::Filter::Fsr
                        } else {
                            bigame_core::gamescope::Filter::Linear
                        },
                        frame_limit: if fps > 0 {
                            bigame_core::gamescope::FrameLimit::NestedRefresh(fps)
                        } else {
                            bigame_core::gamescope::FrameLimit::None
                        },
                        ..bigame_core::gamescope::Config::default()
                    })
                }
            } else {
                None
            };

            #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
            {
                let dll = w.fg_dll_path.text().to_string();
                p.fg_dll_path = if dll.is_empty() { None } else { Some(dll) };
                p.fg_multiplier = w.fg_multiplier.value() as u32;
                p.fg_flow_scale = w.fg_flow_scale.value() as u32;
                p.fg_perf_mode = w.fg_perf_mode.is_active();
                p.fg_hdr = w.fg_hdr.is_active();
                p.fg_present_mode = w.fg_present_mode.selected();
            }
        }
        let profile_clone = cfg.borrow().clone();

        // Block save only on hard errors (empty/invalid name, zero resolution).
        let errors = bigame_core::profiles::critical_errors(&profile_clone);
        if !errors.is_empty() {
            toast::show(btn, &errors.join("; "));
            return;
        }

        // Advisory warnings (VCache on non-AMD, scheduler mismatch, etc.) —
        // show as toast but do NOT block saving.
        let warnings = bigame_core::profiles::validate(&profile_clone);
        let soft: Vec<_> = warnings
            .iter()
            .filter(|w| !errors.contains(w))
            .cloned()
            .collect();
        if !soft.is_empty() {
            toast::show(btn, &format!("⚠ {}", soft.join("; ")));
        }

        btn.set_sensitive(false);
        btn.set_label(&i18n("Saving…"));
        let btn_ref = btn.clone();
        glib::spawn_future_local(async move {
            // Off the main thread: the call may wait on a Polkit password
            // prompt, and the window must keep drawing meanwhile. And the
            // result is reported -- it used to say "saved" whatever happened.
            let result =
                gio::spawn_blocking(move || bigame_core::profiles::save(&profile_clone)).await;
            match result {
                Ok(Ok(())) => toast::show(&btn_ref, &i18n("Profile saved")),
                Ok(Err(e)) => {
                    tracing::warn!(error = %format!("{e:#}"), "profile not saved");
                    toast::show(
                        &btn_ref,
                        &i18n("Could not save: %s").replace("%s", &format!("{e:#}")),
                    );
                }
                Err(_) => toast::show(&btn_ref, &i18n("Could not save: %s").replace("%s", "")),
            }
            glib::timeout_add_local_once(std::time::Duration::from_secs(2), move || {
                btn_ref.set_sensitive(true);
                btn_ref.set_label(&i18n("Save Profile"));
            });
        });
    });

    let toolbar = adw::ToolbarView::new();
    let detail_header = adw::HeaderBar::new();
    detail_header.set_show_end_title_buttons(false);
    detail_header.set_show_start_title_buttons(false);

    // Delete button (only for existing profiles)
    if !profile.name.is_empty() {
        // Export button
        let export_btn = gtk4::Button::builder()
            .icon_name("document-save-as-symbolic")
            .tooltip_text(i18n("Export Profile"))
            .build();
        let export_name = profile.name.clone();
        export_btn.connect_clicked(move |btn| {
            let dialog = gtk4::FileDialog::builder()
                .title(i18n("Export Profile"))
                .initial_name(format!("{export_name}.conf"))
                .build();
            let btn_ref = btn.clone();
            let name = export_name.clone();
            let win = btn.root().and_downcast::<gtk4::Window>();
            dialog.save(win.as_ref(), gio::Cancellable::NONE, move |result| {
                if let Ok(file) = result {
                    if let Some(path) = file.path() {
                        match bigame_core::profiles::export(&name, &path) {
                            Ok(()) => toast::show(&btn_ref, &i18n("Profile exported")),
                            Err(e) => toast::show(
                                &btn_ref,
                                &i18n("Export failed: %s").replace("%s", &e.to_string()),
                            ),
                        }
                    }
                }
            });
        });
        detail_header.pack_end(&export_btn);

        // Activate profile button — marks profile as selected in UI context.
        let activate_btn = gtk4::Button::builder()
            .icon_name("media-playback-start-symbolic")
            .tooltip_text(i18n("Activate Profile"))
            .build();
        activate_btn.connect_clicked(move |btn| {
            let btn_ref = btn.clone();
            toast::show(&btn_ref, &i18n("Profile activated"));
        });
        detail_header.pack_end(&activate_btn);

        let delete_btn = gtk4::Button::builder()
            .icon_name("user-trash-symbolic")
            .tooltip_text(i18n("Delete Profile"))
            .css_classes(["destructive-action"])
            .build();
        let profile_name = profile.name.clone();
        delete_btn.connect_clicked(move |btn| {
            let dialog = adw::AlertDialog::builder()
                .heading(i18n("Delete Profile?"))
                .body(i18n("Remove \"%s\" permanently?").replace("%s", &profile_name))
                .build();
            dialog.add_response("cancel", &i18n("Cancel"));
            dialog.add_response("delete", &i18n("Delete"));
            dialog.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
            dialog.set_default_response(Some("cancel"));
            dialog.set_close_response("cancel");

            let name = profile_name.clone();
            let btn_ref = btn.clone();
            dialog.connect_response(None, move |_dlg, response| {
                if response == "delete" {
                    // Report what happened, not what was attempted. This used
                    // to show "Profile deleted" and then drop the future that
                    // would have done the deleting.
                    let n = name.clone();
                    btn_ref.set_sensitive(false);
                    let feedback = btn_ref.clone();
                    glib::spawn_future_local(async move {
                        let result =
                            gio::spawn_blocking(move || bigame_core::profiles::delete(&n)).await;
                        match result {
                            Ok(Ok(())) => toast::show(&feedback, &i18n("Profile deleted")),
                            Ok(Err(e)) => {
                                feedback.set_sensitive(true);
                                toast::show(
                                    &feedback,
                                    &i18n("Could not delete profile: %s")
                                        .replace("%s", &e.to_string()),
                                );
                            }
                            Err(_) => {
                                feedback.set_sensitive(true);
                                toast::show(&feedback, &i18n("Could not delete profile"));
                            }
                        }
                    });
                }
            });

            let widget = btn.root().and_downcast::<gtk4::Window>();
            dialog.present(widget.as_ref());
        });
        detail_header.pack_end(&delete_btn);
    }

    toolbar.add_top_bar(&detail_header);
    toolbar.set_content(Some(&page));

    let title = if profile.name.is_empty() {
        i18n("New Profile")
    } else {
        profile.name.clone()
    };
    adw::NavigationPage::builder()
        .title(&title)
        .child(&toolbar)
        .build()
}

/// Rebuild the library grid from installed games plus existing profiles.
///
/// The two sources are merged rather than concatenated. A detected game and a
/// profile keyed on that game's executable are the same card — which is the
/// whole point of keying profiles on the process name, and is what makes the
/// "already has a profile" state visible at a glance.
pub(crate) fn refresh_profile_list(list_box: &gtk4::FlowBox, nav: &adw::NavigationView) {
    while let Some(child) = list_box.first_child() {
        list_box.remove(&child);
    }

    let entries = build_library();
    if entries.is_empty() {
        let empty = adw::StatusPage::builder()
            .icon_name("applications-games-symbolic")
            .title(i18n("No games found"))
            .description(i18n(
                "Install a game through Steam, Lutris or Heroic, or create a profile by hand.",
            ))
            .build();
        list_box.insert(&empty, -1);
        return;
    }

    for entry in entries {
        let nav_activate = nav.clone();
        let nav_menu = nav.clone();
        let card = game_card::build(
            &entry,
            move |entry| {
                // Existing profile opens for editing; a new one starts from the
                // game's real process name, which is the whole fix for profiles
                // that never matched anything.
                if entry.has_profile {
                    nav_activate.push(&build_detail_page(&entry.key));
                } else {
                    nav_activate.push(&build_detail_page_for(&GameProfile {
                        name: entry.key.clone(),
                        ..GameProfile::default()
                    }));
                }
            },
            move |entry, anchor| show_card_menu(entry, anchor, &nav_menu),
        );
        list_box.insert(&card, -1);
    }
}

/// Merge detected games and existing profiles into one list of cards.
fn build_library() -> Vec<game_card::Entry> {
    let profile_names = bigame_core::profiles::list_names();
    let mut entries: Vec<game_card::Entry> = Vec::new();

    for game in bigame_core::games::detect_all() {
        let key = game.profile_key().to_owned();
        let has_profile = profile_names.iter().any(|n| n == &key);
        entries.push(game_card::Entry {
            title: game.name.clone(),
            source: source_label(game.source),
            cover: game.cover.clone(),
            launch_command: game.launch_command.clone(),
            system_profile: has_profile && bigame_core::profiles::is_system_profile(&key),
            key_is_verified: game.has_real_executable(),
            target: game
                .install_path
                .clone()
                .map(|root| bigame_core::graphics::Target {
                    name: game.name.clone(),
                    process: key.clone(),
                    app_id: game.app_id.clone(),
                    install_root: root,
                }),
            has_profile,
            key,
        });
    }

    // Profiles with no matching installed game — the ones falcond ships, plus
    // anything the user wrote by hand.
    for name in &profile_names {
        if entries.iter().any(|e| &e.key == name) {
            continue;
        }
        entries.push(game_card::Entry {
            key_is_verified: looks_like_a_process_name(name),
            title: name.clone(),
            key: name.clone(),
            source: i18n("Profile"),
            cover: None,
            launch_command: None,
            has_profile: true,
            system_profile: bigame_core::profiles::is_system_profile(name),
            target: None,
        });
    }

    entries.sort_by_key(|e| e.title.to_lowercase());
    entries
}

/// Where a game came from, for display: launchers by their names, a game from
/// the application menu in the user's language.
pub(crate) fn source_label(source: bigame_core::games::Source) -> String {
    match source {
        bigame_core::games::Source::Native => i18n("Native"),
        other => other.label().to_owned(),
    }
}

/// Whether a profile name could plausibly be a process name.
///
/// falcond matches `/proc/<pid>/comm`, so a profile named after a display title
/// can never activate. This is exactly the damage the old game detection did on
/// this machine: it wrote profiles called `Arc Raiders` and `Dead by Daylight`
/// while the processes are `PioneerGame.exe` and
/// `DeadByDaylight-Win64-Shipping.exe`.
///
/// The check is deliberately conservative — a name containing a space and no
/// file extension is the signature of a display title, and everything else is
/// given the benefit of the doubt. Its only effect is to show a warning, so a
/// false positive costs a tooltip and a false negative costs nothing that was
/// not already broken.
fn looks_like_a_process_name(name: &str) -> bool {
    !name.contains(' ') || name.contains('.')
}

/// Overflow menu for one card.
fn show_card_menu(entry: &game_card::Entry, anchor: &gtk4::Widget, nav: &adw::NavigationView) {
    let menu = gio::Menu::new();
    // Measuring needs a handle on the game's own process, which only exists
    // for games that start without a launcher.
    if entry.launch_command.is_some() {
        menu.append(Some(&i18n("Measure the difference")), Some("card.measure"));
    }
    if entry.target.is_some() {
        menu.append(Some(&i18n("AI Graphics…")), Some("card.ai"));
    }
    if entry.has_profile {
        menu.append(Some(&i18n("Edit profile")), Some("card.edit"));
        if !entry.system_profile {
            menu.append(Some(&i18n("Delete profile")), Some("card.delete"));
        }
    } else {
        menu.append(Some(&i18n("Create profile")), Some("card.edit"));
    }

    let group = gio::SimpleActionGroup::new();

    if let Some(command) = entry.launch_command.clone() {
        let measure = gio::SimpleAction::new("measure", None);
        let title = entry.title.clone();
        let anchor = anchor.clone();
        measure.connect_activate(move |_, _| {
            crate::views::measure_dialog::present(&anchor, &title, &command);
        });
        group.add_action(&measure);
    }

    if let Some(target) = entry.target.clone() {
        let ai = gio::SimpleAction::new("ai", None);
        let anchor = anchor.clone();
        ai.connect_activate(move |_, _| {
            crate::views::ai_graphics::open(&anchor, target.clone(), None);
        });
        group.add_action(&ai);
    }

    let edit = gio::SimpleAction::new("edit", None);
    {
        let nav = nav.clone();
        let entry = entry.clone();
        edit.connect_activate(move |_, _| {
            if entry.has_profile {
                nav.push(&build_detail_page(&entry.key));
            } else {
                nav.push(&build_detail_page_for(&GameProfile {
                    name: entry.key.clone(),
                    ..GameProfile::default()
                }));
            }
        });
    }
    group.add_action(&edit);

    if entry.has_profile && !entry.system_profile {
        let delete = gio::SimpleAction::new("delete", None);
        let entry = entry.clone();
        let anchor_ref = anchor.clone();
        delete.connect_activate(move |_, _| {
            let dialog = adw::AlertDialog::new(
                Some(&i18n("Delete this profile?")),
                Some(
                    &i18n("The profile for %s will be removed. This cannot be undone.")
                        .replace("%s", &entry.title),
                ),
            );
            dialog.add_response("cancel", &i18n("Cancel"));
            dialog.add_response("delete", &i18n("Delete"));
            dialog.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
            dialog.set_default_response(Some("cancel"));
            dialog.set_close_response("cancel");

            let key = entry.key.clone();
            let anchor_inner = anchor_ref.clone();
            dialog.connect_response(None, move |_, response| {
                if response != "delete" {
                    return;
                }
                let key = key.clone();
                let anchor = anchor_inner.clone();
                glib::spawn_future_local(async move {
                    match bigame_core::profiles::delete(&key) {
                        Ok(()) => toast::show(&anchor, &i18n("Profile deleted")),
                        Err(e) => toast::show(
                            &anchor,
                            &i18n("Could not delete profile: %s").replace("%s", &e.to_string()),
                        ),
                    }
                });
            });
            dialog.present(Some(&anchor_ref));
        });
        group.add_action(&delete);
    }

    let popover = gtk4::PopoverMenu::from_model(Some(&menu));
    popover.set_parent(anchor);
    popover.insert_action_group("card", Some(&group));
    // `closed` is emitted before the chosen item's action is activated;
    // unparenting right away detached the popover — and the "card" actions
    // inserted on it — first, so no item in this menu did anything. Let the
    // activation run, then unparent.
    popover.connect_closed(|p| {
        let p = p.clone();
        glib::idle_add_local_once(move || p.unparent());
    });
    popover.popup();
}

#[cfg(test)]
mod tests {
    use super::looks_like_a_process_name;

    #[test]
    fn display_titles_are_flagged_as_unmatched_profiles() {
        // The two profiles the old detection wrote on this bench.
        assert!(!looks_like_a_process_name("Arc Raiders"));
        assert!(!looks_like_a_process_name("Dead by Daylight"));
    }

    #[test]
    fn real_process_names_are_accepted() {
        for name in [
            "PioneerGame.exe",
            "DeadByDaylight-Win64-Shipping.exe",
            "cs2",
            "Cyberpunk2077.exe",
            "Civ7_linux_Vulkan_FinalRelease",
            "ffxiv_dx11.exe",
            // A space is fine when there is also an extension — Lutris titles
            // such as "Power Bomberman.exe" really do run under that name.
            "Power Bomberman.exe",
        ] {
            assert!(looks_like_a_process_name(name), "{name} should be accepted");
        }
    }
}
