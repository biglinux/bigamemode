//! The notice that a launcher has to be reopened, sliding down from the top
//! of the window.
//!
//! A launcher keeps the environment it was opened with, and the games it
//! starts inherit it: a Turbo preset, a change of preset, or Turbo switched
//! off reaches them only once the launcher is opened again. Whether one is
//! behind is read from its own process
//! ([`bigame_core::launchers::behind_the_session`]) for Steam, Heroic and
//! Lutris alike, never assumed, so a launcher that does not need it is never
//! asked to reopen.
//!
//! Reopening closes the launcher the way it closes itself and never while it
//! runs a game ([`bigame_core::launchers::reopen`]); what went wrong is said
//! in the notice. "Not now" leaves that launcher alone until Turbo or its
//! preset changes again. With the window hidden in the tray, the desktop's
//! notification says the same, with the same button.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use gtk4::{gio, glib};
use libadwaita as adw;

use bigame_core::launchers::Launcher;

use crate::i18n::{error_text, i18n};
use crate::widgets::notice::{Kind, Notice};

/// The desktop notification's id, so a newer one replaces it.
const NOTIFICATION_ID: &str = "launcher-reopen";

/// How a reopen went: an id of its own, so folding the notice away does not
/// withdraw it with the question.
const RESULT_ID: &str = "launcher-reopen-result";

/// The notice, with what it is asking about.
pub struct LauncherNotice {
    revealer: gtk4::Revealer,
    notice: Notice,
    /// The launchers behind the session, in the order they are asked about.
    queue: RefCell<Vec<Launcher>>,
    /// The ones answered "Not now", until Turbo or its preset changes.
    dismissed: RefCell<Vec<Launcher>>,
    /// A launcher is being reopened: nothing else is asked meanwhile.
    reopening: Cell<bool>,
}

/// The notice's title for `launcher`, one sentence per launcher so each
/// language can agree its words with the name.
fn title(launcher: Launcher) -> String {
    match launcher {
        Launcher::Steam => i18n("Steam needs to be reopened"),
        Launcher::Heroic { .. } => i18n("Heroic needs to be reopened"),
        Launcher::Lutris { .. } => i18n("Lutris needs to be reopened"),
    }
}

fn body() -> String {
    i18n(
        "It keeps the settings it was opened with, so its games get the current Turbo settings only after it is reopened. It is closed the way it closes itself, never while it runs a game.",
    )
}

/// The name a launcher goes by in the `app.reopen-launcher` action.
fn key(launcher: Launcher) -> &'static str {
    match launcher {
        Launcher::Steam => "steam",
        Launcher::Heroic { flatpak: false } => "heroic",
        Launcher::Heroic { flatpak: true } => "heroic-flatpak",
        Launcher::Lutris { flatpak: false } => "lutris",
        Launcher::Lutris { flatpak: true } => "lutris-flatpak",
    }
}

fn from_key(key: &str) -> Option<Launcher> {
    Some(match key {
        "steam" => Launcher::Steam,
        "heroic" => Launcher::Heroic { flatpak: false },
        "heroic-flatpak" => Launcher::Heroic { flatpak: true },
        "lutris" => Launcher::Lutris { flatpak: false },
        "lutris-flatpak" => Launcher::Lutris { flatpak: true },
        _ => return None,
    })
}

