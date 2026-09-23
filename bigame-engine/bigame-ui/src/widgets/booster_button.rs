//! The Booster Mode control.
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum State {
    /// Idle, ready to be activated.
    Ready,
    /// Reading hardware and capabilities.
    Analyzing,
    /// Applying and verifying changes.
    Optimizing {
        /// What is being changed right now.
        step: String,
    },
    /// Every planned change applied and verified.
    Active {
        /// How many.
        count: usize,
    },
    /// Nothing needed changing — the machine was already configured well.
    AlreadyOptimal,
    /// Some changes took and some did not.
    Partial {
        /// Verified.
        applied: usize,
        /// Attempted.
        total: usize,
    },
    /// Nothing could be applied.
    Error {
        /// Why.
        detail: String,
    },
    /// Putting the captured baseline back.
    Restoring,
}

impl State {
    /// Heading shown inside the button.
    #[must_use]
    pub fn title(&self) -> String {
        match self {
            Self::Ready => i18n("Booster Mode"),
            Self::Analyzing => i18n("Analyzing"),
            Self::Optimizing { .. } => i18n("Optimizing"),
            Self::Active { .. } => i18n("Booster Mode Active"),
            Self::AlreadyOptimal => i18n("Already Optimal"),
            Self::Partial { .. } => i18n("Partly Applied"),
            Self::Error { .. } => i18n("Could Not Optimize"),
            Self::Restoring => i18n("Restoring"),
        }
    }

    /// Supporting line shown under the heading.
    #[must_use]
    pub fn subtitle(&self) -> String {
        match self {
            Self::Ready => i18n("Tap to prepare this system for gaming"),
            Self::Analyzing => i18n("Reading hardware and capabilities"),
            Self::Optimizing { step } => step.clone(),
            Self::Active { count } => {
                // Translators: {} is the number of verified optimizations.
                ngettext_count(*count, "1 optimization active", "{} optimizations active")
            }
            Self::AlreadyOptimal => i18n("No changes were needed"),
            Self::Partial { applied, total } => format!(
                "{} {} {}",
                applied,
                i18n("of"),
                // Translators: completes "N of M applied".
                format_args!("{total} {}", i18n("applied"))
            ),
            Self::Error { detail } => detail.clone(),
            Self::Restoring => i18n("Returning to your previous settings"),
        }
    }

