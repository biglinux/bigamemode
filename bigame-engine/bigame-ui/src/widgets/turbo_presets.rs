//! The Turbo preset picker, under the Turbo control on Home.
//!
//! A preset is chosen before Turbo is switched on and is in force until it
//! is switched off ([`bigame_core::turbo_preset`]); while Turbo is on the
//! picker is locked and says which preset is in force.

use std::rc::Rc;

use gtk4::prelude::*;

use bigame_core::turbo_preset::{self, Machine, Preset};

use crate::i18n::i18n;

/// The picker.
pub struct PresetPicker {
    root: gtk4::Box,
    buttons: Vec<(Preset, gtk4::ToggleButton)>,
    description: gtk4::Label,
    note: gtk4::Label,
    machine: Machine,
    /// Games that generate frames, read once off the main thread.
    generating: std::cell::RefCell<Vec<String>>,
}

/// The icon a preset is shown with, here and in its help.
pub(crate) fn icon(preset: Preset) -> &'static str {
    match preset {
        Preset::Standard => "applications-games-symbolic",
        Preset::MoreFps => "power-profile-performance-symbolic",
        Preset::Locked60 => "power-profile-balanced-symbolic",
        Preset::Enhanced => "starred-symbolic",
    }
}

/// What `preset` does, and what this machine leaves out of it.
fn describe(preset: Preset, machine: Machine, generating: &[String]) -> String {
    let mut text = i18n(preset.description());
    let levers = turbo_preset::levers(preset, machine);
    if preset == Preset::Enhanced {
        if !machine.vkbasalt {
            text.push(' ');
            text.push_str(&i18n(
                "vkBasalt is not installed, so there is no sharpening.",
            ));
        }
        if !machine.fsr4 {
            text.push(' ');
            text.push_str(&i18n("This GPU has no FSR 4, so that part is left out."));
        }
    }
    if levers.frame_cap.is_some_and(|f| f > 0) {
        text.push(' ');
        text.push_str(&i18n(
            "A native Linux game is capped only when BiGame-mode starts it (Profiles → Launch).",
        ));
        if !generating.is_empty() {
            text.push(' ');
            text.push_str(
                &i18n(
                    "With frame generation the cap counts the frames shown, so these games render about half of it: %s.",
                )
                .replace("%s", &generating.join(", ")),
            );
        }
    }
    text
}

impl PresetPicker {
    /// Build it, showing the preset chosen last.
    #[must_use]
    pub fn new() -> Rc<Self> {
        let heading = gtk4::Label::new(Some(&i18n("What should games favour?")));
        heading.add_css_class("heading");

        let row = gtk4::FlowBox::builder()
            .selection_mode(gtk4::SelectionMode::None)
            .homogeneous(true)
            .min_children_per_line(2)
            .max_children_per_line(4)
            .column_spacing(8)
            .row_spacing(8)
            .halign(gtk4::Align::Center)
            .build();

        let chosen = turbo_preset::chosen();
        let mut buttons = Vec::new();
        let mut first: Option<gtk4::ToggleButton> = None;
        for preset in turbo_preset::ALL {
            let image = gtk4::Image::from_icon_name(icon(preset));
            image.set_pixel_size(18);
            let label = gtk4::Label::new(Some(&i18n(preset.label())));
            label.set_wrap(true);
            label.set_justify(gtk4::Justification::Center);
            label.set_max_width_chars(12);
            let content = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
            content.append(&image);
            content.append(&label);
            let button = gtk4::ToggleButton::builder()
                .child(&content)
                .css_classes(["turbo-preset"])
                .width_request(108)
                .active(preset == chosen)
                .build();
            button.update_property(&[gtk4::accessible::Property::Label(&i18n(preset.label()))]);
            if let Some(first) = &first {
                button.set_group(Some(first));
            } else {
                first = Some(button.clone());
            }
            // A FlowBox child takes the focus itself; the button should.
            let child = gtk4::FlowBoxChild::new();
            child.set_focusable(false);
            child.set_child(Some(&button));
            row.append(&child);
            buttons.push((preset, button));
        }

        let machine = Machine::detect();
        // Two lines on the page; the whole text as a tooltip.
        let description = gtk4::Label::new(Some(&describe(chosen, machine, &[])));
        description.add_css_class("dim-label");
        description.add_css_class("caption");
        description.set_wrap(true);
        description.set_lines(2);
        description.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        description.set_justify(gtk4::Justification::Center);
        description.set_max_width_chars(70);
        description.set_tooltip_text(Some(&describe(chosen, machine, &[])));

        let note = gtk4::Label::new(None);
        note.add_css_class("caption");
        note.add_css_class("turbo-preset-note");
        note.set_wrap(true);
        note.set_justify(gtk4::Justification::Center);
        note.set_visible(false);

        let root = gtk4::Box::new(gtk4::Orientation::Vertical, 6);
        root.set_halign(gtk4::Align::Center);
        root.append(&heading);
        root.append(&row);
        root.append(&description);
        root.append(&note);

        let me = Rc::new(Self {
            root,
            buttons,
            description,
            note,
            machine,
            generating: std::cell::RefCell::new(Vec::new()),
        });
        {
            let weak = Rc::downgrade(&me);
            gtk4::glib::spawn_future_local(async move {
                let games = gtk4::gio::spawn_blocking(turbo_preset::frame_generation_games)
                    .await
                    .unwrap_or_default();
                if let Some(me) = weak.upgrade() {
                    *me.generating.borrow_mut() = games;
                    me.refresh();
                }
            });
        }
        for (preset, button) in &me.buttons {
            let (preset, weak) = (*preset, Rc::downgrade(&me));
            button.connect_toggled(move |b| {
                if !b.is_active() {
                    return;
                }
                if let Err(e) = turbo_preset::set_chosen(preset) {
                    tracing::warn!(error = %format!("{e:#}"), "could not keep the Turbo preset");
                }
                if let Some(me) = weak.upgrade() {
                    me.refresh();
                }
            });
        }
        me
    }

    /// Describe the preset shown now again.
    fn refresh(&self) {
        let shown = self
            .buttons
            .iter()
            .find(|(_, b)| b.is_active())
            .map_or(Preset::Standard, |(p, _)| *p);
        let text = describe(shown, self.machine, &self.generating.borrow());
        self.description.set_label(&text);
        self.description.set_tooltip_text(Some(&text));
    }

    /// The widget.
    #[must_use]
    pub fn widget(&self) -> &gtk4::Box {
        &self.root
    }

    /// Lock the choice while Turbo is on or switching; unlock it when Turbo
    /// is off. With Turbo on (`in_force`), the preset shown is the one in
    /// force; while it is still switching on, the choice it is reading stays.
    pub fn set_locked(&self, locked: bool, in_force: bool) {
        for (_, b) in &self.buttons {
            b.set_sensitive(!locked);
        }
        if locked && in_force {
            let active = turbo_preset::active();
            // The one in force is the one shown, even if another was chosen
            // from elsewhere meanwhile.
            if let Some((_, b)) = self.buttons.iter().find(|(p, _)| *p == active) {
                b.set_active(true);
            }
        }
        self.note
            .set_label(&i18n("Turn Turbo off to choose another preset."));
        self.note.set_visible(locked);
    }
}