impl LauncherNotice {
    /// Build it, hidden, and give `app` the action the desktop
    /// notification's button runs.
    #[must_use]
    pub fn new(app: &adw::Application) -> Rc<Self> {
        let notice = Notice::new(Kind::Info, "", &body());
        notice.widget().add_css_class("top-notice");
        let clamp = adw::Clamp::builder()
            .maximum_size(600)
            .child(notice.widget())
            .margin_top(6)
            .margin_start(12)
            .margin_end(12)
            .build();
        let revealer = gtk4::Revealer::builder()
            .transition_type(gtk4::RevealerTransitionType::SlideDown)
            .transition_duration(250)
            .valign(gtk4::Align::Start)
            .halign(gtk4::Align::Fill)
            .child(&clamp)
            .reveal_child(false)
            .visible(false)
            .build();
        // Hidden once folded away, so it takes no clicks meant for the page.
        revealer.connect_child_revealed_notify(|r| {
            if !r.is_child_revealed() {
                r.set_visible(false);
            }
        });

        let me = Rc::new(Self {
            revealer,
            notice,
            queue: RefCell::new(Vec::new()),
            dismissed: RefCell::new(Vec::new()),
            reopening: Cell::new(false),
        });

        let action = gio::SimpleAction::new("reopen-launcher", Some(glib::VariantTy::STRING));
        {
            let weak = Rc::downgrade(&me);
            action.connect_activate(move |_, target| {
                let launcher = target.and_then(|v| v.str()).and_then(from_key);
                if let (Some(me), Some(launcher)) = (weak.upgrade(), launcher) {
                    me.reopen(launcher);
                }
            });
        }
        app.add_action(&action);
        me
    }

    /// The widget, for the window to lay over its pages.
    #[must_use]
    pub fn widget(&self) -> &gtk4::Revealer {
        &self.revealer
    }

    /// Turbo or its preset changed: read again which launchers are behind,
    /// the ones answered "Not now" before included. `announce`: the change
    /// was asked for (from the tray, with the window hidden), so the
    /// desktop says it too.
    pub fn check(self: &Rc<Self>, announce: bool) {
        self.dismissed.borrow_mut().clear();
        self.read(announce);
    }

    /// Read again without asking anew: a launcher reopened or closed by hand
    /// leaves, one answered "Not now" stays quiet.
    pub fn recheck(self: &Rc<Self>) {
        if self.revealer.is_visible() {
            self.read(false);
        }
    }

    /// Read off the main thread which launchers are behind, and show the
    /// first one not dismissed. `announce`: with the window hidden, say it
    /// in a desktop notification too.
    fn read(self: &Rc<Self>, announce: bool) {
        if self.reopening.get() {
            return;
        }
        let me = Rc::clone(self);
        glib::spawn_future_local(async move {
            let Ok(behind) = gio::spawn_blocking(bigame_core::launchers::behind_the_session).await
            else {
                return;
            };
            if me.reopening.get() {
                return;
            }
            let queue: Vec<Launcher> = behind
                .into_iter()
                .filter(|l| !me.dismissed.borrow().contains(l))
                .collect();
            *me.queue.borrow_mut() = queue;
            me.show_first(announce);
        });
    }

    /// Ask about the first launcher in the queue, or fold away.
    fn show_first(self: &Rc<Self>, announce: bool) {
        let Some(launcher) = self.queue.borrow().first().copied() else {
            self.hide();
            return;
        };
        self.notice.set(Kind::Info, &title(launcher), &body());
        self.set_actions(launcher);
        self.revealer.set_visible(true);
        self.revealer.set_reveal_child(true);
        if announce && !self.window_shown() {
            notify_desktop(launcher);
        }
    }

    /// Whether the window is on screen; hidden in the tray, only the
    /// desktop's notifications reach the user.
    fn window_shown(&self) -> bool {
        self.revealer
            .root()
            .and_downcast::<gtk4::Window>()
            .is_some_and(|w| w.is_visible())
    }

    fn set_actions(self: &Rc<Self>, launcher: Launcher) {
        self.notice.clear_actions();
        let weak = Rc::downgrade(self);
        self.notice.add_action(&i18n("Not now"), false, move |_| {
            if let Some(me) = weak.upgrade() {
                me.dismissed.borrow_mut().push(launcher);
                me.queue.borrow_mut().retain(|l| *l != launcher);
                me.show_first(false);
            }
        });
        let weak = Rc::downgrade(self);
        self.notice.add_action(
            &i18n("Reopen %s").replace("%s", launcher.name()),
            true,
            move |_| {
                if let Some(me) = weak.upgrade() {
                    me.reopen(launcher);
                }
            },
        );
    }

