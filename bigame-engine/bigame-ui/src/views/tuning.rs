//! Tuning: everything applied to games, in one page.
//!
//! Top to bottom, from what everyone touches to what few will: the system
//! while a game runs (falcond), the display and Gamescope, upscaling and
//! sharpening, frame generation, the overlay, and the advanced facts.
//! Basic controls are visible; the rest sits inside expanders. What the
//! machine cannot do is said as *not supported* or *missing*, with the fix,
//! never shown as a broken control.
//!
//! Two kinds of settings live here, and each says which it is: falcond's
//! (written through the privileged helper, which reloads falcond) and the
//! launch settings in `video.toml` (read when BiGame-mode starts a game;
//! Wine FSR and vkBasalt also go into the session environment).
//!
//! Two technologies doing the same job are never left on together in
//! silence: the page says so and offers the one-click way out.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use gtk4::{gio, glib};
use libadwaita as adw;

use bigame_core::capabilities::Support;
use bigame_core::models::{FrameGenBackend, GamescopeFilter, WineFsrMode};
use bigame_core::overview::State;
use bigame_core::video_config;

use crate::i18n::i18n;
use crate::widgets::status::Chip;

/// Shared mutable config state for coordinated writes.
type SharedConfig = Rc<RefCell<bigame_core::config::FalcondConfig>>;

/// Build the Tuning page.
#[must_use]
pub fn build() -> adw::PreferencesPage {
    let page = adw::PreferencesPage::new();

    let config = bigame_core::config::read().unwrap_or_default();
    let shared = Rc::new(RefCell::new(config));
    let video = video_config::load();

    page.add(&build_system_group(&shared));
    let (gamescope_group, gamescope_scales) = build_gamescope_group(&video);
    page.add(&gamescope_group);
    page.add(&build_upscaling_group(&video, &gamescope_scales));
    page.add(&build_framegen_group(&video));
    page.add(&build_overlay_group());
    page.add(&build_advanced_group());

    page
}

/// Write the shared config through the privileged helper, on a background thread.
fn save_config(shared: &SharedConfig) {
    let cfg = shared.borrow().clone();
    // Not awaited here: zbus runs on Tokio, and the main thread has no runtime.
    glib::spawn_future_local(async move {
        match gio::spawn_blocking(move || bigame_core::config::write_blocking(&cfg)).await {
            Ok(Ok(())) => {}
            Ok(Err(e)) => tracing::error!("config write failed: {e:#}"),
            Err(_) => tracing::error!("config write failed: the worker thread panicked"),
        }
    });
}

fn save_upscaling(f: impl FnOnce(&mut bigame_core::models::UpscalingSettings)) {
    let mut cfg = video_config::load();
    f(&mut cfg.upscaling);
    if let Err(e) = video_config::save(&cfg) {
        tracing::warn!("failed to save video config: {e:#}");
    }
}

fn save_framegen(f: impl FnOnce(&mut bigame_core::models::FrameGenSettings)) {
    let mut cfg = video_config::load();
    f(&mut cfg.frame_gen);
    if let Err(e) = video_config::save(&cfg) {
        tracing::warn!("failed to save video config: {e:#}");
    }
    if let Err(e) = bigame_core::fg::sync_global_enablement(&cfg.frame_gen) {
        tracing::warn!("failed to sync global lsfg-vk state: {e:#}");
    }
}

/// A row that says something is not there, with the command that installs
/// it — in place of a control that could not work.
fn missing_row(title: &str, what: &str, command: &str) -> adw::ActionRow {
    let row = adw::ActionRow::builder()
        .title(title)
        .subtitle(format!("{what}\n→ {command}"))
        .subtitle_lines(3)
        .use_markup(false)
        .build();
    let chip = Chip::new(State::Missing);
    row.add_suffix(chip.widget());
    let copy = gtk4::Button::builder()
        .icon_name("edit-copy-symbolic")
        .tooltip_text(i18n("Copy the command"))
        .valign(gtk4::Align::Center)
        .css_classes(["flat"])
        .build();
    let command = command.to_owned();
    copy.connect_clicked(move |b| {
        b.clipboard().set_text(&command);
        crate::widgets::toast::show(b, &i18n("Copied"));
    });
    row.add_suffix(&copy);
    row
}

