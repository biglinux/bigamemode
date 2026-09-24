//! Log viewer: aggregated gaming-related system logs.
//!
//! Shows logs from multiple sources:
//! - falcond status file (`/tmp/falcond_status`)
//! - journalctl (falcond and related gaming services)
//! - dmesg (kernel gaming/GPU messages)
//! - BiGame-mode application log

use std::fmt::Write as _;
use std::time::Duration;

use adw::prelude::*;
use gtk4::{gio, glib};
use libadwaita as adw;

use crate::i18n::i18n;

/// Build the log viewer page with multiple log sources.
pub fn build() -> adw::PreferencesPage {
    let page = adw::PreferencesPage::new();

    // ── Section 1: Falcond Status ───────────────────────────────────────────
    let status_group = adw::PreferencesGroup::new();
    status_group.set_title(&i18n("Falcond Status"));
    status_group.set_description(Some(&i18n("Live status from /tmp/falcond_status")));

    let status_text = build_log_textview();
    let status_scroll = build_scroll(200);
    status_scroll.set_child(Some(&status_text));
    status_group.add(&status_scroll);
    page.add(&status_group);

    // ── Section 2: Gaming Services Journal ──────────────────────────
    let journal_group = adw::PreferencesGroup::new();
    journal_group.set_title(&i18n("Gaming Services Journal"));
    journal_group.set_description(Some(&i18n("Logs from falcond and related gaming services")));

    let refresh_btn = gtk4::Button::builder()
        .icon_name("view-refresh-symbolic")
        .tooltip_text(i18n("Refresh"))
        .css_classes(["circular", "flat"])
        .build();
    journal_group.set_header_suffix(Some(&refresh_btn));

    let journal_text = build_log_textview();
    let journal_scroll = build_scroll(300);
    journal_scroll.set_child(Some(&journal_text));
    journal_group.add(&journal_scroll);
    page.add(&journal_group);

    // ── Section 3: Kernel / GPU Messages ────────────────────────────────────
    let kernel_group = adw::PreferencesGroup::new();
    kernel_group.set_title(&i18n("Kernel &amp; GPU Messages"));
    kernel_group.set_description(Some(&i18n(
        "Recent dmesg entries related to GPU and gaming",
    )));

    let kernel_text = build_log_textview();
    let kernel_scroll = build_scroll(250);
    kernel_scroll.set_child(Some(&kernel_text));
    kernel_group.add(&kernel_scroll);
    page.add(&kernel_group);

    // ── Section 4: Application Log ──────────────────────────────────────────
    let app_group = adw::PreferencesGroup::new();
    app_group.set_title(&i18n("Application Log"));
    app_group.set_description(Some(&i18n("BiGame-mode runtime events")));

    let app_text = build_log_textview();
    let app_scroll = build_scroll(200);
    app_scroll.set_child(Some(&app_text));
    app_group.add(&app_scroll);
    page.add(&app_group);

    // ── Refresh button ──────────────────────────────────────────────────────
    {
        let s = status_text.clone();
        let j = journal_text.clone();
        let k = kernel_text.clone();
        let a = app_text.clone();
        refresh_btn.connect_clicked(move |_| {
            refresh_all(&s, &j, &k, &a);
        });
    }

    // ── Initial load ────────────────────────────────────────────────────────
    refresh_all(&status_text, &journal_text, &kernel_text, &app_text);

    // ── Auto-refresh every 5 seconds ────────────────────────────────────────
    {
        let s = status_text;
        let j = journal_text;
        let k = kernel_text;
        let a = app_text;
        glib::timeout_add_local(Duration::from_secs(5), move || {
            refresh_all(&s, &j, &k, &a);
            glib::ControlFlow::Continue
        });
    }

    page
}

fn build_log_textview() -> gtk4::TextView {
    let text = gtk4::TextView::builder()
        .editable(false)
        .cursor_visible(false)
        .monospace(true)
        .wrap_mode(gtk4::WrapMode::WordChar)
        .top_margin(8)
        .bottom_margin(8)
        .left_margin(12)
        .right_margin(12)
        .build();
    text.add_css_class("card");
    text
}

fn build_scroll(min_height: i32) -> gtk4::ScrolledWindow {
    gtk4::ScrolledWindow::builder()
        .hscrollbar_policy(gtk4::PolicyType::Automatic)
        .vscrollbar_policy(gtk4::PolicyType::Automatic)
        .min_content_height(min_height)
        .build()
}

