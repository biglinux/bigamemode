//! Application setup and lifecycle.
//!
//! Supports background daemon mode: closing the window hides it
//! instead of quitting. Re-activate to show again.

use adw::prelude::*;
use gtk4::glib;
use libadwaita as adw;

use crate::i18n::i18n;
use crate::style;
use crate::tray;
use crate::window;

thread_local! {
    /// The tray's action channel and the application it acts on.
    static TRAY: std::cell::RefCell<
        Option<(std::sync::mpsc::Receiver<tray::TrayAction>, glib::WeakRef<adw::Application>)>,
    > = const { std::cell::RefCell::new(None) };
}

/// Handle whatever the tray has sent. Called on the main thread, when the
/// tray thread wakes it.
pub fn drain_tray_actions() {
    TRAY.with(|t| {
        let guard = t.borrow();
        let Some((rx, app)) = guard.as_ref() else {
            return;
        };
        let Some(app) = app.upgrade() else {
            return;
        };
        while let Ok(action) = rx.try_recv() {
            match action {
                tray::TrayAction::Activate => {
                    if let Some(w) = app
                        .active_window()
                        .or_else(|| app.windows().into_iter().next())
                    {
                        w.set_visible(true);
                        w.present();
                    }
                }
                // The same actions Home's button and picker answer to: the
                // tray never switches anything beside them.
                tray::TrayAction::SetTurbo(on) => {
                    app.change_action_state("turbo", &on.to_variant());
                }
                tray::TrayAction::SetPreset(preset) => {
                    app.change_action_state("turbo-preset", &preset.id().to_variant());
                }
                tray::TrayAction::Quit => request_quit(&app),
            }
        }
    });
}