/// A row that says the hardware cannot do something — a fact, not a fault.
fn unsupported_row(title: &str, why: &str) -> adw::ActionRow {
    let row = adw::ActionRow::builder()
        .title(title)
        .subtitle(why)
        .subtitle_lines(3)
        .use_markup(false)
        .build();
    let chip = Chip::new(State::Unsupported);
    row.add_suffix(chip.widget());
    row
}

// ── System performance (falcond) ─────────────────────────────────────────────

/// What falcond applies while a game runs: performance mode, the
/// scheduler, 3D V-Cache, and its own settings.
#[allow(clippy::too_many_lines)]
fn build_system_group(shared: &SharedConfig) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::new();
    group.set_title(&i18n("System performance"));
    group.set_description(Some(&i18n(
        "Applied by falcond while a game runs and undone when it exits. Saved through the privileged helper.",
    )));

    // Performance mode
    let perf_row = adw::SwitchRow::builder()
        .title(i18n("Performance mode"))
        .subtitle(i18n(
            "The performance power profile while a game with performance mode runs",
        ))
        .active(shared.borrow().enable_performance_mode)
        .build();
    group.add(&perf_row);
    {
        let cfg = Rc::clone(shared);
        perf_row.connect_active_notify(move |row| {
            cfg.borrow_mut().enable_performance_mode = row.is_active();
            save_config(&cfg);
        });
    }

    // Scheduler: the control when it can work, the reason when it cannot.
    let caps = bigame_core::capabilities::SchedExtCaps::detect();
    let detected = bigame_core::sched::detect_installed();
    let has_schedulers = detected.len() > 1; // "none" is always there
    match caps.switchable() {
        Support::Unsupported(_) => group.add(&unsupported_row(
            &i18n("CPU scheduler (sched-ext)"),
            &i18n("The running kernel was built without sched_ext. A kernel with it (BigLinux's default) is needed."),
        )),
        Support::NotInstalled(pkg) if pkg == "scx-tools" => group.add(&missing_row(
            &i18n("CPU scheduler (sched-ext)"),
            &i18n("Schedulers are installed, but scx_loader is not; falcond switches schedulers only through it."),
            "sudo pacman -S scx-tools && sudo systemctl enable --now scx_loader",
        )),
        Support::NotInstalled(_) => group.add(&missing_row(
            &i18n("CPU scheduler (sched-ext)"),
            &i18n("No sched-ext scheduler is installed. Game profiles can then ask for one."),
            "sudo pacman -S scx-scheds scx-tools",
        )),
        Support::ServiceDown(_) => group.add(&missing_row(
            &i18n("CPU scheduler (sched-ext)"),
            &i18n("scx_loader is installed but its service is not running."),
            "sudo systemctl enable --now scx_loader",
        )),
        Support::Available => {
            let expander = adw::ExpanderRow::builder()
                .title(i18n("CPU scheduler (sched-ext)"))
                .subtitle(i18n(
                    "For games without a scheduler in their profile. A game's profile overrides this.",
                ))
                .build();
            let sched_strs: Vec<&str> = detected.iter().map(String::as_str).collect();
            let sched_model = gtk4::StringList::new(&sched_strs);
            let sched_row = adw::ComboRow::builder()
                .title(i18n("Scheduler"))
                .subtitle(i18n("none keeps the kernel's default"))
                .model(&sched_model)
                .sensitive(has_schedulers)
                .build();
            sched_row.set_selected(crate::views::profiles::find_index(
                &sched_model,
                &shared.borrow().scx_sched,
            ));
            let info_btn = gtk4::Button::builder()
                .icon_name("dialog-information-symbolic")
                .valign(gtk4::Align::Center)
                .css_classes(["flat", "circular"])
                .tooltip_text(i18n("Learn about Schedulers"))
                .build();
            info_btn.connect_clicked(|btn| {
                if let Some(win) = btn.root().and_downcast::<gtk4::Window>() {
                    crate::widgets::scheduler_info::show(&win);
                }
            });
            sched_row.add_suffix(&info_btn);
            expander.add_row(&sched_row);

            let mode_model =
                gtk4::StringList::new(&["default", "gaming", "power", "latency", "server"]);
            let mode_row = adw::ComboRow::builder()
                .title(i18n("Mode"))
                .subtitle(i18n("The scheduler's tuning preset"))
                .model(&mode_model)
                .sensitive(has_schedulers)
                .build();
            mode_row.set_selected(crate::views::profiles::find_index(
                &mode_model,
                &shared.borrow().scx_sched_props,
            ));
            expander.add_row(&mode_row);

            let installed = adw::ActionRow::builder()
                .title(i18n("Installed"))
                .subtitle(if caps.installed.is_empty() {
                    i18n("none")
                } else {
                    caps.installed.join(", ")
                })
                .use_markup(false)
                .build();
            expander.add_row(&installed);

            let cfg = Rc::clone(shared);
            let sm = sched_model.clone();
            sched_row.connect_selected_notify(move |row| {
                if let Some(val) = sm.string(row.selected()) {
                    cfg.borrow_mut().scx_sched = val.to_string();
                    save_config(&cfg);
                }
            });
            let cfg = Rc::clone(shared);
            let mm = mode_model.clone();
            mode_row.connect_selected_notify(move |row| {
                if let Some(val) = mm.string(row.selected()) {
                    cfg.borrow_mut().scx_sched_props = val.to_string();
                    save_config(&cfg);
                }
            });
            group.add(&expander);
        }
    }

    // 3D V-Cache: a control on a CPU that has one, a fact on one that does not.
    if bigame_core::vcache::is_available() {
        let model = gtk4::StringList::new(&["none", "cache", "freq"]);
        let row = adw::ComboRow::builder()
            .title(i18n("3D V-Cache"))
            .subtitle(i18n(
                "Which CCD games prefer: the one with the extra cache, or the faster one",
            ))
            .model(&model)
            .build();
        row.set_selected(crate::views::profiles::find_index(
            &model,
            &shared.borrow().vcache_mode,
        ));
        let cfg = Rc::clone(shared);
        row.connect_selected_notify(move |row| {
            if let Some(val) = model.string(row.selected()) {
                cfg.borrow_mut().vcache_mode = val.to_string();
                save_config(&cfg);
            }
        });
        group.add(&row);
    } else {
        group.add(&unsupported_row(
            &i18n("3D V-Cache"),
            &i18n("This processor has no 3D V-Cache (AMD X3D only). Nothing to do."),
        ));
    }

    // falcond's own settings, rarely touched.
    let more = adw::ExpanderRow::builder()
        .title(i18n("falcond's settings"))
        .subtitle(i18n(
            "How it looks for games, and which profile set it uses",
        ))
        .build();

    let poll_adj = gtk4::Adjustment::new(
        f64::from(shared.borrow().poll_interval_ms),
        500.0,
        60_000.0,
        500.0,
        1000.0,
        0.0,
    );
    let poll_row = adw::SpinRow::new(Some(&poll_adj), 500.0, 0);
    poll_row.set_title(&i18n("Scan interval (ms)"));
    poll_row.set_subtitle(&i18n(
        "How often falcond looks for a running game. Lower reacts sooner and costs more; the default is 9000.",
    ));
    more.add_row(&poll_row);
    {
        let cfg = Rc::clone(shared);
        poll_row.connect_changed(move |row| {
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let val = row.value() as u32;
            cfg.borrow_mut().poll_interval_ms = val;
            save_config(&cfg);
        });
    }

    let device_model = gtk4::StringList::new(&["none", "handheld", "htpc"]);
    let device_row = adw::ComboRow::builder()
        .title(i18n("Profile set"))
        .subtitle(i18n(
            "none is the desktop set; handheld and htpc are falcond's other sets",
        ))
        .model(&device_model)
        .build();
    device_row.set_selected(crate::views::profiles::find_index(
        &device_model,
        &shared.borrow().profile_mode,
    ));
    more.add_row(&device_row);
    {
        let cfg = Rc::clone(shared);
        device_row.connect_selected_notify(move |row| {
            if let Some(val) = device_model.string(row.selected()) {
                cfg.borrow_mut().profile_mode = val.to_string();
                save_config(&cfg);
            }
        });
    }

    // The governor, for reference: power-profiles-daemon sets it.
    let gov_row = adw::ActionRow::builder()
        .title(i18n("CPU governor"))
        .subtitle(i18n("Reading…"))
        .use_markup(false)
        .build();
    more.add_row(&gov_row);
    {
        let row = gov_row.clone();
        glib::spawn_future_local(async move {
            let (available, current) = gio::spawn_blocking(|| {
                let avail = std::fs::read_to_string(
                    "/sys/devices/system/cpu/cpu0/cpufreq/scaling_available_governors",
                )
                .unwrap_or_default();
                let curr = std::fs::read_to_string(
                    "/sys/devices/system/cpu/cpu0/cpufreq/scaling_governor",
                )
                .unwrap_or_default();
                (avail, curr)
            })
            .await
            .unwrap_or_default();
            let current = current.trim();
            row.set_subtitle(&if current.is_empty() {
                i18n("No cpufreq driver reports a governor")
            } else {
                i18n("%c now, set by power-profiles-daemon through the power profile (available: %a)")
                    .replace("%c", current)
                    .replace("%a", available.trim())
            });
        });
    }
    group.add(&more);

    group
}

