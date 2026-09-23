//! Application setup and lifecycle.
//!
//! Supports background daemon mode: closing the window hides it
//! instead of quitting. Re-activate to show again.

use adw::prelude::*;
use gtk4::glib;
use libadwaita as adw;
use std::path::PathBuf;

use crate::i18n::i18n;
use crate::style;
use crate::tray;
use crate::window;

/// Reverse-domain application identifier.
const APP_ID: &str = "com.biglinux.BiGameMode";

/// Create, configure, and run the BiGame-mode application.
///
/// The app stays alive in the background after the window is closed.
/// Re-activating (e.g. via desktop file) will re-present the window.
pub fn run() -> adw::glib::ExitCode {
    let app = adw::Application::builder().application_id(APP_ID).build();

    app.connect_startup(|app| {
        style::load_css();
        // Keep app alive even when all windows are closed.
        // Intentionally leak the guard — app should never release.
        std::mem::forget(app.hold());

        // Falcond D-Bus status service: re-broadcasts /tmp/falcond_status as D-Bus signal.
        // External tools subscribe to com.biglinux.BiGameMode1 instead of polling the file.
        bigame_core::dbus::service::start();

        // Quit action for explicit exit
        let quit = adw::gio::ActionEntry::builder("quit")
            .activate(|app: &adw::Application, _, _| app.quit())
            .build();

        // About dialog action
        let about = adw::gio::ActionEntry::builder("about")
            .activate(|app: &adw::Application, _, _| {
                show_about_dialog(app);
            })
            .build();

        app.add_action_entries([quit, about]);

        // Keyboard shortcuts
        app.set_accels_for_action("app.quit", &["<Control>q"]);
    });

    app.connect_activate(|app| {
        // Re-present existing window or build new one
        if let Some(win) = app.active_window() {
            win.present();
        } else {
            let (win, error_indicator) = window::build(app);
            // Hide on close instead of destroying
            win.connect_close_request(|w| {
                w.set_visible(false);
                glib::Propagation::Stop
            });
            win.present();

            // System tray — poll actions from GTK main loop
            let (tray_handle, tray_rx) = tray::spawn();
            let app_ref = app.clone();

            // Poll tray actions
            glib::timeout_add_local(std::time::Duration::from_millis(250), move || {
                while let Ok(action) = tray_rx.try_recv() {
                    match action {
                        tray::TrayAction::Activate => {
                            if let Some(w) = app_ref.active_window() {
                                w.set_visible(true);
                                w.present();
                            }
                        }
                        tray::TrayAction::Quit => {
                            app_ref.quit();
                        }
                        tray::TrayAction::SwitchProfile(name) => {
                            tracing::info!("Tray: switching to profile '{name}'");
                        }
                    }
                }
                glib::ControlFlow::Continue
            });

            // Update tray status periodically
            let th_ref = tray_handle.clone();
            glib::timeout_add_local(std::time::Duration::from_secs(2), move || {
                let turbo_active = bigame_core::dbus::power_profile_get()
                    .is_some_and(|p| p.eq_ignore_ascii_case("performance"));
                let falcond_running = bigame_core::dbus::falcond_is_running();
                let missing_runtime = detect_missing_runtime_packages();

                let status = if !falcond_running {
                    error_indicator.set_error_with_action(
                        &i18n("Service Not Running or Crashed"),
                        &i18n("BiGameMode background daemon (falcond) is not running.\nThis can happen if the configuration files got corrupted by an older version of the UI."),
                        &i18n("Click 'Repair & Enable' to automatically reset corrupted configurations and start the service."),
                        &i18n("Repair & Enable"),
                        vec![
                            "pkexec".into(),
                            "sh".into(),
                            "-c".into(),
                            "rm -f /etc/falcond/config.conf; rm -f /usr/share/falcond/profiles/user/*.conf; systemctl enable --now falcond".into(),
                        ],
                    );
                    tray::Status::Warning
                } else if !missing_runtime.is_empty() {
                    let missing_csv = missing_runtime.join(", ");
                    let install_hint = install_missing_packages_hint(&missing_runtime);
                    if let Some(cmd) = install_missing_packages_action(&missing_runtime) {
                        let copy_cmd = install_missing_packages_shell_command(&missing_runtime)
                            .unwrap_or_default();
                        error_indicator.set_error_with_action_and_copy(
                            &i18n("Missing Runtime Dependencies"),
                            &format!(
                                "{}: {}",
                                i18n("Required packages were not found in the system"),
                                missing_csv
                            ),
                            &install_hint,
                            &i18n("Install Missing Packages"),
                            cmd,
                            &i18n("Copy Install Command"),
                            &copy_cmd,
                        );
                    } else {
                        error_indicator.set_error(
                            &i18n("Missing Runtime Dependencies"),
                            &format!(
                                "{}: {}",
                                i18n("Required packages were not found in the system"),
                                missing_csv
                            ),
                            &install_hint,
                        );
                    }
                    tray::Status::Warning
                } else {
                    error_indicator.clear();
                    if turbo_active {
                        tray::Status::Active
                    } else {
                        tray::Status::Idle
                    }
                };

                th_ref.set_status(status);
                glib::ControlFlow::Continue
            });
        }
    });

    app.run()
}

#[must_use]
fn detect_missing_runtime_packages() -> Vec<String> {
    let cfg = bigame_core::video_config::load();
    let mut missing = Vec::new();

    if cfg.upscaling.gamescope_enabled && !binary_in_path("gamescope") {
        missing.push("gamescope".to_string());
    }
    // vkbasalt is a Vulkan implicit layer (no CLI binary). Detect via layer manifest or libvkbasalt.so.
    if cfg.upscaling.vkbasalt_enabled && !vkbasalt_installed() {
        missing.push("vkbasalt".to_string());
    }

    missing
}

