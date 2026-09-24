//! Dashboard view: real-time telemetry + booster toggle.

use std::time::Duration;

use adw::prelude::*;
use gtk4::{gio, glib};
use libadwaita as adw;

use crate::i18n::i18n;

/// Telemetry polling interval.
const POLL_INTERVAL: Duration = Duration::from_secs(1);

/// Build the Dashboard view with live telemetry polling.
///
/// Spawns a `glib` future that reads sysfs every second
/// and updates the telemetry row subtitles via `gio::spawn_blocking`.
#[must_use]
#[allow(clippy::too_many_lines)]
pub fn build() -> adw::PreferencesPage {
    let page = adw::PreferencesPage::new();

    // Real-time Dashboard Grid
    let metrics_group = adw::PreferencesGroup::new();
    metrics_group.set_title(&i18n("Real-time Telemetry"));

    let row1 = gtk4::Box::new(gtk4::Orientation::Horizontal, 12);
    row1.set_homogeneous(true);
    let row2 = gtk4::Box::new(gtk4::Orientation::Horizontal, 12);
    row2.set_homogeneous(true);

    let (cpu_card, cpu_val, cpu_spark) = make_dashboard_card(&i18n("CPU Freq"), "cpu-symbolic");
    let (gpu_card, gpu_val, gpu_spark) =
        make_dashboard_card(&i18n("GPU Freq"), "video-display-symbolic");
    let (temp_card, temp_val, temp_spark) =
        make_dashboard_card(&i18n("GPU Temp"), "freon-gpu-temperature-symbolic");
    row1.append(&cpu_card);
    row1.append(&gpu_card);
    row1.append(&temp_card);

    let (ram_card, ram_val, ram_spark) = make_dashboard_card(&i18n("RAM Usage"), "memory-symbolic");
    let (disk_card, disk_val, disk_spark) =
        make_dashboard_card(&i18n("Disk I/O"), "drive-harddisk-symbolic");
    let (ping_card, ping_val, ping_spark) =
        make_dashboard_card(&i18n("Latency"), "network-wireless-symbolic");
    row2.append(&ram_card);
    row2.append(&disk_card);
    row2.append(&ping_card);

    let metrics_vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
    metrics_vbox.append(&row1);
    metrics_vbox.append(&row2);
    metrics_group.add(&metrics_vbox);
    page.add(&metrics_group);

    // Performance status. Booster Mode itself lives on Home and is deliberately
    // not duplicated here: two controls writing the same state is the class of
    // conflict this project is trying to remove, not reproduce.
    let booster_group = adw::PreferencesGroup::new();
    booster_group.set_title(&i18n("Performance"));

    // Power profile indicator
    let power_row = adw::ActionRow::builder()
        .title(i18n("Power Profile"))
        .subtitle(i18n("Checking…"))
        .build();
    booster_group.add(&power_row);

    let (turbo_row, turbo_badge) = make_runtime_status_row(
        &i18n("Turbo Mode"),
        &i18n("Required to apply advanced video optimizations"),
        &i18n("Checking…"),
        "dim-label",
    );
    booster_group.add(&turbo_row);

    // lsfg-vk implicit layer status (static check — installation-level)
    // Covers both package naming variants and all standard Vulkan layer dirs.
    let lsfg_installed = lsfg_is_installed();
    let lsfg_row = adw::ActionRow::builder()
        .title(i18n("Frame Generation (lsfg-vk)"))
        .subtitle(i18n(
            "Vulkan implicit layer · No Steam launch options needed",
        ))
        .build();
    let lsfg_badge = gtk4::Label::builder()
        .label(if lsfg_installed {
            i18n("Ready")
        } else {
            i18n("Not installed")
        })
        .css_classes(if lsfg_installed {
            ["success-badge"]
        } else {
            ["warning-badge"]
        })
        .valign(gtk4::Align::Center)
        .build();
    lsfg_row.add_suffix(&lsfg_badge);
    booster_group.add(&lsfg_row);
    page.add(&booster_group);

    // Video runtime status section (clear visual proof while game is running)
    let runtime_group = adw::PreferencesGroup::new();
    runtime_group.set_title(&i18n("Video Runtime Status"));

    let (gamescope_rt_row, gamescope_rt_badge) = make_runtime_status_row(
        &i18n("Gamescope Upscaling"),
        &i18n("Runtime detection for Gamescope wrapper"),
        &i18n("Waiting"),
        "dim-label",
    );
    runtime_group.add(&gamescope_rt_row);

    let (wine_fsr_rt_row, wine_fsr_rt_badge) = make_runtime_status_row(
        &i18n("Wine Fullscreen FSR"),
        &i18n("Checks WINE_FULLSCREEN_FSR in game process environment"),
        &i18n("Waiting"),
        "dim-label",
    );
    runtime_group.add(&wine_fsr_rt_row);

    let (vkbasalt_rt_row, vkbasalt_rt_badge) = make_runtime_status_row(
        &i18n("vkBasalt Injection"),
        &i18n("Checks ENABLE_VKBASALT in game process environment"),
        &i18n("Waiting"),
        "dim-label",
    );
    runtime_group.add(&vkbasalt_rt_row);

    let (framegen_rt_row, framegen_rt_badge) = make_runtime_status_row(
        &i18n("Frame Generation Backend"),
        &i18n("Shows selected backend status during active game"),
        &i18n("Waiting"),
        "dim-label",
    );
    runtime_group.add(&framegen_rt_row);

    let diagnostics_row = adw::ActionRow::builder()
        .title(i18n("Runtime Diagnostics"))
        .subtitle(i18n("Collects current checks for Turbo/game/env detection"))
        .build();
    let diagnostics_btn = gtk4::Button::builder()
        .label(i18n("Run Diagnostics"))
        .valign(gtk4::Align::Center)
        .build();
    let save_diag_btn = gtk4::Button::builder()
        .icon_name("document-save-symbolic")
        .tooltip_text(i18n("Save diagnostics log"))
        .valign(gtk4::Align::Center)
        .css_classes(["flat"])
        .build();
    let copy_diag_btn = gtk4::Button::builder()
        .icon_name("edit-copy-symbolic")
        .tooltip_text(i18n("Copy diagnostics report"))
        .valign(gtk4::Align::Center)
        .css_classes(["flat"])
        .build();
    diagnostics_btn.connect_clicked(|btn| {
        let btn_ref = btn.clone();
        gtk4::glib::spawn_future_local(async move {
            let report = gio::spawn_blocking(build_runtime_diagnostics_report)
                .await
                .unwrap_or_else(|_| i18n("Diagnostics failed"));

            if let Some(root) = btn_ref.root() {
                if let Ok(win) = root.downcast::<adw::ApplicationWindow>() {
                    let dlg = adw::AlertDialog::builder()
                        .heading(i18n("Runtime Diagnostics Report"))
                        .body(&report)
                        .build();
                    dlg.add_response("ok", &i18n("Close"));
                    dlg.set_default_response(Some("ok"));
                    dlg.present(Some(&win));
                    return;
                }
            }
            tracing::warn!("unable to present diagnostics dialog (no root window)");
        });
    });
    save_diag_btn.connect_clicked(|btn| {
        let btn_ref = btn.clone();
        gtk4::glib::spawn_future_local(async move {
            let result = gio::spawn_blocking(|| {
                let report = build_runtime_diagnostics_report();
                let path = std::env::var("HOME")
                    .map_or_else(
                        |_| std::path::PathBuf::from("/tmp"),
                        std::path::PathBuf::from,
                    )
                    .join("bigame-diagnostics.log");
                std::fs::write(&path, report)
                    .map(|()| path)
                    .map_err(|e| format!("{}: {}", i18n("Failed to save diagnostics"), e))
            })
            .await;

            match result {
                Ok(Ok(path)) => crate::widgets::toast::show(
                    &btn_ref,
                    &format!("{}: {}", i18n("Diagnostics saved"), path.display()),
                ),
                Ok(Err(err)) => crate::widgets::toast::show(&btn_ref, &err),
                Err(_) => {
                    crate::widgets::toast::show(&btn_ref, &i18n("Failed to save diagnostics"));
                }
            }
        });
    });
    copy_diag_btn.connect_clicked(|btn| {
        let btn_ref = btn.clone();
        gtk4::glib::spawn_future_local(async move {
            let report = gio::spawn_blocking(build_runtime_diagnostics_report)
                .await
                .unwrap_or_else(|_| i18n("Diagnostics failed"));

            if let Some(display) = gtk4::gdk::Display::default() {
                display.clipboard().set_text(&report);
                crate::widgets::toast::show(&btn_ref, &i18n("Diagnostics copied"));
            } else {
                crate::widgets::toast::show(&btn_ref, &i18n("Failed to copy diagnostics"));
            }
        });
    });
    diagnostics_row.add_suffix(&diagnostics_btn);
    diagnostics_row.add_suffix(&save_diag_btn);
    diagnostics_row.add_suffix(&copy_diag_btn);
    runtime_group.add(&diagnostics_row);

    page.add(&runtime_group);

    // Falcond status section
    let falcond_group = adw::PreferencesGroup::new();
    falcond_group.set_title(&i18n("Falcond Daemon"));

    let active_profile_row = adw::ActionRow::builder()
        .title(i18n("Active Profile"))
        .subtitle("—")
        .build();
    // Status badge: "Running" (green) / "Stopped" (red)
    let falcond_badge = gtk4::Label::builder()
        .label(i18n("Stopped"))
        .css_classes(["error-badge"])
        .valign(gtk4::Align::Center)
        .build();
    active_profile_row.add_suffix(&falcond_badge);
    falcond_group.add(&active_profile_row);

    let scx_row = adw::ActionRow::builder()
        .title(i18n("SCX Scheduler"))
        .subtitle("—")
        .build();
    falcond_group.add(&scx_row);

    let vcache_row = adw::ActionRow::builder()
        .title(i18n("VCache Mode"))
        .subtitle("—")
        .build();
    falcond_group.add(&vcache_row);
    page.add(&falcond_group);

    // Troubleshooting: solvable issues + hardware hints
    let troubleshoot_group = build_troubleshooting_group();
    page.add(&troubleshoot_group);

    // Detected games section — refresh button re-detects on demand
    let games_group = build_games_group();
    page.add(&games_group);

    // Live telemetry + daemon status polling (1Hz, main thread context)
    spawn_telemetry_poller(
        cpu_val,
        gpu_val,
        temp_val,
        disk_val,
        ping_val,
        ram_val,
        power_row,
        turbo_row,
        turbo_badge,
        active_profile_row.clone(),
        scx_row.clone(),
        vcache_row.clone(),
        falcond_badge.clone(),
        lsfg_badge.clone(),
        gamescope_rt_row,
        gamescope_rt_badge,
        wine_fsr_rt_row,
        wine_fsr_rt_badge,
        vkbasalt_rt_row,
        vkbasalt_rt_badge,
        framegen_rt_row,
        framegen_rt_badge,
        cpu_spark,
        gpu_spark,
        temp_spark,
        disk_spark,
        ping_spark,
        ram_spark,
    );

    // Immediate falcond status refresh via file-change events
    spawn_status_watcher(active_profile_row, scx_row, vcache_row, falcond_badge);

    page
}

