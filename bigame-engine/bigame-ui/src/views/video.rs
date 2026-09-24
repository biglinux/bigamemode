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

use bigame_core::models::{FrameGenBackend, GamescopeFilter, WineFsrMode};
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
        &i18n("Ultra"),
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

/// Frame generation for every game: lsfg-vk, when it is installed.
///
/// Upscaling and frame generation through `OptiScaler` are per game, in each
/// game's AI Graphics (Profiles), where they are planned, installed with a
/// backup and verified. The controls this group used to have — an
/// `OptiScaler` backend that copied DLLs over the game's own, an "AFMF"
/// backend setting a `RADV_PERFTEST` option RADV does not have, a mode and an
/// on-screen indicator nothing read — did nothing, or harm, and are gone.
fn build_framegen_group(cfg: &video_config::VideoConfig) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::new();
    group.set_title(&i18n("Frame Generation"));
    group.set_description(Some(&i18n(
        "lsfg-vk generates extra frames for every game. Upscaling and frame generation \
         for one game are in that game's AI Graphics, in Profiles.",
    )));

    if !bigame_core::fg::layer_installed() {
        let row = adw::ActionRow::builder()
            .title(i18n("lsfg-vk"))
            .subtitle(i18n("Not installed"))
            .use_markup(false)
            .build();
        group.add(&row);
        return group;
    }

    let lsfg_row = adw::SwitchRow::builder()
        .title(i18n("lsfg-vk"))
        .subtitle(i18n(
            "Raises the presented frame rate, not the rendered one, and adds latency",
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
    group.add(&lsfg_row);
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
