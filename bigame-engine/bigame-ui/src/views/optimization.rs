//! Optimization: everything the machine does for games, in one place.
//!
//! Four tabs, in the order a person reads them:
//!
//! - **Overview** — the machine at a glance and what would help it
//!   (`details::problems`), each recommendation with its state and, when a
//!   page fixes it, a way there;
//! - **Tuning** — the general configuration every game gets
//!   ([`super::tuning`]);
//! - **Details** — what is really in force for the game, item by item, with
//!   the evidence;
//! - **Network and apps** — the connection and DNS, the programs competing
//!   with the game, and Steam's launch options.
//!
//! Simple mode shows Overview and Network and apps; Tuning and Details are
//! advanced. The tabs share one reading of the system, taken only while a
//! tab it feeds is on screen.

use std::rc::Rc;

use adw::prelude::*;
use libadwaita as adw;

use crate::i18n::i18n;
use crate::views::details;
use crate::widgets::layout::Page;
use crate::widgets::tabs::{self, Tabs};

/// The tabs, by name.
pub const TABS: [&str; 4] = ["overview", "tuning", "details", "around"];

/// The tabs shown only in advanced mode.
pub const ADVANCED_TABS: [&str; 2] = ["tuning", "details"];

/// The Optimization page.
pub struct Optimization {
    tabs: Tabs,
    /// Tuning's place: Restore Defaults rebuilds what is in it.
    tuning: gtk4::Box,
}

impl Optimization {
    /// Build the page; its readings start the first time it is on screen.
    #[must_use]
    pub fn build() -> Rc<Self> {
        let tabs = Tabs::new();
        let pages = details::Pages {
            overview: Page::new(),
            details: Page::new(),
            around: Page::new(),
        };
        pages.overview.intro(&i18n(
            "How this machine stands for games, and what would help it. Each recommendation says whether it is recommended, already adequate, not supported here, or needs a measurement to tell.",
        ));
        pages.around.intro(&i18n(
            "The connection and its DNS, the programs that compete with the game, and the Steam launch options that call a missing program. Nothing here changes on its own: a DNS is set, and a program paused or closed, only when you ask.",
        ));
        let tuning = tabs::slot();
        tabs::fill(&tuning, &super::tuning::build());
        tabs.add(
            "overview",
            &i18n("Overview"),
            "speedometer-symbolic",
            pages.overview.widget(),
        );
        tabs.add(
            "tuning",
            &i18n("Tuning"),
            "preferences-system-symbolic",
            &tuning,
        );
        tabs.add(
            "details",
            &i18n("Details"),
            "view-list-bullet-symbolic",
            pages.details.widget(),
        );
        tabs.add(
            "around",
            &i18n("Network and apps"),
            "network-wireless-symbolic",
            pages.around.widget(),
        );
        details::build(pages, tabs.widget().upcast_ref());
        Rc::new(Self { tabs, tuning })
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

    /// Build Tuning again, so it reads back what is now in force.
    pub fn rebuild_tuning(&self) {
        tabs::fill(&self.tuning, &super::tuning::build());
    }

    /// Show or hide the advanced tabs.
    pub fn set_advanced(&self, advanced: bool) {
        for t in ADVANCED_TABS {
            self.tabs.set_shown(t, advanced);
        }
    }
}
