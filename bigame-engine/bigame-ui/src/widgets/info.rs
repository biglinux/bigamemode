//! The ⓘ button: an explanation one click away instead of on the page.
//!
//! Technical rows keep a short subtitle and put the rest here — what the
//! setting is, what it changes, who controls it, and what was measured — so
//! the page stays readable for someone who only wants to play.

use gtk4::prelude::*;

use crate::i18n::i18n;

/// An info button whose popover shows `heading` and `text`.
#[must_use]
pub fn button(heading: &str, text: &str) -> gtk4::MenuButton {
    let title = gtk4::Label::new(Some(heading));
    title.add_css_class("heading");
    title.set_xalign(0.0);
    title.set_wrap(true);

    let body = gtk4::Label::new(Some(text));
    body.set_xalign(0.0);
    body.set_wrap(true);
    body.set_max_width_chars(48);
    body.set_selectable(true);

    let content = gtk4::Box::new(gtk4::Orientation::Vertical, 6);
    content.set_margin_top(12);
    content.set_margin_bottom(12);
    content.set_margin_start(12);
    content.set_margin_end(12);
    content.append(&title);
    content.append(&body);

    let popover = gtk4::Popover::new();
    popover.set_child(Some(&content));

    let button = gtk4::MenuButton::builder()
        .icon_name("help-about-symbolic")
        .popover(&popover)
        .valign(gtk4::Align::Center)
        .css_classes(["flat", "circular"])
        .tooltip_text(i18n("More information"))
        .build();
    button.update_property(&[gtk4::accessible::Property::Label(&format!(
        "{} {heading}",
        i18n("More information about")
    ))]);
    button
}