// ── Display and Gamescope ────────────────────────────────────────────────────

/// Gamescope: on or off, and when on, the filter, sharpness and sizes.
/// Returns the group and a cell that says whether Gamescope is set to
/// upscale (a render size below the output), for the conflict check.
#[allow(clippy::too_many_lines)]
fn build_gamescope_group(
    cfg: &video_config::VideoConfig,
) -> (adw::PreferencesGroup, Rc<std::cell::Cell<bool>>) {
    let group = adw::PreferencesGroup::new();
    group.set_title(&i18n("Display and Gamescope"));
    group.set_description(Some(&i18n(
        "For games started from BiGame-mode (Profiles → Launch). A game's profile can force Gamescope on or off.",
    )));
    let scales = Rc::new(std::cell::Cell::new(
        cfg.upscaling.gamescope_enabled && cfg.upscaling.base_width > 0,
    ));

    let caps = bigame_core::capabilities::Capabilities::detect();
    let Some(gs) = caps.gamescope.as_ref() else {
        group.add(&missing_row(
            "Gamescope",
            &i18n("Not installed. It wraps the game in a micro-compositor: scaling, a frame limit, a stable fullscreen."),
            "sudo pacman -S gamescope",
        ));
        return (group, scales);
    };

    let expander = adw::ExpanderRow::builder()
        .title("Gamescope")
        .subtitle(
            i18n("Version %v · wraps games started from BiGame-mode").replace(
                "%v",
                &gs.version
                    .map_or_else(|| i18n("unknown"), |v| v.to_string()),
            ),
        )
        .show_enable_switch(true)
        .enable_expansion(cfg.upscaling.gamescope_enabled)
        .build();

    let filter_items = gtk4::StringList::new(&[
        "FSR 1.0 (FidelityFX)",
        "NIS (NVIDIA Image Scaling)",
        &i18n("Integer scaling"),
    ]);
    let filter_row = adw::ComboRow::new();
    filter_row.set_title(&i18n("Upscaling filter"));
    filter_row.set_subtitle(&i18n("Used when the render size is below the output size"));
    filter_row.set_model(Some(&filter_items));
    filter_row.set_selected(match cfg.upscaling.gamescope_filter {
        GamescopeFilter::Fsr => 0,
        GamescopeFilter::Nis => 1,
        GamescopeFilter::Integer => 2,
    });
    expander.add_row(&filter_row);

    let sharpness_adj = gtk4::Adjustment::new(
        f64::from(cfg.upscaling.gamescope_sharpness.min(20)),
        0.0,
        20.0,
        1.0,
        5.0,
        0.0,
    );
    let sharpness_row = adw::SpinRow::new(Some(&sharpness_adj), 1.0, 0);
    sharpness_row.set_title(&i18n("FSR sharpness"));
    sharpness_row.set_subtitle(&i18n("0 = sharpest · 20 = softest"));
    sharpness_row.set_sensitive(cfg.upscaling.gamescope_filter == GamescopeFilter::Fsr);
    expander.add_row(&sharpness_row);

    let render_width = make_res_spinbutton(cfg.upscaling.base_width, 7680);
    let render_height = make_res_spinbutton(cfg.upscaling.base_height, 4320);
    let base_res_row = make_resolution_row(
        &i18n("Render size"),
        &i18n("The game draws at this size; 0 = the game's own"),
        &render_width,
        &render_height,
    );
    expander.add_row(&base_res_row);

    let output_width = make_res_spinbutton(cfg.upscaling.target_width, 7680);
    let output_height = make_res_spinbutton(cfg.upscaling.target_height, 4320);
    let target_res_row = make_resolution_row(
        &i18n("Output size"),
        &i18n("Upscaled to this size; 0 = the same as the render size"),
        &output_width,
        &output_height,
    );
    expander.add_row(&target_res_row);

    // Signal handlers.
    {
        let scales = Rc::clone(&scales);
        let rw = render_width.clone();
        expander.connect_enable_expansion_notify(move |e| {
            let enabled = e.enables_expansion();
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            scales.set(enabled && rw.value() as u32 > 0);
            save_upscaling(|u| u.gamescope_enabled = enabled);
        });
    }
    {
        let scales = Rc::clone(&scales);
        let ex = expander.clone();
        render_width.connect_value_changed(move |spin| {
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let v = spin.value() as u32;
            scales.set(ex.enables_expansion() && v > 0);
            save_upscaling(|u| u.base_width = v);
        });
    }
    render_height.connect_value_changed(|spin| {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let v = spin.value() as u32;
        save_upscaling(|u| u.base_height = v);
    });
    output_width.connect_value_changed(|spin| {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let v = spin.value() as u32;
        save_upscaling(|u| u.target_width = v);
    });
    output_height.connect_value_changed(|spin| {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let v = spin.value() as u32;
        save_upscaling(|u| u.target_height = v);
    });
    {
        let sharpness = sharpness_row.clone();
        filter_row.connect_selected_notify(move |row| {
            sharpness.set_sensitive(row.selected() == 0);
            let filter = match row.selected() {
                1 => GamescopeFilter::Nis,
                2 => GamescopeFilter::Integer,
                _ => GamescopeFilter::Fsr,
            };
            save_upscaling(|u| u.gamescope_filter = filter);
        });
    }
    sharpness_row.connect_changed(|row| {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let v = row.value() as u8;
        save_upscaling(|u| u.gamescope_sharpness = v);
    });

    group.add(&expander);
    (group, scales)
}