thread_local! {
    /// Work that must finish once started (a measurement, a Turbo change):
    /// how many are running, and whether Quit waits for them.
    static BUSY: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
    static QUIT_WHEN_DONE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Held while work runs that quitting would leave half done; Quit then asks
/// first and, if told to, waits for the last one to be dropped.
#[must_use = "the work counts as running only while this is held"]
pub struct Busy(());

impl Busy {
    /// Mark work as running until the returned value is dropped.
    pub fn hold() -> Self {
        BUSY.with(|b| b.set(b.get() + 1));
        Self(())
    }
}

impl Drop for Busy {
    fn drop(&mut self) {
        let left = BUSY.with(|b| {
            b.set(b.get().saturating_sub(1));
            b.get()
        });
        if left == 0 && QUIT_WHEN_DONE.with(std::cell::Cell::get) {
            // Out of the drop: quitting tears down what may be dropping us.
            glib::idle_add_local_once(|| {
                if let Some(app) = gtk4::gio::Application::default() {
                    app.quit();
                }
            });
        }
    }
}

/// Quit, unless work is running that quitting would cut short: then ask,
/// and quit once it has finished.
fn request_quit(app: &adw::Application) {
    if BUSY.with(std::cell::Cell::get) == 0 {
        app.quit();
        return;
    }
    let Some(win) = app
        .active_window()
        .or_else(|| app.windows().into_iter().next())
    else {
        QUIT_WHEN_DONE.with(|q| q.set(true));
        return;
    };
    win.set_visible(true);
    win.present();
    let dialog = adw::AlertDialog::new(
        Some(&i18n("Quit when finished?")),
        Some(&i18n(
            "Big Game Mode is in the middle of something it has to finish, such as a measurement or switching Turbo. Quitting now could leave a change applied.",
        )),
    );
    dialog.add_response("cancel", &i18n("Cancel"));
    dialog.add_response("quit", &i18n("Quit When Finished"));
    dialog.set_default_response(Some("cancel"));
    dialog.set_close_response("cancel");
    dialog.connect_response(None, |_, response| {
        if response != "quit" {
            return;
        }
        if BUSY.with(std::cell::Cell::get) == 0 {
            if let Some(app) = gtk4::gio::Application::default() {
                app.quit();
            }
        } else {
            QUIT_WHEN_DONE.with(|q| q.set(true));
        }
    });
    dialog.present(Some(&win));
}

/// Reverse-domain application identifier.
pub(crate) const APP_ID: &str = "com.biglinux.BiGameMode";

/// The product's name, as people see it. Identifiers — the application id,
/// the D-Bus names, the package, the paths — keep their technical names.
pub(crate) const NAME: &str = "Big Game Mode";

/// Create, configure, and run the Big Game Mode application.
///
/// The app stays alive in the background after the window is closed.
/// Re-activating (e.g. via desktop file) will re-present the window.
pub fn run() -> adw::glib::ExitCode {
    // `--background` (from the login autostart entry) starts everything --
    // tray, game watcher, profile offer -- without showing the window. It is
    // taken out of the arguments because GApplication rejects options it was
    // not told about.
    let args: Vec<String> = std::env::args().collect();
    let background = std::cell::Cell::new(args.iter().any(|a| a == "--background"));
    let args: Vec<String> = args.into_iter().filter(|a| a != "--background").collect();

    // The name notifications and the desktop show; left unset, GLib uses the
    // program name and notifications are signed "bigame-ui".
    adw::glib::set_application_name(NAME);
    replace_outdated_instance();
    let app = adw::Application::builder().application_id(APP_ID).build();

    app.connect_startup(|app| {
        style::load_css();
        crate::theme::apply_saved();
        // Keep app alive even when all windows are closed.
        // Intentionally leak the guard — app should never release.
        std::mem::forget(app.hold());

        // Re-broadcasts falcond's status file as a D-Bus signal, so other tools
        // can subscribe to com.biglinux.BiGameMode1 instead of reading the file.
        bigame_core::dbus::service::start();

        // An AI Graphics apply cut short by a crash or a power loss is rolled
        // back before anything reads the game's files.
        std::thread::spawn(|| {
            match bigame_core::graphics::transaction::recover(&bigame_core::graphics::state_dir()) {
                Ok(done) => {
                    for (game, outcome) in done {
                        match outcome {
                            Ok(_) => tracing::info!(target: "graphics", game, "interrupted apply rolled back"),
                            Err(e) => tracing::warn!(target: "graphics", game, error = %e, "interrupted apply could not be rolled back"),
                        }
                    }
                }
                Err(e) => tracing::warn!(target: "graphics", error = %e, "could not check for interrupted applies"),
            }
        });

        // An lsfg-vk file in the layout an earlier Big Game Mode wrote makes
        // lsfg-vk ignore it entirely; convert it before a game starts.
        std::thread::spawn(|| match bigame_core::fg::convert_legacy_file() {
            Ok(true) => tracing::info!(target: "fg", "lsfg-vk configuration converted to the installed lsfg-vk's layout"),
            Ok(false) => {}
            Err(e) => tracing::warn!(target: "fg", error = %format!("{e:#}"), "could not convert the lsfg-vk configuration"),
        });

        // Booster changes still in force while Turbo is off (falcond stopped
        // from outside, or the application killed mid-way) are put back.
        std::thread::spawn(|| match bigame_core::turbo::reconcile_blocking() {
            Ok(0) => {}
            Ok(n) => tracing::info!(target: "turbo", restored = n, "left-over Booster changes restored"),
            Err(e) => tracing::warn!(target: "turbo", error = %e, "could not check for left-over Booster changes"),
        });

        // An autostart entry an older version wrote gets TryExec.
        std::thread::spawn(crate::settings::refresh_autostart_entry);

        // Programs a previous run paused from Details and could not resume
        // (it was killed, or crashed) are resumed before anything else: a
        // program must never stay frozen because Big Game Mode went away.
        match bigame_core::processes::resume_all() {
            0 => {}
            n => tracing::info!(target: "processes", resumed = n, "programs left paused by an earlier run resumed"),
        }

        let quit = adw::gio::ActionEntry::builder("quit")
            .activate(|app: &adw::Application, _, _| request_quit(app))
            .build();

        let about = adw::gio::ActionEntry::builder("about")
            .activate(|app: &adw::Application, _, _| {
                show_about_dialog(app);
            })
            .build();

        app.add_action_entries([quit, about]);

        app.set_accels_for_action("app.quit", &["<Control>q"]);

        // Which game is running, for Home and the first-run profile offer.
        // Started here, not with the window: the application keeps running
        // with its window closed, and that is when games are played.
        crate::game_watch::start();
        crate::profile_offer::install(app);
    });

    app.connect_activate(move |app| {
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
            // Started at login: stay out of the way until asked for.
            if background.replace(false) {
                win.set_visible(false);
            } else {
                win.present();
            }

            // System tray — poll actions from GTK main loop
            let (tray_handle, tray_rx) = tray::spawn();

            // Tray actions arrive when the tray thread wakes the main loop.
            TRAY.with(|t| *t.borrow_mut() = Some((tray_rx, app.downgrade())));

            let tray_handle = std::rc::Rc::new(tray_handle);
            follow_turbo(app, &tray_handle);
            start_status_loop(tray_handle, &error_indicator);
        }
    });

    // Quitting (the tray's Quit, Ctrl+Q) resumes what Details paused: the
    // Resume button goes away with the window, so the programs must not
    // stay behind frozen.
    app.connect_shutdown(|_| match bigame_core::processes::resume_all() {
        0 => {}
        n => tracing::info!(target: "processes", resumed = n, "paused programs resumed on quit"),
    });

    app.run_with_args(&args)
}

