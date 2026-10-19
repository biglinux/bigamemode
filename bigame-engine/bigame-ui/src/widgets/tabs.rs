//! A page of tabs: a row of tab buttons over a stack of pages, the way
//! Optimization and Records gather what used to be pages of their own.
//!
//! Each tab is a [`crate::widgets::layout::Page`] (or any widget) with a
//! name, a title and an icon. A tab can be hidden (simple mode hides the
//! advanced ones); a hidden tab that was on screen gives way to the first
//! one. A tab whose content is rebuilt keeps a [`slot`] and changes what
//! is in it, so its place among the tabs never moves.

use adw::prelude::*;
use libadwaita as adw;

/// A page of tabs.
#[derive(Clone)]
pub struct Tabs {
    root: gtk4::Box,
    stack: adw::ViewStack,
}

impl Tabs {
    /// No tabs yet.
    #[must_use]
    pub fn new() -> Self {
        let stack = adw::ViewStack::builder()
            .vexpand(true)
            .hhomogeneous(false)
            .vhomogeneous(false)
            .build();
        let switcher = adw::InlineViewSwitcher::builder()
            .stack(&stack)
            .display_mode(adw::InlineViewSwitcherDisplayMode::Both)
            .halign(gtk4::Align::Start)
            .build();
        switcher.add_css_class("page-tabs");
        // Lined up with the page's cards: the same clamp and margins.
        let clamp = adw::Clamp::builder()
            .maximum_size(1280)
            .tightening_threshold(1280)
            .margin_top(12)
            .margin_start(18)
            .margin_end(18)
            .child(&switcher)
            .build();
        let scroll = gtk4::ScrolledWindow::builder()
            .vscrollbar_policy(gtk4::PolicyType::Never)
            .hscrollbar_policy(gtk4::PolicyType::Automatic)
            .propagate_natural_height(true)
            .child(&clamp)
            .build();
        let root = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        root.append(&scroll);
        root.append(&stack);
        Self { root, stack }
    }

    /// The page, to put in the window.
    #[must_use]
    pub fn widget(&self) -> &gtk4::Box {
        &self.root
    }

    /// Add a tab named `name`.
    pub fn add(&self, name: &str, title: &str, icon: &str, child: &impl IsA<gtk4::Widget>) {
        let page = self.stack.add_titled(child, Some(name), title);
        page.set_icon_name(Some(icon));
    }

    /// The tab on screen.
    #[must_use]
    pub fn current(&self) -> String {
        self.stack
            .visible_child_name()
            .map(|n| n.to_string())
            .unwrap_or_default()
    }

    /// Show the tab `name`, if there is one and it is not hidden.
    pub fn show(&self, name: &str) -> bool {
        let Some(child) = self.stack.child_by_name(name) else {
            return false;
        };
        if !self.stack.page(&child).is_visible() {
            return false;
        }
        self.stack.set_visible_child(&child);
        true
    }

    /// Show or hide the tab `name`.
    pub fn set_shown(&self, name: &str, shown: bool) {
        let Some(child) = self.stack.child_by_name(name) else {
            return;
        };
        let was_current = self.current() == name;
        self.stack.page(&child).set_visible(shown);
        if !shown && was_current {
            self.show_first();
        }
    }

    /// Show the first tab that is not hidden.
    pub fn show_first(&self) {
        let mut child = self.stack.first_child();
        while let Some(c) = child {
            if self.stack.page(&c).is_visible() {
                self.stack.set_visible_child(&c);
                return;
            }
            child = c.next_sibling();
        }
    }

    /// Call `f` with the tab's name whenever the tab on screen changes.
    pub fn connect_changed(&self, f: impl Fn(&str) + 'static) {
        self.stack.connect_visible_child_name_notify(move |s| {
            if let Some(name) = s.visible_child_name() {
                f(&name);
            }
        });
    }
}

/// A tab's content that is rebuilt: put the new one in with [`fill`].
#[must_use]
pub fn slot() -> gtk4::Box {
    let b = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    b.set_vexpand(true);
    b
}

/// Put `child` in `slot`, in place of what was there.
pub fn fill(slot: &gtk4::Box, child: &impl IsA<gtk4::Widget>) {
    while let Some(old) = slot.first_child() {
        slot.remove(&old);
    }
    child.as_ref().set_vexpand(true);
    slot.append(child);
}

impl Default for Tabs {
    fn default() -> Self {
        Self::new()
    }
}