// ── Upscaling and sharpening ─────────────────────────────────────────────────

/// Wine FSR and vkBasalt, with the conflict check against Gamescope's
/// upscaling.
#[allow(clippy::too_many_lines)]
fn build_upscaling_group(
    cfg: &video_config::VideoConfig,
    gamescope_scales: &Rc<std::cell::Cell<bool>>,
) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::new();
    group.set_title(&i18n("Upscaling and sharpening"));
    group.set_description(Some(&i18n(
        "Set in the session environment and read by every game started afterwards. A launcher already running (Steam) keeps its old environment: close and reopen it.",
    )));

    // Two upscalers in series is a mistake the page names and can undo.
    let banner = adw::Banner::builder()
        .title(i18n(
            "Wine FSR and Gamescope upscaling are both on: two upscalers in series. Keep one.",
        ))
        .button_label(i18n("Turn Wine FSR off"))
        .revealed(false)
        .build();
    group.add(&banner);

    let wine_row = adw::SwitchRow::builder()
        .title("Wine FSR")
        .subtitle(i18n(
            "Wine's own upscaling for Proton games in exclusive fullscreen (WINE_FULLSCREEN_FSR=1)",
        ))
        .active(cfg.upscaling.wine_fsr_enabled)
        .build();
    group.add(&wine_row);

    let wine_quality_items = gtk4::StringList::new(&[
        &i18n("Performance"),
        &i18n("Balanced"),
        &i18n("Quality"),
        &i18n("Ultra"),
    ]);
    let wine_quality_row = adw::ComboRow::new();
    wine_quality_row.set_title(&i18n("Wine FSR quality"));
    wine_quality_row.set_model(Some(&wine_quality_items));
    wine_quality_row.set_selected(match cfg.upscaling.wine_fsr_mode {
        WineFsrMode::Performance => 0,
        WineFsrMode::Balanced => 1,
        WineFsrMode::Quality => 2,
        WineFsrMode::Ultra => 3,
    });
    wine_quality_row.set_sensitive(cfg.upscaling.wine_fsr_enabled);
    group.add(&wine_quality_row);

    let check_conflict = {
        let banner = banner.clone();
        let wine = wine_row.clone();
        let scales = Rc::clone(gamescope_scales);
        Rc::new(move || banner.set_revealed(wine.is_active() && scales.get()))
    };
    check_conflict();
    {
        let wq = wine_quality_row.clone();
        let check = Rc::clone(&check_conflict);
        wine_row.connect_active_notify(move |row| {
            wq.set_sensitive(row.is_active());
            save_upscaling(|u| u.wine_fsr_enabled = row.is_active());
            check();
        });
    }
    {
        let wine = wine_row.clone();
        banner.connect_button_clicked(move |_| wine.set_active(false));
    }
    // Gamescope's render size lives in the group above; the banner is
    // re-checked when this group comes back on screen after a change there.
    {
        let check = Rc::clone(&check_conflict);
        group.connect_map(move |_| check());
    }
    wine_quality_row.connect_selected_notify(|row| {
        let mode = match row.selected() {
            0 => WineFsrMode::Performance,
            1 => WineFsrMode::Balanced,
            3 => WineFsrMode::Ultra,
            _ => WineFsrMode::Quality,
        };
        save_upscaling(|u| u.wine_fsr_mode = mode);
    });

    // vkBasalt: a look, not a speed-up; only where its layer is installed.
    if bigame_core::capabilities::vkbasalt_installed() {
        let vkb_expander = adw::ExpanderRow::builder()
            .title("vkBasalt")
            .subtitle(i18n(
                "Visual filters (sharpening, colour) for Vulkan and Proton games. A look, not a speed-up: it costs a little GPU time.",
            ))
            .show_enable_switch(true)
            .enable_expansion(cfg.upscaling.vkbasalt_enabled)
            .build();
        let vkb_conf_row = adw::EntryRow::builder()
            .title(i18n("Configuration file"))
            .text(cfg.upscaling.vkbasalt_config_path.as_deref().unwrap_or(""))
            .build();
        vkb_expander.add_row(&vkb_conf_row);
        vkb_expander.connect_enable_expansion_notify(|e| {
            let on = e.enables_expansion();
            save_upscaling(|u| u.vkbasalt_enabled = on);
        });
        vkb_conf_row.connect_changed(|row| {
            let text = row.text().to_string();
            save_upscaling(|u| {
                u.vkbasalt_config_path = if text.is_empty() { None } else { Some(text) };
            });
        });
        group.add(&vkb_expander);
    } else {
        group.add(&missing_row(
            "vkBasalt",
            &i18n(
                "Not installed. Visual filters (sharpening, colour) for Vulkan and Proton games.",
            ),
            "sudo pacman -S vkbasalt",
        ));
    }

    let note = adw::ActionRow::builder()
        .title(i18n("AI Graphics"))
        .subtitle(i18n(
            "Upscaling inside the game (FSR 4, XeSS, DLSS through OptiScaler) is per game, from the game's card in Profiles. A game with it installed launches without Wine FSR and without a Gamescope render size.",
        ))
        .subtitle_lines(4)
        .use_markup(false)
        .build();
    note.add_prefix(&gtk4::Image::from_icon_name("dialog-information-symbolic"));
    group.add(&note);

    group
}

