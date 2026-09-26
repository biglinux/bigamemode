//! Profile creation wizard — child-friendly guided flow.
//!
//! Opens as an `adw::Dialog` with 9 steps; each step explains
//! one tuning option in plain language and at the end assembles
//! a `GameProfile` from the user's choices.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use libadwaita as adw;

use crate::i18n::i18n;
use bigame_core::profiles::GameProfile;

const STEPS: usize = 9;

// No per-game CPU governor step: falcond does not read one. The power
// profile it does set follows "performance mode" (step 2).
const STEP_IDS: &[&str; STEPS] = &[
    "game",      // 1 – executable name
    "perf",      // 2 – performance mode (turbo vs normal)
    "sched",     // 3 – sched-ext scheduler
    "vcache",    // 4 – AMD VCache mode
    "gamescope", // 5 – Gamescope display layer
    "fg",        // 6 – Frame Generation (LSFG-VK)
    "idle",      // 7 – Idle inhibit (screen sleep)
    "ai",        // 8 – AI Graphics (optional)
    "review",    // 9 – summary + save
];

/// Building a widget tree is inherently linear — splitting it yields helpers
/// with a single caller and no independent meaning — so the length lint is
/// allowed here rather than worked around.
#[allow(clippy::too_many_lines)]
/// Open the wizard dialog attached to `parent`.
pub fn open(parent: &impl IsA<gtk4::Widget>, on_saved: impl Fn(GameProfile) + 'static) {
    open_internal(parent, None, on_saved);
}

/// Open the wizard with a pre-filled suggested program name.
pub fn open_with_suggested_name(
    parent: &impl IsA<gtk4::Widget>,
    suggested_name: &str,
    on_saved: impl Fn(GameProfile) + 'static,
) {
    open_internal(parent, Some(suggested_name.to_string()), on_saved);
}

