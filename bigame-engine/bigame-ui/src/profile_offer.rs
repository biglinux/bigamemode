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

use crate::i18n::i18n;

const NOTIFICATION_ID: &str = "profile-offer";

/// How long a game must have been running before a profile is offered.
const SETTLE_SECS: u64 = 20;

thread_local! {
    static OFFERED: RefCell<HashSet<String>> = RefCell::new(HashSet::new());
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
    let turbo_on = bigame_core::systemd::Reader::system()
        .and_then(|r| r.unit_state(bigame_core::turbo::BACKEND_UNIT))
        .is_some_and(|u| u.is_active());
    if !turbo_on {
        return false;
    }
    // A process that has only just started may be a helper that runs before
    // the game -- Steam's installer script did exactly that -- so the offer
    // waits until the process has lived a while.
    if bigame_core::running::running_for(game.pid).is_none_or(|secs| secs < SETTLE_SECS) {
        return false;
    }
    if OFFERED.with(|o| o.borrow().contains(&game.process_name))
        || declined().contains(&game.process_name)
    {
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
        move |game| {
            let Some(game) = game.cloned() else {
                app.withdraw_notification(NOTIFICATION_ID);
                if let Some(name) = LAST_GAME.with(|g| g.borrow_mut().take()) {
                    if crate::settings::load().notifications_enabled {
                        let n = gio::Notification::new(&format!("{name} {}", i18n("closed")));
                        n.set_body(Some(&i18n(
                            "Everything the game's profile changed has been put back.",
                        )));
                        app.send_notification(Some("game-exit"), &n);
                    }
                }
                return;
            };
            LAST_GAME.with(|g| *g.borrow_mut() = Some(game.display_name.clone()));
            glib::spawn_future_local(async move {
                // Wait until the process has settled, then ask again whether it
                // is still the running game.
                let age = bigame_core::running::running_for(game.pid).unwrap_or(0);
                if age < SETTLE_SECS {
                    glib::timeout_future_seconds(u32::try_from(SETTLE_SECS - age).unwrap_or(20))
                        .await;
                }
                if crate::game_watch::current().map(|g| g.pid) != Some(game.pid) {
                    return;
                }
                let check = game.clone();
                let offer = gio::spawn_blocking(move || should_offer(&check))
                    .await
                    .unwrap_or(false);
                if offer {
                    OFFERED.with(|o| o.borrow_mut().insert(game.process_name.clone()));
                    notify_offer(&app, &game);
                } else if crate::settings::load().notifications_enabled {
                    notify_detected(&app, &game);
                }
            });
        }
    ));
}

fn notify_offer(app: &adw::Application, game: &GameIdentity) {
    let target = game.process_name.to_variant();
    let notification =
        gio::Notification::new(&format!("{} {}", game.display_name, i18n("is running")));
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

/// A game started and Turbo is handling it: say which profile is in force.
fn notify_detected(app: &adw::Application, game: &GameIdentity) {
    let turbo_on = bigame_core::systemd::Reader::system()
        .and_then(|r| r.unit_state(bigame_core::turbo::BACKEND_UNIT))
        .is_some_and(|u| u.is_active());
    if !turbo_on {
        return;
    }
    let profile = bigame_core::status::read()
        .and_then(|s| s.active_profile)
        .map_or_else(
            || i18n("no profile yet"),
            |p| {
                if p == "Proton" {
                    i18n("falcond's general Proton profile")
                } else {
                    p
                }
            },
        );
    let n = gio::Notification::new(&format!("{} · {}", i18n("Turbo"), game.display_name));
    n.set_body(Some(&format!("{}: {profile}", i18n("Profile"))));
    app.send_notification(Some("game-launch"), &n);
}

fn recommendation_for(process: &str) -> Option<(GameIdentity, Recommendation)> {
    let game = crate::game_watch::current().filter(|g| g.process_name == process)?;
    let rec = recommend::recommend(
        &game,
        &bigame_core::hardware::Hardware::detect(),
        &bigame_core::capabilities::Capabilities::detect(),
    );
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
        return Created::Failed(format!("{e:#}"));
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
    let Some((game, rec)) = recommendation_for(process) else {
        tracing::warn!(
            process,
            "profile requested for a game that is no longer running"
        );
        return;
    };
    let app = app.clone();
    glib::spawn_future_local(async move {
        let saving = rec.clone();
        let outcome = gio::spawn_blocking(move || save_and_verify(&saving))
            .await
            .unwrap_or_else(|_| Created::Failed("the worker thread failed".into()));
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
    let Some((game, rec)) = recommendation_for(process) else {
        return;
    };
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
            .subtitle(format!("{} — {}", i18n(d.evidence.label()), d.why))
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
        .heading(format!("{} {}", game.display_name, i18n("is running")))
        .body(i18n(
            "BiGame-mode has no profile for this game yet. This is the profile it would create, and why each value was chosen.",
        ))
        .extra_child(&body)
        .build();
    dialog.add_responses(&[
        ("later", &i18n("Not now")),
        ("general", &i18n("Use general optimization")),
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