// ── Frame generation ─────────────────────────────────────────────────────────

/// lsfg-vk: the global switch and the per-game entries, in one place.
fn build_framegen_group(cfg: &video_config::VideoConfig) -> adw::PreferencesGroup {
    if !bigame_core::fg::layer_installed() {
        let group = adw::PreferencesGroup::new();
        group.set_title(&i18n("Frame generation"));
        group.add(&missing_row(
            "lsfg-vk",
            &i18n("Not installed. Generates extra frames between rendered ones (Lossless Scaling's method, as a Vulkan layer). It needs your own Lossless.dll."),
            "sudo pacman -S lsfg-vk",
        ));
        return group;
    }
    let active_game = crate::game_watch::current()
        .map(|g| g.process_name)
        .or_else(|| bigame_core::status::read().and_then(|s| s.active_profile))
        .unwrap_or_default();
    let group = crate::widgets::fg_controls::build_tuning_fg_group(&active_game);
    group.set_title(&i18n("Frame generation"));

    // The global switch goes first: the per-game entries below only count
    // while it is on.
    let lsfg_row = adw::SwitchRow::builder()
        .title(i18n("lsfg-vk for every game with an entry"))
        .subtitle(i18n(
            "Raises the presented frame rate, not the rendered one, and adds latency. Off sets every entry aside; on puts them back.",
        ))
        .active(cfg.frame_gen.enabled && cfg.frame_gen.backend == FrameGenBackend::LsfgVk)
        .build();
    lsfg_row.connect_active_notify(|row| {
        let on = row.is_active();
        save_framegen(|f| {
            f.enabled = on;
            f.backend = if on {
                FrameGenBackend::LsfgVk
            } else {
                FrameGenBackend::None
            };
        });
    });
    // ExpanderRow-free group: the switch is inserted at the top by
    // rebuilding the order — PreferencesGroup appends, so add and move.
    group.add(&lsfg_row);
    if let Some(list) = lsfg_row.parent().and_downcast::<gtk4::ListBox>() {
        list.remove(&lsfg_row);
        list.prepend(&lsfg_row);
    }

    let note = adw::ActionRow::builder()
        .title(i18n("One frame generator at a time"))
        .subtitle(i18n(
            "A game whose AI Graphics has OptiScaler generating frames launches with lsfg-vk turned off for it.",
        ))
        .subtitle_lines(3)
        .use_markup(false)
        .build();
    note.add_prefix(&gtk4::Image::from_icon_name("dialog-information-symbolic"));
    group.add(&note);
    group
}

