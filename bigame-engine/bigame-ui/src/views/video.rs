//! Advanced Video Settings view: spatial upscaling and frame generation.
//!
//! Settings are stored globally in `$XDG_CONFIG_HOME/bigame-mode/video.toml`.
//! They represent system-wide defaults on game launch; future work will allow
//! per-profile overrides.
//!
//! Layout:
//! - `AdwExpanderRow` "Spatial Upscaling" (Gamescope filter, Wine FSR, vkBasalt)
//! - `AdwExpanderRow` "Frame Generation" (`OptiScaler`, AFMF, lsfg-vk)

use adw::prelude::*;
use libadwaita as adw;

use bigame_core::models::{FrameGenBackend, FrameGenMode, GamescopeFilter, WineFsrMode};
use bigame_core::video_config;

use crate::i18n::i18n;

/// Build the Advanced Video Settings preferences page.
#[must_use]
#[allow(clippy::too_many_lines)]
pub fn build() -> adw::PreferencesPage {
    let page = adw::PreferencesPage::new();
    let cfg = video_config::load();

    page.add(&build_relogin_notice_group());
    page.add(&build_upscaling_group(&cfg));
    page.add(&build_framegen_group(&cfg));

    page
}

// ── Relogin notice ────────────────────────────────────────────────────────────

fn build_relogin_notice_group() -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::new();
    let row = adw::ActionRow::builder()
        .title(i18n("Restart Steam after changing settings"))
        .subtitle(i18n(
            "Wine FSR, vkBasalt and Frame Generation env vars are written to \
             ~/.config/environment.d/bigame-mode.conf and pushed into the user \
             systemd manager. Already-running launchers (like Steam) keep their \
             old environment — close and reopen Steam so newly launched games \
             inherit the new variables.",
        ))
        .build();
    let icon = gtk4::Image::from_icon_name("dialog-information-symbolic");
    icon.add_css_class("dim-label");
    row.add_prefix(&icon);
    group.add(&row);
    group
}

// ── Spatial Upscaling ────────────────────────────────────────────────────────

