//! The interface theme: a design (Default or Gamer) and a colour scheme.
//!
//! **Default** is libadwaita plus the application's own stylesheet, the look
//! BiGame-mode always had. **Gamer** is a second stylesheet loaded on top of
//! it, at a higher priority, and unloaded to go back: nothing in Default
//! depends on it, so choosing Default again reproduces Default exactly.
//!
//! The colour scheme is chosen once for both designs. Gamer's light and dark
//! variants are media queries in its one stylesheet, and libadwaita drives
//! the media query from the scheme set here, so there is no second switch
//! and no reload when the desktop changes between light and dark.
//!
//! Gamer is a look, never a behaviour: no widget is built differently for it.
//! With the desktop's high-contrast preference on, Default is shown whatever
//! the choice, because Gamer's tinted surfaces would undo what high contrast
//! is for.

use std::cell::RefCell;

use adw::prelude::*;
use libadwaita as adw;

/// Resource path of the Gamer stylesheet (bundled by build.rs).
const GAMER_CSS: &str = "/com/biglinux/BiGameMode/gamer.css";

/// Above the application's stylesheet, so Gamer wins where both style the
/// same node, and below user overrides (`gtk.css`, priority USER).
const GAMER_PRIORITY: u32 = gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION + 1;

/// The design the interface is drawn with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Design {
    /// libadwaita and the application's stylesheet.
    #[default]
    Default,
    /// The Gamer design system, on top of Default.
    Gamer,
}

impl Design {
    /// Read from `settings.toml`: anything but `"gamer"` is Default, so an
    /// unknown value from a newer version never breaks the window.
    #[must_use]
    pub fn from_setting(value: Option<&str>) -> Self {
        match value {
            Some("gamer") => Self::Gamer,
            _ => Self::Default,
        }
    }

    /// Written to `settings.toml`; Default is the absence of the key.
    #[must_use]
    pub fn to_setting(self) -> Option<String> {
        match self {
            Self::Default => None,
            Self::Gamer => Some("gamer".to_owned()),
        }
    }

    /// The name used for the toggle in the Settings page.
    #[must_use]
    pub fn id(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::Gamer => "gamer",
        }
    }

    /// The design a toggle name stands for.
    #[must_use]
    pub fn from_id(id: &str) -> Self {
        Self::from_setting(Some(id))
    }
}

/// Light, dark, or whatever the desktop uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Scheme {
    /// Follow the desktop.
    #[default]
    System,
    /// Always light.
    Light,
    /// Always dark.
    Dark,
}

impl Scheme {
    /// Read from `settings.toml` (`"light"`, `"dark"`, or absent).
    #[must_use]
    pub fn from_setting(value: Option<&str>) -> Self {
        match value {
            Some("light") => Self::Light,
            Some("dark") => Self::Dark,
            _ => Self::System,
        }
    }

    /// Written to `settings.toml`; System is the absence of the key.
    #[must_use]
    pub fn to_setting(self) -> Option<String> {
        match self {
            Self::System => None,
            Self::Light => Some("light".to_owned()),
            Self::Dark => Some("dark".to_owned()),
        }
    }

    /// The name used for the toggle in the Settings page.
    #[must_use]
    pub fn id(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Light => "light",
            Self::Dark => "dark",
        }
    }

    /// The scheme a toggle name stands for.
    #[must_use]
    pub fn from_id(id: &str) -> Self {
        Self::from_setting(Some(id))
    }

    fn adw(self) -> adw::ColorScheme {
        match self {
            Self::System => adw::ColorScheme::Default,
            Self::Light => adw::ColorScheme::ForceLight,
            Self::Dark => adw::ColorScheme::ForceDark,
        }
    }
}

thread_local! {
    /// The Gamer stylesheet while it is loaded. GTK objects live on the main
    /// thread, and so does every caller of this module.
    static GAMER: RefCell<Option<gtk4::CssProvider>> = const { RefCell::new(None) };
    /// The design last chosen, re-applied when high contrast changes.
    static CHOSEN: std::cell::Cell<Design> = const { std::cell::Cell::new(Design::Default) };
}

/// The design and scheme saved in `settings.toml`.
#[must_use]
pub fn saved() -> (Design, Scheme) {
    let s = crate::settings::load();
    (
        Design::from_setting(s.theme.as_deref()),
        Scheme::from_setting(s.color_scheme.as_deref()),
    )
}

/// Apply the saved theme. Called once at start-up, after the application's
/// own stylesheet is registered.
pub fn apply_saved() {
    let (design, scheme) = saved();
    adw::StyleManager::default().set_color_scheme(scheme.adw());
    set_design_now(design);

    // High contrast can be switched while the application runs.
    adw::StyleManager::default().connect_high_contrast_notify(|_| {
        set_design_now(CHOSEN.with(std::cell::Cell::get));
    });
}