    fn hide(&self) {
        self.revealer.set_reveal_child(false);
        if let Some(app) = gio::Application::default() {
            app.withdraw_notification(NOTIFICATION_ID);
        }
    }

    /// Reopen `launcher` off the main thread and say how it went.
    fn reopen(self: &Rc<Self>, launcher: Launcher) {
        if self.reopening.replace(true) {
            return;
        }
        if let Some(app) = gio::Application::default() {
            app.withdraw_notification(NOTIFICATION_ID);
            app.withdraw_notification(RESULT_ID);
        }
        self.notice.clear_actions();
        self.notice.set(
            Kind::Info,
            &title(launcher),
            &i18n("Closing %s and opening it again…").replace("%s", launcher.name()),
        );
        let me = Rc::clone(self);
        glib::spawn_future_local(async move {
            let done = gio::spawn_blocking(move || bigame_core::launchers::reopen(launcher)).await;
            me.reopening.set(false);
            match done {
                Ok(Ok(())) => {
                    let done = i18n("%s was reopened with the current settings")
                        .replace("%s", launcher.name());
                    // Started from the desktop's notification, with the
                    // window hidden: the answer goes where the question was.
                    if me.window_shown() {
                        crate::widgets::toast::show(&me.revealer, &done);
                    } else {
                        notify_result(&done, None, None);
                    }
                    me.queue.borrow_mut().retain(|l| *l != launcher);
                    me.show_first(false);
                }
                Ok(Err(e)) => {
                    // Said where it was asked, with the way to try again.
                    tracing::warn!(launcher = launcher.name(), error = %format!("{e:#}"), "could not reopen the launcher");
                    let failed =
                        i18n("%s could not be opened again").replace("%s", launcher.name());
                    me.notice.set(Kind::Error, &failed, &error_text(&e));
                    if !me.window_shown() {
                        notify_result(&failed, Some(&error_text(&e)), Some(launcher));
                    }
                    me.set_actions(launcher);
                    me.revealer.set_visible(true);
                    me.revealer.set_reveal_child(true);
                }
                Err(_) => me.show_first(false),
            }
        });
    }
}

/// With the window hidden in the tray, the desktop says what the notice
/// would, with the same button.
fn notify_desktop(launcher: Launcher) {
    let Some(app) = gio::Application::default() else {
        return;
    };
    let n = gio::Notification::new(&title(launcher));
    n.set_body(Some(&body()));
    n.add_button_with_target_value(
        &i18n("Reopen %s").replace("%s", launcher.name()),
        "app.reopen-launcher",
        Some(&key(launcher).to_variant()),
    );
    app.send_notification(Some(NOTIFICATION_ID), &n);
}

/// How a reopen asked from the desktop's notification went, said there too;
/// a failure keeps the button to try again.
fn notify_result(title: &str, body: Option<&str>, retry: Option<Launcher>) {
    let Some(app) = gio::Application::default() else {
        return;
    };
    let n = gio::Notification::new(title);
    n.set_body(body);
    if let Some(launcher) = retry {
        n.add_button_with_target_value(
            &i18n("Reopen %s").replace("%s", launcher.name()),
            "app.reopen-launcher",
            Some(&key(launcher).to_variant()),
        );
    }
    app.send_notification(Some(RESULT_ID), &n);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_launcher_goes_by_a_name_the_action_reads_back() {
        for l in [
            Launcher::Steam,
            Launcher::Heroic { flatpak: false },
            Launcher::Heroic { flatpak: true },
            Launcher::Lutris { flatpak: false },
            Launcher::Lutris { flatpak: true },
        ] {
            assert_eq!(from_key(key(l)), Some(l));
        }
        assert_eq!(from_key("rm -rf"), None);
    }
}