/// Watch `/tmp/falcond_status` for file changes and update daemon rows immediately.
///
/// Uses `gio::FileMonitor` — triggers on every falcond status write with no
/// extra polling overhead. Falls back gracefully if the file doesn't exist yet.
fn spawn_status_watcher(
    active_profile_row: adw::ActionRow,
    scx_row: adw::ActionRow,
    vcache_row: adw::ActionRow,
    badge: gtk4::Label,
) {
    let file = gio::File::for_path(bigame_core::status::status_path());
    let Ok(monitor) = file.monitor_file(gio::FileMonitorFlags::NONE, gio::Cancellable::NONE) else {
        return; // inotify not available — polling still covers this
    };

    monitor.connect_changed(move |_mon, _file, _other, event| {
        // Only react to actual writes, not attribute/access events
        if !matches!(
            event,
            gio::FileMonitorEvent::Changed | gio::FileMonitorEvent::Created
        ) {
            return;
        }
        let status = bigame_core::status::read();
        apply_falcond_status(
            status.as_ref(),
            &active_profile_row,
            &scx_row,
            &vcache_row,
            &badge,
        );
    });

    // Keep the monitor alive for the lifetime of the dashboard widget.
    // Leak is intentional — the dashboard lives as long as the app.
    std::mem::forget(monitor);
}

/// Apply a parsed `FalcondStatus` (or None) to the three daemon rows + badge.
fn apply_falcond_status(
    status: Option<&bigame_core::status::FalcondStatus>,
    active_profile_row: &adw::ActionRow,
    scx_row: &adw::ActionRow,
    vcache_row: &adw::ActionRow,
    badge: &gtk4::Label,
) {
    let has_schedulers = bigame_core::sched::detect_installed().len() > 1;
    let vcache_available = bigame_core::vcache::is_available();

    if let Some(st) = status {
        badge.set_text(&i18n("Running"));
        badge.remove_css_class("error-badge");
        badge.add_css_class("success-badge");

        // Active profile: name when running, idle hint otherwise
        match st.active_profile.as_deref() {
            Some(p) if !p.is_empty() && p != "None" => active_profile_row.set_subtitle(p),
            _ => active_profile_row.set_subtitle(&i18n("Idle — no game running")),
        }

        // SCX: distinguish not-installed / disabled / active
        let scx = if st.current_scx.is_empty() {
            &st.config_scx
        } else {
            &st.current_scx
        };
        if !has_schedulers {
            scx_row.set_subtitle(&i18n("Not installed — sudo pacman -S scx-scheds"));
        } else if scx.is_empty() || scx == "none" {
            scx_row.set_subtitle(&i18n("Disabled (none selected in Tuning)"));
        } else {
            scx_row.set_subtitle(scx);
        }

        // VCache: distinguish hardware unavailable / disabled / active
        let vc = if st.current_vcache.is_empty() {
            &st.config_vcache
        } else {
            &st.current_vcache
        };
        if !vcache_available {
            let cpu = read_cpu_model_sync();
            vcache_row.set_subtitle(&format!(
                "{} — {cpu}",
                i18n("Not available (requires X3D CPU)")
            ));
        } else if vc.is_empty() || vc == "none" {
            vcache_row.set_subtitle(&i18n("Disabled"));
        } else {
            vcache_row.set_subtitle(vc);
        }
    } else {
        badge.set_text(&i18n("Stopped"));
        badge.remove_css_class("success-badge");
        badge.add_css_class("error-badge");
        active_profile_row.set_subtitle(&i18n("Daemon not running"));
        scx_row.set_subtitle("—");
        vcache_row.set_subtitle("—");
    }
}