fn refresh_all(
    status_tv: &gtk4::TextView,
    journal_tv: &gtk4::TextView,
    kernel_tv: &gtk4::TextView,
    app_tv: &gtk4::TextView,
) {
    load_status(status_tv);
    load_journal(journal_tv);
    load_kernel(kernel_tv);
    load_app_log(app_tv);
}

/// Read falcond status file directly.
fn load_status(text_view: &gtk4::TextView) {
    let tv = text_view.clone();
    glib::spawn_future_local(async move {
        let text = gio::spawn_blocking(|| {
            let status_path = bigame_core::status::status_path().display().to_string();
            match std::fs::read_to_string(&status_path) {
                Ok(content) if !content.trim().is_empty() => {
                    let mut out = String::new();
                    let _ = writeln!(out, "── {status_path} ──");
                    out.push_str(&content);
                    out.push_str("\n\n");

                    // Also show profile files
                    let profiles_dir = "/usr/share/falcond/profiles/user";
                    if let Ok(entries) = std::fs::read_dir(profiles_dir) {
                        let files: Vec<_> = entries
                            .filter_map(std::result::Result::ok)
                            .map(|e| e.file_name().to_string_lossy().into_owned())
                            .collect();
                        if files.is_empty() {
                            out.push_str("── Saved Profiles: (none) ──\n");
                        } else {
                            let _ = writeln!(out, "── Saved Profiles ({}) ──", files.len());
                            for f in &files {
                                let _ = writeln!(out, "  • {f}");
                            }
                        }
                    }
                    out
                }
                Ok(_) => format!("Status file is empty: {status_path}\n\nThe falcond daemon may not be running.\nCheck: systemctl status falcond"),
                Err(e) => format!("Cannot read {status_path}: {e}\n\nThe falcond daemon is not running or not installed.\n\nTo start it manually, you may need to install the falcond package\nor run the daemon directly."),
            }
        })
        .await;

        if let Ok(t) = text {
            tv.buffer().set_text(&t);
        }
    });
}

/// Read journal logs from multiple gaming-related services.
fn load_journal(text_view: &gtk4::TextView) {
    let tv = text_view.clone();
    glib::spawn_future_local(async move {
        let text = gio::spawn_blocking(|| {
            let mut combined = String::new();

            // Try multiple service names and approaches
            let sources = [
                // systemd user unit
                vec![
                    "journalctl",
                    "--user-unit=falcond",
                    "--no-pager",
                    "-n",
                    "50",
                    "--reverse",
                ],
                // systemd system unit
                vec![
                    "journalctl",
                    "-u",
                    "falcond",
                    "--no-pager",
                    "-n",
                    "50",
                    "--reverse",
                ],
                // Grep for falcond/bigame keywords in full journal
                vec![
                    "journalctl",
                    "--no-pager",
                    "-n",
                    "100",
                    "--reverse",
                    "--grep=falcond|bigame|scx|lsfg",
                ],
            ];

            let labels = [
                "falcond (user service)",
                "falcond (system service)",
                "System journal (gaming keywords)",
            ];

            for (args, label) in sources.iter().zip(labels.iter()) {
                let cmd = args[0];
                let cmd_args = &args[1..];
                if let Ok(out) = std::process::Command::new(cmd).args(cmd_args).output() {
                    let stdout = String::from_utf8_lossy(&out.stdout);
                    let stderr = String::from_utf8_lossy(&out.stderr);
                    if !stdout.trim().is_empty() && !stdout.contains("-- No entries --") {
                        let _ = writeln!(combined, "── {label} ──");
                        combined.push_str(stdout.trim_end());
                        combined.push_str("\n\n");
                    } else if !stderr.trim().is_empty() && stderr.contains("No entries") {
                        // Skip silently
                    }
                }
            }

            if combined.is_empty() {
                combined.push_str("No journal entries found for gaming services.\n\n");
                combined.push_str("This can happen when:\n");
                combined.push_str("  • falcond is not installed as a systemd service\n");
                combined.push_str("  • No gaming activity has been logged yet\n");
            }

            combined
        })
        .await;

        if let Ok(t) = text {
            tv.buffer().set_text(&t);
        }
    });
}

