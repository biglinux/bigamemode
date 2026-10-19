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

use std::rc::Rc;

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
    /// The Reports tab's place: rebuilt for every new Turbo report.
    reports: gtk4::Box,
}

impl Records {
    /// Build the page, with the last Turbo report there is.
    #[must_use]
    pub fn build() -> Rc<Self> {
        let tabs = Tabs::new();
        let reports = tabs::slot();
        tabs::fill(
            &reports,
            reports_page(Report::load_last().as_ref()).widget(),
        );
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
        Rc::new(Self { tabs, reports })
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

    /// Put `report` in the Reports tab; `open`: show the tab too.
    pub fn show_report(&self, report: &Report, open: bool) {
        tabs::fill(&self.reports, reports_page(Some(report)).widget());
        if open {
            self.tabs.show("reports");
        }
    }

    /// Show or hide the advanced tabs.
    pub fn set_advanced(&self, advanced: bool) {
        for t in ADVANCED_TABS {
            self.tabs.set_shown(t, advanced);
        }
    }
}

/// The Reports tab: the Turbo report, then the support report.
fn reports_page(report: Option<&Report>) -> Page {
    let page = super::report::page(report);
    page.section(&i18n("For support"));
    page.add(&super::details::extras::report_group());
    page
}