/// Read CPU model name from `/proc/cpuinfo` (synchronous, very fast — no allocation loop).
fn read_cpu_model_sync() -> String {
    std::fs::read_to_string("/proc/cpuinfo")
        .ok()
        .and_then(|c| {
            c.lines()
                .find(|l| l.starts_with("model name"))
                .and_then(|l| l.split(':').nth(1))
                .map(|s| s.trim().to_owned())
        })
        .unwrap_or_else(|| "Unknown CPU".to_owned())
}

/// Spawn async poller reading sysfs / D-Bus / status files and updating rows.
///
/// Uses `gio::spawn_blocking` for I/O, updates widgets on main thread.
#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
fn spawn_telemetry_poller(
    cpu_val: gtk4::Label,
    gpu_val: gtk4::Label,
    temp_val: gtk4::Label,
    disk_val: gtk4::Label,
    ping_val: gtk4::Label,
    ram_val: gtk4::Label,
    power_row: adw::ActionRow,
    turbo_row: adw::ActionRow,
    turbo_badge: gtk4::Label,
    active_profile_row: adw::ActionRow,
    scx_row: adw::ActionRow,
    vcache_row: adw::ActionRow,
    falcond_badge: gtk4::Label,
    lsfg_badge: gtk4::Label,
    gamescope_rt_row: adw::ActionRow,
    gamescope_rt_badge: gtk4::Label,
    wine_fsr_rt_row: adw::ActionRow,
    wine_fsr_rt_badge: gtk4::Label,
    vkbasalt_rt_row: adw::ActionRow,
    vkbasalt_rt_badge: gtk4::Label,
    framegen_rt_row: adw::ActionRow,
    framegen_rt_badge: gtk4::Label,
    cpu_spark: crate::widgets::sparkline::SparkHandle,
    gpu_spark: crate::widgets::sparkline::SparkHandle,
    temp_spark: crate::widgets::sparkline::SparkHandle,
    disk_spark: crate::widgets::sparkline::SparkHandle,
    ping_spark: crate::widgets::sparkline::SparkHandle,
    ram_spark: crate::widgets::sparkline::SparkHandle,
) {
    glib::spawn_future_local(async move {
        let mut prev_disk: Option<(u64, u64)> = None;
        let mut prev_is_lsfg = false;
        let mut prev_runtime: Option<(bool, bool, bool, bool, bool)> = None;
        loop {
            // Nothing is read while this page is not on screen -- including
            // the one-per-second `ping`, which used to run for as long as
            // the application did, game or no game. Game launch and exit
            // notifications moved to the application-wide game watcher.
            if !ping_val.is_mapped() {
                glib::timeout_future(POLL_INTERVAL).await;
                continue;
            }
            // CPU
            let cpu_text = gio::spawn_blocking(read_cpu_freq)
                .await
                .unwrap_or_else(|_| "N/A".into());
            if let Some(mhz) = cpu_text
                .split_whitespace()
                .next()
                .and_then(|s| s.parse::<f64>().ok())
            {
                cpu_spark.push(mhz);
                cpu_val.set_text(&format!("{:.1} GHz", mhz / 1000.0));
            } else {
                cpu_val.set_text(&cpu_text);
            }

            // GPU freq
            let gpu_text = gio::spawn_blocking(read_gpu_freq)
                .await
                .unwrap_or_else(|_| "N/A".into());
            gpu_val.set_text(&gpu_text);
            if let Ok(mhz) = gpu_text
                .trim_end_matches(|c: char| !c.is_ascii_digit())
                .parse::<f64>()
            {
                gpu_spark.push(mhz);
            }

            // GPU temp
            let (temp_text, css_class) = gio::spawn_blocking(read_gpu_temp)
                .await
                .unwrap_or(("N/A".into(), "temp-normal"));
            temp_val.remove_css_class("temp-normal");
            temp_val.remove_css_class("temp-warm");
            temp_val.remove_css_class("temp-hot");
            temp_val.add_css_class(css_class);
            temp_val.set_text(&temp_text);
            temp_spark.set_color(match css_class {
                "temp-warm" => Some((1.0, 0.65, 0.0)),
                "temp-hot" => Some((0.9, 0.15, 0.15)),
                _ => None,
            });
            if let Ok(c) = temp_text
                .chars()
                .take_while(char::is_ascii_digit)
                .collect::<String>()
                .parse::<f64>()
            {
                temp_spark.push(c);
            }

            // Disk I/O
            let cur_disk = gio::spawn_blocking(read_disk_sectors).await.ok().flatten();
            if let (Some(prev), Some(cur)) = (prev_disk, cur_disk) {
                let read_kb = (cur.0.saturating_sub(prev.0) * 512) / 1024;
                let write_kb = (cur.1.saturating_sub(prev.1) * 512) / 1024;
                disk_val.set_text(&format!("{read_kb}R {write_kb}W KB/s"));
                disk_spark.push(f64::from(
                    u32::try_from(read_kb + write_kb).unwrap_or(u32::MAX),
                ));
            } else {
                disk_spark.push(0.0);
            }
            prev_disk = cur_disk;

            // Ping
            let target = crate::settings::load().ping_target;
            let ping_text = gio::spawn_blocking(move || read_ping_latency(&target))
                .await
                .unwrap_or_else(|_| "N/A".into());
            ping_val.set_text(&ping_text);
            if let Some(ms) = ping_text
                .split_whitespace()
                .next()
                .and_then(|s| s.parse::<f64>().ok())
            {
                ping_spark.push(ms);
            }

            // RAM
            let ram_text = gio::spawn_blocking(read_ram_usage)
                .await
                .unwrap_or_else(|_| "N/A".into());
            {
                let parts: Vec<&str> = ram_text.split_whitespace().collect();
                if parts.len() >= 3 {
                    if let (Ok(used), Ok(total)) =
                        (parts[0].parse::<f64>(), parts[2].parse::<f64>())
                    {
                        if total > 0.0 {
                            let perc = used / total * 100.0;
                            ram_spark.push(perc);
                            ram_val.set_text(&format!("{:.1} GB ({:.0}%)", used / 1024.0, perc));
                        }
                    }
                } else {
                    ram_val.set_text(&ram_text);
                }
            }

            // Falcond daemon status (read /tmp/falcond_status)
            let falcond = gio::spawn_blocking(bigame_core::status::read)
                .await
                .ok()
                .flatten();

            apply_falcond_status(
                falcond.as_ref(),
                &active_profile_row,
                &scx_row,
                &vcache_row,
                &falcond_badge,
            );

            let pp_text = gio::spawn_blocking(|| {
                bigame_core::dbus::power_profile_get().unwrap_or_else(|| i18n("Unavailable"))
            })
            .await
            .unwrap_or_else(|_| "N/A".into());
            power_row.set_subtitle(&pp_text);

            let turbo_enabled = pp_text.eq_ignore_ascii_case("performance");
            if turbo_enabled {
                turbo_badge.set_text(&i18n("Active"));
                turbo_badge.remove_css_class("warning-badge");
                turbo_badge.remove_css_class("error-badge");
                turbo_badge.add_css_class("success-badge");
                turbo_row.set_subtitle(&i18n("Performance profile active"));
            } else {
                turbo_badge.set_text(&i18n("Inactive"));
                turbo_badge.remove_css_class("success-badge");
                turbo_badge.remove_css_class("error-badge");
                turbo_badge.add_css_class("warning-badge");
                turbo_row.set_subtitle(&i18n("Enable Booster Mode to apply video optimizations"));
            }

            let active_game = falcond
                .as_ref()
                .and_then(|s| s.active_profile.clone())
                .filter(|p| !p.is_empty() && p != "None");
            let has_active_game = active_game.is_some();

            let runtime = gio::spawn_blocking({
                let active_game = active_game.clone();
                move || collect_video_runtime(active_game.as_deref())
            })
            .await
            .unwrap_or_default();

            if runtime.lsfg_active != prev_is_lsfg {
                prev_is_lsfg = runtime.lsfg_active;
                if runtime.lsfg_active {
                    tracing::info!("LSFG-VK active for current game context");
                } else {
                    tracing::info!("LSFG-VK inactive for current game context");
                }
            }

            if !runtime.lsfg_enabled {
                lsfg_badge.set_text(&i18n("Off"));
                lsfg_badge.remove_css_class("success-badge");
                lsfg_badge.remove_css_class("warning-badge");
                lsfg_badge.add_css_class("dim-label");
            } else if runtime.lsfg_active {
                lsfg_badge.set_text(&i18n("Active (Generating Frames)"));
                lsfg_badge.remove_css_class("dim-label");
                lsfg_badge.remove_css_class("warning-badge");
                lsfg_badge.add_css_class("success-badge");
            } else if runtime.lsfg_installed {
                lsfg_badge.set_text(&i18n("Ready"));
                lsfg_badge.remove_css_class("dim-label");
                lsfg_badge.remove_css_class("warning-badge");
                lsfg_badge.add_css_class("success-badge");
            } else {
                lsfg_badge.set_text(&i18n("Not installed"));
                lsfg_badge.remove_css_class("dim-label");
                lsfg_badge.remove_css_class("success-badge");
                lsfg_badge.add_css_class("warning-badge");
            }

            apply_runtime_feature_status(
                &gamescope_rt_row,
                &gamescope_rt_badge,
                runtime.cfg.upscaling.gamescope_enabled,
                runtime.gamescope_active,
                turbo_enabled,
                has_active_game,
            );

            apply_runtime_feature_status(
                &wine_fsr_rt_row,
                &wine_fsr_rt_badge,
                runtime.cfg.upscaling.wine_fsr_enabled,
                runtime.wine_fsr_active,
                turbo_enabled,
                has_active_game,
            );

            apply_runtime_feature_status(
                &vkbasalt_rt_row,
                &vkbasalt_rt_badge,
                runtime.cfg.upscaling.vkbasalt_enabled,
                runtime.vkbasalt_active,
                turbo_enabled,
                has_active_game,
            );

            let fg_enabled = runtime.cfg.frame_gen.enabled
                && runtime.cfg.frame_gen.backend != bigame_core::models::FrameGenBackend::None;
            let fg_active = match runtime.cfg.frame_gen.backend {
                bigame_core::models::FrameGenBackend::OptiScaler => runtime.optiscaler_active,
                bigame_core::models::FrameGenBackend::Afmf => runtime.afmf_active,
                bigame_core::models::FrameGenBackend::LsfgVk => runtime.lsfg_active,
                bigame_core::models::FrameGenBackend::None => false,
            };

            let snapshot = (
                turbo_enabled,
                runtime.gamescope_active,
                runtime.wine_fsr_active,
                runtime.vkbasalt_active,
                fg_active,
            );
            if prev_runtime != Some(snapshot) {
                prev_runtime = Some(snapshot);
                tracing::info!(
                    turbo = turbo_enabled,
                    gamescope = runtime.gamescope_active,
                    wine_fsr = runtime.wine_fsr_active,
                    vkbasalt = runtime.vkbasalt_active,
                    framegen = fg_active,
                    "dashboard runtime status changed"
                );
            }

            apply_runtime_feature_status(
                &framegen_rt_row,
                &framegen_rt_badge,
                fg_enabled,
                fg_active,
                turbo_enabled,
                has_active_game,
            );

            let fg_conflict = turbo_enabled
                && has_active_game
                && runtime.cfg.frame_gen.enabled
                && ((matches!(
                    runtime.cfg.frame_gen.backend,
                    bigame_core::models::FrameGenBackend::OptiScaler
                        | bigame_core::models::FrameGenBackend::Afmf
                ) && runtime.lsfg_enabled
                    && runtime.lsfg_active)
                    || (runtime.cfg.frame_gen.backend
                        == bigame_core::models::FrameGenBackend::LsfgVk
                        && runtime.cfg.frame_gen.optiscaler_enabled
                        && runtime.optiscaler_active));
            if fg_conflict {
                framegen_rt_badge.set_text(&i18n("Conflict"));
                framegen_rt_badge.remove_css_class("success-badge");
                framegen_rt_badge.remove_css_class("warning-badge");
                framegen_rt_badge.remove_css_class("dim-label");
                framegen_rt_badge.add_css_class("error-badge");
                framegen_rt_row.set_subtitle(&i18n(
                    "Conflicting frame generation pipelines detected (disable one backend)",
                ));
            }
            glib::timeout_future(POLL_INTERVAL).await;
        }
    });
}