/// Read kernel messages related to GPU/gaming from dmesg.
fn load_kernel(text_view: &gtk4::TextView) {
    let tv = text_view.clone();
    glib::spawn_future_local(async move {
        let text = gio::spawn_blocking(|| {
            // Use dmesg with grep for relevant keywords
            let output = std::process::Command::new("dmesg")
                .args(["--time-format=reltime", "--level=warn,err,info"])
                .output();

            match output {
                Ok(out) => {
                    let stdout = String::from_utf8_lossy(&out.stdout);
                    let keywords = ["amdgpu", "radeon", "nvidia", "gpu", "drm", "vulkan",
                                    "sched_ext", "scx_", "vcache"];

                    let filtered: Vec<&str> = stdout
                        .lines()
                        .filter(|line| {
                            let lower = line.to_lowercase();
                            keywords.iter().any(|kw| lower.contains(kw))
                        })
                        .collect();

                    if filtered.is_empty() {
                        "No GPU/gaming kernel messages found.\n\nThis is normal if no GPU errors occurred.".into()
                    } else {
                        // Show last 50 relevant lines
                        let start = filtered.len().saturating_sub(50);
                        filtered[start..].join("\n")
                    }
                }
                Err(e) => format!("Cannot read dmesg: {e}\n\nTry running the app with elevated privileges."),
            }
        })
        .await;

        if let Ok(t) = text {
            tv.buffer().set_text(&t);
        }
    });
}

/// Show BiGame-mode application events.
fn load_app_log(text_view: &gtk4::TextView) {
    let tv = text_view.clone();
    glib::spawn_future_local(async move {
        let text = gio::spawn_blocking(|| {
            let mut log = String::new();

            // Power profile
            if let Some(pp) = bigame_core::dbus::power_profile_get() {
                let _ = writeln!(log, "Power profile: {pp}");
            } else {
                log.push_str("Power profile: unavailable\n");
            }

            // Falcond running?
            let falcond = bigame_core::dbus::falcond_is_running();
            let _ = writeln!(log, "Falcond status file exists: {falcond}");

            // Installed schedulers
            let scheds = bigame_core::sched::detect_installed();
            let _ = writeln!(
                log,
                "Installed schedulers: {}",
                if scheds.is_empty() {
                    "none".to_string()
                } else {
                    scheds.join(", ")
                }
            );

            // Profile count
            let profiles = bigame_core::profiles::list_names();
            let _ = writeln!(log, "Saved profiles: {}", profiles.len());
            for p in &profiles {
                let _ = writeln!(log, "  • {p}");
            }

            // VCache support
            let vcache_path = "/sys/devices/system/cpu/cpu0/cpufreq/amd_3d_vcache_mode";
            let vcache = std::path::Path::new(vcache_path).exists();
            let _ = writeln!(log, "AMD VCache support: {vcache}");

            // CPU governor
            if let Ok(gov) =
                std::fs::read_to_string("/sys/devices/system/cpu/cpu0/cpufreq/scaling_governor")
            {
                let _ = writeln!(log, "CPU governor: {}", gov.trim());
            }

            // LSFG-VK
            let mut lsfg_active = false;
            if let Some(status) = bigame_core::status::read() {
                if let Some(active) = status.active_profile {
                    let _ = writeln!(log, "Active game profile: {active}");
                    if !active.is_empty() && active != "None" {
                        lsfg_active = bigame_core::processes::find_by_cmdline(&active)
                            .into_iter()
                            .any(|pid| {
                                bigame_core::processes::maps_contain(
                                    pid,
                                    &["liblsfg-vk.so", "VK_LAYER_LSFGVK", "lsfg-vk"],
                                )
                            });
                    }
                }
            }
            if lsfg_active {
                log.push_str("Lossless Scaling (LSFG-VK): Active (Generating Frames)\n");
            } else {
                let installed =
                    std::path::Path::new("/usr/share/vulkan/implicit_layer.d/lsfg-vk.json")
                        .exists()
                        || std::path::Path::new("/etc/vulkan/implicit_layer.d/lsfg-vk.json")
                            .exists();
                let _ = writeln!(
                    log,
                    "Lossless Scaling (LSFG-VK): {}",
                    if installed {
                        "Ready / Inactive"
                    } else {
                        "Not installed"
                    }
                );
            }

            log
        })
        .await;

        if let Ok(t) = text {
            tv.buffer().set_text(&t);
        }
    });
}
