//! The Turbo preset picker, under the Turbo control on Home.
//!
//! With Turbo off it chooses the preset for when Turbo is switched on; with
//! Turbo on it changes the preset in force, for the games started from then
//! on ([`bigame_core::turbo_preset`]). A click here and a choice in the tray
//! take the same path: [`PresetPicker::select`] then whoever
//! [`PresetPicker::connect_picked`] asked, which is Home.

use std::rc::Rc;

use gtk4::prelude::*;

use bigame_core::turbo_preset::{self, FrameGenerators, Machine, Preset};

use crate::i18n::i18n;

/// The picker.
pub struct PresetPicker {
    root: gtk4::Box,
    buttons: Vec<(Preset, gtk4::ToggleButton)>,
    description: gtk4::Label,
    note: gtk4::Label,
    /// What the machine lets a preset use, read once off the main thread
    /// (sched-ext, `/usr/bin`, the GPUs); `None` until then.
    machine: std::cell::Cell<Option<Machine>>,
    /// Games that generate frames, read once off the main thread.
    generating: std::cell::RefCell<FrameGenerators>,
    /// Told of every preset picked, by a click or by [`Self::select`].
    picked: std::cell::RefCell<Vec<Picked>>,
    /// Set while the buttons are moved to show a preset, not picked.
    quiet: std::cell::Cell<bool>,
}

/// Something told of every preset picked.
type Picked = Box<dyn Fn(Preset)>;

/// What a pick does now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Turbo is off: the pick is for when it is switched on.
    Next,
    /// Turbo is on: the pick changes the preset in force.
    Live,
    /// Turbo, or a preset, is switching: nothing can be picked.
    Locked,
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