/// Read CPU core 0 frequency from sysfs (synchronous, run on background thread).
fn read_cpu_freq() -> String {
    std::fs::read_to_string("/sys/devices/system/cpu/cpu0/cpufreq/scaling_cur_freq")
        .ok()
        .and_then(|s| s.trim().parse::<u64>().ok())
        .map_or_else(|| "N/A".into(), |khz| format!("{} MHz", khz / 1000))
}

/// Read AMD GPU frequency from sysfs (synchronous, run on background thread).
fn read_gpu_freq() -> String {
    let Ok(content) = std::fs::read_to_string("/sys/class/drm/card1/device/pp_dpm_sclk") else {
        return "N/A".into();
    };
    // Active frequency line contains '*', format: "1: 1800Mhz *"
    for line in content.lines() {
        if line.contains('*') {
            if let Some(freq) = line.split_whitespace().nth(1) {
                return freq.to_owned();
            }
        }
    }
    "N/A".into()
}

/// Read GPU temperature from hwmon (synchronous, run on background thread).
///
/// Returns (formatted string, CSS class for color coding).
fn read_gpu_temp() -> (String, &'static str) {
    for i in 0..10 {
        let path = format!("/sys/class/hwmon/hwmon{i}/temp1_input");
        if let Ok(content) = std::fs::read_to_string(&path) {
            if let Ok(millideg) = content.trim().parse::<i64>() {
                let celsius = millideg / 1000;
                let class = match celsius {
                    0..=60 => "temp-normal",
                    61..=80 => "temp-warm",
                    _ => "temp-hot",
                };
                return (format!("{celsius}°C"), class);
            }
        }
    }
    ("N/A".into(), "temp-normal")
}