#[must_use]
fn vkbasalt_installed() -> bool {
    const LAYER_PATHS: &[&str] = &[
        "/usr/share/vulkan/implicit_layer.d/vkBasalt.json",
        "/usr/share/vulkan/implicit_layer.d/vkBasalt.x86_64.json",
        "/usr/share/vulkan/implicit_layer.d/vkBasalt.i686.json",
        "/usr/lib/libvkbasalt.so",
        "/usr/lib32/libvkbasalt.so",
    ];
    LAYER_PATHS.iter().any(|p| std::path::Path::new(p).exists())
}

#[must_use]
fn install_missing_packages_hint(missing: &[String]) -> String {
    if let Some(cmd) = install_missing_packages_shell_command(missing) {
        return format!(
            "{}\n1) {}\n2) {}\n3) {}\n\n{}\n{}",
            i18n("Troubleshooting"),
            i18n("Install missing packages"),
            i18n("Restart BiGameMode"),
            i18n("Run Runtime Diagnostics again after opening a game"),
            i18n("Command"),
            cmd
        );
    }

    format!(
        "{}\n1) {}\n2) {}\n3) {}\n\n{}: {}",
        i18n("Troubleshooting"),
        i18n("Install missing packages with your package manager"),
        i18n("Restart BiGameMode"),
        i18n("Run Runtime Diagnostics again after opening a game"),
        i18n("Missing packages"),
        missing.join(", ")
    )
}

#[must_use]
fn install_missing_packages_action(missing: &[String]) -> Option<Vec<String>> {
    if missing.is_empty() {
        return None;
    }
    // Prefer pamac-installer (full GUI window with graphical polkit auth).
    if binary_in_path("pamac-installer") {
        let mut argv = vec!["pamac-installer".to_string()];
        argv.extend(missing.iter().cloned());
        return Some(argv);
    }
    // Fallback: non-interactive pacman via pkexec so it doesn't block on stdin.
    if binary_in_path("pacman") {
        let cmd = format!("pacman -S --needed --noconfirm {}", missing.join(" "));
        return Some(vec!["pkexec".into(), "sh".into(), "-c".into(), cmd]);
    }
    None
}

#[must_use]
fn install_missing_packages_shell_command(missing: &[String]) -> Option<String> {
    if missing.is_empty() {
        return None;
    }
    if binary_in_path("pamac-installer") {
        return Some(format!("pamac-installer {}", missing.join(" ")));
    }
    if binary_in_path("pacman") {
        return Some(format!("sudo pacman -S --needed {}", missing.join(" ")));
    }
    None
}

#[must_use]
fn binary_in_path(binary: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };

    std::env::split_paths(&path).any(|dir: PathBuf| dir.join(binary).is_file())
}

/// Present the About dialog with system information.
fn show_about_dialog(app: &adw::Application) {
    let sys_info = collect_system_info();

    let dialog = adw::AboutDialog::builder()
        .application_name("BiGame-mode")
        .application_icon(APP_ID)
        .version(env!("CARGO_PKG_VERSION"))
        .developer_name("Rafael Ruscher")
        .website("https://github.com/ruscher/bigamemode")
        .issue_url("https://github.com/ruscher/bigamemode/issues")
        .license_type(gtk4::License::Gpl30)
        .comments(i18n("Performance tuning for Linux gaming"))
        .debug_info(&sys_info)
        .debug_info_filename("bigame-mode-debug.txt")
        .build();

    dialog.add_credit_section(
        Some(&i18n("Developers")),
        &["Rafael Ruscher <rruscher@gmail.com>"],
    );
    dialog.add_credit_section(
        Some(&i18n("Special Thanks")),
        &[
            "Barnabé di Kartola",
            "Alessandro (System Infotech)",
            "Pacheco (System Infotech)",
        ],
    );

    if let Some(win) = app.active_window() {
        dialog.present(Some(&win));
    }
}

/// Collect system information for the About dialog debug section.
fn collect_system_info() -> String {
    let mut lines = Vec::new();

    lines.push(format!("BiGame-mode {}", env!("CARGO_PKG_VERSION")));
    lines.push(String::new());

    // Kernel
    if let Ok(kernel) = std::fs::read_to_string("/proc/version") {
        if let Some(first) = kernel.lines().next() {
            lines.push(format!("Kernel: {first}"));
        }
    }

    // CPU model
    if let Ok(cpuinfo) = std::fs::read_to_string("/proc/cpuinfo") {
        for line in cpuinfo.lines() {
            if let Some(model) = line.strip_prefix("model name") {
                if let Some(val) = model.split_once(':').map(|(_, v)| v.trim()) {
                    lines.push(format!("CPU: {val}"));
                    break;
                }
            }
        }
    }

    // GPU (DRI device)
    if let Ok(output) = std::process::Command::new("lspci").output() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines() {
            if line.contains("VGA") || line.contains("3D controller") {
                lines.push(format!("GPU: {line}"));
                break;
            }
        }
    }

    // Installed sched-ext schedulers
    let scheds = bigame_core::sched::detect_installed();
    lines.push(format!("Schedulers: {}", scheds.join(", ")));

    // Power profile
    if let Some(pp) = bigame_core::dbus::power_profile_get() {
        lines.push(format!("Power profile: {pp}"));
    }

    lines.join("\n")
}
