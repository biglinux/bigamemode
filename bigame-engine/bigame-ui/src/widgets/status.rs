//! The one way every page says how something stands.
//!
//! A [`State`](bigame_core::overview::State) has a label, an icon and a
//! colour class; the label and the icon carry the meaning, the colour only
//! repeats it. The widgets here are the chip (a badge with the label), the
//! fact list (label / value rows inside an expander) and the status row (an
//! expander whose visible line is the state and whose body explains it).

use adw::prelude::*;
use bigame_core::overview::State;
use libadwaita as adw;

use crate::i18n::i18n;

/// The label people read.
#[must_use]
pub fn label(state: State) -> String {
    match state {
        State::Active => i18n("Active"),
        State::Waiting => i18n("Waiting for a game"),
        State::NotDetected => i18n("Not detected"),
        State::Configured => i18n("Configured"),
        State::Off => i18n("Off"),
        State::Missing => i18n("Missing dependency"),
        State::Unsupported => i18n("Not supported"),
        State::Error => i18n("Error"),
    }
}

/// The symbolic icon that goes with the label.
#[must_use]
pub fn icon(state: State) -> &'static str {
    match state {
        State::Active => "emblem-ok-symbolic",
        State::Waiting => "media-playback-pause-symbolic",
        State::NotDetected => "dialog-warning-symbolic",
        State::Configured => "emblem-default-symbolic",
        State::Off => "radio-symbolic",
        State::Missing => "package-x-generic-symbolic",
        State::Unsupported => "action-unavailable-symbolic",
        State::Error => "dialog-error-symbolic",
    }
}

/// The CSS class of the chip: success, warning, error, or neutral.
#[must_use]
pub fn css(state: State) -> &'static str {
    match state {
        State::Active => "state-active",
        State::Waiting | State::Configured => "state-waiting",
        State::NotDetected | State::Missing => "state-attention",
        State::Error => "state-error",
        State::Off | State::Unsupported => "state-neutral",
    }
}

const ALL_CSS: [&str; 5] = [
    "state-active",
    "state-waiting",
    "state-attention",
    "state-error",
    "state-neutral",
];

/// A chip: icon and label in a rounded badge.
#[derive(Clone)]
pub struct Chip {
    root: gtk4::Box,
    image: gtk4::Image,
    text: gtk4::Label,
}

impl Chip {
    /// A chip showing `state`.
    #[must_use]
    pub fn new(state: State) -> Self {
        let image = gtk4::Image::from_icon_name(icon(state));
        image.set_pixel_size(14);
        let text = gtk4::Label::new(Some(&label(state)));
        text.add_css_class("caption");
        text.set_single_line_mode(true);
        let root = gtk4::Box::new(gtk4::Orientation::Horizontal, 5);
        root.add_css_class("state-chip");
        root.add_css_class(css(state));
        root.set_valign(gtk4::Align::Center);
        root.append(&image);
        root.append(&text);
        Self { root, image, text }
    }

    /// The widget.
    #[must_use]
    pub fn widget(&self) -> &gtk4::Box {
        &self.root
    }

    /// Show `state`, with an optional short text instead of its label
    /// (`scx_lavd` rather than "Active").
    pub fn set(&self, state: State, text: Option<&str>) {
        self.image.set_icon_name(Some(icon(state)));
        self.text.set_label(text.unwrap_or(&label(state)));
        for c in ALL_CSS {
            self.root.remove_css_class(c);
        }
        self.root.add_css_class(css(state));
        self.root
            .set_tooltip_text(text.map(|_| label(state)).as_deref());
    }
}

/// One fact in an expander's body: a label on the left, a value on the
/// right, the value selectable so a path or a command can be copied.
#[must_use]
pub fn fact_row(name: &str, value: &str) -> adw::ActionRow {
    let row = adw::ActionRow::builder()
        .title(name)
        .use_markup(false)
        .build();
    // Wide enough for a path or a scheduler list before it wraps; the row
    // title is short, so the value can take most of the width.
    let v = gtk4::Label::builder()
        .label(value)
        .selectable(true)
        .wrap(true)
        .wrap_mode(gtk4::pango::WrapMode::WordChar)
        .xalign(1.0)
        .justify(gtk4::Justification::Right)
        .width_chars(28)
        .max_width_chars(60)
        .css_classes(["dim-label", "fact-value"])
        .valign(gtk4::Align::Center)
        .build();
    row.add_suffix(&v);
    row
}