// ── Overlay ──────────────────────────────────────────────────────────────────

/// `MangoHud` is chosen per game; this says whether it can be.
fn build_overlay_group() -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::new();
    group.set_title(&i18n("Overlay"));
    if bigame_core::capabilities::which("mangohud").is_some() {
        let row = adw::ActionRow::builder()
            .title("MangoHud")
            .subtitle(i18n(
                "Installed. Chosen per game in Profiles (Off, On, Forced); Details shows whether it loaded in the running game.",
            ))
            .subtitle_lines(3)
            .use_markup(false)
            .build();
        let chip = Chip::new(State::Configured);
        chip.set(State::Configured, Some(&i18n("Per game")));
        row.add_suffix(chip.widget());
        group.add(&row);
    } else {
        group.add(&missing_row(
            "MangoHud",
            &i18n("Not installed. The performance overlay; it also captures the frametimes Measure the difference uses."),
            "sudo pacman -S mangohud",
        ));
    }
    group
}

// ── Advanced ─────────────────────────────────────────────────────────────────

/// Facts for the person who knows what a scheduler flag is: collapsed, last.
#[allow(clippy::too_many_lines)]
fn build_advanced_group() -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::new();
    group.set_title(&i18n("Advanced"));

    let expander = adw::ExpanderRow::builder()
        .title(i18n("Show advanced options"))
        .subtitle(i18n(
            "sched-ext availability, Gamescope's accepted options, the environment file",
        ))
        .build();
    group.add(&expander);

    let caps = bigame_core::capabilities::Capabilities::detect();
    let scx = &caps.sched_ext;

    let scx_status = adw::ActionRow::builder()
        .title(i18n("sched-ext availability"))
        .subtitle(match scx.switchable().describe() {
            Some(reason) => reason,
            None => i18n("Available — falcond applies the scheduler you configure above"),
        })
        .use_markup(false)
        .build();
    scx_status.add_prefix(&gtk4::Image::from_icon_name(
        if scx.switchable().is_available() {
            "object-select-symbolic"
        } else {
            "dialog-warning-symbolic"
        },
    ));
    expander.add_row(&scx_status);

    let env_row = adw::ActionRow::builder()
        .title(i18n("Environment file"))
        .subtitle(i18n(
            "Wine FSR and vkBasalt are written to ~/.config/environment.d/bigame-mode.conf and pushed into the user systemd manager, so every game started afterwards inherits them.",
        ))
        .subtitle_lines(4)
        .use_markup(false)
        .build();
    expander.add_row(&env_row);

    let gamescope_row = adw::ActionRow::builder()
        .title(i18n("Gamescope options this build accepts"))
        .subtitle(match &caps.gamescope {
            Some(gs) => i18n("Version %v — %n options detected from --help")
                .replace(
                    "%v",
                    &gs.version
                        .map_or_else(|| i18n("unknown"), |v| v.to_string()),
                )
                .replace("%n", &gs.flags.len().to_string()),
            None => i18n("Gamescope is not installed"),
        })
        .use_markup(false)
        .build();
    expander.add_row(&gamescope_row);

    // The generated command line is the honest "advanced options" box: the
    // arguments come from capabilities, so seeing what they came out as is
    // what helps.
    if let Some(gs) = caps.gamescope.as_ref() {
        let sample = bigame_core::gamescope::Config {
            render_width: 1920,
            render_height: 1080,
            filter: bigame_core::gamescope::Filter::Fsr,
            sharpness: 5,
            ..bigame_core::gamescope::Config::default()
        };
        let built = sample.to_args(gs);
        // `use_markup` off before the text goes in: `<game>` is not markup.
        let preview = adw::ActionRow::builder()
            .title(i18n("Example command line"))
            .use_markup(false)
            .build();
        preview.set_subtitle(&format!("gamescope {} -- <game>", built.args.join(" ")));
        preview.set_subtitle_selectable(true);
        expander.add_row(&preview);

        for unsupported in &built.unsupported {
            let row = adw::ActionRow::builder()
                .title(i18n("Not supported by this Gamescope"))
                .subtitle(format!("--{} — {}", unsupported.flag, unsupported.effect))
                .use_markup(false)
                .build();
            row.add_prefix(&gtk4::Image::from_icon_name("dialog-warning-symbolic"));
            expander.add_row(&row);
        }
    }

    group
}

