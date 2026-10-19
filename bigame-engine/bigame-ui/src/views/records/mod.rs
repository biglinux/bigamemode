//! Records: what happened, kept to read afterwards.
//!
//! Three tabs:
//!
//! - **Reports** — the last Turbo report (what Turbo applied, left to each
//!   game, put back, and why) and the support report to paste into a forum
//!   or an issue;
//! - **Games that crashed** — the games that closed with an error, with
//!   what showed it and what else was in the game (`crashes`);
//! - **Logs** — the journal and log files of a game session
//!   ([`super::logs`]), in advanced mode.

mod crashes;

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use gtk4::glib;
use libadwaita as adw;

use bigame_core::turbo::Report;

use crate::i18n::i18n;
use crate::widgets::layout::Page;
use crate::widgets::tabs::{self, Tabs};

/// The tabs, by name.
pub const TABS: [&str; 3] = ["reports", "crashes", "logs"];

/// The tabs shown only in advanced mode.
pub const ADVANCED_TABS: [&str; 1] = ["logs"];

/// The Records page.
pub struct Records {
    tabs: Tabs,
    /// The Reports tab's place: its page is built again whenever it comes
    /// on screen, so "Right now" is now.
    reports: gtk4::Box,
    /// The Turbo report shown there.
    report: RefCell<Option<Report>>,
    /// The support report: built once and moved into each new page, so it
    /// is generated once, the first time it is seen, not with every page.
    support: adw::PreferencesGroup,
}

impl Records {
    /// Build the page, with the last Turbo report there is.
    #[must_use]
    pub fn build() -> Rc<Self> {
        let tabs = Tabs::new();
        let reports = tabs::slot();
        tabs.add(
            "reports",
            &i18n("Reports"),
            "view-list-bullet-symbolic",
            &reports,
        );
        let crashes = Page::new();
        crashes.add(&crashes::group());
        tabs.add(
            "crashes",
            &i18n("Games that crashed"),
            "dialog-error-symbolic",
            crashes.widget(),
        );
        tabs.add(
            "logs",
            &i18n("Logs"),
            "utilities-terminal-symbolic",
            &super::logs::build(),
        );
        let me = Rc::new(Self {
            tabs,
            reports,
            report: RefCell::new(Report::load_last()),
            support: super::details::extras::report_group(),
        });
        me.rebuild();
        // On screen again: the page is built anew, after the map, so what
        // is in force right now is read now.
        {
            let weak = Rc::downgrade(&me);
            me.reports.connect_map(move |_| {
                let weak = weak.clone();
                glib::idle_add_local_once(move || {
                    if let Some(me) = weak.upgrade() {
                        me.rebuild();
                    }
                });
            });
        }
        // A game starting or ending while the report is on screen.
        {
            let weak = Rc::downgrade(&me);
            crate::game_watch::subscribe(move |_| {
                let Some(me) = weak.upgrade() else {
                    return glib::ControlFlow::Break;
                };
                if me.reports.is_mapped() {
                    me.rebuild();
                }
                glib::ControlFlow::Continue
            });
        }
        me
    }

    /// The page, to put in the window.
    #[must_use]
    pub fn widget(&self) -> &gtk4::Box {
        self.tabs.widget()
    }

    /// The tabs.
    #[must_use]
    pub fn tabs(&self) -> &Tabs {
        &self.tabs
    }

    /// Keep `report` for the Reports tab; `open`: show the tab too. Built
    /// now only if it is on screen and stays so; otherwise when it is next
    /// shown.
    pub fn show_report(&self, report: &Report, open: bool) {
        *self.report.borrow_mut() = Some(report.clone());
        if open {
            if self.reports.is_mapped() {
                self.rebuild();
            } else {
                self.tabs.show("reports");
            }
        } else if self.reports.is_mapped() {
            self.rebuild();
        }
    }

    /// Show or hide the advanced tabs.
    pub fn set_advanced(&self, advanced: bool) {
        for t in ADVANCED_TABS {
            self.tabs.set_shown(t, advanced);
        }
    }

    /// The Reports tab: the Turbo report, then the support report.
    fn rebuild(&self) {
        let page = super::report::page(self.report.borrow().as_ref());
        page.section(&i18n("For support"));
        if let Some(old) = self.support.parent().and_downcast::<gtk4::Box>() {
            old.remove(&self.support);
        }
        page.add(&self.support);
        tabs::fill(&self.reports, page.widget());
    }
}