/// Building a widget tree is inherently linear — splitting it yields helpers
/// with a single caller and no independent meaning — so the length lint is
/// allowed here rather than worked around.
#[allow(clippy::too_many_lines, clippy::needless_pass_by_value)]
fn open_internal(
    parent: &impl IsA<gtk4::Widget>,
    suggested_name: Option<String>,
    // Moved into the GTK closures that outlive this call.
    on_saved: impl Fn(GameProfile) + 'static,
) {
    let parent_w: gtk4::Widget = parent.clone().upcast();
    let profile = Rc::new(RefCell::new(GameProfile::default()));
    let current = Rc::new(RefCell::new(0usize));
    let on_saved_cb = Rc::new(on_saved);

    // ── Dialog ────────────────────────────────────────────────────────
    let dialog = adw::Dialog::builder()
        .title(i18n("Create Profile"))
        .content_width(580)
        .content_height(640)
        .build();

    // ── Header: flat + progress dots as title ─────────────────────────
    let header = adw::HeaderBar::new();
    header.add_css_class("flat");

    let dots_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    dots_row.set_halign(gtk4::Align::Center);
    dots_row.set_valign(gtk4::Align::Center); // Fix vertical stretching
    header.set_show_title(false);

    let dots: Vec<gtk4::Box> = (0..STEPS)
        .map(|_| {
            let d = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
            d.add_css_class("progress-dot");
            d.set_valign(gtk4::Align::Center); // Ensure dots don't stretch
            dots_row.append(&d);
            d
        })
        .collect();
    dots[0].add_css_class("active");

    // ── Step pages ────────────────────────────────────────────────────
    let stack = gtk4::Stack::new();
    stack.set_transition_duration(250);
    stack.set_vexpand(true);

    // Step 1 — Game executable name
    let name_entry = adw::EntryRow::builder()
        .title(i18n("Program name (what you see in Task Manager)"))
        .build();
    if let Some(name) = suggested_name.as_deref() {
        name_entry.set_text(name);
    }
    let name_group = adw::PreferencesGroup::new();
    name_group.add_css_class("wizard-input-card");
    name_group.add(&name_entry);
    stack.add_named(
        &wizard_step(
                        &i18n("Program Name"),
            &i18n(
                "Enter the exact name of the executable you want to trigger this profile.\nExamples: minecraft, dota2, steam",
            ),
            Some(&name_group),
        ),
        Some("game"),
    );

    // Step 2 — Performance mode
    let (perf_group, perf_normal, _perf_turbo) = build_radio_group(&[
        (
            "",
            &i18n("Normal"),
            &i18n("Saves electricity. Good for calm games like Minecraft."),
        ),
        (
            "",
            &i18n("Turbo"),
            &i18n("Maximum power! Like a race car. Best for fast action games."),
        ),
    ]);
    perf_group.add_css_class("wizard-input-card");
    stack.add_named(
        &wizard_step(
                        &i18n("Performance Mode"),
            &i18n(
                "Turbo mode prevents power-saving features to maximize framerates at the cost of higher power consumption.",
            ),
            Some(&perf_group),
        ),
        Some("perf"),
    );

    // Step 3 — Scheduler
    let installed = bigame_core::sched::detect_installed();
    let sched_choices: Vec<&str> = {
        let mut v = vec![""];
        v.extend(installed.iter().map(String::as_str));
        v
    };
    let sched_model = gtk4::StringList::new(&sched_choices);
    let sched_combo = adw::ComboRow::builder()
        .title(i18n("Scheduler"))
        .subtitle(i18n(
            "Which scheduler to use (leave blank = system default)",
        ))
        .model(&sched_model)
        .build();
    let modes = ["default", "gaming", "power", "latency"];
    let mode_model = gtk4::StringList::new(&modes);
    let mode_combo = adw::ComboRow::builder()
        .title(i18n("Scheduler Mode"))
        .subtitle(i18n("How the scheduler should prioritise this game"))
        .model(&mode_model)
        .build();
    let sched_group = adw::PreferencesGroup::new();
    sched_group.add_css_class("wizard-input-card");
    sched_group.add(&sched_combo);
    sched_group.add(&mode_combo);
    stack.add_named(
        &wizard_step(
                        &i18n("Scheduler Priority"),
            &i18n(
                "A custom scheduler can dramatically improve frametimes and reduce stuttering. Leave blank to use the system default.",
            ),
            Some(&sched_group),
        ),
        Some("sched"),
    );

    // Step 4 — VCache Mode (AMD)
    let vcache_available = bigame_core::vcache::is_available();
    let (vcache_group, vcache_off, vcache_cache) = build_radio_group(&[
        (
            "",
            &i18n("Off (default)"),
            &i18n("Standard memory. Works for all computers."),
        ),
        (
            "",
            &i18n("Cache mode"),
            &i18n("Puts game data in super-fast memory. For AMD Ryzen with 3D V-Cache."),
        ),
        (
            "",
            &i18n("Frequency mode"),
            &i18n("Adjusts memory speed. Advanced — only for AMD 3D V-Cache CPUs."),
        ),
    ]);
    vcache_group.add_css_class("wizard-input-card");

    let vcache_desc = if vcache_available {
        i18n(
            "Your processor supports 3D V-Cache allocation. Selecting Cache mode can significantly improve gaming performance.",
        )
    } else {
        i18n(
            "Your processor does not support 3D V-Cache dynamic allocation. This setting has been disabled.",
        )
    };
    stack.add_named(
        &wizard_step(
            &i18n("Memory Optimization"),
            &vcache_desc,
            Some(&vcache_group),
        ),
        Some("vcache"),
    );

    // Step 5 — Gamescope
    let gs_switch = adw::SwitchRow::builder()
        .title(i18n("Enable Gamescope"))
        .subtitle(i18n("Wrap the game in a special display layer"))
        .build();
    let gs_width = adw::SpinRow::new(
        Some(&gtk4::Adjustment::new(
            1920.0, 640.0, 7680.0, 1.0, 10.0, 0.0,
        )),
        1.0,
        0,
    );
    gs_width.set_title(&i18n("Width (pixels)"));
    gs_width.set_sensitive(false);
    let gs_height = adw::SpinRow::new(
        Some(&gtk4::Adjustment::new(
            1080.0, 480.0, 4320.0, 1.0, 10.0, 0.0,
        )),
        1.0,
        0,
    );
    gs_height.set_title(&i18n("Height (pixels)"));
    gs_height.set_sensitive(false);
    let gs_fsr = adw::SwitchRow::builder()
        .title(i18n("FSR Sharpening"))
        .subtitle(i18n("Makes the image sharper (AMD FidelityFX)"))
        .sensitive(false)
        .build();
    let gs_fps = adw::SpinRow::new(
        Some(&gtk4::Adjustment::new(0.0, 0.0, 500.0, 1.0, 10.0, 0.0)),
        1.0,
        0,
    );
    gs_fps.set_title(&i18n("Framerate Limit (0 = unlimited)"));
    gs_fps.set_sensitive(false);
    let gs_group = adw::PreferencesGroup::new();
    gs_group.add_css_class("wizard-input-card");
    gs_group.add(&gs_switch);
    gs_group.add(&gs_width);
    gs_group.add(&gs_height);
    gs_group.add(&gs_fsr);
    gs_group.add(&gs_fps);

    // Sensitivity on/off
    {
        let w = gs_width.clone();
        let h = gs_height.clone();
        let f = gs_fsr.clone();
        let fps = gs_fps.clone();
        gs_switch.connect_active_notify(move |sw| {
            let on = sw.is_active();
            w.set_sensitive(on);
            h.set_sensitive(on);
            f.set_sensitive(on);
            fps.set_sensitive(on);
        });
    }

    stack.add_named(
        &wizard_step(
                        &i18n("Display Layer"),
            &i18n(
                "Gamescope provides an isolated compositor for the game, enabling resolution scaling, framerate limiting, and FidelityFX Super Resolution (FSR).",
            ),
            Some(&gs_group),
        ),
        Some("gamescope"),
    );

    // Step 6 — Frame Generation (LSFG-VK)
    let fg_mult_adj = gtk4::Adjustment::new(1.0, 1.0, 4.0, 1.0, 1.0, 0.0);
    let fg_mult_row = adw::SpinRow::new(Some(&fg_mult_adj), 1.0, 0);
    fg_mult_row.set_title(&i18n("Multiplier (1-4x)"));
    fg_mult_row.set_subtitle(&i18n("Generated frames per real frame"));

    let fg_flow_adj = gtk4::Adjustment::new(100.0, 0.0, 100.0, 1.0, 10.0, 0.0);
    let fg_flow_row = adw::SpinRow::new(Some(&fg_flow_adj), 1.0, 0);
    fg_flow_row.set_title(&i18n("Flow Scale (%)"));
    fg_flow_row.set_subtitle(&i18n("Optical flow vector scaling"));

    let fg_perf_row = adw::SwitchRow::builder()
        .title(i18n("FG Performance Mode"))
        .subtitle(i18n("Prioritize latency over quality"))
        .active(false)
        .build();

    let fg_group = adw::PreferencesGroup::new();
    fg_group.add_css_class("wizard-input-card");
    fg_group.add(&fg_mult_row);
    fg_group.add(&fg_flow_row);
    fg_group.add(&fg_perf_row);

    stack.add_named(
        &wizard_step(
                        &i18n("Frame Generation"),
            &i18n(
                "LSFG-VK inserts synthetically generated frames to multiply your framerate, providing a smoother visual experience at the cost of slight input latency.",
            ),
            Some(&fg_group),
        ),
        Some("fg"),
    );

    // Step 7 — Idle Inhibit (screen sleep)
    let idle_switch = adw::SwitchRow::builder()
        .title(i18n("Keep Screen Awake"))
        .subtitle(i18n("Prevent the screen from turning off while playing"))
        .build();
    let idle_group = adw::PreferencesGroup::new();
    idle_group.add_css_class("wizard-input-card");
    idle_group.add(&idle_switch);
    stack.add_named(
        &wizard_step(
            &i18n("Idle Behavior"),
            &i18n(
                "Inhibits the screen saver and automatic screen sleep while the game is running.",
            ),
            Some(&idle_group),
        ),
        Some("idle"),
    );

    // Step 8 — AI Graphics (optional, nothing applied without a plan)
    let (ai_group, ai_recommended, ai_advanced) = build_radio_group(&[
        (
            "",
            &i18n("Recommended"),
            &i18n(
                "BiGame-mode looks at the game and your graphics card and shows what it would do. Nothing changes until you press Apply.",
            ),
        ),
        (
            "",
            &i18n("Advanced…"),
            &i18n("Choose the upscaler and frame generation yourself."),
        ),
        (
            "",
            &i18n("Not now"),
            &i18n("Leave the game's graphics as they are."),
        ),
    ]);
    ai_group.add_css_class("wizard-input-card");
    stack.add_named(
        &wizard_step(
                        &i18n("AI Graphics"),
            &i18n(
                "Improve image quality and performance using technologies such as DLSS, FSR, XeSS, OptiScaler and compatible neural-rendering features.\nEnable AI Graphics for this game?",
            ),
            Some(&ai_group),
        ),
        Some("ai"),
    );
    let ai_choice = Rc::new(std::cell::Cell::new(
        bigame_core::graphics::config::Mode::Off,
    ));

    // Step 9 — Summary (populated just before showing)
    let summary_box = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
    summary_box.set_margin_top(12);
    summary_box.set_margin_bottom(12);
    summary_box.set_margin_start(20);
    summary_box.set_margin_end(20);
    let summary_page = wizard_step(
        &i18n("Profile Summary"),
        &i18n("Review your profile settings before saving."),
        Some(&summary_box),
    );
    stack.add_named(&summary_page, Some("review"));

    // ── Bottom navigation bar ─────────────────────────────────────────
    let back_btn = gtk4::Button::builder()
        .icon_name("go-previous-symbolic")
        .tooltip_text(i18n("Back"))
        .css_classes(["circular", "flat"])
        .visible(false)
        .build();

    let next_btn = gtk4::Button::builder()
        .icon_name("go-next-symbolic")
        .css_classes(["suggested-action", "circular"])
        .hexpand(false)
        .build();

    let nav_bar = gtk4::CenterBox::new();
    nav_bar.set_margin_top(16);
    nav_bar.set_margin_bottom(24);
    nav_bar.set_margin_start(24);
    nav_bar.set_margin_end(24);
    nav_bar.set_start_widget(Some(&back_btn));
    nav_bar.set_center_widget(Some(&dots_row));
    nav_bar.set_end_widget(Some(&next_btn));

    // ── Layout ────────────────────────────────────────────────────────
    let vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    vbox.append(&stack);
    vbox.append(&nav_bar);

    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&vbox));
    dialog.set_child(Some(&toolbar));

    // ══ Navigation: Next ══════════════════════════════════════════════
    {
        let profile = profile.clone();
        let current = current.clone();
        let stack = stack.clone();
        let dots = dots.clone();
        let back_btn = back_btn.clone();
        let next_btn = next_btn.clone();
        let dialog = dialog.clone();
        let summary_box = summary_box.clone();
        let on_saved_ref = on_saved_cb.clone();

        next_btn.clone().connect_clicked(move |_| {
            let step = *current.borrow();

            // Collect value for the current step into the profile
            {
                let mut p = profile.borrow_mut();
                match step {
                    0 => p.name = name_entry.text().to_string(),
                    1 => p.performance_mode = !perf_normal.is_active(),
                    2 => {
                        if let Some(s) = sched_model.string(sched_combo.selected()) {
                            p.scx_sched = s.to_string();
                        }
                        let idx = mode_combo.selected() as usize;
                        p.scx_sched_props =
                            modes.get(idx).copied().unwrap_or("default").to_string();
                    }
                    3 => {
                        p.vcache_mode = if vcache_off.is_active() {
                            "none".into()
                        } else if vcache_cache.is_active() {
                            "cache".into()
                        } else {
                            "freq".into()
                        };
                    }
                    4 =>
                    {
                        #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
                        if gs_switch.is_active() {
                            let fps = gs_fps.value() as u32;
                            p.gamescope = Some(bigame_core::gamescope::Config {
                                render_width: gs_width.value() as u32,
                                render_height: gs_height.value() as u32,
                                filter: if gs_fsr.is_active() {
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
                            });
                        } else {
                            p.gamescope = None;
                        }
                    }
                    5 => {
                        #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
                        {
                            p.fg_multiplier = fg_mult_row.value() as u32;
                            p.fg_flow_scale = fg_flow_row.value() as u32;
                            p.fg_perf_mode = fg_perf_row.is_active();
                        }
                    }
                    6 => {
                        p.idle_inhibit = idle_switch.is_active();
                    }
                    7 => {
                        use bigame_core::graphics::config::Mode;
                        ai_choice.set(if ai_recommended.is_active() {
                            Mode::Recommended
                        } else if ai_advanced.is_active() {
                            Mode::Advanced
                        } else {
                            Mode::Off
                        });
                    }
                    _ => {}
                }
            }

            let next_step = step + 1;

            if next_step >= STEPS {
                // ── Save ──────────────────────────────────────────────
                let p = profile.borrow().clone();

                // Basic validation
                if p.name.trim().is_empty() {
                    crate::widgets::toast::show(&next_btn, &i18n("Game name cannot be empty"));
                    return;
                }

                next_btn.set_sensitive(false);
                next_btn.set_label(&i18n("Saving…"));

                let next_btn_ref = next_btn.clone();
                let dialog_ref = dialog.clone();
                let p_clone = p.clone();
                let on_saved_final = on_saved_ref.clone();
                let ai_choice_final = ai_choice.clone();
                let parent_w = parent_w.clone();

                gtk4::glib::spawn_future_local(async move {
                    tracing::info!(profile = %p_clone.name, "wizard save requested");
                    // Off the main thread: the helper may wait on a Polkit
                    // password prompt.
                    let to_save = p_clone.clone();
                    let saved = gtk4::gio::spawn_blocking(move || bigame_core::profiles::save(&to_save))
                        .await
                        .unwrap_or_else(|_| Err(anyhow::anyhow!("save panicked")));
                    match saved {
                        Ok(()) => {
                            tracing::info!(profile = %p_clone.name, "wizard save succeeded");
                            on_saved_final(p_clone.clone());
                            dialog_ref.close();
                            let mode = ai_choice_final.get();
                            if mode != bigame_core::graphics::config::Mode::Off {
                                let process = p_clone.name.clone();
                                let found = gtk4::gio::spawn_blocking(move || {
                                    bigame_core::graphics::target_for_process(&process)
                                })
                                .await
                                .ok()
                                .flatten();
                                match found {
                                    Some(target) => {
                                        crate::views::ai_graphics::open(&parent_w, target, Some(mode));
                                    }
                                    None => crate::widgets::toast::show(
                                        &parent_w,
                                        &i18n("AI Graphics needs the game's install folder, and no installed game runs as this program"),
                                    ),
                                }
                            }
                        }
                        Err(e) => {
                            tracing::warn!(error = %format!("{e:#}"), "wizard profile not saved");
                            crate::widgets::toast::show(
                                &next_btn_ref,
                                &i18n("Could not save: %s").replace("%s", &format!("{e:#}")),
                            );
                            next_btn_ref.set_sensitive(true);
                            next_btn_ref.set_label(&i18n("Save Profile"));
                        }
                    }
                });
                return;
            }

            *current.borrow_mut() = next_step;

            // Populate summary step before showing it
            if next_step == STEPS - 1 {
                populate_summary(&summary_box, &profile.borrow());
                next_btn.remove_css_class("circular");
                next_btn.add_css_class("pill");
                next_btn.set_icon_name("");
                next_btn.set_label(&i18n("Save Profile"));
            }

            stack.set_transition_type(gtk4::StackTransitionType::SlideLeft);
            stack.set_visible_child_name(STEP_IDS[next_step]);
            update_dots(&dots, next_step);
            back_btn.set_visible(true);
        });
    }

    // ══ Navigation: Back ══════════════════════════════════════════════
    {
        let current = current.clone();
        let stack = stack.clone();
        let dots = dots.clone();
        let back_btn = back_btn.clone();
        let next_btn = next_btn.clone();

        back_btn.clone().connect_clicked(move |_| {
            let step = *current.borrow();
            if step == 0 {
                return;
            }
            let prev = step - 1;
            *current.borrow_mut() = prev;

            // Reset "Save" label if going back from last step
            if step == STEPS - 1 {
                next_btn.remove_css_class("pill");
                next_btn.add_css_class("circular");
                next_btn.set_label("");
                next_btn.set_icon_name("go-next-symbolic");
            }

            stack.set_transition_type(gtk4::StackTransitionType::SlideRight);
            stack.set_visible_child_name(STEP_IDS[prev]);
            update_dots(&dots, prev);
            if prev == 0 {
                back_btn.set_visible(false);
            }
        });
    }

    dialog.present(Some(parent));
}