// ── Resolution input helpers ─────────────────────────────────────────────────

/// A `SpinButton` clamped to [0, `max_val`] for resolution inputs; 0 = auto.
fn make_res_spinbutton(current: u32, max_val: u32) -> gtk4::SpinButton {
    let adj = gtk4::Adjustment::new(f64::from(current), 0.0, f64::from(max_val), 1.0, 10.0, 0.0);
    let spin = gtk4::SpinButton::new(Some(&adj), 1.0, 0);
    spin.set_valign(gtk4::Align::Center);
    spin.set_width_chars(6);
    spin
}

/// Two `SpinButton`s (width × height) in an `AdwActionRow`.
fn make_resolution_row(
    title: &str,
    subtitle: &str,
    w_spin: &gtk4::SpinButton,
    h_spin: &gtk4::SpinButton,
) -> adw::ActionRow {
    let separator = gtk4::Label::builder()
        .label("×")
        .margin_start(4)
        .margin_end(4)
        .valign(gtk4::Align::Center)
        .css_classes(["dim-label"])
        .build();
    let row = adw::ActionRow::builder()
        .title(title)
        .subtitle(subtitle)
        .build();
    row.add_suffix(w_spin);
    row.add_suffix(&separator);
    row.add_suffix(h_spin);
    row
}