/// Read aggregate disk sectors (read, written) from `/proc/diskstats`.
///
/// Sums fields 3 (sectors read) and 7 (sectors written) across all block devices.
fn read_disk_sectors() -> Option<(u64, u64)> {
    let content = std::fs::read_to_string("/proc/diskstats").ok()?;
    let (mut read_total, mut write_total) = (0u64, 0u64);
    for line in content.lines() {
        let fields: Vec<&str> = line.split_whitespace().collect();
        // diskstats: major minor name rd_ios rd_merge rd_sectors ...
        // Index 5 = sectors read, index 9 = sectors written
        if fields.len() >= 10 {
            let dev = fields[2];
            // Skip partitions — only count whole devices (no trailing digit for sd*, no p\d for nvme)
            if dev.starts_with("sd") && dev.len() == 3
                || dev.starts_with("nvme") && dev.contains("n1") && !dev.contains('p')
                || dev.starts_with("vd") && dev.len() == 3
            {
                read_total += fields[5].parse::<u64>().unwrap_or(0);
                write_total += fields[9].parse::<u64>().unwrap_or(0);
            }
        }
    }
    Some((read_total, write_total))
}

/// Measure network latency via a single ICMP ping to configurable target.
fn read_ping_latency(target: &str) -> String {
    let output = std::process::Command::new("ping")
        .args(["-c", "1", "-W", "1", target])
        .output();
    match output {
        Ok(out) if out.status.success() => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            // Parse "rtt min/avg/max/mdev = X.XX/Y.YY/Z.ZZ/W.WW ms" — locale-independent.
            // The per-packet "time=" keyword is translated (e.g., "tempo=" in pt-BR) but the
            // rtt summary line is always in English.
            for line in stdout.lines() {
                if let Some(rest) = line.strip_prefix("rtt ") {
                    // rest: "min/avg/max/mdev = X.XX/Y.YY/Z.ZZ/W.WW ms"
                    if let Some(eq_pos) = rest.find('=') {
                        let values = rest[eq_pos + 1..].trim();
                        if let Some(slash) = values.find('/') {
                            return format!("{} ms", &values[..slash]);
                        }
                    }
                }
            }
            "N/A".into()
        }
        _ => "Timeout".into(),
    }
}

/// Read RAM usage from `/proc/meminfo`.
fn read_ram_usage() -> String {
    let Ok(content) = std::fs::read_to_string("/proc/meminfo") else {
        return "N/A".into();
    };
    let mut mem_total = 0u64;
    let mut mem_avail = 0u64;
    for line in content.lines() {
        if let Some(rest) = line.strip_prefix("MemTotal:") {
            mem_total = rest
                .split_whitespace()
                .next()
                .and_then(|v| v.parse().ok())
                .unwrap_or(0);
        } else if let Some(rest) = line.strip_prefix("MemAvailable:") {
            mem_avail = rest
                .split_whitespace()
                .next()
                .and_then(|v| v.parse().ok())
                .unwrap_or(0);
        }
    }
    if mem_total == 0 {
        return "N/A".into();
    }
    let used_mb = (mem_total - mem_avail) / 1024;
    let total_mb = mem_total / 1024;
    format!("{used_mb} / {total_mb} MB")
}

/// Create a beautiful dashboard card with an embedded sparkline.
fn make_dashboard_card(
    title: &str,
    icon: &str,
) -> (
    gtk4::Box,
    gtk4::Label,
    crate::widgets::sparkline::SparkHandle,
) {
    let card = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
    card.add_css_class("card");
    // Make it expand and stand out, but enforce a minimum width to avoid jitter
    card.set_hexpand(true);
    card.set_vexpand(true);
    card.set_width_request(220);

    let header = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    header.set_margin_top(12);
    header.set_margin_start(12);
    header.set_margin_end(12);

    let img = gtk4::Image::from_icon_name(icon);
    img.add_css_class("dim-label");
    header.append(&img);

    let title_lbl = gtk4::Label::new(Some(title));
    title_lbl.add_css_class("dim-label");
    title_lbl.add_css_class("caption");
    title_lbl.set_halign(gtk4::Align::Start);
    header.append(&title_lbl);

    card.append(&header);

    let val_label = gtk4::Label::new(Some("—"));
    val_label.add_css_class("title-2"); // large text, but not too large to break layout
    val_label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
    val_label.set_lines(1);
    val_label.set_margin_start(12);
    val_label.set_margin_end(12);
    val_label.set_margin_top(4);
    val_label.set_halign(gtk4::Align::Start);
    card.append(&val_label);

    let spark = crate::widgets::sparkline::build();
    spark.area.set_vexpand(true);
    spark.area.set_valign(gtk4::Align::End);
    spark.area.set_margin_start(12);
    spark.area.set_margin_end(12);
    spark.area.set_margin_bottom(12);
    card.append(&spark.area);

    (card, val_label, spark)
}

/// Build a row with a right-side badge used for runtime feature status.
fn make_runtime_status_row(
    title: &str,
    subtitle: &str,
    badge_text: &str,
    badge_class: &str,
) -> (adw::ActionRow, gtk4::Label) {
    let row = adw::ActionRow::builder()
        .title(title)
        .subtitle(subtitle)
        .build();
    let badge = gtk4::Label::builder()
        .label(badge_text)
        .css_classes([badge_class])
        .valign(gtk4::Align::Center)
        .build();
    row.add_suffix(&badge);
    (row, badge)
}

/// Independent presence flags for the runtime status rows; a flat set of
/// yes/no answers rather than a state machine.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Default)]
struct VideoRuntime {
    cfg: bigame_core::video_config::VideoConfig,
    lsfg_installed: bool,
    lsfg_enabled: bool,
    lsfg_active: bool,
    gamescope_active: bool,
    wine_fsr_active: bool,
    vkbasalt_active: bool,
    optiscaler_active: bool,
    afmf_active: bool,
}

/// Collect runtime feature flags for the current game context.
#[must_use]
fn collect_video_runtime(active_game: Option<&str>) -> VideoRuntime {
    let cfg = bigame_core::video_config::load();
    let lsfg_installed = lsfg_is_installed();
    let lsfg_enabled = bigame_core::fg::has_any_active_profile();
    let Some(game) = active_game else {
        return VideoRuntime {
            cfg,
            lsfg_installed,
            lsfg_enabled,
            ..VideoRuntime::default()
        };
    };

    let pids = find_game_pids(game);
    let gamescope_active = is_gamescope_running();
    let wine_fsr_active = pids
        .iter()
        .any(|pid| process_env_has_key(*pid, "WINE_FULLSCREEN_FSR"));
    let vkbasalt_active = pids
        .iter()
        .any(|pid| process_env_has_key(*pid, "ENABLE_VKBASALT"));
    let afmf_active = pids
        .iter()
        .any(|pid| process_env_contains(*pid, "RADV_PERFTEST", "afmf"));
    let lsfg_active = is_lsfg_active(&pids);
    let optiscaler_active = is_optiscaler_active(&pids);

    VideoRuntime {
        cfg,
        lsfg_installed,
        lsfg_enabled,
        lsfg_active,
        gamescope_active,
        wine_fsr_active,
        vkbasalt_active,
        optiscaler_active,
        afmf_active,
    }
}