// ── Helper: Update progress dot CSS classes ───────────────────────────────

fn update_dots(dots: &[gtk4::Box], current: usize) {
    for (i, dot) in dots.iter().enumerate() {
        dot.remove_css_class("active");
        dot.remove_css_class("completed");
        if i < current {
            dot.add_css_class("completed");
        } else if i == current {
            dot.add_css_class("active");
        }
    }
}

// ── Helper: Wizard step page layout ──────────────────────────────────────

fn wizard_step(
    title: &str,
    description: &str,
    input: Option<&impl IsA<gtk4::Widget>>,
) -> gtk4::ScrolledWindow {
    let vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
    vbox.set_margin_top(16);
    vbox.set_margin_bottom(16);
    vbox.set_margin_start(24);
    vbox.set_margin_end(24);

    let title_lbl = gtk4::Label::new(Some(title));
    title_lbl.set_halign(gtk4::Align::Center);
    title_lbl.add_css_class("title-2");
    title_lbl.add_css_class("wizard-step-title");
    vbox.append(&title_lbl);

    let desc_lbl = gtk4::Label::new(Some(description));
    desc_lbl.set_halign(gtk4::Align::Center);
    desc_lbl.set_justify(gtk4::Justification::Center);
    desc_lbl.set_wrap(true);
    desc_lbl.set_wrap_mode(gtk4::pango::WrapMode::Word);
    desc_lbl.set_max_width_chars(60);
    desc_lbl.add_css_class("body");
    desc_lbl.add_css_class("dim-label");
    desc_lbl.add_css_class("wizard-step-desc");
    vbox.append(&desc_lbl);

    if let Some(w) = input {
        let clamp = adw::Clamp::builder()
            .maximum_size(440)
            .tightening_threshold(360)
            .child(w)
            .build();
        vbox.append(&clamp);
    }

    let scroll = gtk4::ScrolledWindow::new();
    scroll.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);
    scroll.set_propagate_natural_height(true);
    scroll.set_child(Some(&vbox));
    scroll
}

