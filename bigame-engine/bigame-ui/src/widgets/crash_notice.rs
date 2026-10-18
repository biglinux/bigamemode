//! The notice that a game closed with an error, sliding down from the top
//! of the window, whichever page is shown.
//!
//! It says what showed the crash, what the game said, and what was in the
//! game besides the game itself ([`crate::crash_watch`]). It offers to leave
//! what can be left out of that one game from its next start, to open the
//! game's own report, and to put the notice away. A crash found while the
//! window was hidden in the tray is shown when it opens.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use gtk4::{gio, glib};
use libadwaita as adw;

use bigame_core::game_crash::Crash;

use crate::crash_watch::{self, Event};
use crate::i18n::i18n;
use crate::widgets::notice::{Kind, Notice};

/// The notice, with the crash it is about.
pub struct CrashNotice {
    revealer: gtk4::Revealer,
    notice: Notice,
    crash: RefCell<Option<Crash>>,
}

impl CrashNotice {
    /// Build it, showing the last crash not yet dismissed, if any.
    #[must_use]
    pub fn new() -> Rc<Self> {
        let notice = Notice::new(Kind::Error, "", "");
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
            crash: RefCell::new(None),
        });
        if let Some((crash, week)) = crash_watch::last() {
            me.show(&crash, week);
        }
        let weak = Rc::downgrade(&me);
        crash_watch::subscribe(move |event| {
            let Some(me) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            match event {
                Event::Crashed(crash, times) => me.show(crash, *times),
                Event::Busy => me.busy(),
                Event::LeftOut { ok, text } => me.left_out(*ok, text),
            }
            glib::ControlFlow::Continue
        });
        me
    }

    /// The widget, for the window to lay over its pages.
    #[must_use]
    pub fn widget(&self) -> &gtk4::Revealer {
        &self.revealer
    }

    fn show(self: &Rc<Self>, crash: &Crash, week: usize) {
        *self.crash.borrow_mut() = Some(crash.clone());
        self.notice.set(
            Kind::Error,
            &crash_watch::title(crash),
            &crash_watch::body(crash, week),
        );
        self.set_actions(crash);
        self.revealer.set_visible(true);
        self.revealer.set_reveal_child(true);
    }

    fn set_actions(self: &Rc<Self>, crash: &Crash) {
        self.notice.clear_actions();
        let weak = Rc::downgrade(self);
        self.notice.add_action(&i18n("Dismiss"), false, move |_| {
            if let Some(me) = weak.upgrade() {
                crash_watch::dismiss();
                me.hide();
            }
        });
        if let Some(dir) = crash.report.clone() {
            self.notice
                .add_action(&i18n("Open the game's report"), false, move |button| {
                    let file = gio::File::for_path(&dir);
                    let window = button.root().and_downcast::<gtk4::Window>();
                    gtk4::FileLauncher::new(Some(&file)).launch(
                        window.as_ref(),
                        gio::Cancellable::NONE,
                        |_| {},
                    );
                });
        }
        if !crash.removable().is_empty() {
            let process = crash.process.clone();
            self.notice
                .add_action(&i18n("Leave them out of this game"), true, move |_| {
                    crash_watch::leave_out(&process);
                });
        }
    }

    fn busy(&self) {
        if let Some(crash) = self.crash.borrow().as_ref() {
            self.notice.clear_actions();
            self.notice.set(
                Kind::Info,
                &crash_watch::title(crash),
                &i18n("Leaving them out of the game's launch settings…"),
            );
        }
    }

    fn left_out(self: &Rc<Self>, ok: bool, text: &str) {
        let Some(crash) = self.crash.borrow().clone() else {
            return;
        };
        if ok {
            self.notice.clear_actions();
            self.notice.set(
                Kind::Success,
                &i18n("Left out of %s").replace("%s", &crash.game),
                text,
            );
            let weak = Rc::downgrade(self);
            self.notice.add_action(&i18n("Dismiss"), false, move |_| {
                if let Some(me) = weak.upgrade() {
                    me.hide();
                }
            });
        } else {
            self.notice.set(
                Kind::Error,
                &i18n("Not everything was left out of %s").replace("%s", &crash.game),
                text,
            );
            // The way to try again.
            self.set_actions(&crash);
        }
        self.revealer.set_visible(true);
        self.revealer.set_reveal_child(true);
    }

    fn hide(&self) {
        self.revealer.set_reveal_child(false);
        self.crash.borrow_mut().take();
    }
}