/// Build a human-readable diagnostics snapshot for runtime status troubleshooting.
#[must_use]
fn build_runtime_diagnostics_report() -> String {
    let pp = bigame_core::dbus::power_profile_get().unwrap_or_else(|| i18n("Unavailable"));
    let turbo = pp.eq_ignore_ascii_case("performance");
    let st = bigame_core::status::read();
    let active = st
        .as_ref()
        .and_then(|s| s.active_profile.clone())
        .filter(|p| !p.is_empty() && p != "None");
    let runtime = collect_video_runtime(active.as_deref());

    let backend = match runtime.cfg.frame_gen.backend {
        bigame_core::models::FrameGenBackend::None => "None",
        bigame_core::models::FrameGenBackend::OptiScaler => "OptiScaler",
        bigame_core::models::FrameGenBackend::Afmf => "AFMF",
        bigame_core::models::FrameGenBackend::LsfgVk => "lsfg-vk",
    };

    let guidance = if active.is_none() {
        format!(
            "\n{guide_title}\n- {g1}\n- {g2}\n- {g3}\n",
            guide_title = i18n("Troubleshooting"),
            g1 = i18n("Create profile for the game executable in Dashboard"),
            g2 = i18n("Launch game from Dashboard button (Gamescope)"),
            g3 = i18n("Do not launch directly from Steam/Lutris if you need BiGame injections"),
        )
    } else {
        String::new()
    };

    format!(
        "{title}\n\n- {pp_k}: {pp}\n- {turbo_k}: {turbo}\n- {game_k}: {game}\n\n{cfg_title}\n- gamescope_enabled: {gs_cfg}\n- wine_fsr_enabled: {wine_cfg}\n- vkbasalt_enabled: {vkb_cfg}\n- framegen_enabled: {fg_cfg}\n- framegen_backend: {backend}\n\n{det_title}\n- gamescope_active: {gs_det}\n- wine_fsr_active: {wine_det}\n- vkbasalt_active: {vkb_det}\n- afmf_active: {afmf_det}\n- optiscaler_staged: {opti_det}\n- lsfg_installed: {lsfg_inst}\n- lsfg_active: {lsfg_det}\n",
        title = i18n("BiGameMode Runtime Diagnostics"),
        pp_k = i18n("Power Profile"),
        pp = pp,
        turbo_k = i18n("Turbo Mode"),
        turbo = turbo,
        game_k = i18n("Active Profile"),
        game = active.unwrap_or_else(|| i18n("None")),
        cfg_title = i18n("Configured Features"),
        gs_cfg = runtime.cfg.upscaling.gamescope_enabled,
        wine_cfg = runtime.cfg.upscaling.wine_fsr_enabled,
        vkb_cfg = runtime.cfg.upscaling.vkbasalt_enabled,
        fg_cfg = runtime.cfg.frame_gen.enabled,
        backend = backend,
        det_title = i18n("Detected Runtime Signals"),
        gs_det = runtime.gamescope_active,
        wine_det = runtime.wine_fsr_active,
        vkb_det = runtime.vkbasalt_active,
        afmf_det = runtime.afmf_active,
        opti_det = runtime.optiscaler_active,
        lsfg_inst = runtime.lsfg_installed,
        lsfg_det = runtime.lsfg_active,
    ) + &guidance
}

/// Each flag drives one independent aspect of the row's appearance; bundling
/// them into a struct would only move the same parameters behind a name.
#[allow(clippy::fn_params_excessive_bools)]
/// Update feature row + badge based on config/runtime/turbo/game state.
fn apply_runtime_feature_status(
    row: &adw::ActionRow,
    badge: &gtk4::Label,
    enabled: bool,
    detected_active: bool,
    turbo_enabled: bool,
    has_active_game: bool,
) {
    if !enabled {
        badge.set_text(&i18n("Off"));
        badge.remove_css_class("success-badge");
        badge.remove_css_class("warning-badge");
        badge.remove_css_class("error-badge");
        badge.add_css_class("dim-label");
        row.set_subtitle(&i18n("Disabled in settings"));
        return;
    }
    if !turbo_enabled {
        badge.set_text(&i18n("Blocked"));
        badge.remove_css_class("success-badge");
        badge.remove_css_class("dim-label");
        badge.remove_css_class("error-badge");
        badge.add_css_class("warning-badge");
        row.set_subtitle(&i18n("Requires Turbo Mode (Booster)"));
        return;
    }
    if !has_active_game {
        badge.set_text(&i18n("Waiting"));
        badge.remove_css_class("success-badge");
        badge.remove_css_class("warning-badge");
        badge.remove_css_class("error-badge");
        badge.add_css_class("dim-label");
        row.set_subtitle(&i18n("No active game profile"));
        return;
    }
    if detected_active {
        badge.set_text(&i18n("Active"));
        badge.remove_css_class("warning-badge");
        badge.remove_css_class("dim-label");
        badge.remove_css_class("error-badge");
        badge.add_css_class("success-badge");
        row.set_subtitle(&i18n("Detected during current game"));
    } else {
        badge.set_text(&i18n("Pending"));
        badge.remove_css_class("success-badge");
        badge.remove_css_class("dim-label");
        badge.remove_css_class("error-badge");
        badge.add_css_class("warning-badge");
        row.set_subtitle(&i18n("Enabled but not detected in game process"));
    }
}

#[must_use]
fn find_game_pids(game_name: &str) -> Vec<u32> {
    bigame_core::processes::find_by_cmdline(game_name)
}

#[must_use]
fn process_env_has_key(pid: u32, key: &str) -> bool {
    let path = format!("/proc/{pid}/environ");
    let Ok(bytes) = std::fs::read(path) else {
        return false;
    };
    bytes
        .split(|b| *b == 0)
        .filter_map(|entry| std::str::from_utf8(entry).ok())
        .any(|s| s.starts_with(&format!("{key}=")))
}

#[must_use]
fn process_env_contains(pid: u32, key: &str, needle: &str) -> bool {
    let path = format!("/proc/{pid}/environ");
    let Ok(bytes) = std::fs::read(path) else {
        return false;
    };
    bytes
        .split(|b| *b == 0)
        .filter_map(|entry| std::str::from_utf8(entry).ok())
        .find_map(|s| s.strip_prefix(&format!("{key}=")))
        .is_some_and(|v| v.contains(needle))
}

#[must_use]
fn is_gamescope_running() -> bool {
    !bigame_core::processes::find_by_cmdline("gamescope").is_empty()
}

#[must_use]
fn is_lsfg_active(pids: &[u32]) -> bool {
    pids.iter().any(|&pid| {
        bigame_core::processes::maps_contain(pid, &["liblsfg-vk.so", "VK_LAYER_LSFGVK", "lsfg-vk"])
    })
}

#[must_use]
fn is_optiscaler_active(pids: &[u32]) -> bool {
    pids.iter()
        .any(|&pid| bigame_core::processes::maps_contain(pid, &["nvngx.dll", "OptiScaler"]))
}

/// Check whether the lsfg-vk Vulkan implicit layer is installed.
fn lsfg_is_installed() -> bool {
    const LAYER_PATHS: &[&str] = &[
        "/etc/vulkan/implicit_layer.d/VkLayer_LS_frame_generation.json",
        "/etc/vulkan/implicit_layer.d/VkLayer_LSFGVK_frame_generation.json",
        "/usr/share/vulkan/implicit_layer.d/VkLayer_LSFGVK_frame_generation.json",
        "/usr/share/vulkan/implicit_layer.d/VkLayer_LS_frame_generation.json",
        "/usr/local/share/vulkan/implicit_layer.d/VkLayer_LSFGVK_frame_generation.json",
        "/usr/local/share/vulkan/implicit_layer.d/VkLayer_LS_frame_generation.json",
    ];
    LAYER_PATHS.iter().any(|p| std::path::Path::new(p).exists())
        || std::path::Path::new("/usr/lib/liblsfg-vk.so").exists()
}