/// A paragraph in an expander's body: what something means, or why it is
/// not working.
#[must_use]
pub fn note_row(text: &str) -> adw::ActionRow {
    let row = adw::ActionRow::builder()
        .title(text)
        .use_markup(false)
        .title_lines(0)
        .build();
    row.add_css_class("fact-note");
    row
}

/// A command to copy, never run: a row with the command in monospace and a
/// copy button.
#[must_use]
pub fn command_row(command: &str) -> adw::ActionRow {
    let row = adw::ActionRow::builder()
        .title(command)
        .use_markup(false)
        .title_lines(0)
        .build();
    row.add_css_class("monospace");
    let copy = gtk4::Button::builder()
        .icon_name("edit-copy-symbolic")
        .tooltip_text(i18n("Copy the command"))
        .valign(gtk4::Align::Center)
        .css_classes(["flat"])
        .build();
    copy.update_property(&[gtk4::accessible::Property::Label(&i18n("Copy the command"))]);
    let text = command.to_owned();
    copy.connect_clicked(move |b| {
        b.clipboard().set_text(&text);
        crate::widgets::toast::show(b, &i18n("Copied"));
    });
    row.add_suffix(&copy);
    row
}

/// An expander row whose visible line is a state, and whose body is
/// rebuilt from facts each refresh.
#[derive(Clone)]
pub struct StatusRow {
    row: adw::ExpanderRow,
    chip: Chip,
    body: std::rc::Rc<std::cell::RefCell<Vec<gtk4::Widget>>>,
}

impl StatusRow {
    /// A row titled `title`, with a `what` line under it.
    #[must_use]
    pub fn new(title: &str, what: &str, icon_name: &str) -> Self {
        let row = adw::ExpanderRow::builder()
            .title(title)
            .subtitle(what)
            .use_markup(false)
            .build();
        row.set_subtitle_lines(2);
        let image = gtk4::Image::from_icon_name(icon_name);
        image.add_css_class("dim-label");
        row.add_prefix(&image);
        let chip = Chip::new(State::Off);
        row.add_suffix(chip.widget());
        Self {
            row,
            chip,
            body: std::rc::Rc::new(std::cell::RefCell::new(Vec::new())),
        }
    }

    /// The widget.
    #[must_use]
    pub fn widget(&self) -> &adw::ExpanderRow {
        &self.row
    }

    /// The state, an optional short text on the chip, and the subtitle.
    pub fn set_state(&self, state: State, chip_text: Option<&str>, subtitle: &str) {
        self.chip.set(state, chip_text);
        self.row.set_subtitle(subtitle);
        self.row.set_tooltip_text(Some(&label(state)));
    }

    /// Replace the body with `rows`.
    pub fn set_body(&self, rows: Vec<gtk4::Widget>) {
        for w in self.body.borrow_mut().drain(..) {
            self.row.remove(&w);
        }
        for w in rows {
            self.row.add_row(&w);
            self.body.borrow_mut().push(w);
        }
    }
}

/// The body of a status row, built line by line.
#[derive(Default)]
pub struct Body {
    rows: Vec<gtk4::Widget>,
}

impl Body {
    /// Start empty.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// A label / value fact.
    #[must_use]
    pub fn fact(mut self, name: &str, value: &str) -> Self {
        self.rows.push(fact_row(name, value).upcast());
        self
    }

    /// A fact, when there is a value.
    #[must_use]
    pub fn fact_opt(self, name: &str, value: Option<&str>) -> Self {
        match value {
            Some(v) => self.fact(name, v),
            None => self,
        }
    }

    /// A paragraph.
    #[must_use]
    pub fn note(mut self, text: &str) -> Self {
        self.rows.push(note_row(text).upcast());
        self
    }

    /// A command to copy.
    #[must_use]
    pub fn command(mut self, command: &str) -> Self {
        self.rows.push(command_row(command).upcast());
        self
    }

    /// The rows.
    #[must_use]
    pub fn build(self) -> Vec<gtk4::Widget> {
        self.rows
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_state_has_a_label_an_icon_and_a_class() {
        for s in [
            State::Active,
            State::Waiting,
            State::NotDetected,
            State::Configured,
            State::Off,
            State::Missing,
            State::Unsupported,
            State::Error,
        ] {
            assert!(!label(s).is_empty());
            assert!(icon(s).ends_with("-symbolic"));
            assert!(ALL_CSS.contains(&css(s)));
        }
    }

    #[test]
    fn a_hardware_limit_and_off_are_neutral_never_attention() {
        assert_eq!(css(State::Unsupported), "state-neutral");
        assert_eq!(css(State::Off), "state-neutral");
        assert_eq!(css(State::NotDetected), "state-attention");
    }
}
