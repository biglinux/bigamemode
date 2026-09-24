//! The Turbo Mode control.
//!
//! This replaces an `AdwSwitchRow` whose entire implementation was one
//! discarded D-Bus call. A switch is the wrong affordance for this: it implies
//! a setting that is simply on or off, when what actually happens is a
//! multi-step operation that can succeed, partly succeed, or fail, and that
//! takes long enough to need feedback while it runs.
//!
//! So it is a large button with an explicit state machine. Every state
//! corresponds to something the engine is really doing — there is no timed
//! animation standing in for work, and the control never shows "active" for an
//! operation that did not verify.

use adw::prelude::*;
use libadwaita as adw;

use crate::i18n::i18n;

/// What the control is showing.
///
/// Turbo is the master switch: off means BiGame-mode is not intervening in
/// games at all; on means it may detect games and optimize them. "On" is not
/// a count of changes -- on a machine that is already well configured Turbo
/// can be on with nothing global to change, because the work happens per game.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum State {
    /// Turbo is off.
    Off,
    /// A transition is running.
    Working {
        /// What is happening right now.
        step: String,
    },
    /// Turbo is on.
    On {
        /// What it is doing: watching for games, or optimizing one.
        detail: String,
    },
    /// On, but something that was attempted did not take effect.
    Partial {
        /// What failed.
        detail: String,
    },
    /// Turbo could not be turned on.
    Error {
        /// Why.
        detail: String,
    },
    /// Turning off.
    Restoring,
}

impl State {
    /// Heading shown inside the button.
    #[must_use]
    pub fn title(&self) -> String {
        match self {
            Self::Off => i18n("Turbo Mode"),
            Self::Working { .. } => i18n("Turning On"),
            Self::On { .. } | Self::Partial { .. } => i18n("Turbo Mode On"),
            Self::Error { .. } => i18n("Turbo Could Not Start"),
            Self::Restoring => i18n("Turning Off"),
        }
    }

    /// Supporting line shown under the heading.
    #[must_use]
    pub fn subtitle(&self) -> String {
        match self {
            Self::Off => i18n("Off · BiGame-mode is not intervening in games"),
            Self::Working { step } => step.clone(),
            Self::On { detail } | Self::Partial { detail } | Self::Error { detail } => {
                detail.clone()
            }
            Self::Restoring => i18n("Putting everything back as it was"),
        }
    }

    /// Symbolic icon name for this state.
    #[must_use]
    pub fn icon(&self) -> &'static str {
        match self {
            Self::Off => "power-profile-performance-symbolic",
            Self::Working { .. } | Self::Restoring => "content-loading-symbolic",
            Self::On { .. } => "object-select-symbolic",
            Self::Partial { .. } => "dialog-warning-symbolic",
            Self::Error { .. } => "dialog-error-symbolic",
        }
    }

    /// CSS class carrying this state's colour treatment.
    #[must_use]
    pub fn css_class(&self) -> &'static str {
        match self {
            Self::Off => "booster-ready",
            Self::Working { .. } | Self::Restoring => "booster-working",
            Self::On { .. } => "booster-active",
            Self::Partial { .. } => "booster-partial",
            Self::Error { .. } => "booster-error",
        }
    }

    /// Whether the control accepts input in this state.
    ///
    /// Transient states are not clickable: letting someone start a second run
    /// while the first is halfway through is how a machine ends up in a state
    /// no snapshot describes.
    #[must_use]
    pub fn is_interactive(&self) -> bool {
        !matches!(self, Self::Working { .. } | Self::Restoring)
    }

    /// Whether Turbo is on, and therefore whether a click turns it off.
    #[must_use]
    pub fn is_on(&self) -> bool {
        matches!(self, Self::On { .. } | Self::Partial { .. })
    }

    /// Every CSS class this widget may carry, so the old one can be removed
    /// without the caller tracking which it was.
    #[must_use]
    pub fn all_css_classes() -> [&'static str; 5] {
        [
            "booster-ready",
            "booster-working",
            "booster-active",
            "booster-partial",
            "booster-error",
        ]
    }
}

/// The Turbo Mode control.
pub struct BoosterButton {
    button: gtk4::Button,
    icon: gtk4::Image,
    title: gtk4::Label,
    subtitle: gtk4::Label,
    spinner: adw::Spinner,
    state: std::cell::RefCell<State>,
}