/// Build Troubleshooting section: lists solvable issues + hardware limitations.
///
/// Shows "All OK" row when nothing is wrong.
fn build_troubleshooting_group() -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::new();
    group.set_title(&i18n("Troubleshooting"));

    let mut issue_count = 0u32;

    // SCX schedulers: solvable via package install
    let sched_detected = bigame_core::sched::detect_installed();
    let has_schedulers = sched_detected.len() > 1; // "none" always present
    if !has_schedulers {
        let row = make_troubleshoot_row(
            &i18n("SCX Scheduler not installed"),
            &i18n("Install scx-scheds to use sched-ext schedulers"),
            &i18n("sudo pacman -S scx-scheds"),
            "warning-badge",
            &i18n("Solvable"),
        );
        group.add(&row);
        issue_count += 1;
    }

    // lsfg-vk: solvable via package install
    if !lsfg_is_installed() {
        let row = make_troubleshoot_row(
            &i18n("Frame Generation (lsfg-vk) not installed"),
            &i18n("Install lsfg-vk to enable Vulkan frame generation"),
            &i18n("Install from BigLinux/AUR: lsfg-vk"),
            "warning-badge",
            &i18n("Solvable"),
        );
        group.add(&row);
        issue_count += 1;
    }

    // AMD 3D V-Cache: hardware limitation, not solvable via software
    if !bigame_core::vcache::is_available() {
        let row = make_troubleshoot_row(
            &i18n("AMD 3D V-Cache not available"),
            &i18n("VCache Mode requires an AMD X3D CPU (e.g. 5800X3D, 7800X3D, 9800X3D)"),
            &i18n("This CPU does not have 3D V-Cache hardware"),
            "dim-label",
            &i18n("Hardware"),
        );
        group.add(&row);
        issue_count += 1;
    }

    if issue_count == 0 {
        let row = make_troubleshoot_row(
            &i18n("System fully configured"),
            &i18n("All features are available and ready"),
            "",
            "success-badge",
            &i18n("OK"),
        );
        group.add(&row);
    }

    group
}

/// Build a single troubleshooting `ActionRow` with title, subtitle, hint, and badge.
fn make_troubleshoot_row(
    title: &str,
    subtitle: &str,
    hint: &str,
    badge_class: &str,
    badge_text: &str,
) -> adw::ExpanderRow {
    let row = adw::ExpanderRow::builder()
        .title(title)
        .subtitle(subtitle)
        .build();

    let badge = gtk4::Label::builder()
        .label(badge_text)
        .css_classes([badge_class])
        .valign(gtk4::Align::Center)
        .build();
    row.add_suffix(&badge);

    if !hint.is_empty() {
        let hint_row = adw::ActionRow::builder()
            .title(hint)
            .selectable(false)
            .build();
        hint_row.add_css_class("monospace");
        row.add_row(&hint_row);
    }

    row
}

/// Build the Detected Games group with a refresh button in the header.
///
/// The refresh button rebuilds the group in-place without restarting the app.
fn build_games_group() -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::new();
    group.set_title(&i18n("Detected Games"));

    // Guided onboarding: mirrors the Profiles "Create with Wizard" entry point.
    let onboarding = build_games_onboarding_row();
    group.add(&onboarding);

    populate_games_rows(&group);

    let refresh_btn = gtk4::Button::builder()
        .icon_name("view-refresh-symbolic")
        .tooltip_text(i18n("Re-detect installed games"))
        .valign(gtk4::Align::Center)
        .css_classes(["flat"])
        .build();
    refresh_btn.connect_clicked(move |btn| {
        // Walk up widget tree: button → PreferencesGroup → PreferencesPage
        // gtk4-rs 0.9.x ancestor() takes a glib::Type, not a generic param.
        let Some(old_group_w) = btn.ancestor(adw::PreferencesGroup::static_type()) else {
            return;
        };
        let Ok(old_group) = old_group_w.downcast::<adw::PreferencesGroup>() else {
            return;
        };
        let Some(parent_widget) = old_group.parent() else {
            return;
        };
        let Ok(page) = parent_widget.downcast::<adw::PreferencesPage>() else {
            return;
        };
        old_group.unparent();
        page.add(&build_games_group());
    });
    group.set_header_suffix(Some(&refresh_btn));
    group
}

/// Build a step-by-step helper row for first-time setup in Dashboard.
fn build_games_onboarding_row() -> adw::ExpanderRow {
    let row = adw::ExpanderRow::builder()
        .title(i18n("Setup Assistant (step-by-step)"))
        .subtitle(i18n("Recommended flow to make Turbo features work in-game"))
        .build();

    let s1 = adw::ActionRow::builder()
        .title(i18n("1. Create profile with Wizard"))
        .subtitle(i18n("Use guided setup to create game profile safely"))
        .build();
    let wizard_btn = gtk4::Button::builder()
        .label(i18n("Create with Wizard (guided)"))
        .css_classes(["suggested-action"])
        .valign(gtk4::Align::Center)
        .build();
    wizard_btn.connect_clicked(move |btn| {
        let btn_ref = btn.clone();
        crate::views::profile_wizard::open(btn, move |_profile| {
            crate::widgets::toast::show(&btn_ref, &i18n("Profile created"));
            refresh_detected_games_group(&btn_ref);
        });
    });
    s1.add_suffix(&wizard_btn);
    row.add_row(&s1);

    let s2 = adw::ActionRow::builder()
        .title(i18n("2. Enable Turbo + Video features"))
        .subtitle(i18n(
            "Turbo Mode must be active to apply Gamescope/FSR/vkBasalt/FrameGen",
        ))
        .build();
    row.add_row(&s2);

    let s3 = adw::ActionRow::builder()
        .title(i18n("3. Launch from Dashboard"))
        .subtitle(i18n("Use 'Launch (Turbo)' for BiGameMode injections"))
        .build();
    row.add_row(&s3);

    let s4 = adw::ActionRow::builder()
        .title(i18n("4. Validate Runtime Status"))
        .subtitle(i18n(
            "Check Video Runtime Status and Active Profile during gameplay",
        ))
        .build();
    row.add_row(&s4);

    row
}

/// Rebuild only the "Detected Games" group in-place.
fn refresh_detected_games_group(trigger: &impl IsA<gtk4::Widget>) {
    let Some(old_group_w) = trigger.ancestor(adw::PreferencesGroup::static_type()) else {
        return;
    };
    let Ok(old_group) = old_group_w.downcast::<adw::PreferencesGroup>() else {
        return;
    };
    let Some(parent_widget) = old_group.parent() else {
        return;
    };
    let Ok(page) = parent_widget.downcast::<adw::PreferencesPage>() else {
        return;
    };
    old_group.unparent();
    page.add(&build_games_group());
}

/// Resolve how a detected game should be launched.
///
/// Returns `(program, args)`:
/// - Steam: `("steam", ["-applaunch", "<appid>"])`
/// - Others: `(executable, [])` (direct process launch)
#[must_use]
fn resolve_launch_command(source: &str, executable: &str) -> (String, Vec<String>) {
    if source == "Steam" {
        if let Some(appid) = find_steam_appid_by_installdir(executable) {
            return ("steam".to_string(), vec!["-applaunch".to_string(), appid]);
        }
        tracing::warn!(
            source = %source,
            executable = %executable,
            "steam appid not found by installdir; falling back to direct launch"
        );
    }
    (executable.to_string(), Vec::new())
}

