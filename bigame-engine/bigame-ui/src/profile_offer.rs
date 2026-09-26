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

fn decline_forever(process: &str) {
    let mut set = declined();
    set.insert(process.to_owned());
    if let Some(path) = never_path() {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(json) = serde_json::to_vec_pretty(&set) {
            let _ = std::fs::write(path, json);
        }
    }
    tracing::info!(process, "will not offer a profile for this game again");
}

/// Whether this game should be offered a profile now.
fn should_offer(game: &GameIdentity) -> bool {
    if !crate::settings::load().offer_profiles {
        return false;
    }
    let turbo_on = bigame_core::systemd::Reader::shared()
        .and_then(|r| r.unit_state(bigame_core::turbo::BACKEND_UNIT))
        .is_some_and(|u| u.is_active());
    if !turbo_on {
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
    bigame_core::running::matching_profile(&game.process_name, &mode).is_none()
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
                decline_forever(process);
                app.withdraw_notification(NOTIFICATION_ID);
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
                let n = gio::Notification::new(&i18n("%s closed").replace("%s", &name));
                n.set_body(Some(&i18n(
                    "Everything the game's profile changed has been put back.",
                )));
                app.send_notification(Some("game-exit"), &n);
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
        "BiGame-mode has no profile for this game yet. Create one tuned for this machine?",
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
    let turbo_on = bigame_core::systemd::Reader::shared()
        .and_then(|r| r.unit_state(bigame_core::turbo::BACKEND_UNIT))
        .is_some_and(|u| u.is_active());
    if !turbo_on {
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
            present_review(&app, &process, &game, &rec);
        }
    });
}

fn present_review(
    app: &adw::Application,
    process: &str,
    game: &GameIdentity,
    rec: &Recommendation,
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
    for d in rec.decisions.iter().filter(|d| d.key != "scx_sched_props") {
        let row = adw::ActionRow::builder()
            .title(format!("{} = {}", d.key, d.value))
            .subtitle(format!("{} — {}", i18n(d.evidence.label()), tr(&d.why)))
            .subtitle_lines(4)
            .use_markup(false)
            .build();
        list.append(&row);
    }
    let never = gtk4::CheckButton::with_label(&i18n("Don't ask again for this game"));
    let body = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
    body.append(&list);
    body.append(&never);

    let dialog = adw::AlertDialog::builder()
        .heading(i18n("%s is running").replace("%s", &game.display_name))
        .body(i18n(
            "BiGame-mode has no profile for this game yet. This is the profile it would create, and why each value was chosen.",
        ))
        .extra_child(&body)
        .build();
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
    dialog.connect_response(None, move |_, response| {
        if never.is_active() {
            decline_forever(&process);
        }
        if response == "create" {
            create_for(&app, &process);
        } else {
            app.withdraw_notification(NOTIFICATION_ID);
        }
    });
    dialog.present(window.as_ref());
}