impl BoosterButton {
    /// Build the control in its [`State::Off`] state.
    #[must_use]
    pub fn new() -> std::rc::Rc<Self> {
        let icon = gtk4::Image::from_icon_name(State::Off.icon());
        icon.set_pixel_size(48);

        let spinner = adw::Spinner::new();
        spinner.set_size_request(48, 48);
        spinner.set_visible(false);

        let art = gtk4::Overlay::new();
        art.set_child(Some(&icon));
        art.add_overlay(&spinner);
        art.set_halign(gtk4::Align::Center);

        let title = gtk4::Label::new(Some(&State::Off.title()));
        title.add_css_class("title-1");

        let subtitle = gtk4::Label::new(Some(&State::Off.subtitle()));
        subtitle.add_css_class("dim-label");
        subtitle.set_wrap(true);
        subtitle.set_justify(gtk4::Justification::Center);
        subtitle.set_max_width_chars(34);

        let content = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
        content.set_halign(gtk4::Align::Center);
        content.set_valign(gtk4::Align::Center);
        content.append(&art);
        content.append(&title);
        content.append(&subtitle);

        let button = gtk4::Button::builder()
            .child(&content)
            .halign(gtk4::Align::Center)
            .valign(gtk4::Align::Center)
            .width_request(280)
            .height_request(280)
            .css_classes(["booster-button", "booster-ready"])
            .build();

        // Screen readers announce the state, not just the word "button".
        button.update_property(&[
            gtk4::accessible::Property::Label(&State::Off.title()),
            gtk4::accessible::Property::Description(&State::Off.subtitle()),
        ]);

        std::rc::Rc::new(Self {
            button,
            icon,
            title,
            subtitle,
            spinner,
            state: std::cell::RefCell::new(State::Off),
        })
    }

    /// The underlying widget, for packing into a container.
    #[must_use]
    pub fn widget(&self) -> &gtk4::Button {
        &self.button
    }

    /// The state currently displayed.
    #[must_use]
    pub fn state(&self) -> State {
        self.state.borrow().clone()
    }

    /// Move the control to `state` and update everything that depends on it.
    pub fn set_state(&self, state: &State) {
        for class in State::all_css_classes() {
            self.button.remove_css_class(class);
        }
        self.button.add_css_class(state.css_class());

        let working = !state.is_interactive();
        self.spinner.set_visible(working);
        self.icon.set_visible(!working);
        if !working {
            self.icon.set_icon_name(Some(state.icon()));
        }

        let title = state.title();
        let subtitle = state.subtitle();
        self.title.set_label(&title);
        self.subtitle.set_label(&subtitle);
        self.button.set_sensitive(state.is_interactive());

        self.button.update_property(&[
            gtk4::accessible::Property::Label(&title),
            gtk4::accessible::Property::Description(&subtitle),
        ]);

        *self.state.borrow_mut() = state.clone();
    }

    /// Run `handler` when the control is activated.
    pub fn connect_activated<F: Fn() + 'static>(self: &std::rc::Rc<Self>, handler: F) {
        self.button.connect_clicked(move |_| handler());
    }
}

/// Whether the desktop has asked for reduced motion.
///
/// GTK exposes this through `gtk-enable-animations`, which the platform sets
/// from the accessibility preference. Honouring it is why the pulse animation
/// is applied through a CSS class rather than hardcoded into the widget.
#[must_use]
pub fn animations_enabled() -> bool {
    gtk4::Settings::default().is_some_and(|s| s.is_gtk_enable_animations())
}

/// Apply or remove the idle pulse, respecting the reduced-motion preference.
pub fn set_pulse(button: &gtk4::Button, pulsing: bool) {
    if pulsing && animations_enabled() {
        button.add_css_class("booster-pulse");
    } else {
        button.remove_css_class("booster-pulse");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all() -> Vec<State> {
        vec![
            State::Off,
            State::Working { step: "x".into() },
            State::On {
                detail: "Watching for games".into(),
            },
            State::Partial {
                detail: "1 thing failed".into(),
            },
            State::Error {
                detail: "daemon unreachable".into(),
            },
            State::Restoring,
        ]
    }

    #[test]
    fn transient_states_are_not_clickable() {
        assert!(!State::Working { step: "x".into() }.is_interactive());
        assert!(!State::Restoring.is_interactive());
        assert!(State::Off.is_interactive());
        assert!(State::On { detail: "x".into() }.is_interactive());
        assert!(State::Error { detail: "x".into() }.is_interactive());
    }

    #[test]
    fn on_means_a_click_turns_it_off() {
        assert!(State::On { detail: "x".into() }.is_on());
        assert!(State::Partial { detail: "x".into() }.is_on());
        assert!(!State::Off.is_on());
        assert!(!State::Error { detail: "x".into() }.is_on());
    }

    #[test]
    fn every_state_has_text_and_a_tracked_class() {
        let classes = State::all_css_classes();
        for state in all() {
            assert!(!state.title().is_empty(), "{state:?} has no title");
            assert!(!state.subtitle().is_empty(), "{state:?} has no subtitle");
            assert!(!state.icon().is_empty());
            assert!(classes.contains(&state.css_class()), "{state:?}");
        }
    }
}