/// Ask a running instance whose program was replaced on disk to quit, and
/// wait for it to go.
///
/// `GApplication` is single-instance: opening Big Game Mode activates the one
/// already running. After a package upgrade that is the tray instance the
/// login started, still the old version, and it would stay what the user
/// sees until the next login. Its executable then reads as `… (deleted)` in
/// `/proc`; a build started from elsewhere next to an installed instance
/// does not, and leaves it alone. It is asked through its own exported
/// `quit` action, so it shuts down as from the tray (paused programs are
/// resumed), and this process becomes the instance.
fn replace_outdated_instance() {
    use adw::gio;
    use adw::glib::variant::ToVariant as _;

    let Ok(bus) = gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE) else {
        return;
    };
    let owner_pid = || {
        bus.call_sync(
            Some("org.freedesktop.DBus"),
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
            "GetConnectionUnixProcessID",
            Some(&(APP_ID,).to_variant()),
            Some(glib::VariantTy::new("(u)").expect("valid type string")),
            gio::DBusCallFlags::NONE,
            2000,
            gio::Cancellable::NONE,
        )
        .ok()
        .and_then(|v| v.get::<(u32,)>())
        .map(|(pid,)| pid)
    };
    let Some(pid) = owner_pid() else {
        return;
    };
    let replaced = std::fs::read_link(format!("/proc/{pid}/exe"))
        .is_ok_and(|p| p.to_string_lossy().ends_with(" (deleted)"));
    if pid == std::process::id() || !replaced {
        return;
    }
    tracing::info!(
        pid,
        "an instance of an earlier version is running; asking it to quit"
    );
    let object_path = format!("/{}", APP_ID.replace('.', "/"));
    let platform_data = std::collections::HashMap::<String, glib::Variant>::new();
    if let Err(e) = bus.call_sync(
        Some(APP_ID),
        &object_path,
        "org.gtk.Actions",
        "Activate",
        Some(&("quit", Vec::<glib::Variant>::new(), platform_data).to_variant()),
        None,
        gio::DBusCallFlags::NONE,
        2000,
        gio::Cancellable::NONE,
    ) {
        tracing::warn!(pid, error = %e, "the earlier instance did not take the request to quit");
        return;
    }
    // Its shutdown resumes paused programs and releases the name; five
    // seconds is far more than that takes.
    for _ in 0..50 {
        if owner_pid() != Some(pid) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    tracing::warn!(
        pid,
        "the earlier instance is still running; opening it instead"
    );
}