    /// Symbolic icon name for this state.
    #[must_use]
    pub fn icon(&self) -> &'static str {
        match self {
            Self::Ready => "power-profile-performance-symbolic",
            Self::Analyzing | Self::Optimizing { .. } | Self::Restoring => {
                "content-loading-symbolic"
            }
            Self::Active { .. } | Self::AlreadyOptimal => "emblem-ok-symbolic",
            Self::Partial { .. } => "dialog-warning-symbolic",
            Self::Error { .. } => "dialog-error-symbolic",
        }
    }

    /// CSS class carrying this state's colour treatment.
    #[must_use]
    pub fn css_class(&self) -> &'static str {
        match self {
            Self::Ready => "booster-ready",
            Self::Analyzing | Self::Optimizing { .. } | Self::Restoring => "booster-working",
            Self::Active { .. } | Self::AlreadyOptimal => "booster-active",
            Self::Partial { .. } => "booster-partial",
            Self::Error { .. } => "booster-error",
        }
    }

    /// Whether the control accepts input in this state.
    ///
    /// Transient states are not clickable: letting someone start a second run
    /// while the first is halfway through applying changes is how a machine
    /// ends up in a state no snapshot describes.
    #[must_use]
    pub fn is_interactive(&self) -> bool {
        !matches!(
            self,
            Self::Analyzing | Self::Optimizing { .. } | Self::Restoring
        )
    }

    /// Whether Booster is on, and therefore whether a click turns it off.
    #[must_use]
    pub fn is_on(&self) -> bool {
        matches!(self, Self::Active { .. } | Self::Partial { .. })
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

/// Simple plural selection.
///
/// gettext's `ngettext` is the right tool once translations carry plural forms;
/// until the catalogue does, this keeps the English strings correct rather than
/// emitting "1 optimizations".
fn ngettext_count(n: usize, one: &str, many: &str) -> String {
    if n == 1 {
        i18n(one)
    } else {
        i18n(many).replace("{}", &n.to_string())
    }
}

/// The Booster Mode control.
pub struct BoosterButton {
    button: gtk4::Button,
    icon: gtk4::Image,
    title: gtk4::Label,
    subtitle: gtk4::Label,
    spinner: adw::Spinner,
    state: std::cell::RefCell<State>,
}

impl BoosterButton {
    /// Build the control in its [`State::Ready`] state.
    #[must_use]
    pub fn new() -> std::rc::Rc<Self> {
        let icon = gtk4::Image::from_icon_name(State::Ready.icon());
        icon.set_pixel_size(48);

        let spinner = adw::Spinner::new();
        spinner.set_size_request(48, 48);
        spinner.set_visible(false);

        let art = gtk4::Overlay::new();
        art.set_child(Some(&icon));
        art.add_overlay(&spinner);
        art.set_halign(gtk4::Align::Center);

        let title = gtk4::Label::new(Some(&State::Ready.title()));
        title.add_css_class("title-1");

        let subtitle = gtk4::Label::new(Some(&State::Ready.subtitle()));
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
            gtk4::accessible::Property::Label(&State::Ready.title()),
            gtk4::accessible::Property::Description(&State::Ready.subtitle()),
        ]);

        std::rc::Rc::new(Self {
            button,
            icon,
            title,
            subtitle,
            spinner,
            state: std::cell::RefCell::new(State::Ready),
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

    #[test]
    fn transient_states_are_not_clickable() {
        // Starting a second run mid-apply would leave the machine in a state
        // no snapshot describes.
        assert!(!State::Analyzing.is_interactive());
        assert!(!State::Optimizing { step: "x".into() }.is_interactive());
        assert!(!State::Restoring.is_interactive());

        assert!(State::Ready.is_interactive());
        assert!(State::Active { count: 3 }.is_interactive());
        assert!(State::AlreadyOptimal.is_interactive());
        assert!(
            State::Partial {
                applied: 1,
                total: 2
            }
            .is_interactive()
        );
        assert!(State::Error { detail: "x".into() }.is_interactive());
    }

    #[test]
    fn only_applied_states_count_as_on() {
        assert!(State::Active { count: 1 }.is_on());
        assert!(
            State::Partial {
                applied: 1,
                total: 2
            }
            .is_on()
        );

        // "Already optimal" changed nothing, so there is nothing to turn off.
        assert!(!State::AlreadyOptimal.is_on());
        assert!(!State::Ready.is_on());
        assert!(!State::Error { detail: "x".into() }.is_on());
        assert!(!State::Analyzing.is_on());
    }

    #[test]
    fn every_state_has_a_distinct_css_class_that_is_tracked() {
        let all = State::all_css_classes();
        for state in [
            State::Ready,
            State::Analyzing,
            State::Optimizing { step: "x".into() },
            State::Active { count: 1 },
            State::AlreadyOptimal,
            State::Partial {
                applied: 1,
                total: 2,
            },
            State::Error { detail: "x".into() },
            State::Restoring,
        ] {
            assert!(
                all.contains(&state.css_class()),
                "{:?} uses an untracked class {}",
                state,
                state.css_class()
            );
        }
    }

    #[test]
    fn every_state_has_non_empty_text() {
        for state in [
            State::Ready,
            State::Analyzing,
            State::Optimizing {
                step: "Applying CPU governor".into(),
            },
            State::Active { count: 6 },
            State::AlreadyOptimal,
            State::Partial {
                applied: 1,
                total: 3,
            },
            State::Error {
                detail: "daemon unreachable".into(),
            },
            State::Restoring,
        ] {
            assert!(!state.title().is_empty(), "{state:?} has no title");
            assert!(!state.subtitle().is_empty(), "{state:?} has no subtitle");
            assert!(!state.icon().is_empty());
        }
    }

    #[test]
    fn plurals_read_correctly() {
        assert_eq!(
            State::Active { count: 1 }.subtitle(),
            "1 optimization active"
        );
        assert_eq!(
            State::Active { count: 6 }.subtitle(),
            "6 optimizations active"
        );
    }

    #[test]
    fn optimizing_shows_the_real_step() {
        // The step text comes from the engine, so the control can never show
        // progress for work that is not happening.
        let s = State::Optimizing {
            step: "Applying GPU power level (card1)".into(),
        };
        assert_eq!(s.subtitle(), "Applying GPU power level (card1)");
    }
}
