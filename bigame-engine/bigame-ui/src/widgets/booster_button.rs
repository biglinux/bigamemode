//! The Turbo Mode control.
//!
//! A switch is the wrong affordance for this: it implies a setting that is
//! simply on or off, when what actually happens is a multi-step operation that
//! can succeed, partly succeed, or fail, and that takes long enough to need
//! feedback while it runs.
//!
//! So it is a large button with an explicit state machine. Every state
//! corresponds to something the engine is really doing — there is no timed
//! animation standing in for work, and the control never shows "active" for an
//! operation that did not verify.

use gtk4::glib;
use gtk4::prelude::*;

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
            Self::Off => i18n("Turbo mode inactive"),
            Self::Working { .. } => i18n("Turning On"),
            Self::On { .. } | Self::Partial { .. } => i18n("Turbo mode activated"),
            Self::Error { .. } => i18n("Turbo Could Not Start"),
            Self::Restoring => i18n("Turning Off"),
        }
    }

    /// Supporting line shown under the heading.
    #[must_use]
    pub fn subtitle(&self) -> String {
        match self {
            Self::Off => i18n("Off · Big Game Mode is not intervening in games"),
            Self::Working { step } => step.clone(),
            Self::On { detail } | Self::Partial { detail } | Self::Error { detail } => {
                detail.clone()
            }
            Self::Restoring => i18n("Putting everything back as it was"),
        }
    }

    /// Where the artwork stands in this state ([`super::turbo_art`]): off 0,
    /// on 1; switching on starts from the first spark and is moved on by
    /// each real stage ([`BoosterButton::set_progress`]).
    #[must_use]
    pub fn level(&self) -> f64 {
        match self {
            Self::Off | Self::Error { .. } => 0.0,
            Self::Working { .. } => 0.12,
            Self::Restoring => 0.2,
            Self::On { .. } | Self::Partial { .. } => 1.0,
        }
    }

    fn tint(&self) -> super::turbo_art::Tint {
        match self {
            Self::Partial { .. } => super::turbo_art::Tint::Warning,
            Self::Error { .. } => super::turbo_art::Tint::Error,
            _ => super::turbo_art::Tint::Spectrum,
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

/// What the artwork is drawing, and where it is heading.
struct Art {
    /// Shown now, 0 … 1.
    level: f64,
    /// Where the state wants it.
    target: f64,
    tint: super::turbo_art::Tint,
    hover: bool,
    /// For what turns: seconds it has turned, which only advance while it
    /// does, so it resumes where it stopped.
    time: f64,
    /// Until when it turns after a change of state; while hovered it turns
    /// as well. At rest the disc is still and the frame clock is let go.
    spin_until: Option<std::time::Instant>,
    /// The frame clock's time at the last tick, µs.
    last_tick: i64,
    /// When the artwork was last redrawn while nothing but its turning moved.
    last_draw: std::time::Instant,
}

/// Something told of every state the control shows.
type Listener = Box<dyn Fn(&State)>;

/// Redraw at most this often when only the turning moves: the disc is
/// something to glance at, and a game may be running.
const IDLE_FRAME: std::time::Duration = std::time::Duration::from_millis(33);

/// How long the disc turns after its state changes.
const SPIN: std::time::Duration = std::time::Duration::from_secs(4);

/// The frame-clock callback that animates the disc, while there is one.
type Ticker = std::rc::Rc<std::cell::RefCell<Option<gtk4::TickCallbackId>>>;

/// The Turbo Mode control.
pub struct BoosterButton {
    button: gtk4::Button,
    caption: gtk4::Label,
    area: gtk4::DrawingArea,
    art: std::rc::Rc<std::cell::RefCell<Art>>,
    ticker: Ticker,
    state: std::cell::RefCell<State>,
    /// Called with every state shown.
    listeners: std::cell::RefCell<Vec<Listener>>,
}

/// The disc's size.
const SIZE: i32 = 200;

impl BoosterButton {
    /// Build the control in its [`State::Off`] state.
    #[must_use]
    pub fn new() -> std::rc::Rc<Self> {
        use std::cell::RefCell;
        use std::rc::Rc;

        let art = Rc::new(RefCell::new(Art {
            level: 0.0,
            target: 0.0,
            tint: super::turbo_art::Tint::Spectrum,
            hover: false,
            time: 0.0,
            spin_until: None,
            last_tick: 0,
            last_draw: std::time::Instant::now(),
        }));

        let area = art_area(&art);
        // The frame clock ticks a widget that is realized, not only one on
        // screen, and a page in a view stack stays realized: the callback
        // lives only while the disc is mapped and something moves.
        let ticker: Ticker = Rc::new(RefCell::new(None));
        {
            let (art, ticker) = (Rc::clone(&art), Rc::clone(&ticker));
            area.connect_map(move |area| wake(area, &art, &ticker));
        }
        {
            let ticker = Rc::clone(&ticker);
            area.connect_unmap(move |_| {
                if let Some(id) = ticker.borrow_mut().take() {
                    id.remove();
                }
            });
        }

        // Nothing is written on the disc: the symbol says on or off, and
        // what is happening goes under it (`caption`).
        let button = gtk4::Button::builder()
            .child(&area)
            .halign(gtk4::Align::Center)
            .valign(gtk4::Align::Center)
            .css_classes(["booster-button", "booster-ready"])
            .build();
        {
            let motion = gtk4::EventControllerMotion::new();
            let (enter, leave) = (Rc::clone(&art), Rc::clone(&art));
            let (a1, a2) = (area.clone(), area.clone());
            let ticking = Rc::clone(&ticker);
            motion.connect_enter(move |_, _, _| {
                enter.borrow_mut().hover = true;
                a1.queue_draw();
                wake(&a1, &enter, &ticking);
            });
            motion.connect_leave(move |_| {
                leave.borrow_mut().hover = false;
                a2.queue_draw();
            });
            button.add_controller(motion);
        }

        let caption = gtk4::Label::new(None);
        caption.add_css_class("turbo-caption");
        caption.set_wrap(true);
        caption.set_justify(gtk4::Justification::Center);
        caption.set_max_width_chars(40);
        caption.set_visible(false);

        // Screen readers announce the state, not just the word "button".
        button.update_property(&[
            gtk4::accessible::Property::Label(&State::Off.title()),
            gtk4::accessible::Property::Description(&State::Off.subtitle()),
        ]);

        Rc::new(Self {
            button,
            caption,
            area,
            art,
            ticker,
            state: RefCell::new(State::Off),
            listeners: RefCell::new(Vec::new()),
        })
    }

    /// The underlying widget, for packing into a container.
    #[must_use]
    pub fn widget(&self) -> &gtk4::Button {
        &self.button
    }

    /// The line under the disc, packed by the page right after
    /// [`Self::widget`]: what is happening while switching, or what went
    /// wrong; nothing while plainly on or off.
    #[must_use]
    pub fn caption(&self) -> &gtk4::Label {
        &self.caption
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

        let title = state.title();
        let subtitle = state.subtitle();
        let caption = match state {
            State::Off | State::On { .. } => None,
            State::Working { step } => Some(step.clone()),
            State::Restoring => Some(subtitle.clone()),
            State::Partial { detail } | State::Error { detail } => Some(detail.clone()),
        };
        self.caption.set_visible(caption.is_some());
        self.caption
            .set_label(caption.as_deref().unwrap_or_default());
        self.button.set_sensitive(state.is_interactive());

        {
            let mut art = self.art.borrow_mut();
            art.target = state.level();
            art.tint = state.tint();
            art.spin_until = Some(std::time::Instant::now() + SPIN);
        }
        self.area.queue_draw();
        wake(&self.area, &self.art, &self.ticker);

        self.button.update_property(&[
            gtk4::accessible::Property::Label(&title),
            gtk4::accessible::Property::Description(&subtitle),
        ]);

        *self.state.borrow_mut() = state.clone();
        for listener in self.listeners.borrow().iter() {
            listener(state);
        }
    }

    /// Call `f` with every state the control shows from now on.
    pub fn connect_state_changed(&self, f: impl Fn(&State) + 'static) {
        self.listeners.borrow_mut().push(Box::new(f));
    }

    /// While switching on, how far it has really got (0 … 1): each stage
    /// the engine reports moves the artwork on, from the sparks to the
    /// colours filling the mesh.
    pub fn set_progress(&self, done: f64) {
        if matches!(*self.state.borrow(), State::Working { .. }) {
            {
                let mut art = self.art.borrow_mut();
                art.target = art.target.max(0.12 + done.clamp(0.0, 1.0) * 0.76);
                art.spin_until = Some(std::time::Instant::now() + SPIN);
            }
            wake(&self.area, &self.art, &self.ticker);
        }
    }

    /// Run `handler` when the control is activated.
    pub fn connect_activated<F: Fn() + 'static>(self: &std::rc::Rc<Self>, handler: F) {
        self.button.connect_clicked(move |_| handler());
    }
}

/// The drawing area for the artwork, redrawn as `art` moves.
fn art_area(art: &std::rc::Rc<std::cell::RefCell<Art>>) -> gtk4::DrawingArea {
    use std::rc::Rc;
    let area = gtk4::DrawingArea::new();
    area.set_content_width(SIZE);
    area.set_content_height(SIZE);
    {
        let art = Rc::clone(art);
        area.set_draw_func(move |_, cr, width, height| {
            let a = art.borrow();
            let size = f64::from(width.min(height));
            cr.translate(
                (f64::from(width) - size) / 2.0,
                (f64::from(height) - size) / 2.0,
            );
            let time = if animations_enabled() { a.time } else { 0.0 };
            super::turbo_art::draw(
                cr,
                size,
                super::turbo_art::Look {
                    level: a.level,
                    time,
                    tint: a.tint,
                    hover: a.hover,
                },
            );
        });
    }
    area
}

/// Run the frame-clock callback while `area` is on screen and something
/// moves: the level towards its target, and what turns for a while after a
/// change or while hovered. It lets itself go once the disc is at rest.
fn wake(area: &gtk4::DrawingArea, art: &std::rc::Rc<std::cell::RefCell<Art>>, ticker: &Ticker) {
    if !area.is_mapped() || ticker.borrow().is_some() {
        return;
    }
    let (art, slot) = (std::rc::Rc::clone(art), std::rc::Rc::clone(ticker));
    let id = area.add_tick_callback(move |area, clock| {
        let mut a = art.borrow_mut();
        let now = clock.frame_time();
        #[allow(clippy::cast_precision_loss)]
        let dt = if a.last_tick == 0 {
            0.0
        } else {
            ((now - a.last_tick) as f64 / 1e6).min(0.1)
        };
        a.last_tick = now;
        let animate = animations_enabled();
        let moving = (a.target - a.level).abs() > 0.002;
        if moving {
            a.level = if animate {
                // About a second from off to on.
                a.level + (a.target - a.level) * (1.0 - (-dt * 3.5).exp())
            } else {
                a.target
            };
        }
        let spinning = a.hover || a.spin_until.is_some_and(|t| std::time::Instant::now() < t);
        let turning = animate && a.level > 0.02 && spinning;
        if turning {
            a.time += dt;
        }
        if moving || (turning && a.last_draw.elapsed() >= IDLE_FRAME) {
            a.last_draw = std::time::Instant::now();
            area.queue_draw();
        }
        if moving || turning {
            return glib::ControlFlow::Continue;
        }
        // At rest: the last step drawn exactly, and the clock let go.
        a.level = a.target;
        a.last_tick = 0;
        area.queue_draw();
        // Ended by returning Break: the id is dropped, not removed.
        slot.borrow_mut().take();
        glib::ControlFlow::Break
    });
    *ticker.borrow_mut() = Some(id);
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
            assert!(classes.contains(&state.css_class()), "{state:?}");
        }
    }
}