/// Search Steam appmanifest files and return appid for matching `installdir`.
#[must_use]
fn find_steam_appid_by_installdir(installdir: &str) -> Option<String> {
    let home = std::env::var("HOME").ok()?;
    let steam_dirs = [
        std::path::Path::new(&home).join(".steam/steam/steamapps"),
        std::path::Path::new(&home).join(".local/share/Steam/steamapps"),
    ];

    for dir in steam_dirs {
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let is_manifest = path
                .file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with("appmanifest_"));
            if !is_manifest {
                continue;
            }
            let Ok(content) = std::fs::read_to_string(&path) else {
                continue;
            };
            let mut appid: Option<String> = None;
            let mut dir_name: Option<String> = None;
            for line in content.lines() {
                let trimmed = line.trim();
                if trimmed.starts_with("\"appid\"") {
                    let parts: Vec<&str> = trimmed.split('"').collect();
                    if parts.len() >= 4 {
                        appid = Some(parts[3].to_string());
                    }
                }
                if trimmed.starts_with("\"installdir\"") {
                    let parts: Vec<&str> = trimmed.split('"').collect();
                    if parts.len() >= 4 {
                        dir_name = Some(parts[3].to_string());
                    }
                }
            }
            if dir_name.as_deref() == Some(installdir) {
                if let Some(id) = appid {
                    return Some(id);
                }
            }
        }
    }

    None
}

/// The process name a profile for `game` should be keyed on.
///
/// Delegates entirely to `bigame_core::games`, which scans the install
/// directory, filters store helpers and crash handlers, and ranks the rest by
/// size. This view used to carry its own copy of that heuristic; keeping two
/// implementations of "which binary is the game" meant they could disagree,
/// and the one that decided what a profile was named is the one that has to be
/// right.
#[must_use]
fn suggest_profile_program_name(game: &bigame_core::games::DetectedGame) -> String {
    game.profile_key().to_owned()
}

/// Building a widget tree is inherently linear — splitting it yields helpers
/// with a single caller and no independent meaning — so the length lint is
/// allowed here rather than worked around.
#[allow(clippy::too_many_lines)]
/// Populate game rows into a `PreferencesGroup` (called by `build_games_group`).
fn populate_games_rows(group: &adw::PreferencesGroup) {
    let detected = bigame_core::games::detect_all();
    if detected.is_empty() {
        let row = adw::ActionRow::builder()
            .title(i18n("No games detected"))
            .subtitle(i18n("Install games via Steam, Lutris, or Heroic"))
            .build();
        group.add(&row);
        return;
    }
    for game in detected.iter().take(20) {
        let row = adw::ActionRow::builder()
            .title(&*game.name)
            .subtitle(game.source.label())
            .build();
        row.add_prefix(&gtk4::Image::from_icon_name("applications-games-symbolic"));

        // "Create Profile" button per game
        let profile_exists = bigame_core::profiles::list_names()
            .iter()
            .any(|n| n == game.profile_key());
        if profile_exists {
            let badge = gtk4::Label::builder()
                .label(i18n("Profile exists"))
                .css_classes(["dim-label"])
                .build();
            row.add_suffix(&badge);
        } else {
            let btn = gtk4::Button::builder()
                .label(i18n("Create with Wizard (guided)"))
                .tooltip_text(i18n("Open guided profile setup"))
                .valign(gtk4::Align::Center)
                .css_classes(["pill", "flat"])
                .build();
            let suggested_profile = suggest_profile_program_name(game);
            let game_name = game.name.clone();
            btn.connect_clicked(move |b| {
                let btn_ref = b.clone();
                let game_name_for_log = game_name.clone();
                let exe_for_wizard = suggested_profile.clone();
                tracing::info!(
                    game = %game_name_for_log,
                    executable = %exe_for_wizard,
                    "dashboard create-profile wizard opened"
                );
                crate::views::profile_wizard::open_with_suggested_name(
                    b,
                    &exe_for_wizard,
                    move |_profile| {
                        tracing::info!(
                            game = %game_name_for_log,
                            "dashboard create-profile wizard saved"
                        );
                        crate::widgets::toast::show(&btn_ref, &i18n("Profile created"));
                        refresh_detected_games_group(&btn_ref);
                    },
                );
            });
            row.add_suffix(&btn);
        }

        // Gamescope launch button — uses launcher::LaunchPlan to apply
        // VideoConfig (upscaling filter, Wine FSR, vkBasalt, frame gen env vars)
        // on top of per-game profile gamescope settings. OptiScaler DLLs are
        // staged into the game directory when configured and install path is known.
        let gs_btn = gtk4::Button::builder()
            .label(i18n("Launch (Turbo)"))
            .tooltip_text(i18n("Launch game with BiGameMode video features"))
            .valign(gtk4::Align::Center)
            .css_classes(["suggested-action"])
            .build();
        let exe = game.profile_key().to_owned();
        let source = game.source.label();
        let game_name = game.name.clone();
        let install_path = game.install_path.clone();
        gs_btn.connect_clicked(move |b| {
            let gs_cfg = bigame_core::profiles::load(&exe)
                .ok()
                .and_then(|p| p.gamescope);
            let btn_ref = b.clone();
            let game_dir = install_path.clone();
            let exe_for_launch = exe.clone();
            let game_name_for_launch = game_name.clone();
            let game_name_for_result = game_name.clone();
            gtk4::glib::spawn_future_local(async move {
                let result = gio::spawn_blocking(move || {
                    let (launch_program, launch_args) =
                        resolve_launch_command(source, &exe_for_launch);

                    tracing::info!(
                        game = %game_name_for_launch,
                        source = %source,
                        profile = %exe_for_launch,
                        launch_program = %launch_program,
                        launch_args = ?launch_args,
                        "dashboard launch requested"
                    );

                    let video = bigame_core::video_config::load();
                    // For Steam applaunch path we avoid invasive staging to keep launch stable.
                    if launch_program != "steam" {
                        bigame_core::launcher::maybe_stage_optiscaler(
                            &video.frame_gen,
                            game_dir.as_deref(),
                        );
                    }

                    bigame_core::launcher::LaunchPlan::build_with_args_for_game(
                        &launch_program,
                        &launch_args,
                        &exe_for_launch,
                        &video,
                        gs_cfg.as_ref(),
                    )
                    .spawn()
                    .map(|mut child| {
                        // Reaped off the UI thread. Dropping the handle instead
                        // left every exited Gamescope a zombie for the life of
                        // the UI, and a zombie still matches "is it running".
                        std::thread::spawn(move || {
                            let _ = child.wait();
                        });
                    })
                    .map_err(|e| anyhow::anyhow!(e))
                })
                .await;

                match result {
                    Ok(Ok(())) => {
                        tracing::info!(game = %game_name_for_result, "dashboard launch succeeded");
                        crate::widgets::toast::show(&btn_ref, &i18n("Launch command sent"));
                    }
                    Ok(Err(e)) => {
                        tracing::error!(game = %game_name_for_result, error = %e, "dashboard launch failed");
                        crate::widgets::toast::show(
                            &btn_ref,
                            &i18n("Launch failed. Open terminal logs / Runtime Diagnostics."),
                        );
                    }
                    Err(e) => {
                        tracing::error!(game = %game_name_for_result, error = ?e, "dashboard launch task failed");
                        crate::widgets::toast::show(
                            &btn_ref,
                            &i18n("Launch task failed. Open terminal logs / Runtime Diagnostics."),
                        );
                    }
                }
            });
        });
        row.add_suffix(&gs_btn);

        group.add(&row);
    }
}