// ── Helper: Radio choice group ────────────────────────────────────────────

/// Build a radio group as `adw::ActionRow + adw::CheckButton`.
fn build_radio_group(
    choices: &[(&str, &str, &str)],
) -> (adw::PreferencesGroup, gtk4::CheckButton, gtk4::CheckButton) {
    let group = adw::PreferencesGroup::new();
    let mut first_check: Option<gtk4::CheckButton> = None;
    let mut second_check: Option<gtk4::CheckButton> = None;

    for (idx, (_emoji, label, sublabel)) in choices.iter().enumerate() {
        let check = match &first_check {
            None => gtk4::CheckButton::new(),
            Some(first) => gtk4::CheckButton::builder().group(first).build(),
        };
        check.set_valign(gtk4::Align::Center);

        let row = adw::ActionRow::builder()
            .title(label.to_string())
            .subtitle(*sublabel)
            .activatable_widget(&check)
            .build();
        row.add_prefix(&check);

        if idx == 0 {
            check.set_active(true);
            first_check = Some(check.clone());
        }
        if idx == 1 {
            second_check = Some(check.clone());
        }

        group.add(&row);
    }

    let first = first_check.unwrap_or_default();
    let second = second_check.unwrap_or_default();
    (group, first, second)
}

// ── Helper: Populate summary step ────────────────────────────────────────

