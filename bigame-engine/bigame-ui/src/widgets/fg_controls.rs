//! Frame Generation (LSFG-VK) control widgets for real-time Tuning.
//!
//! Writes directly to `lsfg-vk` TOML.

use adw::prelude::*;
use gtk4::gio;
use libadwaita as adw;
use std::cell::Cell;
use std::rc::Rc;

use crate::i18n::i18n;

/// Building a widget tree is inherently linear — splitting it yields helpers
/// with a single caller and no independent meaning — so the length lint is
/// allowed here rather than worked around.
#[allow(clippy::too_many_lines)]
/// Build a group of controls for Frame Generation settings in Tuning.
pub fn build_tuning_fg_group(active_game: &str) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::new();
    group.set_title(&i18n("Frame Generation (LSFG-VK)"));
    group.set_description(Some(&i18n(
        "Select a profile to tune Frame Generation settings in real-time.",
    )));

    // ── DLL Path (global) ─────────────────────────────────────
    let init_dll = bigame_core::fg::read_global_dll().unwrap_or_default();
    let dll_row = adw::EntryRow::builder()
        .title(i18n("Path to Lossless.dll (Global)"))
        .text(&init_dll)
        .build();

    let file_btn = gtk4::Button::builder()
        .icon_name("folder-open-symbolic")
        .valign(gtk4::Align::Center)
        .css_classes(["flat"])
        .build();
    dll_row.add_suffix(&file_btn);

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
    dll_row.add_suffix(&info_btn);

    let row_clone = dll_row.clone();
    file_btn.connect_clicked(move |btn| {
        let dialog = gtk4::FileDialog::builder()
            .title(i18n("Select Lossless.dll"))
            .modal(true)
            .build();
        let dll_filter = gtk4::FileFilter::new();
        dll_filter.set_name(Some(&format!("{} (*.dll)", i18n("DLL files"))));
        dll_filter.add_pattern("*.dll");
        let filters = gio::ListStore::new::<gtk4::FileFilter>();
        filters.append(&dll_filter);
        dialog.set_filters(Some(&filters));

        let r = row_clone.clone();
        if let Some(win) = btn.root().and_downcast::<gtk4::Window>() {
            dialog.open(Some(&win), gio::Cancellable::NONE, move |res| {
                if let Ok(file) = res {
                    if let Some(path) = file.path() {
                        r.set_text(&path.to_string_lossy());
                        let _ = bigame_core::fg::write_global_dll(Some(
                            path.to_string_lossy().to_string(),
                        ));
                    }
                }
            });
        }
    });

    dll_row.connect_changed(|r| {
        let text = r.text().to_string();
        let dll = if text.is_empty() { None } else { Some(text) };
        let _ = bigame_core::fg::write_global_dll(dll);
    });

    group.add(&dll_row);

    // ── Target Profile Combo ──────────────────────────────────────────────────
    let profiles = bigame_core::profiles::list_names();
    // With no profile yet the list shows a placeholder, which must never be
    // read back as a profile name.
    let has_profiles = !profiles.is_empty();
    let placeholder = i18n("No profiles yet");
    let mut model_strings: Vec<&str> = profiles.iter().map(std::string::String::as_str).collect();
    if !has_profiles {
        model_strings.push(&placeholder);
    }

    let target_model = gtk4::StringList::new(&model_strings);
    let target_row = adw::ComboRow::builder()
        .title(i18n("Target Profile"))
        .model(&target_model)
        .build();

    // Auto-select active_game if it exists
    let mut selected_idx = 0;
    if !active_game.is_empty() {
        if let Some(idx) = model_strings.iter().position(|&s| s == active_game) {
            selected_idx = u32::try_from(idx).unwrap_or(0);
        }
    }
    target_row.set_selected(selected_idx);
    group.add(&target_row);

    let is_sensitive = !profiles.is_empty();
    let initial_target = if profiles.is_empty() {
        ""
    } else {
        &profiles[selected_idx as usize]
    };

    let (init_mult, init_flow, init_perf, init_hdr, init_present) = if initial_target.is_empty() {
        (2, 100, false, false, 0)
    } else {
        bigame_core::fg::read_profile(initial_target)
    };

    let init_enabled = init_mult > 1;

    // ── Explicit ON/OFF switch for LSFG generation ─────────────────────────
    let enabled_row = adw::SwitchRow::builder()
        .title(i18n("Enable LSFG Frame Generation"))
        .subtitle(i18n("When off, lsfg-vk has no entry for this game"))
        .active(init_enabled)
        .sensitive(is_sensitive)
        .build();
    group.add(&enabled_row);

    // ── Multiplier Slider (1–20x) ─────────────────────────────────────────
    let multiplier_row = adw::ActionRow::builder()
        .title(i18n("Multiplier"))
        .subtitle(i18n("Frames generated per real frame (1-20x)"))
        .sensitive(is_sensitive && init_enabled)
        .build();

    let adj = gtk4::Adjustment::new(f64::from(init_mult), 1.0, 20.0, 1.0, 5.0, 0.0);
    let scale = gtk4::Scale::builder()
        .adjustment(&adj)
        .digits(0)
        .draw_value(true)
        .value_pos(gtk4::PositionType::Right)
        .hexpand(true)
        .valign(gtk4::Align::Center)
        .width_request(200)
        .build();

    scale.add_mark(1.0, gtk4::PositionType::Bottom, Some("1x"));
    scale.add_mark(10.0, gtk4::PositionType::Bottom, Some("10x"));
    scale.add_mark(20.0, gtk4::PositionType::Bottom, Some("20x"));
    multiplier_row.add_suffix(&scale);
    group.add(&multiplier_row);

    // ── Flow Scale Slider (25–100%) ───────────────────────────────────────
    let flow_row = adw::ActionRow::builder()
        .title(i18n("Flow Scale"))
        .subtitle(i18n(
            "Motion estimation resolution (25–100%). Lower = faster.",
        ))
        .sensitive(is_sensitive && init_enabled)
        .build();

    let flow_adj = gtk4::Adjustment::new(f64::from(init_flow), 25.0, 100.0, 1.0, 10.0, 0.0);
    let flow_scale = gtk4::Scale::builder()
        .adjustment(&flow_adj)
        .digits(0)
        .draw_value(true)
        .value_pos(gtk4::PositionType::Right)
        .hexpand(true)
        .valign(gtk4::Align::Center)
        .width_request(200)
        .build();

    flow_scale.add_mark(25.0, gtk4::PositionType::Bottom, Some("25%"));
    flow_scale.add_mark(50.0, gtk4::PositionType::Bottom, Some("50%"));
    flow_scale.add_mark(100.0, gtk4::PositionType::Bottom, Some("100%"));
    flow_row.add_suffix(&flow_scale);
    group.add(&flow_row);

    // ── Performance Mode ─────────────────────────────────────────────────
    let perf_row = adw::SwitchRow::builder()
        .title(i18n("Performance Mode"))
        .active(init_perf)
        .sensitive(is_sensitive && init_enabled)
        .build();
    group.add(&perf_row);

    // ── HDR Mode ─────────────────────────────────────────────────────────
    let hdr_row = adw::SwitchRow::builder()
        .title(i18n("HDR Mode"))
        .active(init_hdr)
        .sensitive(is_sensitive && init_enabled)
        .build();
    group.add(&hdr_row);

    // ── Present Mode ─────────────────────────────────────────────────────
    let present_model = gtk4::StringList::new(&[
        &i18n("VSync/FIFO (default)"),
        &i18n("Recommended"),
        &i18n("Mailbox"),
        &i18n("Immediate"),
    ]);
    let present_row = adw::ComboRow::builder()
        .title(i18n("Present Mode"))
        .model(&present_model)
        .sensitive(is_sensitive && init_enabled)
        .build();
    present_row.set_selected(init_present);
    group.add(&present_row);

    // ── Handlers ─────────────────────────────────────────────────────────
    if is_sensitive {
        let is_updating = Rc::new(Cell::new(false));

        let tm_clone = target_model.clone();
        let enabled_upd = enabled_row.clone();
        let multiplier_row_upd = multiplier_row.clone();
        let flow_row_upd = flow_row.clone();
        let scale_upd = scale.clone();
        let flow_upd = flow_scale.clone();
        let perf_upd = perf_row.clone();
        let hdr_upd = hdr_row.clone();
        let pres_upd = present_row.clone();
        let is_upd = is_updating.clone();

        target_row.connect_selected_notify(move |r| {
            if !has_profiles {
                return;
            }
            if let Some(target) = tm_clone.string(r.selected()) {
                let (m, f, p, h, pm) = bigame_core::fg::read_profile(&target);
                let enabled = m > 1;
                is_upd.set(true);
                enabled_upd.set_active(enabled);
                multiplier_row_upd.set_sensitive(enabled);
                flow_row_upd.set_sensitive(enabled);
                scale_upd.set_value(f64::from(m));
                flow_upd.set_value(f64::from(f));
                perf_upd.set_active(p);
                hdr_upd.set_active(h);
                pres_upd.set_selected(pm);
                perf_upd.set_sensitive(enabled);
                hdr_upd.set_sensitive(enabled);
                pres_upd.set_sensitive(enabled);
                is_upd.set(false);
            }
        });

        let tm_save = target_model.clone();
        let t_row = target_row.clone();
        let enabled_ref = enabled_row.clone();
        let multiplier_row_ref = multiplier_row.clone();
        let flow_row_ref = flow_row.clone();
        let scale_ref = scale.clone();
        let flow_ref = flow_scale.clone();
        let perf_ref = perf_row.clone();
        let hdr_ref = hdr_row.clone();
        let pres_ref = present_row.clone();
        let target_row_for_toast = target_row.clone();
        let is_upd_save = is_updating.clone();

        let save_fn = Rc::new({
            move || {
                if is_upd_save.get() || !has_profiles {
                    return;
                }
                if let Some(target) = tm_save.string(t_row.selected()) {
                    let name_str = target.to_string();
                    // SpinRow values are clamped to their adjustment range,
                    // which is small and positive for both of these.
                    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                    let mut mult = scale_ref.value() as u32;
                    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                    let flow = flow_ref.value() as u32;
                    let perf = perf_ref.is_active();
                    let hdr = hdr_ref.is_active();
                    let pres = pres_ref.selected();

                    if !enabled_ref.is_active() {
                        mult = 1;
                    } else if mult < 2 {
                        mult = 2;
                        is_upd_save.set(true);
                        scale_ref.set_value(2.0);
                        is_upd_save.set(false);
                    }

                    let controls_enabled = mult > 1;
                    multiplier_row_ref.set_sensitive(controls_enabled);
                    flow_row_ref.set_sensitive(controls_enabled);
                    perf_ref.set_sensitive(controls_enabled);
                    hdr_ref.set_sensitive(controls_enabled);
                    pres_ref.set_sensitive(controls_enabled);

                    if mult > 1 && !bigame_core::fg::is_lossless_dll_ready() {
                        crate::widgets::toast::show(
                            &target_row_for_toast,
                            &i18n("LSFG-VK requires a valid Lossless.dll path"),
                        );
                        // Revert to disabled state (1x) when dependency is missing.
                        is_upd_save.set(true);
                        scale_ref.set_value(1.0);
                        is_upd_save.set(false);
                        let _ = bigame_core::fg::write_profile(&name_str, 1, flow, perf, hdr, pres);
                        return;
                    }

                    // A multiplier above 1 here means lsfg-vk is wanted:
                    // make it the frame-generation backend, so the Video
                    // page and the launch agree with Tuning.
                    if mult > 1 {
                        let mut vcfg = bigame_core::video_config::load();
                        if vcfg.frame_gen.backend != bigame_core::models::FrameGenBackend::LsfgVk {
                            vcfg.frame_gen.backend = bigame_core::models::FrameGenBackend::LsfgVk;
                            if let Err(e) = bigame_core::video_config::save(&vcfg) {
                                tracing::warn!("failed to select lsfg-vk as the backend: {e:#}");
                            }
                        }
                    }

                    // Write to lsfg-vk's TOML, which it hot-reloads.
                    if let Err(e) =
                        bigame_core::fg::write_profile(&name_str, mult, flow, perf, hdr, pres)
                    {
                        tracing::warn!("failed to write lsfg profile: {e:#}");
                        crate::widgets::toast::show(
                            &target_row_for_toast,
                            &format!("{}: {}", i18n("LSFG write failed"), e),
                        );
                    }

                    // Nothing is written to the falcond profile here. These
                    // values are lsfg-vk's, written to its own configuration
                    // above, which is also where the sliders read them back
                    // from. falcond does not read them, and a profile save
                    // goes through the privileged helper and reloads falcond,
                    // dropping and re-applying the running game's profile.
                }
            }
        });

        let s1 = save_fn.clone();
        scale.connect_value_changed(move |_| s1());

        let s0 = save_fn.clone();
        enabled_row.connect_active_notify(move |_| s0());

        let s2 = save_fn.clone();
        flow_scale.connect_value_changed(move |_| s2());

        let s3 = save_fn.clone();
        perf_row.connect_active_notify(move |_| s3());

        let s4 = save_fn.clone();
        hdr_row.connect_active_notify(move |_| s4());

        let s5 = save_fn.clone();
        present_row.connect_selected_notify(move |_| s5());
    }

    group
}
