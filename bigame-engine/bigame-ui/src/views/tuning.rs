//! Tuning: the general configuration — what every game gets.
//!
//! The same six sections as a game's profile, in the same order and with
//! the same rows (`widgets::optimization`): Performance, Display, Image
//! quality, Frame generation, Monitoring, Advanced. A profile replaces what
//! it sets for its game; this page is everything else. Basic controls are
//! visible and the rest sits in expanders; what the machine cannot do is
//! said as *not supported* or *missing*, with the fix.
//!
//! Two kinds of settings live here, and each says which it is: falcond's
//! (written through the privileged helper, which reloads falcond) and the
//! launch settings in `video.toml` (read when BiGame-mode starts a game;
//! Wine FSR and vkBasalt also go into the session environment). Every
//! change is saved at once; a save that fails is said, with its reason.
//!
//! Two technologies doing the same job are never left on together in
//! silence: switching one on while the other is on asks which to keep
//! (`widgets::notice::ask_conflict`), and a configuration that already has
//! both — from an older version — is named with the two ways out.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use gtk4::{gio, glib};
use libadwaita as adw;

use bigame_core::models::{FrameGenBackend, GamescopeFilter, WineFsrMode};
use bigame_core::optimization::{self as opt, Feature, PROFILE_SETS, VCACHE_MODES};
use bigame_core::overview::State;
use bigame_core::video_config::{self, VideoConfig};

use crate::i18n::{error_text, i18n, tr};
use crate::widgets::notice::{self, Kind, Notice};
use crate::widgets::optimization::{self as ui, Machine, Picker, Scope};
use crate::widgets::status::Chip;

/// falcond's configuration as the page last saved it.
type SharedConfig = Rc<RefCell<bigame_core::config::FalcondConfig>>;

/// The launch settings as the page holds them; every change writes them.
type SharedVideo = Rc<RefCell<VideoConfig>>;

/// Build the Tuning page.
#[must_use]
pub fn build() -> adw::PreferencesPage {
    let page = adw::PreferencesPage::new();
    let m = Machine::detect();
    let shared = Rc::new(RefCell::new(m.general.clone()));
    let video = Rc::new(RefCell::new(m.video.clone()));

    page.add(&ui::scope_banner(Scope::General));
    page.add(&build_performance(&shared, &m));
    let display = Display::build(&video, &m);
    page.add(&display.group);
    page.add(&build_image_quality(&video, &display));
    page.add(&build_frame_generation(&video, &m));
    page.add(&build_monitoring(&m));
    page.add(&build_advanced(&shared, &m));
    page
}

/// Write falcond's configuration through the helper, off the main thread,
/// and say so when it fails.
fn save_config(shared: &SharedConfig, anchor: &impl IsA<gtk4::Widget>) {
    save_config_then(shared, anchor, || {});
}

/// [`save_config`], running `after` once the helper has written it.
fn save_config_then(
    shared: &SharedConfig,
    anchor: &impl IsA<gtk4::Widget>,
    after: impl Fn() + 'static,
) {
    let cfg = shared.borrow().clone();
    let anchor = anchor.clone().upcast::<gtk4::Widget>();
    // Not awaited here: zbus runs on Tokio, and the main thread has no runtime.
    glib::spawn_future_local(async move {
        let result = gio::spawn_blocking(move || bigame_core::config::write_blocking(&cfg)).await;
        let failed = match result {
            Ok(Ok(())) => {
                after();
                return;
            }
            Ok(Err(e)) => error_text(&e),
            Err(_) => i18n("the worker thread stopped"),
        };
        crate::widgets::toast::error(
            &anchor,
            &i18n("Could not save falcond's configuration"),
            &failed,
        );
    });
}

/// Write the launch settings (and the session environment), and say so when
/// it fails.
fn save_video(video: &SharedVideo, anchor: &impl IsA<gtk4::Widget>) {
    if let Err(e) = video_config::save(&video.borrow()) {
        crate::widgets::toast::error(
            anchor,
            &i18n("Could not save the launch settings"),
            &error_text(&e),
        );
        return;
    }
    schedule_steam_gamescope(anchor.upcast_ref());
}