/// Keep the tray's Turbo and preset in step with Home's `app.turbo` and
/// `app.turbo-preset`, as their state changes: no polling.
fn follow_turbo(app: &adw::Application, tray_handle: &std::rc::Rc<tray::TrayHandle>) {
    let show_turbo = {
        let tray_handle = std::rc::Rc::clone(tray_handle);
        move |app: &adw::Application| {
            let on = app
                .action_state("turbo")
                .and_then(|v| v.get::<bool>())
                .unwrap_or(false);
            tray_handle.set_turbo(on, app.is_action_enabled("turbo"));
        }
    };
    let show_preset = {
        let tray_handle = std::rc::Rc::clone(tray_handle);
        move |app: &adw::Application| {
            let preset = app
                .action_state("turbo-preset")
                .and_then(|v| v.str().and_then(bigame_core::turbo_preset::Preset::from_id))
                .unwrap_or_default();
            tray_handle.set_preset(preset, app.is_action_enabled("turbo-preset"));
        }
    };
    show_turbo(app);
    show_preset(app);
    {
        let show = show_turbo.clone();
        app.connect_action_state_changed(Some("turbo"), move |app, _, _| {
            if let Some(app) = app.downcast_ref::<adw::Application>() {
                show(app);
            }
        });
    }
    app.connect_action_enabled_changed(Some("turbo"), move |app, _, _| {
        if let Some(app) = app.downcast_ref::<adw::Application>() {
            show_turbo(app);
        }
    });
    {
        let show = show_preset.clone();
        app.connect_action_state_changed(Some("turbo-preset"), move |app, _, _| {
            if let Some(app) = app.downcast_ref::<adw::Application>() {
                show(app);
            }
        });
    }
    app.connect_action_enabled_changed(Some("turbo-preset"), move |app, _, _| {
        if let Some(app) = app.downcast_ref::<adw::Application>() {
            show_preset(app);
        }
    });
}

/// Keep the tray's warning and the error indicator in step with the real
/// state.
fn start_status_loop(
    tray_handle: std::rc::Rc<tray::TrayHandle>,
    error_indicator: &std::sync::Arc<crate::widgets::error_indicator::ErrorIndicator>,
) {
    // Tray and error indicator, from the systems that hold the state.
    //
    // Every ten seconds, read on a worker over the shared bus connection:
    // the window may be hidden in the tray while a game runs, and the main
    // loop must not wait on systemd. A stopped falcond is not an error: it
    // is what Turbo off means. Only a unit systemd reports as failed is, and
    // nothing offered here deletes anything.
    let busy = std::rc::Rc::new(std::cell::Cell::new(false));
    let read = std::rc::Rc::new({
        let error_indicator = std::sync::Arc::clone(error_indicator);
        move || {
            if busy.replace(true) {
                return;
            }
            let (busy, tray_handle, error_indicator) = (
                std::rc::Rc::clone(&busy),
                std::rc::Rc::clone(&tray_handle),
                std::sync::Arc::clone(&error_indicator),
            );
            glib::spawn_future_local(async move {
                let reading = gtk4::gio::spawn_blocking(|| {
                    let unit = bigame_core::systemd::Reader::shared()
                        .and_then(|r| r.unit_state(bigame_core::turbo::BACKEND_UNIT));
                    (unit, detect_missing_runtime_packages())
                })
                .await;
                busy.set(false);
                if let Ok((unit, missing_runtime)) = reading {
                    show_status(
                        unit.as_ref(),
                        &missing_runtime,
                        &tray_handle,
                        &error_indicator,
                    );
                }
            });
        }
    });
    // After "Install Missing Packages", read again at once rather than up
    // to ten seconds later.
    {
        let read = std::rc::Rc::clone(&read);
        error_indicator.connect_action_done(move || read());
    }
    glib::timeout_add_local(std::time::Duration::from_secs(10), move || {
        read();
        glib::ControlFlow::Continue
    });
}

