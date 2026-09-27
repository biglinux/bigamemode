//! The one way a page says something needs attention.
//!
//! Information, a warning, a conflict, an error or a success look the same
//! everywhere: an icon, a short title, one or two sentences, and the actions
//! that resolve it under the text (so a long translation wraps instead of
//! squeezing the buttons). The icon and the title carry the meaning; the
//! colour only repeats it.
//!
//! A conflict between two technologies is asked, never just reported:
//! [`ask_conflict`] offers "Keep A" and "Use B", and nothing changes until
//! one is chosen.

use adw::prelude::*;
use libadwaita as adw;

use bigame_core::optimization::Conflict;

use crate::i18n::i18n;

/// What a notice is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Something worth knowing.
    Info,
    /// It works, with a caveat.
    Warning,
    /// Two settings collide.
    Conflict,
    /// Something failed.
    Error,
    /// Something was done.
    Success,
}

fn icon(kind: Kind) -> &'static str {
    match kind {
        Kind::Info => "dialog-information-symbolic",
        Kind::Warning | Kind::Conflict => "dialog-warning-symbolic",
        Kind::Error => "dialog-error-symbolic",
        Kind::Success => "emblem-ok-symbolic",
    }
}

fn css(kind: Kind) -> &'static str {
    match kind {
        Kind::Info => "notice-info",
        Kind::Warning => "notice-warning",
        Kind::Conflict => "notice-conflict",
        Kind::Error => "notice-error",
        Kind::Success => "notice-success",
    }
}

/// A notice, ready to go into a page or a group.
#[derive(Clone)]
pub struct Notice {
    root: gtk4::Box,
    title: gtk4::Label,
    body: gtk4::Label,
    image: gtk4::Image,
    actions: gtk4::Box,
    kind: std::rc::Rc<std::cell::Cell<Kind>>,
}

impl Notice {
    /// A notice of `kind` with `title` and `body` (`body` may be empty).
    #[must_use]
    pub fn new(kind: Kind, title: &str, body: &str) -> Self {
        let image = gtk4::Image::from_icon_name(icon(kind));
        image.set_valign(gtk4::Align::Start);
        image.add_css_class("notice-icon");

        let title_label = gtk4::Label::builder()
            .label(title)
            .wrap(true)
            .wrap_mode(gtk4::pango::WrapMode::WordChar)
            .xalign(0.0)
            .css_classes(["heading"])
            .build();
        let body_label = gtk4::Label::builder()
            .label(body)
            .wrap(true)
            .wrap_mode(gtk4::pango::WrapMode::WordChar)
            .xalign(0.0)
            .visible(!body.is_empty())
            .build();
        let actions = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
        actions.set_halign(gtk4::Align::Start);
        actions.set_margin_top(4);
        actions.set_visible(false);

        let text = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
        text.set_hexpand(true);
        text.append(&title_label);
        text.append(&body_label);
        text.append(&actions);

        let root = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Horizontal)
            .spacing(12)
            .css_classes(["notice", css(kind)])
            .accessible_role(if matches!(kind, Kind::Error | Kind::Conflict) {
                gtk4::AccessibleRole::Alert
            } else {
                gtk4::AccessibleRole::Group
            })
            .build();
        root.append(&image);
        root.append(&text);
        root.update_property(&[gtk4::accessible::Property::Label(title)]);
        Self {
            root,
            title: title_label,
            body: body_label,
            image,
            actions,
            kind: std::rc::Rc::new(std::cell::Cell::new(kind)),
        }
    }

    /// The widget.
    #[must_use]
    pub fn widget(&self) -> &gtk4::Box {
        &self.root
    }

    /// Change what the notice says, keeping its actions.
    pub fn set(&self, kind: Kind, title: &str, body: &str) {
        self.root.remove_css_class(css(self.kind.get()));
        self.root.add_css_class(css(kind));
        self.kind.set(kind);
        self.image.set_icon_name(Some(icon(kind)));
        self.title.set_label(title);
        self.body.set_label(body);
        self.body.set_visible(!body.is_empty());
        self.root
            .update_property(&[gtk4::accessible::Property::Label(title)]);
    }

    /// Add an action button; the first `suggested` one is the main action.
    pub fn add_action(&self, label: &str, suggested: bool, f: impl Fn(&gtk4::Button) + 'static) {
        let button = gtk4::Button::with_label(label);
        if suggested {
            button.add_css_class("suggested-action");
        }
        button.connect_clicked(f);
        self.actions.append(&button);
        self.actions.set_visible(true);
    }

    /// Remove every action.
    pub fn clear_actions(&self) {
        while let Some(child) = self.actions.first_child() {
            self.actions.remove(&child);
        }
        self.actions.set_visible(false);
    }

    /// Show or hide the notice.
    pub fn set_visible(&self, visible: bool) {
        self.root.set_visible(visible);
    }
}

/// The name people read for a feature.
#[must_use]
pub fn feature_name(f: bigame_core::optimization::Feature) -> String {
    i18n(f.label())
}

/// Ask which of two colliding technologies to keep. `on_choice(true)` means
/// the requested one is used (the caller turns the other off);
/// `on_choice(false)` keeps what was on, and is also what closing the dialog
/// does — nothing changes behind the user's back.
pub fn ask_conflict(
    anchor: &impl IsA<gtk4::Widget>,
    conflict: &Conflict,
    on_choice: impl Fn(bool) + 'static,
) {
    let requested = feature_name(conflict.requested);
    let active = feature_name(conflict.active);
    let effect = if conflict.requested.generates_frames() {
        i18n("The result is artefacts, added latency and unpredictable behaviour.")
    } else {
        i18n("The image would be scaled twice, which blurs it and costs time for nothing.")
    };
    let why = sentence(&i18n(conflict.why));
    let dialog = adw::AlertDialog::builder()
        .heading(
            i18n("%r cannot be used together with %a")
                .replace("%r", &requested)
                .replace("%a", &active),
        )
        .body(format!(
            "{why}. {effect} {}",
            i18n("Choose the one to keep; the other is turned off.")
        ))
        .build();
    dialog.add_response("keep", &i18n("Keep %s").replace("%s", &active));
    dialog.add_response("use", &i18n("Use %s").replace("%s", &requested));
    dialog.set_response_appearance("use", adw::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("keep"));
    dialog.set_close_response("keep");
    dialog.connect_response(None, move |_, response| on_choice(response == "use"));
    dialog.present(Some(anchor));
}

/// Start a sentence with a capital: the matrix writes its reasons as
/// clauses.
#[must_use]
pub fn sentence(s: &str) -> String {
    let mut c = s.chars();
    c.next()
        .map(|f| f.to_uppercase().chain(c).collect())
        .unwrap_or_default()
}