thread_local! {
    /// Bumped by every change; a refresh runs only if no change came after
    /// the one that scheduled it.
    static STEAM_REFRESH: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// A Steam game whose profile says Gamescope Always takes Tuning's sizes and
/// filter in its launch options: after a change settles (a spin button
/// sends one per step), bring them in line, and say what happened.
fn schedule_steam_gamescope(anchor: &gtk4::Widget) {
    let generation = STEAM_REFRESH.with(|g| {
        g.set(g.get().wrapping_add(1));
        g.get()
    });
    let anchor = anchor.clone();
    glib::timeout_add_local_once(std::time::Duration::from_millis(1500), move || {
        if STEAM_REFRESH.with(std::cell::Cell::get) != generation {
            return;
        }
        glib::spawn_future_local(async move {
            let results = gio::spawn_blocking(bigame_core::optimization::refresh_steam_gamescope)
                .await
                .unwrap_or_default();
            report_steam_gamescope(&anchor, &results);
        });
    });
}

/// Say what bringing the Steam games' Gamescope wrappers in line did.
fn report_steam_gamescope(
    anchor: &gtk4::Widget,
    results: &[(
        String,
        anyhow::Result<bigame_core::steam_gamescope::Applied>,
    )],
) {
    use bigame_core::steam_gamescope::Applied;
    let written: Vec<&str> = results
        .iter()
        .filter(|(_, r)| matches!(r, Ok(Applied::Written(_))))
        .map(|(n, _)| n.as_str())
        .collect();
    let blocked = results
        .iter()
        .any(|(_, r)| matches!(r, Ok(Applied::SteamRunning)));
    if let Some((name, Err(e))) = results.iter().find(|(_, r)| r.is_err()) {
        crate::widgets::toast::error(
            anchor,
            &i18n("Could not update Gamescope in Steam's launch options"),
            &format!("{name}: {e:#}"),
        );
    }
    if !written.is_empty() {
        crate::widgets::toast::show(
            anchor,
            &i18n("Gamescope updated in Steam's launch options: %s")
                .replace("%s", &written.join(", ")),
        );
    }
    if blocked {
        let a = anchor.clone();
        crate::widgets::toast::with_action(
            anchor,
            &i18n("Steam is open: its games with Gamescope keep the old sizes until it is closed"),
            &i18n("Close Steam and apply"),
            move || {
                let a = a.clone();
                glib::spawn_future_local(async move {
                    let done = gio::spawn_blocking(|| {
                        bigame_core::steam::while_closed(
                            bigame_core::optimization::refresh_steam_gamescope,
                        )
                    })
                    .await;
                    match done {
                        Ok(Ok(results)) => report_steam_gamescope(&a, &results),
                        Ok(Err(e)) => crate::widgets::toast::error(
                            &a,
                            &i18n("Steam could not be opened again"),
                            &format!("{e:#}"),
                        ),
                        Err(_) => {}
                    }
                });
            },
        );
    }
}

// ── Performance (falcond) ───────────────────────────────────────────────────

/// Performance mode, the scheduler and 3D V-Cache, as falcond applies them.
#[allow(clippy::too_many_lines)]
fn build_performance(shared: &SharedConfig, m: &Machine) -> adw::PreferencesGroup {
    let group = ui::section(&i18n("Performance"));
    group.set_description(Some(&i18n(
        "Applied by falcond while Turbo is on, and undone when it is turned off. Saved through the privileged helper.",
    )));

    let perf = adw::SwitchRow::builder()
        .title(i18n("Performance mode"))
        .subtitle(i18n(
            "The performance power profile while a game whose profile asks for it runs. Off, no game gets it.",
        ))
        .active(shared.borrow().enable_performance_mode)
        .build();
    group.add(&perf);
    {
        let cfg = Rc::clone(shared);
        perf.connect_active_notify(move |row| {
            cfg.borrow_mut().enable_performance_mode = row.is_active();
            save_config(&cfg, row);
        });
    }

    if let Some(row) = ui::scheduler_unavailable_row(m) {
        group.add(&row);
    } else {
        let sched = Picker::new(
            &i18n("CPU scheduler"),
            &i18n(
                "Loaded while Turbo is on. A game whose profile names another scheduler switches to it while it runs.",
            ),
            &ui::scheduler_items(m, Scope::General),
            &shared.borrow().scx_sched,
        );
        sched
            .row
            .add_suffix(&crate::widgets::scheduler_info::button());
        group.add(&sched.row);
        // What is loaded now, beside what is asked for: in the subtitle, so
        // the choice keeps its width. Read again after a save: the helper
        // restarts falcond, which loads the scheduler as it starts.
        let refresh_now: Rc<dyn Fn(u32)> = {
            let row = sched.row.clone();
            Rc::new(move |delay_ms: u32| {
                let row = row.clone();
                glib::spawn_future_local(async move {
                    glib::timeout_future(std::time::Duration::from_millis(delay_ms.into())).await;
                    let loaded = gio::spawn_blocking(bigame_core::running::loaded_scheduler)
                        .await
                        .ok()
                        .flatten();
                    let now = match loaded {
                        Some(name) => i18n("Now: %s").replace("%s", &name),
                        None => i18n("Now: the kernel's own scheduler"),
                    };
                    row.set_subtitle(&format!(
                        "{}\n{now}",
                        i18n(
                            "Loaded while Turbo is on. A game whose profile names another scheduler switches to it while it runs."
                        )
                    ));
                });
            })
        };
        refresh_now(0);

        let mode = Picker::new(
            &i18n("Scheduler mode"),
            "",
            &ui::mode_items(),
            &shared.borrow().scx_sched_props,
        );
        mode.row
            .add_suffix(&crate::widgets::info::scheduler_modes_button());
        mode.row
            .set_sensitive(!opt::inherits(&shared.borrow().scx_sched));
        group.add(&mode.row);
        {
            let (cfg, mode) = (Rc::clone(shared), mode.clone());
            let row = sched.row.clone();
            let refresh = Rc::clone(&refresh_now);
            sched.connect_changed(move |value| {
                value.clone_into(&mut cfg.borrow_mut().scx_sched);
                mode.row.set_sensitive(!opt::inherits(value));
                let refresh = Rc::clone(&refresh);
                save_config_then(&cfg, &row, move || refresh(3000));
            });
        }
        {
            let cfg = Rc::clone(shared);
            let row = mode.row.clone();
            let refresh = Rc::clone(&refresh_now);
            mode.connect_changed(move |value| {
                value.clone_into(&mut cfg.borrow_mut().scx_sched_props);
                let refresh = Rc::clone(&refresh);
                save_config_then(&cfg, &row, move || refresh(3000));
            });
        }
    }

    if m.vcache {
        let items: Vec<(String, String)> = VCACHE_MODES
            .iter()
            .map(|c| (c.id.to_owned(), i18n(c.label)))
            .collect();
        let vcache = Picker::new(
            &i18n("3D V-Cache"),
            &i18n("Which CCD games prefer while Turbo is on"),
            &items,
            &shared.borrow().vcache_mode,
        );
        vcache
            .row
            .add_suffix(&crate::widgets::info::vcache_button());
        group.add(&vcache.row);
        let cfg = Rc::clone(shared);
        let row = vcache.row.clone();
        vcache.connect_changed(move |value| {
            value.clone_into(&mut cfg.borrow_mut().vcache_mode);
            save_config(&cfg, &row);
        });
    } else {
        group.add(&ui::vcache_unsupported_row());
    }
    group
}

// ── Display (Gamescope) ─────────────────────────────────────────────────────

/// Gamescope: on or off, and when on, the filter, sharpness and sizes.
struct Display {
    group: adw::PreferencesGroup,
    /// The Gamescope switch and the render size, for the conflict check;
    /// `None` without Gamescope.
    rows: Option<(adw::ExpanderRow, gtk4::SpinButton, gtk4::SpinButton)>,
    /// Set while the page itself changes a control, so a handler does not
    /// ask about a change the user did not make.
    quiet: Rc<Cell<bool>>,
}

impl Display {
    #[allow(clippy::too_many_lines)]
    fn build(video: &SharedVideo, m: &Machine) -> Self {
        let group = ui::section(&i18n("Display"));
        group.set_description(Some(&i18n(
            "For games started from BiGame-mode (Profiles → Launch). A game's profile can force Gamescope on or off.",
        )));
        let quiet = Rc::new(Cell::new(false));
        if !m.gamescope {
            group.add(&ui::missing_row(
                "Gamescope",
                &i18n("Not installed. It wraps the game in a micro-compositor: scaling, a frame limit, a stable fullscreen."),
                "sudo pacman -S gamescope",
            ));
            return Self {
                group,
                rows: None,
                quiet,
            };
        }
        let cfg = video.borrow().clone();
        let expander = adw::ExpanderRow::builder()
            .title("Gamescope")
            .subtitle(i18n("Wraps games started from BiGame-mode"))
            .show_enable_switch(true)
            .enable_expansion(cfg.upscaling.gamescope_enabled)
            .build();
        // The version comes from `gamescope --help`, probed off the main thread.
        {
            let expander = expander.clone();
            glib::spawn_future_local(async move {
                let caps = gio::spawn_blocking(bigame_core::capabilities::gamescope_cached)
                    .await
                    .ok()
                    .flatten();
                if let Some(v) = caps.and_then(|c| c.version) {
                    expander.set_subtitle(
                        &i18n("Version %v · wraps games started from BiGame-mode")
                            .replace("%v", &v.to_string()),
                    );
                }
            });
        }

        let filters = gtk4::StringList::new(&[
            "FSR 1.0 (FidelityFX)",
            "NIS (NVIDIA Image Scaling)",
            &i18n("Integer scaling"),
        ]);
        let filter_row = adw::ComboRow::builder()
            .title(i18n("Upscaling filter"))
            .subtitle(i18n("Used when the render size is below the output size"))
            .model(&filters)
            .selected(match cfg.upscaling.gamescope_filter {
                GamescopeFilter::Fsr => 0,
                GamescopeFilter::Nis => 1,
                GamescopeFilter::Integer => 2,
            })
            .build();
        expander.add_row(&filter_row);

        let sharpness_row = adw::SpinRow::new(
            Some(&gtk4::Adjustment::new(
                f64::from(cfg.upscaling.gamescope_sharpness.min(20)),
                0.0,
                20.0,
                1.0,
                5.0,
                0.0,
            )),
            1.0,
            0,
        );
        sharpness_row.set_title(&i18n("FSR sharpness"));
        sharpness_row.set_subtitle(&i18n("0 = sharpest · 20 = softest"));
        sharpness_row.set_sensitive(cfg.upscaling.gamescope_filter == GamescopeFilter::Fsr);
        expander.add_row(&sharpness_row);

        let render_width = res_spin(cfg.upscaling.base_width, 7680);
        let render_height = res_spin(cfg.upscaling.base_height, 4320);
        expander.add_row(&res_row(
            &i18n("Render size"),
            &i18n("The game draws at this size; 0 = the game's own"),
            &render_width,
            &render_height,
        ));
        let output_width = res_spin(cfg.upscaling.target_width, 7680);
        let output_height = res_spin(cfg.upscaling.target_height, 4320);
        expander.add_row(&res_row(
            &i18n("Output size"),
            &i18n("Upscaled to this size; 0 = the same as the render size"),
            &output_width,
            &output_height,
        ));

        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let size = |s: &gtk4::SpinButton| s.value() as u32;
        {
            let video = Rc::clone(video);
            let sharpness_row = sharpness_row.clone();
            filter_row.connect_selected_notify(move |row| {
                sharpness_row.set_sensitive(row.selected() == 0);
                video.borrow_mut().upscaling.gamescope_filter = match row.selected() {
                    1 => GamescopeFilter::Nis,
                    2 => GamescopeFilter::Integer,
                    _ => GamescopeFilter::Fsr,
                };
                save_video(&video, row);
            });
        }
        {
            let video = Rc::clone(video);
            sharpness_row.connect_changed(move |row| {
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                let v = row.value() as u8;
                video.borrow_mut().upscaling.gamescope_sharpness = v;
                save_video(&video, row);
            });
        }
        {
            let video = Rc::clone(video);
            output_width.connect_value_changed(move |s| {
                video.borrow_mut().upscaling.target_width = size(s);
                save_video(&video, s);
            });
        }
        {
            let video = Rc::clone(video);
            output_height.connect_value_changed(move |s| {
                video.borrow_mut().upscaling.target_height = size(s);
                save_video(&video, s);
            });
        }
        group.add(&expander);
        Self {
            group,
            rows: Some((expander, render_width, render_height)),
            quiet,
        }
    }
}

/// A `SpinButton` clamped to [0, `max_val`] for resolution inputs; 0 = auto.
fn res_spin(current: u32, max_val: u32) -> gtk4::SpinButton {
    let adj = gtk4::Adjustment::new(f64::from(current), 0.0, f64::from(max_val), 1.0, 10.0, 0.0);
    let spin = gtk4::SpinButton::new(Some(&adj), 1.0, 0);
    spin.set_valign(gtk4::Align::Center);
    spin.set_width_chars(6);
    spin
}

/// Two `SpinButton`s (width × height) in an `AdwActionRow`.
fn res_row(
    title: &str,
    subtitle: &str,
    w: &gtk4::SpinButton,
    h: &gtk4::SpinButton,
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
    w.update_property(&[gtk4::accessible::Property::Label(&format!(
        "{title} — {}",
        i18n("width")
    ))]);
    h.update_property(&[gtk4::accessible::Property::Label(&format!(
        "{title} — {}",
        i18n("height")
    ))]);
    row.add_suffix(w);
    row.add_suffix(&separator);
    row.add_suffix(h);
    row
}

// ── Image quality ───────────────────────────────────────────────────────────

/// Wine FSR and vkBasalt, with the conflict against Gamescope's upscaling
/// resolved in both directions.
#[allow(clippy::too_many_lines)]
fn build_image_quality(video: &SharedVideo, display: &Display) -> adw::PreferencesGroup {
    let group = ui::section(&i18n("Image quality"));
    group.set_description(Some(&i18n(
        "Set in the session environment for every game started afterwards. A launcher already running (Steam) keeps its old environment: close and reopen it.",
    )));
    let cfg = video.borrow().clone();

    // A configuration that already has both — an older version allowed it.
    let legacy = Notice::new(
        Kind::Conflict,
        &i18n("Wine FSR and Gamescope upscaling are both on"),
        &i18n(
            "Two upscalers in series scale the image twice. Until one is chosen, Wine FSR is switched off at launch for games Gamescope upscales.",
        ),
    );
    legacy.set_visible(!opt::general_conflicts(&cfg).is_empty());
    group.add(legacy.widget());

    let wine = adw::SwitchRow::builder()
        .title("Wine FSR")
        .subtitle(i18n(
            "Wine's own upscaling for Proton games in exclusive fullscreen (WINE_FULLSCREEN_FSR=1)",
        ))
        .active(cfg.upscaling.wine_fsr_enabled)
        .build();
    group.add(&wine);
    let quality_items = gtk4::StringList::new(&[
        &i18n("Performance"),
        &i18n("Balanced"),
        &i18n("Quality"),
        &i18n("Ultra"),
    ]);
    let quality = adw::ComboRow::builder()
        .title(i18n("Wine FSR quality"))
        .model(&quality_items)
        .selected(match cfg.upscaling.wine_fsr_mode {
            WineFsrMode::Performance => 0,
            WineFsrMode::Balanced => 1,
            WineFsrMode::Quality => 2,
            WineFsrMode::Ultra => 3,
        })
        .sensitive(cfg.upscaling.wine_fsr_enabled)
        .build();
    group.add(&quality);
    {
        let video = Rc::clone(video);
        quality.connect_selected_notify(move |row| {
            video.borrow_mut().upscaling.wine_fsr_mode = match row.selected() {
                0 => WineFsrMode::Performance,
                1 => WineFsrMode::Balanced,
                3 => WineFsrMode::Ultra,
                _ => WineFsrMode::Quality,
            };
            save_video(&video, row);
        });
    }

    let quiet = Rc::clone(&display.quiet);
    // Once resolved, the notice says what is on now instead of vanishing.
    let refresh_legacy = {
        let (legacy, video) = (legacy.clone(), Rc::clone(video));
        let had = Cell::new(!opt::general_conflicts(&cfg).is_empty());
        Rc::new(move || {
            let now = !opt::general_conflicts(&video.borrow()).is_empty();
            if had.get() && !now {
                let kept = if video.borrow().upscaling.wine_fsr_enabled {
                    "Wine FSR".to_owned()
                } else {
                    notice::feature_name(Feature::GamescopeUpscaling)
                };
                legacy.clear_actions();
                legacy.set(
                    Kind::Success,
                    &i18n("Resolved: %s is the only upscaler").replace("%s", &kept),
                    "",
                );
            }
            had.set(now);
        })
    };

    // The page's own way to switch a feature off, used by both prompts and
    // by the legacy notice: the control moves, the file follows.
    let turn_off: Rc<dyn Fn(Feature)> = {
        let (video, wine, quiet) = (Rc::clone(video), wine.clone(), Rc::clone(&quiet));
        let rows = display.rows.clone();
        let refresh = Rc::clone(&refresh_legacy);
        Rc::new(move |f: Feature| {
            quiet.set(true);
            match f {
                Feature::WineFsr => wine.set_active(false),
                Feature::GamescopeUpscaling => {
                    if let Some((_, w, h)) = &rows {
                        w.set_value(0.0);
                        h.set_value(0.0);
                    }
                }
                _ => {}
            }
            quiet.set(false);
            opt::turn_off_general(&mut video.borrow_mut(), f);
            save_video(&video, &wine);
            refresh();
        })
    };
    {
        let turn_off = Rc::clone(&turn_off);
        legacy.add_action(
            &i18n("Keep %s").replace("%s", &notice::feature_name(Feature::GamescopeUpscaling)),
            false,
            move |_| {
                turn_off(Feature::WineFsr);
            },
        );
    }
    {
        let turn_off = Rc::clone(&turn_off);
        legacy.add_action(&i18n("Use %s").replace("%s", "Wine FSR"), true, move |_| {
            turn_off(Feature::GamescopeUpscaling);
        });
    }

    // Wine FSR switched on while Gamescope upscales: ask.
    {
        let (video, quality, quiet) = (Rc::clone(video), quality.clone(), Rc::clone(&quiet));
        let turn_off = Rc::clone(&turn_off);
        let refresh = Rc::clone(&refresh_legacy);
        wine.connect_active_notify(move |row| {
            let on = row.is_active();
            quality.set_sensitive(on);
            if quiet.get() {
                return;
            }
            let upscales = opt::gamescope_upscales(&video.borrow().upscaling);
            if on && upscales {
                if let Some(c) = opt::conflict(Feature::WineFsr, Feature::GamescopeUpscaling) {
                    let (turn_off, row2, quiet2) =
                        (Rc::clone(&turn_off), row.clone(), Rc::clone(&quiet));
                    let (video2, anchor) = (Rc::clone(&video), row.clone());
                    notice::ask_conflict(row, &c, move |use_wine| {
                        if use_wine {
                            video2.borrow_mut().upscaling.wine_fsr_enabled = true;
                            turn_off(Feature::GamescopeUpscaling);
                            crate::widgets::toast::show(
                                &anchor,
                                &i18n(
                                    "Wine FSR is on; Gamescope now renders at the game's own size",
                                ),
                            );
                        } else {
                            quiet2.set(true);
                            row2.set_active(false);
                            quiet2.set(false);
                        }
                    });
                    return;
                }
            }
            video.borrow_mut().upscaling.wine_fsr_enabled = on;
            save_video(&video, row);
            refresh();
        });
    }

    // Gamescope's upscaling switched on (the switch, or a render size) while
    // Wine FSR is on: ask. Keeping Wine FSR puts the render size back.
    if let Some((expander, render_w, render_h)) = display.rows.clone() {
        let ask = {
            let (video, wine, quiet) = (Rc::clone(video), wine.clone(), Rc::clone(&quiet));
            let turn_off = Rc::clone(&turn_off);
            let refresh = Rc::clone(&refresh_legacy);
            let (expander, render_w, render_h) =
                (expander.clone(), render_w.clone(), render_h.clone());
            Rc::new(move |before: (bool, u32, u32)| {
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                let after = (
                    expander.enables_expansion(),
                    render_w.value() as u32,
                    render_h.value() as u32,
                );
                {
                    let mut v = video.borrow_mut();
                    v.upscaling.gamescope_enabled = after.0;
                    v.upscaling.base_width = after.1;
                    v.upscaling.base_height = after.2;
                }
                let was = before.0 && before.1 > 0 && before.2 > 0;
                let now = opt::gamescope_upscales(&video.borrow().upscaling);
                if !quiet.get() && now && !was && wine.is_active() {
                    if let Some(c) = opt::conflict(Feature::GamescopeUpscaling, Feature::WineFsr) {
                        let (turn_off, video2, quiet2) =
                            (Rc::clone(&turn_off), Rc::clone(&video), Rc::clone(&quiet));
                        let (expander2, w2, h2) =
                            (expander.clone(), render_w.clone(), render_h.clone());
                        notice::ask_conflict(&expander, &c, move |use_gamescope| {
                            if use_gamescope {
                                turn_off(Feature::WineFsr);
                                crate::widgets::toast::show(
                                    &expander2,
                                    &i18n("Gamescope upscaling is on; Wine FSR was turned off"),
                                );
                            } else {
                                quiet2.set(true);
                                expander2.set_enable_expansion(before.0);
                                w2.set_value(f64::from(before.1));
                                h2.set_value(f64::from(before.2));
                                quiet2.set(false);
                                {
                                    let mut v = video2.borrow_mut();
                                    v.upscaling.gamescope_enabled = before.0;
                                    v.upscaling.base_width = before.1;
                                    v.upscaling.base_height = before.2;
                                }
                                save_video(&video2, &expander2);
                            }
                        });
                        // Nothing is saved until one is chosen: the file never
                        // holds both, even while the question is open.
                        return;
                    }
                }
                save_video(&video, &expander);
                refresh();
            })
        };
        let last = Rc::new(Cell::new((
            cfg.upscaling.gamescope_enabled,
            cfg.upscaling.base_width,
            cfg.upscaling.base_height,
        )));
        let track = {
            let (last, expander, render_w, render_h) = (
                Rc::clone(&last),
                expander.clone(),
                render_w.clone(),
                render_h.clone(),
            );
            let ask = Rc::clone(&ask);
            Rc::new(move || {
                let before = last.get();
                ask(before);
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                last.set((
                    expander.enables_expansion(),
                    render_w.value() as u32,
                    render_h.value() as u32,
                ));
            })
        };
        {
            let track = Rc::clone(&track);
            expander.connect_enable_expansion_notify(move |_| track());
        }
        {
            let track = Rc::clone(&track);
            render_w.connect_value_changed(move |_| track());
        }
        render_h.connect_value_changed(move |_| track());
    }

    // vkBasalt: a look, not a speed-up; only where its layer is installed.
    if bigame_core::capabilities::vkbasalt_installed() {
        let vkb = adw::ExpanderRow::builder()
            .title("vkBasalt")
            .subtitle(i18n(
                "Visual filters (sharpening, colour) for Vulkan and Proton games. A look, not a speed-up: it costs a little GPU time.",
            ))
            .show_enable_switch(true)
            .enable_expansion(cfg.upscaling.vkbasalt_enabled)
            .build();
        let conf = adw::EntryRow::builder()
            .title(i18n("Configuration file"))
            .text(cfg.upscaling.vkbasalt_config_path.as_deref().unwrap_or(""))
            .build();
        vkb.add_row(&conf);
        {
            let video = Rc::clone(video);
            vkb.connect_enable_expansion_notify(move |e| {
                video.borrow_mut().upscaling.vkbasalt_enabled = e.enables_expansion();
                save_video(&video, e);
            });
        }
        {
            let video = Rc::clone(video);
            conf.connect_changed(move |row| {
                let text = row.text().to_string();
                video.borrow_mut().upscaling.vkbasalt_config_path =
                    (!text.is_empty()).then_some(text);
                save_video(&video, row);
            });
        }
        group.add(&vkb);
    } else {
        group.add(&ui::missing_row(
            "vkBasalt",
            &i18n(
                "Not installed. Visual filters (sharpening, colour) for Vulkan and Proton games.",
            ),
            "sudo pacman -S vkbasalt",
        ));
    }

    let ai = Notice::new(
        Kind::Info,
        &i18n("AI Graphics is per game"),
        &i18n(
            "Upscaling inside the game (FSR 4, XeSS, DLSS through OptiScaler) is set from the game's card in Profiles. A game with it launches without Wine FSR and without a Gamescope render size.",
        ),
    );
    group.add(ai.widget());
    group
}

// ── Frame generation ────────────────────────────────────────────────────────

/// lsfg-vk's general switch and its DLL. Each game's own frame generation
/// is in its profile.
fn build_frame_generation(video: &SharedVideo, m: &Machine) -> adw::PreferencesGroup {
    let group = ui::section(&i18n("Frame generation"));
    if !m.lsfg {
        group.add(&ui::missing_row(
            "lsfg-vk",
            &i18n("Not installed. Generates extra frames between rendered ones (Lossless Scaling's method, as a Vulkan layer). It needs your own Lossless.dll."),
            "sudo pacman -S lsfg-vk",
        ));
        return group;
    }
    let switch = adw::SwitchRow::builder()
        .title(i18n("lsfg-vk for every game with an entry"))
        .subtitle(i18n(
            "Raises the presented frame rate, not the rendered one, and adds latency. Off sets every entry aside; on puts them back.",
        ))
        .active(opt::lsfg_general_on(&video.borrow()))
        .build();
    group.add(&switch);
    {
        let video = Rc::clone(video);
        switch.connect_active_notify(move |row| {
            let on = row.is_active();
            {
                let mut v = video.borrow_mut();
                v.frame_gen.enabled = on;
                v.frame_gen.backend = if on {
                    FrameGenBackend::LsfgVk
                } else {
                    FrameGenBackend::None
                };
            }
            save_video(&video, row);
            if let Err(e) = bigame_core::fg::sync_global_enablement(&video.borrow().frame_gen) {
                crate::widgets::toast::error(
                    row,
                    &i18n("Could not update lsfg-vk's file"),
                    &error_text(&e),
                );
            }
        });
    }
    let dll_state = Notice::new(Kind::Warning, "", "");
    let show_dll = {
        let dll_state = dll_state.clone();
        move |ready: bool| {
            if ready {
                dll_state.set_visible(false);
            } else {
                dll_state.set(
                    Kind::Warning,
                    &i18n("lsfg-vk needs your Lossless.dll"),
                    &i18n("Without it lsfg-vk loads and generates nothing, so it is switched off at launch."),
                );
                dll_state.set_visible(true);
            }
        }
    };
    show_dll(m.lsfg_dll);
    group.add(&crate::widgets::fg_controls::dll_row(show_dll));
    group.add(dll_state.widget());
    let per_game = Notice::new(
        Kind::Info,
        &i18n("Each game's frame generation is in its profile"),
        &i18n(
            "Profiles → the game → Edit profile → Frame generation. A game whose AI Graphics has OptiScaler generating frames launches with lsfg-vk off.",
        ),
    );
    group.add(per_game.widget());
    group
}

// ── Monitoring ──────────────────────────────────────────────────────────────

/// `MangoHud`: shown per game (in the profile); how it looks, here.
fn build_monitoring(m: &Machine) -> adw::PreferencesGroup {
    use bigame_core::mangohud::{Style, StyleState};
    let group = ui::section(&i18n("Monitoring"));
    if !m.mangohud {
        group.add(&ui::missing_row(
            "MangoHud",
            &i18n("Not installed. The performance overlay; it also captures the frametimes Measure the difference uses."),
            "sudo pacman -S mangohud",
        ));
        return group;
    }
    let row = adw::ActionRow::builder()
        .title("MangoHud")
        .subtitle(i18n(
            "Installed. Shown per game, from its profile (Off, On, Forced); Details says whether it loaded in the running game.",
        ))
        .subtitle_lines(3)
        .use_markup(false)
        .build();
    let chip = Chip::new(State::Configured);
    chip.set(State::Configured, Some(&i18n("Per game")));
    row.add_suffix(chip.widget());
    group.add(&row);

    let state = bigame_core::mangohud::current_style();
    let own_label = match state {
        StyleState::Defaults => i18n("Default"),
        _ => i18n("My own file"),
    };
    let style = Picker::new(
        &i18n("Overlay style"),
        &i18n(
            "Written to ~/.config/MangoHud/MangoHud.conf; your own file is kept and comes back with “My own file”.",
        ),
        &[
            ("own".to_owned(), own_label),
            ("basic".to_owned(), i18n("Steam Deck — basic")),
            ("full".to_owned(), i18n("Steam Deck — full")),
        ],
        match state {
            StyleState::Style(Style::Basic) => "basic",
            StyleState::Style(Style::Full) => "full",
            _ => "own",
        },
    );
    style.row.set_subtitle_lines(3);
    style.row.add_suffix(&crate::widgets::info::button(
        &i18n("Overlay style"),
        &i18n("Basic is one line across the top, like the Steam Deck's level 2: frame rate, frame times, CPU and GPU load and power, memory and video memory. Full is a column, like its level 3: the GPU and the CPU each with load, temperature, clock and power, then memory, frame rate and frame times. Battery appears only on a laptop. A per-game MangoHud file (wine-<game>.conf) takes precedence, and a Flatpak launcher reads its own copy."),
    ));
    group.add(&style.row);
    let row = style.row.clone();
    style.connect_changed(move |value| {
        let chosen = match value {
            "basic" => Style::Basic,
            "full" => Style::Full,
            _ => Style::Own,
        };
        match bigame_core::mangohud::set_style(chosen) {
            Ok(()) => crate::widgets::toast::show(
                &row,
                &i18n("Saved. It applies the next time a game starts."),
            ),
            Err(e) => crate::widgets::toast::error(
                &row,
                &i18n("Could not change MangoHud's file"),
                &error_text(&e),
            ),
        }
    });
    group
}

// ── Advanced ────────────────────────────────────────────────────────────────

/// falcond's own settings and the facts for the person who knows what a
/// scheduler flag is: collapsed, last.
#[allow(clippy::too_many_lines)]
fn build_advanced(shared: &SharedConfig, m: &Machine) -> adw::PreferencesGroup {
    let group = ui::section(&i18n("Advanced"));

    let falcond = adw::ExpanderRow::builder()
        .title(i18n("falcond's settings"))
        .subtitle(i18n(
            "How it looks for games, and which profile set it uses",
        ))
        .build();
    let poll_row = adw::SpinRow::new(
        Some(&gtk4::Adjustment::new(
            f64::from(shared.borrow().poll_interval_ms),
            500.0,
            60_000.0,
            500.0,
            1000.0,
            0.0,
        )),
        500.0,
        0,
    );
    poll_row.set_title(&i18n("Scan interval (ms)"));
    poll_row.set_subtitle(&i18n(
        "How often falcond looks for a running game. Lower reacts sooner and costs more; the default is 9000.",
    ));
    falcond.add_row(&poll_row);
    {
        let cfg = Rc::clone(shared);
        poll_row.connect_changed(move |row| {
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let val = row.value() as u32;
            cfg.borrow_mut().poll_interval_ms = val;
            save_config(&cfg, row);
        });
    }
    let sets: Vec<(String, String)> = PROFILE_SETS
        .iter()
        .map(|c| (c.id.to_owned(), format!("{} ({})", i18n(c.label), c.id)))
        .collect();
    let set = Picker::new(
        &i18n("Profile set"),
        &i18n("Desktop (none) is the one for a PC at a desk"),
        &sets,
        &shared.borrow().profile_mode,
    );
    set.row
        .add_suffix(&crate::widgets::info::profile_sets_button());
    falcond.add_row(&set.row);
    {
        let cfg = Rc::clone(shared);
        let row = set.row.clone();
        set.connect_changed(move |value| {
            value.clone_into(&mut cfg.borrow_mut().profile_mode);
            save_config(&cfg, &row);
        });
    }
    // The governor, for reference: power-profiles-daemon sets it.
    let gov_row = adw::ActionRow::builder()
        .title(i18n("CPU governor"))
        .subtitle(i18n("Reading…"))
        .use_markup(false)
        .build();
    falcond.add_row(&gov_row);
    {
        let row = gov_row.clone();
        glib::spawn_future_local(async move {
            let (available, current) = gio::spawn_blocking(|| {
                let read = |p: &str| std::fs::read_to_string(p).unwrap_or_default();
                (
                    read("/sys/devices/system/cpu/cpu0/cpufreq/scaling_available_governors"),
                    read("/sys/devices/system/cpu/cpu0/cpufreq/scaling_governor"),
                )
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
    group.add(&falcond);

    let facts = adw::ExpanderRow::builder()
        .title(i18n("Show advanced options"))
        .subtitle(i18n(
            "sched-ext availability, Gamescope's accepted options, the environment file",
        ))
        .build();
    group.add(&facts);
    let scx_status = adw::ActionRow::builder()
        .title(i18n("sched-ext availability"))
        .subtitle(match m.sched.describe_text() {
            Some(reason) => tr(&reason),
            None => i18n("Available — falcond applies the scheduler you configure above"),
        })
        .use_markup(false)
        .build();
    scx_status.add_prefix(&gtk4::Image::from_icon_name(if m.sched.is_available() {
        "object-select-symbolic"
    } else {
        "dialog-warning-symbolic"
    }));
    facts.add_row(&scx_status);
    facts.add_row(
        &adw::ActionRow::builder()
            .title(i18n("Installed"))
            .subtitle(if m.sched_installed.is_empty() {
                i18n("none")
            } else {
                m.sched_installed.join(", ")
            })
            .use_markup(false)
            .build(),
    );
    let env_row = adw::ActionRow::builder()
        .title(i18n("Environment file"))
        .subtitle(i18n(
            "Wine FSR and vkBasalt are written to ~/.config/environment.d/bigame-mode.conf and pushed into the user systemd manager, so every game started afterwards inherits them.",
        ))
        .subtitle_lines(4)
        .use_markup(false)
        .build();
    facts.add_row(&env_row);

    // Gamescope's options come from `gamescope --help`, off the main thread.
    let gamescope_row = adw::ActionRow::builder()
        .title(i18n("Gamescope options this build accepts"))
        .subtitle(if m.gamescope {
            i18n("Reading…")
        } else {
            i18n("Gamescope is not installed")
        })
        .use_markup(false)
        .build();
    facts.add_row(&gamescope_row);
    if m.gamescope {
        let facts = facts.clone();
        glib::spawn_future_local(async move {
            let Some(gs) = gio::spawn_blocking(bigame_core::capabilities::gamescope_cached)
                .await
                .ok()
                .flatten()
            else {
                gamescope_row.set_subtitle(&i18n("Gamescope is not installed"));
                return;
            };
            gamescope_row.set_subtitle(
                &i18n("Version %v — %n options detected from --help")
                    .replace(
                        "%v",
                        &gs.version
                            .map_or_else(|| i18n("unknown"), |v| v.to_string()),
                    )
                    .replace("%n", &gs.flags.len().to_string()),
            );
            // The generated command line is the honest "advanced options"
            // box: the arguments come from capabilities.
            let sample = bigame_core::gamescope::Config {
                render_width: 1920,
                render_height: 1080,
                filter: bigame_core::gamescope::Filter::Fsr,
                sharpness: 5,
                ..bigame_core::gamescope::Config::default()
            };
            let built = sample.to_args(&gs);
            let preview = adw::ActionRow::builder()
                .title(i18n("Example command line"))
                .use_markup(false)
                .build();
            preview.set_subtitle(&format!("gamescope {} -- <game>", built.args.join(" ")));
            preview.set_subtitle_selectable(true);
            facts.add_row(&preview);
            for unsupported in &built.unsupported {
                let row = adw::ActionRow::builder()
                    .title(i18n("Not supported by this Gamescope"))
                    .subtitle(format!(
                        "--{} — {}",
                        unsupported.flag,
                        i18n(unsupported.effect)
                    ))
                    .use_markup(false)
                    .build();
                row.add_prefix(&gtk4::Image::from_icon_name("dialog-warning-symbolic"));
                facts.add_row(&row);
            }
        });
    }
    group
}