fn populate_summary(container: &gtk4::Box, p: &GameProfile) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }

    let group = adw::PreferencesGroup::new();
    group.add_css_class("wizard-input-card");

    add_summary_row(&group, "", &i18n("Game"), &p.name);
    let turbo_str = i18n("Turbo");
    let normal_str = i18n("Normal");
    add_summary_row(
        &group,
        "",
        &i18n("Performance Mode"),
        if p.performance_mode {
            &turbo_str
        } else {
            &normal_str
        },
    );
    if !p.scx_sched.is_empty() {
        add_summary_row(&group, "", &i18n("Scheduler"), &p.scx_sched);
    }
    let vcache_label = match p.vcache_mode.as_str() {
        "cache" => i18n("Cache mode"),
        "freq" => i18n("Frequency mode"),
        _ => i18n("Off"),
    };
    add_summary_row(&group, "", &i18n("VCache"), &vcache_label);
    if p.gamescope.is_some() {
        add_summary_row(&group, "", &i18n("Gamescope"), &i18n("Enabled"));
    }
    if p.fg_multiplier > 1 {
        let fg_val = format!(
            "{}x ({})",
            p.fg_multiplier,
            if p.fg_perf_mode {
                i18n("Perf")
            } else {
                i18n("Quality")
            }
        );
        add_summary_row(&group, "", &i18n("Frame Gen"), &fg_val);
    }
    let idle_label = if p.idle_inhibit {
        i18n("Screen stays awake")
    } else {
        i18n("Normal (screen may sleep)")
    };
    add_summary_row(&group, "", &i18n("Screen Sleep"), &idle_label);

    container.append(&group);
}

fn add_summary_row(group: &adw::PreferencesGroup, _emoji: &str, key: &str, value: &str) {
    let row = adw::ActionRow::builder()
        .title(key.to_string())
        .subtitle(value)
        .build();
    group.add(&row);
}