#[allow(clippy::too_many_lines)]
fn build_upscaling_group(cfg: &video_config::VideoConfig) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::new();
    group.set_title(&i18n("Spatial Upscaling"));
    group.set_description(Some(&i18n(
        "Apply upscaling filters when launching games. \
         Gamescope: best quality, uses more VRAM.",
    )));

    let expander = adw::ExpanderRow::new();
    expander.set_title(&i18n("Gamescope / Wine FSR / vkBasalt"));
    expander.set_subtitle(&i18n("Spatial upscaling and post-processing pipeline"));

    // ── Gamescope toggle ─────────────────────────────────────────────────────
    let gs_row = adw::SwitchRow::builder()
        .title(i18n("Enable Gamescope Upscaling"))
        .subtitle(i18n(
            "Launches games through Gamescope with the selected filter",
        ))
        .active(cfg.upscaling.gamescope_enabled)
        .build();
    expander.add_row(&gs_row);

    // ── Upscaling filter (FSR / NIS / Integer) ───────────────────────────────
    let filter_items = gtk4::StringList::new(&[
        "FSR 1.0 (FidelityFX)",
        "NIS (Nvidia Image Scaling)",
        &i18n("Integer Scaling"),
    ]);
    let filter_row = adw::ComboRow::new();
    filter_row.set_title(&i18n("Upscaling Filter"));
    filter_row.set_subtitle(&i18n("Gamescope upscaling algorithm"));
    filter_row.set_model(Some(&filter_items));
    filter_row.set_selected(match cfg.upscaling.gamescope_filter {
        GamescopeFilter::Fsr => 0,
        GamescopeFilter::Nis => 1,
        GamescopeFilter::Integer => 2,
    });
    filter_row.set_sensitive(cfg.upscaling.gamescope_enabled);
    expander.add_row(&filter_row);

    // ── FSR sharpness (0-20) ─────────────────────────────────────────────────
    let sharpness_adj = gtk4::Adjustment::new(
        f64::from(cfg.upscaling.gamescope_sharpness.min(20)),
        0.0,
        20.0,
        1.0,
        5.0,
        0.0,
    );
    let sharpness_spin = gtk4::SpinButton::new(Some(&sharpness_adj), 1.0, 0);
    sharpness_spin.set_valign(gtk4::Align::Center);
    let sharpness_row = adw::ActionRow::builder()
        .title(i18n("FSR Sharpness"))
        .subtitle(i18n(
            "0 = maximum sharpness · 20 = softest (--fsr-sharpness)",
        ))
        .build();
    sharpness_row.add_suffix(&sharpness_spin);
    sharpness_row.set_activatable_widget(Some(&sharpness_spin));
    sharpness_row.set_sensitive(
        cfg.upscaling.gamescope_enabled && cfg.upscaling.gamescope_filter == GamescopeFilter::Fsr,
    );
    expander.add_row(&sharpness_row);

    // ── Wine/Proton FSR ──────────────────────────────────────────────────────
    // ── Render resolution (game draws at this res, 0 = game native) ─────────
    let render_width = make_res_spinbutton(cfg.upscaling.base_width, 7680);
    let render_height = make_res_spinbutton(cfg.upscaling.base_height, 4320);
    let base_res_row = make_resolution_row(
        &i18n("Render Resolution (Base)"),
        &i18n("Game render resolution (-w/-h). 0 = use game native."),
        &render_width,
        &render_height,
        cfg.upscaling.gamescope_enabled,
    );
    expander.add_row(&base_res_row);

    // ── Output resolution (upscaled to this, 0 = same as base) ──────────────
    let output_width = make_res_spinbutton(cfg.upscaling.target_width, 7680);
    let output_height = make_res_spinbutton(cfg.upscaling.target_height, 4320);
    let target_res_row = make_resolution_row(
        &i18n("Output Resolution (Target)"),
        &i18n("Display output resolution (-W/-H). 0 = same as render."),
        &output_width,
        &output_height,
        cfg.upscaling.gamescope_enabled,
    );
    expander.add_row(&target_res_row);

    let wine_row = adw::SwitchRow::builder()
        .title(i18n("Wine/Proton Fullscreen FSR"))
        .subtitle(i18n(
            "Adds WINE_FULLSCREEN_FSR=1 to game environment (Wine/Proton)",
        ))
        .active(cfg.upscaling.wine_fsr_enabled)
        .build();
    expander.add_row(&wine_row);

    // ── Wine FSR quality preset ──────────────────────────────────────────────
    let wine_quality_items = gtk4::StringList::new(&[
        &i18n("Performance"),
        &i18n("Balanced"),
        &i18n("Quality"),
        "Ultra",
    ]);
    let wine_quality_row = adw::ComboRow::new();
    wine_quality_row.set_title(&i18n("Wine FSR Quality"));
    wine_quality_row.set_subtitle(&i18n("WINE_FULLSCREEN_FSR_MODE value"));
    wine_quality_row.set_model(Some(&wine_quality_items));
    wine_quality_row.set_selected(match cfg.upscaling.wine_fsr_mode {
        WineFsrMode::Performance => 0,
        WineFsrMode::Balanced => 1,
        WineFsrMode::Quality => 2,
        WineFsrMode::Ultra => 3,
    });
    wine_quality_row.set_sensitive(cfg.upscaling.wine_fsr_enabled);
    expander.add_row(&wine_quality_row);

    // ── vkBasalt post-processing ─────────────────────────────────────────────
    let vkb_row = adw::SwitchRow::builder()
        .title(i18n("vkBasalt Post-Processing"))
        .subtitle(i18n(
            "Adds ENABLE_VKBASALT=1 to game environment (requires vkBasalt)",
        ))
        .active(cfg.upscaling.vkbasalt_enabled)
        .build();
    expander.add_row(&vkb_row);

    let vkb_conf_row = adw::EntryRow::builder()
        .title(i18n("vkBasalt Config Path"))
        .text(cfg.upscaling.vkbasalt_config_path.as_deref().unwrap_or(""))
        .sensitive(cfg.upscaling.vkbasalt_enabled)
        .build();
    expander.add_row(&vkb_conf_row);

    // ── Signal handlers ──────────────────────────────────────────────────────
    // Gamescope toggle — re-sensitizes filter, sharpness
    let filter_row_c = filter_row.clone();
    let sharpness_row_c = sharpness_row.clone();
    gs_row.connect_active_notify(move |row| {
        let enabled = row.is_active();
        filter_row_c.set_sensitive(enabled);
        sharpness_row_c.set_sensitive(
            enabled && filter_row_c.selected() == 0, // only FSR has sharpness
        );
        save_upscaling(|u| u.gamescope_enabled = enabled);
    });

    // Resolution rows follow gamescope toggle sensitivity
    let base_res = base_res_row.clone();
    let target_res = target_res_row.clone();
    gs_row.connect_active_notify(move |row| {
        base_res.set_sensitive(row.is_active());
        target_res.set_sensitive(row.is_active());
    });

    // Base resolution signal handlers
    render_width.connect_value_changed(|spin| {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let v = spin.value() as u32;
        save_upscaling(|u| u.base_width = v);
    });
    render_height.connect_value_changed(|spin| {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let v = spin.value() as u32;
        save_upscaling(|u| u.base_height = v);
    });

    // Target resolution signal handlers
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

    // Filter change — sharpness only relevant for FSR
    let sharpness_row_c2 = sharpness_row.clone();
    let gs_row_c = gs_row.clone();
    filter_row.connect_selected_notify(move |row| {
        let is_fsr = row.selected() == 0;
        sharpness_row_c2.set_sensitive(gs_row_c.is_active() && is_fsr);
        let filter = match row.selected() {
            1 => GamescopeFilter::Nis,
            2 => GamescopeFilter::Integer,
            _ => GamescopeFilter::Fsr,
        };
        save_upscaling(|u| u.gamescope_filter = filter);
    });

    sharpness_spin.connect_value_changed(|spin| {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let v = spin.value() as u8;
        save_upscaling(|u| u.gamescope_sharpness = v);
    });

    // Wine FSR toggle — re-sensitizes quality combo
    let wq = wine_quality_row.clone();
    wine_row.connect_active_notify(move |row| {
        wq.set_sensitive(row.is_active());
        save_upscaling(|u| u.wine_fsr_enabled = row.is_active());
    });

    wine_quality_row.connect_selected_notify(|row| {
        let mode = match row.selected() {
            0 => WineFsrMode::Performance,
            1 => WineFsrMode::Balanced,
            3 => WineFsrMode::Ultra,
            _ => WineFsrMode::Quality,
        };
        save_upscaling(|u| u.wine_fsr_mode = mode);
    });

    // vkBasalt toggle — re-sensitizes config path
    let vk_conf = vkb_conf_row.clone();
    vkb_row.connect_active_notify(move |row| {
        vk_conf.set_sensitive(row.is_active());
        save_upscaling(|u| u.vkbasalt_enabled = row.is_active());
    });

    vkb_conf_row.connect_changed(|row| {
        let text = row.text().to_string();
        save_upscaling(|u| {
            u.vkbasalt_config_path = if text.is_empty() { None } else { Some(text) };
        });
    });

    group.add(&expander);
    group
}