/// What `preset` does, and what this machine (once read) leaves out of it.
fn describe(preset: Preset, machine: Option<Machine>, generating: &FrameGenerators) -> String {
    let mut text = i18n(preset.description());
    // The cap does not depend on the machine.
    let levers = turbo_preset::levers(preset, machine.unwrap_or_default());
    if let Some(machine) = machine.filter(|_| preset == Preset::Enhanced) {
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
            "A native Linux game is capped only when Big Game Mode starts it (Profiles → Launch).",
        ));
        // Where the generation sits decides what the cap holds: OptiScaler's
        // runs in the game, above the limiter; lsfg-vk is a layer under it.
        if !generating.shown.is_empty() {
            text.push(' ');
            text.push_str(
                &i18n(
                    "With frame generation the cap counts the frames shown, so these games render about half of it: %s.",
                )
                .replace("%s", &generating.shown.join(", ")),
            );
        }
        if !generating.rendered.is_empty() {
            let games: Vec<String> = generating
                .rendered
                .iter()
                .map(|(game, multiplier)| format!("{game} ×{multiplier}"))
                .collect();
            text.push(' ');
            text.push_str(
                &i18n(
                    "lsfg-vk adds its frames after the cap, so these games render the cap and show it multiplied: %s.",
                )
                .replace("%s", &games.join(", ")),
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

        // Two lines on the page; the whole text as a tooltip.
        let not_read_yet = FrameGenerators::default();
        let description = gtk4::Label::new(Some(&describe(chosen, None, &not_read_yet)));
        description.add_css_class("dim-label");
        description.add_css_class("caption");
        description.set_wrap(true);
        description.set_lines(2);
        description.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        description.set_justify(gtk4::Justification::Center);
        description.set_max_width_chars(70);
        description.set_tooltip_text(Some(&describe(chosen, None, &not_read_yet)));

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
            machine: std::cell::Cell::new(None),
            generating: std::cell::RefCell::new(FrameGenerators::default()),
            picked: std::cell::RefCell::new(Vec::new()),
            quiet: std::cell::Cell::new(false),
        });
        {
            let weak = Rc::downgrade(&me);
            gtk4::glib::spawn_future_local(async move {
                let read = gtk4::gio::spawn_blocking(|| {
                    (Machine::detect(), turbo_preset::frame_generation_games())
                })
                .await;
                let Some(me) = weak.upgrade() else {
                    return;
                };
                if let Ok((machine, games)) = read {
                    me.machine.set(Some(machine));
                    *me.generating.borrow_mut() = games;
                } else {
                    tracing::warn!("the machine could not be read for the presets");
                }
                me.refresh();
            });
        }
        me.follow_clicks();
        me
    }

    /// A click shows the preset's description and tells whoever asked.
    fn follow_clicks(self: &Rc<Self>) {
        for (preset, button) in &self.buttons {
            let (preset, weak) = (*preset, Rc::downgrade(self));
            button.connect_toggled(move |b| {
                if !b.is_active() {
                    return;
                }
                let Some(me) = weak.upgrade() else {
                    return;
                };
                me.refresh();
                if !me.quiet.get() {
                    for f in me.picked.borrow().iter() {
                        f(preset);
                    }
                }
            });
        }
    }

    /// Describe the preset shown now again.
    fn refresh(&self) {
        let text = describe(self.shown(), self.machine.get(), &self.generating.borrow());
        self.description.set_label(&text);
        self.description.set_tooltip_text(Some(&text));
    }

    /// The widget.
    #[must_use]
    pub fn widget(&self) -> &gtk4::Box {
        &self.root
    }

    /// The preset shown.
    #[must_use]
    pub fn shown(&self) -> Preset {
        self.buttons
            .iter()
            .find(|(_, b)| b.is_active())
            .map_or(Preset::Standard, |(p, _)| *p)
    }

    /// Whether a preset can be picked now.
    #[must_use]
    pub fn is_open(&self) -> bool {
        self.buttons.first().is_some_and(|(_, b)| b.is_sensitive())
    }

    /// Pick `preset` as a click on it would.
    pub fn select(&self, preset: Preset) {
        if let Some((_, b)) = self.buttons.iter().find(|(p, _)| *p == preset) {
            b.set_active(true);
        }
    }

    /// Show `preset` without picking it: what is in force, read back.
    pub fn show(&self, preset: Preset) {
        self.quiet.set(true);
        self.select(preset);
        self.quiet.set(false);
    }

    /// Call `f` with every preset picked from now on.
    pub fn connect_picked(&self, f: impl Fn(Preset) + 'static) {
        self.picked.borrow_mut().push(Box::new(f));
    }

    /// Say what a pick does now, and let one be made only when it can be.
    pub fn set_mode(&self, mode: Mode) {
        for (_, b) in &self.buttons {
            b.set_sensitive(mode != Mode::Locked);
        }
        let note = match mode {
            Mode::Next => Some(i18n("Used when Turbo is switched on.")),
            Mode::Live => Some(i18n("A change applies to the games started from now on.")),
            Mode::Locked => None,
        };
        self.note.set_label(note.as_deref().unwrap_or_default());
        self.note.set_visible(note.is_some());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cap_is_explained_per_frame_generator() {
        let machine = Machine {
            vkbasalt: true,
            fsr4: true,
        };
        let generating = FrameGenerators {
            shown: vec!["SOTTR.exe".into()],
            rendered: vec![("Bodycam-Win64-Shipping.exe".into(), 2)],
        };
        let text = describe(Preset::Locked60, Some(machine), &generating);
        // OptiScaler above DXVK's limiter: about half rendered.
        let half = text
            .find("render about half of it: SOTTR.exe")
            .expect(&text);
        // lsfg-vk under the limiter: the cap is rendered, shown multiplied.
        let after = text
            .find("render the cap and show it multiplied: Bodycam-Win64-Shipping.exe ×2")
            .expect(&text);
        assert!(!text[half..after].contains("Bodycam"), "{text}");
        // No cap, nothing to explain.
        let text = describe(Preset::MoreFps, Some(machine), &generating);
        assert!(
            !text.contains("SOTTR.exe") && !text.contains("Bodycam"),
            "{text}"
        );
        // Until the machine is read, nothing is said about what it lacks.
        let bare = Some(Machine::default());
        assert!(
            describe(Preset::Enhanced, bare, &generating).contains("vkBasalt is not installed")
        );
        assert!(
            !describe(Preset::Enhanced, None, &generating).contains("vkBasalt is not installed")
        );
    }
}