/// Put one reading on the tray and the error indicator.
fn show_status(
    unit: Option<&bigame_core::systemd::UnitState>,
    missing_runtime: &[String],
    tray_handle: &tray::TrayHandle,
    error_indicator: &crate::widgets::error_indicator::ErrorIndicator,
) {
    let backend_failed = unit.is_some_and(|u| u.active_state == "failed");
    let warning = if backend_failed {
        error_indicator.set_error(
            &i18n("falcond stopped unexpectedly"),
            &i18n("The per-game optimization service failed, so games are not being optimized."),
            &i18n("Open Logs to see why. Turning Turbo off and on again restarts it."),
        );
        Some(i18n("falcond stopped unexpectedly"))
    } else if !missing_runtime.is_empty() {
        let missing_csv = missing_runtime.join(", ");
        let action = install_missing_packages_action(missing_runtime);
        // The confirmation shows the very command the button runs.
        let install_hint =
            install_missing_packages_hint(missing_runtime, action.as_deref().map(|a| a.join(" ")));
        if let Some(cmd) = action {
            let copy_cmd =
                install_missing_packages_shell_command(missing_runtime).unwrap_or_default();
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
        Some(i18n("Missing Runtime Dependencies"))
    } else {
        error_indicator.clear();
        None
    };

    tray_handle.set_warning(warning);
}

#[must_use]
fn detect_missing_runtime_packages() -> Vec<String> {
    let cfg = bigame_core::video_config::load();
    let mut missing = Vec::new();

    if cfg.upscaling.gamescope_enabled && bigame_core::capabilities::which("gamescope").is_none() {
        missing.push("gamescope".to_string());
    }
    // vkBasalt is a Vulkan implicit layer (no CLI binary).
    if cfg.upscaling.vkbasalt_enabled && !bigame_core::capabilities::vkbasalt_installed() {
        missing.push("vkbasalt".to_string());
    }

    missing
}

#[must_use]
fn install_missing_packages_hint(missing: &[String], runs: Option<String>) -> String {
    if let Some(cmd) = runs.or_else(|| install_missing_packages_shell_command(missing)) {
        return format!(
            "{}\n1) {}\n2) {}\n3) {}\n\n{}\n{}",
            i18n("Troubleshooting"),
            i18n("Install missing packages"),
            i18n("Restart Big Game Mode"),
            i18n("Open Details after starting a game to see what it really got"),
            i18n("Command"),
            cmd
        );
    }

    format!(
        "{}\n1) {}\n2) {}\n3) {}\n\n{}: {}",
        i18n("Troubleshooting"),
        i18n("Install missing packages with your package manager"),
        i18n("Restart Big Game Mode"),
        i18n("Open Details after starting a game to see what it really got"),
        i18n("Missing packages"),
        missing.join(", ")
    )
}

#[must_use]
fn install_missing_packages_action(missing: &[String]) -> Option<Vec<String>> {
    // Offered only when every package installs from this system's
    // repositories: otherwise the user is told to use their package manager.
    if missing.is_empty()
        || !missing
            .iter()
            .all(|p| bigame_core::capabilities::in_repositories(p))
    {
        return None;
    }
    // Prefer pamac-installer (full GUI window with graphical polkit auth).
    if bigame_core::capabilities::which("pamac-installer").is_some() {
        let mut argv = vec!["pamac-installer".to_string()];
        argv.extend(missing.iter().cloned());
        return Some(argv);
    }
    // Fallback: non-interactive pacman via pkexec so it doesn't block on stdin.
    // Arguments are passed as an argv, never through a shell: a root command
    // assembled into a string for `sh -c` is one quoting mistake away from
    // running something else.
    if bigame_core::capabilities::which("pacman").is_some() {
        let mut argv: Vec<String> = ["pkexec", "pacman", "-S", "--needed", "--noconfirm"]
            .iter()
            .map(|s| (*s).to_owned())
            .collect();
        argv.extend(missing.iter().cloned());
        return Some(argv);
    }
    None
}

#[must_use]
fn install_missing_packages_shell_command(missing: &[String]) -> Option<String> {
    // Offered only when every package installs from this system's
    // repositories: otherwise the user is told to use their package manager.
    if missing.is_empty()
        || !missing
            .iter()
            .all(|p| bigame_core::capabilities::in_repositories(p))
    {
        return None;
    }
    if bigame_core::capabilities::which("pamac-installer").is_some() {
        return Some(format!("pamac-installer {}", missing.join(" ")));
    }
    if bigame_core::capabilities::which("pacman").is_some() {
        return Some(format!("sudo pacman -S --needed {}", missing.join(" ")));
    }
    None
}

/// Where the project lives: the About dialog's website, and the base of its
/// issue tracker.
const WEBSITE: &str = "https://github.com/ruscher/bigamemode";

/// Present the About dialog.
fn show_about_dialog(app: &adw::Application) {
    let dialog = adw::AboutDialog::builder()
        .application_name(NAME)
        .application_icon(APP_ID)
        .version(env!("CARGO_PKG_VERSION"))
        // The line under the name: what the product is. The people who make
        // it are in the credits.
        .developer_name(i18n("BigLinux's game mode."))
        .website(WEBSITE)
        .issue_url(format!("{WEBSITE}/issues"))
        .license_type(gtk4::License::Gpl30)
        .comments(format!(
            "{}\n\n{}",
            i18n(
                "Big Game Mode is BigLinux's gaming hub. With one button, Turbo, games run with the right performance profile, applied by falcond when they open and undone when they close. The app shows what is really in force, measures whether a change helped, and takes care of what happens inside the game, such as upscaling and frame generation, always with a full backup and undo.",
            ),
            i18n(
                "One rule runs through the project: nothing is offered that the machine cannot do, and nothing is called an improvement without a measurement.",
            ),
        ))
        .debug_info(i18n("Gathering system information…"))
        .debug_info_filename("bigame-mode-debug.txt")
        .build();
    // Translators put their names here; untranslated, the msgid comes back
    // and there is no one to credit.
    let translators = i18n("translator-credits");
    if translators != "translator-credits" {
        dialog.set_translator_credits(&translators);
    }

    // libadwaita shows "Name <address>" as the name alone, with a button
    // that writes to the address (mailto:), and "Name https://…" as a link
    // to the page: the addresses are reachable without being printed. A
    // social-media handle is neither, so it stays text.
    dialog.add_credit_section(
        Some(&i18n("Lead Developer")),
        &["Rafael Ruscher <rruscher@gmail.com>"],
    );
    dialog.add_credit_section(
        Some(&i18n("Special Thanks")),
        &[
            "Bruno Gonçalves <bigbruno@gmail.com>",
            "Barnabé di Kartola <barnabedikartola@gmail.com>",
            "Alexasandro Pacheco Feliciano (Pacheco) @pachecogameroficial",
            "Alessandro e Silva Xavier (Alessandro) @alessandro741",
            "Narayan Silva (Nara Linux) <narayancloud@gmail.com>",
        ],
    );
    dialog.add_acknowledgement_section(
        Some(&i18n("vkBasalt configuration")),
        &["Narayan (Nara Linux) https://www.youtube.com/watch?v=GGBC-qMB_0Y"],
    );
    // What Big Game Mode drives rather than reimplements (README, "Projetos
    // utilizados").
    dialog.add_acknowledgement_section(
        Some(&i18n("Built on")),
        &[
            "falcond https://git.pika-os.com/general-packages/falcond",
            "sched-ext https://github.com/sched-ext/scx",
            "Gamescope https://github.com/ValveSoftware/gamescope",
            "MangoHud https://github.com/flightlessmango/MangoHud",
            "vkBasalt https://github.com/DadSchoorse/vkBasalt",
            "lsfg-vk https://github.com/PancakeTAS/lsfg-vk",
            "OptiScaler https://github.com/optiscaler/OptiScaler",
        ],
    );

    // The support report Details copies and `bigame-ui --diagnostics`
    // prints: one report, so what support reads here is what it reads
    // everywhere, with the same redaction. It asks systemd and D-Bus and
    // reads the package database, so it is gathered on a worker and filled
    // in when it arrives; the dialog opens at once.
    glib::spawn_future_local(glib::clone!(
        #[weak]
        dialog,
        async move {
            if let Ok(info) =
                gtk4::gio::spawn_blocking(|| bigame_core::diagnostics::report(false)).await
            {
                dialog.set_debug_info(&info);
            }
        }
    ));

    if let Some(win) = app.active_window() {
        dialog.present(Some(&win));
    }
}