// ── Frame Generation ─────────────────────────────────────────────────────────

#[allow(clippy::too_many_lines)]
fn build_framegen_group(cfg: &video_config::VideoConfig) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::new();
    group.set_title(&i18n("Frame Generation"));
    group.set_description(Some(&i18n(
        "Artificial frame generation for compatible games. \
         OptiScaler: FSR 3 frame generation for DLSS3 titles.",
    )));

    // Conflict banner — shown when OptiScaler is selected and lsfg-vk is configured
    let conflict_banner = adw::Banner::new(&i18n(
        "OptiScaler and lsfg-vk both generate frames — disable one to avoid conflicts.",
    ));
    // Use has_any_active_profile() — more precise than config_path().exists():
    // only warns when an lsfg-vk profile actually has multiplier > 1.
    let lsfg_active = bigame_core::fg::has_any_active_profile();
    let opti_conflict = cfg.frame_gen.optiscaler_enabled
        && cfg.frame_gen.backend == FrameGenBackend::OptiScaler
        && lsfg_active;
    conflict_banner.set_revealed(opti_conflict);
    group.add(&conflict_banner);

    let expander = adw::ExpanderRow::new();
    expander.set_title(&i18n("OptiScaler / AFMF / lsfg-vk"));
    expander.set_subtitle(&i18n("Artificial frame generation backends"));

    // ── Master enable ────────────────────────────────────────────────────────
    let enabled_row = adw::SwitchRow::builder()
        .title(i18n("Enable Frame Generation"))
        .subtitle(i18n("Activates the selected backend on game launch"))
        .active(cfg.frame_gen.enabled)
        .build();
    expander.add_row(&enabled_row);

    // ── Backend selector ─────────────────────────────────────────────────────
    let backend_items = gtk4::StringList::new(&[
        &i18n("None"),
        "OptiScaler (FSR 3 / dlssg-to-fsr3)",
        "AFMF (AMD Fluid Motion Frames)",
        "lsfg-vk (Lossless Scaling)",
    ]);
    let backend_row = adw::ComboRow::new();
    backend_row.set_title(&i18n("Backend"));
    backend_row.set_subtitle(&i18n("Frame generation technology to use"));
    backend_row.set_model(Some(&backend_items));
    backend_row.set_selected(match cfg.frame_gen.backend {
        FrameGenBackend::None => 0,
        FrameGenBackend::OptiScaler => 1,
        FrameGenBackend::Afmf => 2,
        FrameGenBackend::LsfgVk => 3,
    });
    expander.add_row(&backend_row);

    let lsfg_switch_row = adw::SwitchRow::builder()
        .title(i18n("Enable LSFG-VK"))
        .subtitle(i18n("When disabled, lsfg-vk backend is set to None"))
        .active(cfg.frame_gen.enabled && cfg.frame_gen.backend == FrameGenBackend::LsfgVk)
        .build();
    expander.add_row(&lsfg_switch_row);

    // ── Mode (FSR3 / XeSS / DLSS / Native) ──────────────────────────────────
    let mode_items = gtk4::StringList::new(&["FSR 3", "XeSS", "DLSS", &i18n("Native")]);
    let mode_row = adw::ComboRow::new();
    mode_row.set_title(&i18n("Mode"));
    mode_row.set_subtitle(&i18n("Rendering mode passed to OptiScaler"));
    mode_row.set_model(Some(&mode_items));
    mode_row.set_selected(match cfg.frame_gen.mode {
        FrameGenMode::Fsr3 => 0,
        FrameGenMode::Xess => 1,
        FrameGenMode::Dlss => 2,
        FrameGenMode::Native => 3,
    });
    expander.add_row(&mode_row);

    // ── OSD (on-screen status overlay) ──────────────────────────────────────
    let osd_row = adw::SwitchRow::builder()
        .title(i18n("Show On-Screen Status (OSD)"))
        .subtitle(i18n(
            "Displays frame generation status overlay while in-game",
        ))
        .active(cfg.frame_gen.osd_enabled)
        .build();
    expander.add_row(&osd_row);

    // ── OptiScaler DLL staging ───────────────────────────────────────────────
    let opti_row = adw::SwitchRow::builder()
        .title(i18n("Stage OptiScaler DLLs"))
        .subtitle(i18n(
            "Copies dxgi.dll/nvngx.dll from source directory into game prefix on launch",
        ))
        .active(cfg.frame_gen.optiscaler_enabled)
        .build();
    expander.add_row(&opti_row);

    let opti_src_row = adw::EntryRow::builder()
        .title(i18n("OptiScaler Source Directory"))
        .text(cfg.frame_gen.optiscaler_source_dir.as_deref().unwrap_or(""))
        .sensitive(cfg.frame_gen.optiscaler_enabled)
        .build();
    expander.add_row(&opti_src_row);

    // ── AFMF experimental ────────────────────────────────────────────────────
    let afmf_row = adw::SwitchRow::builder()
        .title(i18n("Enable AFMF Experimental Variables"))
        .subtitle(i18n(
            "Advanced: sets RADV_PERFTEST=afmf. Full AFMF requires amdgpu-pro; \
             on RADV (open driver) this is experimental.",
        ))
        .active(cfg.frame_gen.afmf_experimental_enabled)
        .build();
    expander.add_row(&afmf_row);

    let afmf_env_row = adw::EntryRow::builder()
        .title(i18n("AFMF Env Override"))
        .text(
            cfg.frame_gen
                .afmf_env_override
                .as_deref()
                .unwrap_or("RADV_PERFTEST=afmf"),
        )
        .sensitive(cfg.frame_gen.afmf_experimental_enabled)
        .build();
    expander.add_row(&afmf_env_row);

    // ── Signal handlers ──────────────────────────────────────────────────────
    enabled_row.connect_active_notify(|row| {
        save_framegen(|f| f.enabled = row.is_active());
    });

    let banner = conflict_banner.clone();
    let opti_src = opti_src_row.clone();
    let opti_row_for_backend = opti_row.clone();
    let opti_row_for_toast = opti_row.clone();
    let lsfg_switch_for_backend = lsfg_switch_row.clone();
    let enabled_for_backend = enabled_row.clone();
    backend_row.connect_selected_notify(move |row| {
        let backend = match row.selected() {
            1 => FrameGenBackend::OptiScaler,
            2 => FrameGenBackend::Afmf,
            3 => FrameGenBackend::LsfgVk,
            _ => FrameGenBackend::None,
        };

        lsfg_switch_for_backend
            .set_active(backend == FrameGenBackend::LsfgVk && enabled_for_backend.is_active());

        // Mutual exclusion: choosing lsfg-vk disables OptiScaler staging.
        if backend == FrameGenBackend::LsfgVk && opti_row_for_backend.is_active() {
            opti_row_for_backend.set_active(false);
            crate::widgets::toast::show(
                &opti_row_for_toast,
                &i18n("OptiScaler disabled automatically (lsfg-vk selected)"),
            );
        }

        // Mutual exclusion: choosing OptiScaler auto-enables DLL staging.
        if backend == FrameGenBackend::OptiScaler && !opti_row_for_backend.is_active() {
            opti_row_for_backend.set_active(true);
        }

        // Refresh conflict banner
        let conflict = backend == FrameGenBackend::OptiScaler
            && opti_src.is_sensitive()
            && bigame_core::fg::has_any_active_profile();
        banner.set_revealed(conflict);
        save_framegen(|f| f.backend = backend);
    });

    let backend_for_lsfg = backend_row.clone();
    let enabled_for_lsfg = enabled_row.clone();
    let opti_for_lsfg = opti_row.clone();
    lsfg_switch_row.connect_active_notify(move |row| {
        let enabled = row.is_active();
        if enabled {
            enabled_for_lsfg.set_active(true);
            backend_for_lsfg.set_selected(3);
            if opti_for_lsfg.is_active() {
                opti_for_lsfg.set_active(false);
                crate::widgets::toast::show(
                    &opti_for_lsfg,
                    &i18n("OptiScaler disabled automatically (lsfg-vk selected)"),
                );
            }
            save_framegen(|f| {
                f.enabled = true;
                f.backend = FrameGenBackend::LsfgVk;
            });
        } else if backend_for_lsfg.selected() == 3 {
            backend_for_lsfg.set_selected(0);
            save_framegen(|f| {
                f.backend = FrameGenBackend::None;
                f.enabled = false;
            });
        }
    });

    mode_row.connect_selected_notify(|row| {
        let mode = match row.selected() {
            1 => FrameGenMode::Xess,
            2 => FrameGenMode::Dlss,
            3 => FrameGenMode::Native,
            _ => FrameGenMode::Fsr3,
        };
        save_framegen(|f| f.mode = mode);
    });

    osd_row.connect_active_notify(|row| {
        save_framegen(|f| f.osd_enabled = row.is_active());
    });

    // OptiScaler toggle — re-sensitizes source path + refreshes banner
    let src = opti_src_row.clone();
    let banner2 = conflict_banner.clone();
    let backend_row_for_opti = backend_row.clone();
    let opti_row_for_toast2 = opti_row.clone();
    opti_row.connect_active_notify(move |row| {
        let enabled = row.is_active();

        // Mutual exclusion: enabling OptiScaler while lsfg-vk is selected
        // automatically switches backend to OptiScaler.
        if enabled && backend_row_for_opti.selected() == 3 {
            backend_row_for_opti.set_selected(1);
            crate::widgets::toast::show(
                &opti_row_for_toast2,
                &i18n("Backend switched to OptiScaler (lsfg-vk was active)"),
            );
        }

        src.set_sensitive(enabled);
        let conflict = enabled && bigame_core::fg::has_any_active_profile();
        banner2.set_revealed(conflict);
        save_framegen(|f| f.optiscaler_enabled = enabled);
    });

    opti_src_row.connect_changed(|row| {
        let text = row.text().to_string();
        save_framegen(|f| {
            f.optiscaler_source_dir = if text.is_empty() { None } else { Some(text) };
        });
    });

    // AFMF toggle — re-sensitizes env override field
    let env_row = afmf_env_row.clone();
    afmf_row.connect_active_notify(move |row| {
        env_row.set_sensitive(row.is_active());
        save_framegen(|f| f.afmf_experimental_enabled = row.is_active());
    });

    afmf_env_row.connect_changed(|row| {
        let text = row.text().to_string();
        save_framegen(|f| {
            f.afmf_env_override = if text.is_empty() { None } else { Some(text) };
        });
    });

    group.add(&expander);
    group
}

// ── Persistence helpers ───────────────────────────────────────────────────────

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

// ── Resolution input helpers ──────────────────────────────────────────────────

/// Build a `SpinButton` clamped to [0, `max_val`] for resolution inputs.
/// Value 0 = "use game native / auto".
fn make_res_spinbutton(current: u32, max_val: u32) -> gtk4::SpinButton {
    let adj = gtk4::Adjustment::new(f64::from(current), 0.0, f64::from(max_val), 1.0, 10.0, 0.0);
    let spin = gtk4::SpinButton::new(Some(&adj), 1.0, 0);
    spin.set_valign(gtk4::Align::Center);
    spin.set_width_chars(6);
    spin
}

/// Wrap two `SpinButton` widgets (width × height) in an `AdwActionRow`.
fn make_resolution_row(
    title: &str,
    subtitle: &str,
    w_spin: &gtk4::SpinButton,
    h_spin: &gtk4::SpinButton,
    sensitive: bool,
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
        .sensitive(sensitive)
        .build();
    row.add_suffix(w_spin);
    row.add_suffix(&separator);
    row.add_suffix(h_spin);
    row
}
