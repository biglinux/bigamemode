//! Offer a profile the first time an unknown game runs.
//!
//! ```text
//! Turbo on → a game starts → falcond has no specific profile for its process
//!          → a notification offers one → created, then verified active
//! ```
//!
//! A notification rather than a dialog, deliberately. The game is usually
//! fullscreen, often under Gamescope, and a window that takes focus from it
//! mid-play is worse than no offer at all; a notification waits in the
//! desktop's queue on Wayland and X11 alike. Clicking it opens the review,
//! which shows every value and why it was chosen.
//!
//! The offer is made once per game per session, never while Turbo is off,
//! and never again for a game the user said not to ask about.

use std::cell::RefCell;
use std::collections::HashSet;
use std::path::PathBuf;

use adw::prelude::*;
use gtk4::gio;
use gtk4::glib;
use libadwaita as adw;

use bigame_core::recommend::{self, Recommendation};
use bigame_core::running::GameIdentity;

use crate::i18n::{error_text, i18n, tr};

const NOTIFICATION_ID: &str = "profile-offer";

/// How long a game must have been running before a profile is offered.
const SETTLE_SECS: u64 = 20;

thread_local! {
    static OFFERED: RefCell<HashSet<String>> = RefCell::new(HashSet::new());
    /// The pid last announced, so a second report of it is not a second notification.
    static ANNOUNCED: std::cell::Cell<Option<u32>> = const { std::cell::Cell::new(None) };
    static LAST_GAME: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// Games the user asked never to be asked about again.
fn never_path() -> Option<PathBuf> {
    let state = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state")))?;
    Some(
        state
            .join("bigame-mode")
            .join("profile-offers-declined.json"),
    )
}

fn declined() -> HashSet<String> {
    never_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

fn decline_forever(process: &str) -> std::io::Result<()> {
    let mut set = declined();
    set.insert(process.to_owned());
    let path = never_path().ok_or_else(|| {
        std::io::Error::other(i18n("no home folder is set (HOME or XDG_STATE_HOME)"))
    })?;
    let json = serde_json::to_vec_pretty(&set).map_err(std::io::Error::other)?;
    crate::settings::write_atomic(&path, &json)?;
    tracing::info!(process, "will not offer a profile for this game again");
    Ok(())
}

/// Whether Turbo is on (falcond's unit is active). Asks systemd, so it runs
/// off the main thread.
fn turbo_is_on() -> bool {
    bigame_core::systemd::Reader::shared()
        .and_then(|r| r.unit_state(bigame_core::turbo::BACKEND_UNIT))
        .is_some_and(|u| u.is_active())
}

/// Whether this game should be offered a profile now.
fn should_offer(game: &GameIdentity) -> bool {
    if !crate::settings::load().offer_profiles {
        return false;
    }
    if !turbo_is_on() {
        return false;
    }
    // A process that has only just started may be a helper that runs before
    // the game (Steam's installer script does), so the offer waits until the
    // process has lived a while.
    if bigame_core::running::running_for(game.pid).is_none_or(|secs| secs < SETTLE_SECS) {
        return false;
    }
    if declined().contains(&game.process_name) {
        return false;
    }
    let mode = bigame_core::config::read()
        .map(|c| c.profile_mode)
        .unwrap_or_default();
    bigame_core::overview::running_profile(game, &mode).is_none()
}

/// Register the actions and start listening for games.
pub fn install(app: &adw::Application) {
    let create = gio::SimpleAction::new("profile-create", Some(glib::VariantTy::STRING));
    create.connect_activate(glib::clone!(
        #[weak]
        app,
        move |_, param| {
            if let Some(process) = param.and_then(glib::Variant::str) {
                create_for(&app, process);
            }
        }
    ));
    let review = gio::SimpleAction::new("profile-review", Some(glib::VariantTy::STRING));
    review.connect_activate(glib::clone!(
        #[weak]
        app,
        move |_, param| {
            if let Some(process) = param.and_then(glib::Variant::str).filter(|p| !p.is_empty()) {
                show_review(&app, process);
            }
        }
    ));
    let never = gio::SimpleAction::new("profile-never", Some(glib::VariantTy::STRING));
    never.connect_activate(glib::clone!(
        #[weak]
        app,
        move |_, param| {
            if let Some(process) = param.and_then(glib::Variant::str) {
                app.withdraw_notification(NOTIFICATION_ID);
                // Asked from a notification, often with the window hidden:
                // a choice that was not kept is said there too.
                if let Err(e) = decline_forever(process) {
                    tracing::warn!(process, error = %e, "could not save \"Don't ask again\"");
                    let n = gio::Notification::new(
                        &i18n("Could not save: %s").replace("%s", &e.to_string()),
                    );
                    app.send_notification(Some(NOTIFICATION_ID), &n);
                }
            }
        }
    ));
    app.add_action(&create);
    app.add_action(&review);
    app.add_action(&never);

    crate::game_watch::subscribe(glib::clone!(
        #[weak]
        app,
        #[upgrade_or]
        glib::ControlFlow::Break,
        move |game| {
            game_changed(&app, game);
            glib::ControlFlow::Continue
        }
    ));
}

/// The running game changed: announce it, offer a profile, or say it closed.
fn game_changed(app: &adw::Application, game: Option<&GameIdentity>) {
    let Some(game) = game.cloned() else {
        app.withdraw_notification(NOTIFICATION_ID);
        if let Some(name) = LAST_GAME.with(|g| g.borrow_mut().take()) {
            if crate::settings::load().notifications_enabled {
                // Only Turbo changes anything for a game: with it off there
                // is nothing that was put back to announce.
                let app = app.clone();
                glib::spawn_future_local(async move {
                    if !gio::spawn_blocking(turbo_is_on).await.unwrap_or(false) {
                        return;
                    }
                    let n = gio::Notification::new(&i18n("%s closed").replace("%s", &name));
                    n.set_body(Some(&i18n(
                        "Everything the game's profile changed has been put back.",
                    )));
                    app.send_notification(Some("game-exit"), &n);
                });
            }
        }
        return;
    };
    // The watch reports a game again when it learns its graphics
    // API; the same process is announced, and offered, once.
    if ANNOUNCED.with(|a| a.replace(Some(game.pid))) == Some(game.pid) {
        return;
    }
    LAST_GAME.with(|g| *g.borrow_mut() = Some(game.display_name.clone()));
    let app = app.clone();
    glib::spawn_future_local(async move {
        // Wait until the process has settled, then ask again whether it
        // is still the running game.
        let age = bigame_core::running::running_for(game.pid).unwrap_or(0);
        if age < SETTLE_SECS {
            glib::timeout_future_seconds(u32::try_from(SETTLE_SECS - age).unwrap_or(20)).await;
        }
        if crate::game_watch::current().map(|g| g.pid) != Some(game.pid) {
            return;
        }
        if OFFERED.with(|o| o.borrow().contains(&game.process_name)) {
            return;
        }
        let check = game.clone();
        let offer = gio::spawn_blocking(move || should_offer(&check))
            .await
            .unwrap_or(false);
        // Marked here, on the main thread, where OFFERED lives; from
        // the blocking thread it is another, empty, set.
        if offer {
            OFFERED.with(|o| o.borrow_mut().insert(game.process_name.clone()));
            notify_offer(&app, &game);
        } else if crate::settings::load().notifications_enabled {
            let detected = gio::spawn_blocking(detected_profile).await.ok().flatten();
            if let Some(profile) = detected {
                notify_detected(&app, &game, &profile);
            }
        }
    });
}

fn notify_offer(app: &adw::Application, game: &GameIdentity) {
    let target = game.process_name.to_variant();
    let notification =
        gio::Notification::new(&i18n("%s is running").replace("%s", &game.display_name));
    notification.set_body(Some(&i18n(
        "Big Game Mode has no profile for this game yet. Create one tuned for this machine?",
    )));
    notification.set_default_action_and_target_value("app.profile-review", Some(&target));
    notification.add_button_with_target_value(
        &i18n("Create profile"),
        "app.profile-create",
        Some(&target),
    );
    notification.add_button_with_target_value(
        &i18n("Don't ask again"),
        "app.profile-never",
        Some(&target),
    );
    app.send_notification(Some(NOTIFICATION_ID), &notification);
    tracing::info!(process = %game.process_name, "offered a profile");
}

/// With Turbo on, the profile falcond applies, as the notification names it;
/// `None` with Turbo off. Reads systemd and falcond's status, so it runs off
/// the main thread.
fn detected_profile() -> Option<String> {
    if !turbo_is_on() {
        return None;
    }
    Some(
        match bigame_core::status::read()
            .and_then(|s| s.active_profile)
            .as_deref()
        {
            None => i18n("no profile yet"),
            Some("Proton") => i18n("falcond's general Proton profile"),
            Some(p) => p.to_owned(),
        },
    )
}

/// A game started and Turbo is handling it: say which profile is in force.
fn notify_detected(app: &adw::Application, game: &GameIdentity, profile: &str) {
    let n = gio::Notification::new(&format!("{} · {}", i18n("Turbo"), game.display_name));
    n.set_body(Some(&format!("{}: {profile}", i18n("Profile"))));
    app.send_notification(Some("game-launch"), &n);
}

/// The profile to offer for the running game `process`. Probing the
/// hardware and capabilities spawns processes and asks D-Bus, and a game is
/// running, so it happens off the main thread.
async fn recommendation_for(process: &str) -> Option<(GameIdentity, Recommendation)> {
    let game = crate::game_watch::current().filter(|g| g.process_name == process)?;
    let probed = game.clone();
    let rec = gio::spawn_blocking(move || {
        recommend::recommend(
            &probed,
            &bigame_core::hardware::Hardware::detect(),
            &bigame_core::capabilities::Capabilities::detect(),
        )
    })
    .await
    .ok()?;
    Some((game, rec))
}

/// What creating a profile came to.
enum Created {
    /// Saved, and falcond reports it active for the running game.
    Active,
    /// Saved; falcond has not switched to it yet (it will at the next start).
    Saved,
    /// Not saved.
    Failed(String),
}

fn save_and_verify(rec: &Recommendation) -> Created {
    let result = bigame_core::dbus_client::daemon_proxy_blocking()
        .and_then(|proxy| Ok(proxy.save_profile(&rec.name, &rec.to_falcond())?));
    if let Err(e) = result {
        return Created::Failed(error_text(&e));
    }
    // falcond reloads and rescans; the specific profile supersedes the
    // generic Proton one for the running game. Seen within a few seconds.
    for _ in 0..40 {
        let active = bigame_core::status::read().and_then(|s| s.active_profile);
        if active.as_deref() == Some(rec.name.as_str()) {
            return Created::Active;
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
    Created::Saved
}

fn create_for(app: &adw::Application, process: &str) {
    app.withdraw_notification(NOTIFICATION_ID);
    let app = app.clone();
    let process = process.to_owned();
    glib::spawn_future_local(async move {
        let Some((game, rec)) = recommendation_for(&process).await else {
            tracing::warn!(
                process,
                "profile requested for a game that is no longer running"
            );
            return;
        };
        let saving = rec.clone();
        let outcome = gio::spawn_blocking(move || save_and_verify(&saving))
            .await
            .unwrap_or_else(|_| Created::Failed(i18n("the worker thread failed")));
        let (title, body) = match &outcome {
            Created::Active => (
                i18n("Profile created and active"),
                format!("{} · {}", game.display_name, rec.name),
            ),
            Created::Saved => (
                i18n("Profile created"),
                i18n("falcond will apply it the next time the game starts."),
            ),
            Created::Failed(e) => (i18n("Could not create the profile"), e.clone()),
        };
        match &outcome {
            Created::Active => {
                tracing::info!(profile = %rec.name, "profile created and verified active");
            }
            Created::Saved => tracing::warn!(profile = %rec.name, "profile saved; not yet active"),
            Created::Failed(e) => {
                tracing::error!(profile = %rec.name, error = %e, "profile not created");
            }
        }
        let notification = gio::Notification::new(&title);
        notification.set_body(Some(&body));
        app.send_notification(Some("profile-result"), &notification);
        crate::game_watch::check();
    });
}

fn show_review(app: &adw::Application, process: &str) {
    let app = app.clone();
    let process = process.to_owned();
    glib::spawn_future_local(async move {
        if let Some((game, rec)) = recommendation_for(&process).await {
            // The falcond profile has no GPU in it; what this GPU gets is
            // AI Graphics' answer, read like the page reads it.
            let probed = process.clone();
            let graphics = gio::spawn_blocking(move || graphics_line(&probed))
                .await
                .ok()
                .flatten();
            present_review(&app, &process, &game, &rec, graphics.as_deref());
        }
    });
}

/// What AI Graphics recommends for the game on the GPU it renders on, in a
/// line: `Radeon RX 9060 XT: the game's own FSR — FSR 4 through Proton…`.
fn graphics_line(process: &str) -> Option<String> {
    use bigame_core::graphics::{self, config, plan::Standing};
    let target = graphics::target_for_process(process)?;
    let cfg = config::AiGraphicsConfig {
        mode: config::Mode::Recommended,
        ..config::AiGraphicsConfig::default()
    };
    let analysis = graphics::analyze(&target, &cfg);
    let mut line = tr(&analysis.plan.summary);
    if matches!(
        analysis.plan.standing,
        Standing::Recommended | Standing::Compatible
    ) {
        // The summary is a phrase, not a sentence.
        if !line.ends_with(['.', '!', '?', '。']) {
            line.push('.');
        }
        line.push(' ');
        line.push_str(&i18n("Apply it in AI Graphics, with the game closed."));
    }
    Some(match analysis.report.gpu() {
        Some(gpu) => format!("{}: {line}", graphics::report::display_name(&gpu.name)),
        None => line,
    })
}

/// A decision as the rest of BiGame-mode names it: the field's title and
/// its value, a scheduler left to Tuning shown with what Tuning runs.
fn decision_row(
    d: &recommend::Decision,
    general: &bigame_core::config::FalcondConfig,
) -> (String, String) {
    use crate::widgets::optimization as o;
    let on_off = |v: &str| if v == "true" { i18n("On") } else { i18n("Off") };
    match d.key.as_str() {
        "name" => (i18n("Process"), d.value.clone()),
        "performance_mode" => (i18n("Performance mode"), on_off(&d.value)),
        "scx_sched" if bigame_core::optimization::inherits(&d.value) => (
            i18n("CPU scheduler"),
            o::inherit_label(&o::scheduler_summary(
                &general.scx_sched,
                &general.scx_sched_props,
            )),
        ),
        "scx_sched" => (i18n("CPU scheduler"), o::scheduler_name(&d.value)),
        "vcache_mode" if d.evidence == recommend::Evidence::Unsupported => {
            (i18n("3D V-Cache"), i18n("Not supported"))
        }
        "vcache_mode" => (i18n("3D V-Cache"), o::vcache_name(&d.value)),
        "idle_inhibit" => (i18n("Keep the screen awake"), on_off(&d.value)),
        _ => (d.key.clone(), d.value.clone()),
    }
}

fn present_review(
    app: &adw::Application,
    process: &str,
    game: &GameIdentity,
    rec: &Recommendation,
    graphics: Option<&str>,
) {
    let window = app
        .active_window()
        .or_else(|| app.windows().into_iter().next());
    if let Some(w) = &window {
        w.set_visible(true);
        w.present();
    }

    let list = gtk4::ListBox::new();
    list.add_css_class("boxed-list");
    list.set_selection_mode(gtk4::SelectionMode::None);
    let general = bigame_core::config::read().unwrap_or_default();
    let row = |title: &str, value: Option<&str>, why: &str| {
        let row = adw::ActionRow::builder()
            .title(title)
            .subtitle(why)
            .use_markup(false)
            .build();
        if let Some(value) = value {
            let label = gtk4::Label::new(Some(value));
            label.add_css_class("dim-label");
            label.set_wrap(true);
            label.set_max_width_chars(20);
            label.set_xalign(1.0);
            label.set_justify(gtk4::Justification::Right);
            row.add_suffix(&label);
        }
        row
    };
    for d in rec.decisions.iter().filter(|d| d.key != "scx_sched_props") {
        let (title, value) = decision_row(d, &general);
        list.append(&row(
            &title,
            Some(&value),
            &format!("{} — {}", i18n(d.evidence.label()), tr(&d.why)),
        ));
    }
    if let Some(line) = graphics {
        list.append(&row(&i18n("AI Graphics"), None, line));
    }
    let never = gtk4::CheckButton::with_label(&i18n("Don't ask again for this game"));
    let body = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
    body.append(&list);
    body.append(&never);

    let dialog = adw::AlertDialog::builder()
        .heading(i18n("%s is running").replace("%s", &game.display_name))
        .body(i18n(
            "Big Game Mode has no profile for this game yet. This is the profile it would create, and why each value was chosen.",
        ))
        .extra_child(&body)
        .build();
    // Room for each reason on a line or two, not a column of words.
    dialog.set_prefer_wide_layout(true);
    // "Not now" leaves the game on falcond's general Proton profile, which is
    // what a separate "Use general optimization" choice also did.
    dialog.add_responses(&[
        ("later", &i18n("Not now")),
        ("create", &i18n("Create profile")),
    ]);
    dialog.set_response_appearance("create", adw::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("create"));
    dialog.set_close_response("later");
    let process = process.to_owned();
    let app = app.clone();
    let anchor = window.clone();
    dialog.connect_response(None, move |_, response| {
        if never.is_active() {
            if let Err(e) = decline_forever(&process) {
                if let Some(w) = &anchor {
                    crate::widgets::toast::error(
                        w,
                        &i18n("Could not save: %s").replace("%s", &e.to_string()),
                        "",
                    );
                }
            }
        }
        if response == "create" {
            create_for(&app, &process);
        } else {
            app.withdraw_notification(NOTIFICATION_ID);
        }
    });
    dialog.present(window.as_ref());
}