/// Choose a design, apply it and keep it for the next start.
pub fn set_design(design: Design) {
    set_design_now(design);
    let mut s = crate::settings::load();
    s.theme = design.to_setting();
    crate::settings::save(&s);
}

/// Choose a colour scheme, apply it and keep it for the next start.
pub fn set_scheme(scheme: Scheme) {
    adw::StyleManager::default().set_color_scheme(scheme.adw());
    let mut s = crate::settings::load();
    s.color_scheme = scheme.to_setting();
    crate::settings::save(&s);
    redraw_all();
}

/// Whether the design shown differs from the one chosen (high contrast).
#[must_use]
pub fn gamer_suspended() -> bool {
    CHOSEN.with(std::cell::Cell::get) == Design::Gamer
        && adw::StyleManager::default().is_high_contrast()
}

fn set_design_now(design: Design) {
    CHOSEN.with(|c| c.set(design));
    let Some(display) = gtk4::gdk::Display::default() else {
        return;
    };
    let wanted = design == Design::Gamer && !adw::StyleManager::default().is_high_contrast();
    GAMER.with(|slot| {
        let mut slot = slot.borrow_mut();
        match (wanted, slot.as_ref()) {
            (true, None) => {
                let provider = gtk4::CssProvider::new();
                follow_scheme(&provider);
                provider.load_from_resource(GAMER_CSS);
                gtk4::style_context_add_provider_for_display(&display, &provider, GAMER_PRIORITY);
                *slot = Some(provider);
            }
            (false, Some(provider)) => {
                gtk4::style_context_remove_provider_for_display(&display, provider);
                *slot = None;
            }
            _ => return,
        }
        drop(slot);
        redraw_all();
    });
}

/// Media queries in an application stylesheet answer from the provider's
/// own `prefers-color-scheme` (GTK 4.20), which libadwaita sets only on its
/// stylesheet; the Gamer provider is told the scheme libadwaita resolved,
/// now and whenever it changes.
fn follow_scheme(provider: &gtk4::CssProvider) {
    fn set(provider: &gtk4::CssProvider, dark: bool) {
        // GtkInterfaceColorScheme is newer than the gtk4-rs bindings, so the
        // value is built from the enum's registered type.
        let value = gtk4::glib::Type::from_name("GtkInterfaceColorScheme")
            .and_then(gtk4::glib::EnumClass::with_type)
            .and_then(|class| class.to_value_by_nick(if dark { "dark" } else { "light" }));
        if let (Some(value), Some(_)) = (value, provider.find_property("prefers-color-scheme")) {
            provider.set_property_from_value("prefers-color-scheme", &value);
        }
    }
    let manager = adw::StyleManager::default();
    set(provider, manager.is_dark());
    let weak = provider.downgrade();
    manager.connect_dark_notify(move |m| {
        if let Some(provider) = weak.upgrade() {
            set(&provider, m.is_dark());
        }
    });
}

/// Charts are drawn by code that reads the palette when it paints; a style
/// change does not repaint them by itself, so every widget is asked to.
fn redraw_all() {
    fn walk(widget: &gtk4::Widget) {
        widget.queue_draw();
        let mut child = widget.first_child();
        while let Some(c) = child {
            walk(&c);
            child = c.next_sibling();
        }
    }
    for window in gtk4::Window::list_toplevels() {
        walk(&window);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn design_round_trips_through_the_settings_file() {
        for d in [Design::Default, Design::Gamer] {
            assert_eq!(Design::from_setting(d.to_setting().as_deref()), d);
            assert_eq!(Design::from_id(d.id()), d);
        }
    }

    #[test]
    fn scheme_round_trips_through_the_settings_file() {
        for s in [Scheme::System, Scheme::Light, Scheme::Dark] {
            assert_eq!(Scheme::from_setting(s.to_setting().as_deref()), s);
            assert_eq!(Scheme::from_id(s.id()), s);
        }
    }

    #[test]
    fn unknown_values_fall_back_to_the_defaults() {
        assert_eq!(Design::from_setting(Some("neon")), Design::Default);
        assert_eq!(Design::from_setting(None), Design::Default);
        assert_eq!(Scheme::from_setting(Some("sepia")), Scheme::System);
    }

    #[test]
    fn defaults_are_not_written_to_the_file() {
        assert_eq!(Design::Default.to_setting(), None);
        assert_eq!(Scheme::System.to_setting(), None);
    }
}
