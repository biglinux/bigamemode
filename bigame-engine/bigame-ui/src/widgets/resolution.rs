//! A screen size chosen from the standard ones.
//!
//! The first item is "none" (0 × 0: the game's own size, or the render
//! size); then the standard sizes, largest first. A size that is not one of
//! them — the main screen's, or one an older version saved — gets an item
//! of its own, in its place by area. In a game's profile an item before all
//! of them leaves the size to the general configuration.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use gtk4::{gio, glib};
use libadwaita as adw;

use bigame_core::screen::{self, STANDARD};

use crate::i18n::i18n;
use crate::widgets::optimization::{cap_subtitle, keep_value_width};

/// One item: a size (`None` follows the general configuration) and how it
/// reads.
struct Item {
    size: Option<(u32, u32)>,
    label: String,
}

/// The row and its items.
pub struct SizePicker {
    /// The row.
    pub row: adw::ComboRow,
    model: gtk4::StringList,
    items: RefCell<Vec<Item>>,
    none_label: String,
    /// The first item's label when the size can follow the general
    /// configuration.
    inherit_label: Option<String>,
    /// The main screen's size, once read.
    main: Cell<Option<(u32, u32)>>,
    /// Set while the items change, so the selection is not taken as a choice.
    quiet: Cell<bool>,
}

/// `2560 × 1440`, and the size's name.
#[must_use]
pub fn label(size: (u32, u32), main: Option<(u32, u32)>) -> String {
    let dims = format!("{} × {}", size.0, size.1);
    if main == Some(size) {
        return i18n("%s — main screen").replace("%s", &dims);
    }
    match STANDARD.iter().find(|s| (s.width, s.height) == size) {
        Some(s) => format!("{dims} — {}", s.name),
        None => dims,
    }
}

/// Items as plain labels: the row's own caps a label at about 20
/// characters, which cuts "3440 × 1440 — main screen".
fn whole_label_factory() -> gtk4::SignalListItemFactory {
    let factory = gtk4::SignalListItemFactory::new();
    factory.connect_setup(|_, item| {
        if let Some(item) = item.downcast_ref::<gtk4::ListItem>() {
            let label = gtk4::Label::new(None);
            label.set_xalign(0.0);
            item.set_child(Some(&label));
        }
    });
    factory.connect_bind(|_, item| {
        let Some(item) = item.downcast_ref::<gtk4::ListItem>() else {
            return;
        };
        let text = item
            .item()
            .and_downcast::<gtk4::StringObject>()
            .map(|s| s.string());
        if let (Some(label), Some(text)) = (item.child().and_downcast::<gtk4::Label>(), text) {
            label.set_label(&text);
        }
    });
    factory
}

/// Either side at 0 means none.
fn normalise(size: (u32, u32)) -> (u32, u32) {
    if size.0 == 0 || size.1 == 0 {
        (0, 0)
    } else {
        size
    }
}

impl SizePicker {
    /// A row titled `title`, whose first item reads `none_label`, showing
    /// `current`.
    #[must_use]
    pub fn new(title: &str, subtitle: &str, none_label: &str, current: (u32, u32)) -> Rc<Self> {
        Self::build(title, subtitle, None, none_label, Some(current))
    }

    /// A row for one game: its first item, `inherit_label`, leaves the size
    /// to the general configuration (`current` `None`).
    #[must_use]
    pub fn inheriting(
        title: &str,
        subtitle: &str,
        inherit_label: &str,
        none_label: &str,
        current: Option<(u32, u32)>,
    ) -> Rc<Self> {
        Self::build(title, subtitle, Some(inherit_label), none_label, current)
    }

    fn build(
        title: &str,
        subtitle: &str,
        inherit_label: Option<&str>,
        none_label: &str,
        current: Option<(u32, u32)>,
    ) -> Rc<Self> {
        let model = gtk4::StringList::new(&[]);
        let row = adw::ComboRow::builder()
            .title(title)
            .subtitle(subtitle)
            .model(&model)
            .use_subtitle(false)
            .factory(&whole_label_factory())
            .build();
        cap_subtitle(row.upcast_ref(), 34);
        let me = Rc::new(Self {
            row,
            model,
            items: RefCell::new(Vec::new()),
            none_label: none_label.to_owned(),
            inherit_label: inherit_label.map(str::to_owned),
            main: Cell::new(None),
            quiet: Cell::new(false),
        });
        me.rebuild(me.normalise_choice(current));
        keep_value_width(&me.row);
        me
    }

    /// `choice` as an item holds it: sizes normalised, and "follow" only
    /// where there is such an item.
    fn normalise_choice(&self, choice: Option<(u32, u32)>) -> Option<(u32, u32)> {
        match choice {
            Some(size) => Some(normalise(size)),
            None if self.inherit_label.is_some() => None,
            None => Some((0, 0)),
        }
    }

    /// Fill the items again (the standard ones, the main screen, `extra`)
    /// and select `selected`.
    fn rebuild(&self, selected: Option<(u32, u32)>) {
        let main = self.main.get();
        let mut sizes: Vec<(u32, u32)> = STANDARD.iter().map(|s| (s.width, s.height)).collect();
        for extra in [main, selected].into_iter().flatten() {
            if extra != (0, 0) && !sizes.contains(&extra) {
                sizes.push(extra);
            }
        }
        sizes.sort_by_key(|&(w, h)| std::cmp::Reverse(u64::from(w) * u64::from(h)));
        let mut items: Vec<Item> = self
            .inherit_label
            .iter()
            .map(|l| Item {
                size: None,
                label: l.clone(),
            })
            .collect();
        items.push(Item {
            size: Some((0, 0)),
            label: self.none_label.clone(),
        });
        items.extend(sizes.into_iter().map(|s| Item {
            size: Some(s),
            label: label(s, main),
        }));
        let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
        let index = items.iter().position(|i| i.size == selected).unwrap_or(0);
        self.quiet.set(true);
        self.model.splice(0, self.model.n_items(), &labels);
        *self.items.borrow_mut() = items;
        self.row.set_selected(u32::try_from(index).unwrap_or(0));
        // The value's width follows its label, which may have changed.
        self.row.notify("selected");
        self.quiet.set(false);
    }

    /// The item chosen: `None` follows the general configuration.
    #[must_use]
    pub fn choice(&self) -> Option<(u32, u32)> {
        self.items
            .borrow()
            .get(self.row.selected() as usize)
            .and_then(|i| i.size)
    }

    /// The size chosen; (0, 0) for none, and for the general configuration.
    #[must_use]
    pub fn value(&self) -> (u32, u32) {
        self.choice().unwrap_or((0, 0))
    }

    /// Show `size`, adding an item for it when it is not one.
    pub fn set(&self, size: (u32, u32)) {
        self.set_choice(Some(size));
    }

    /// Show `choice` (`None`: the general configuration), adding an item for
    /// a size that is not one.
    pub fn set_choice(&self, choice: Option<(u32, u32)>) {
        let choice = self.normalise_choice(choice);
        let index = self.items.borrow().iter().position(|i| i.size == choice);
        if let Some(i) = index.and_then(|i| u32::try_from(i).ok()) {
            self.row.set_selected(i);
        } else {
            self.rebuild(choice);
            // A new item is a change like any other.
            self.row.notify("selected");
        }
    }

    /// Call `f` with the new size whenever the user chooses one.
    ///
    /// The row's handlers hold the picker: it lives as long as its row.
    pub fn connect_changed(self: &Rc<Self>, f: impl Fn((u32, u32)) + 'static) {
        self.connect_choice(move |c| f(c.unwrap_or((0, 0))));
    }

    /// Call `f` with the new choice (`None`: the general configuration)
    /// whenever the user makes one.
    pub fn connect_choice(self: &Rc<Self>, f: impl Fn(Option<(u32, u32)>) + 'static) {
        let me = Rc::clone(self);
        self.row.connect_selected_notify(move |_| {
            if !me.quiet.get() {
                f(me.choice());
            }
        });
    }

    /// Read the main screen's size (off the main thread) and name it in the
    /// list; with `button`, a button beside the value chooses it.
    pub fn with_main_screen(self: &Rc<Self>, button: bool) {
        let detect = gtk4::Button::builder()
            .icon_name("video-display-symbolic")
            .valign(gtk4::Align::Center)
            .css_classes(["flat", "circular"])
            .tooltip_text(i18n("Use the main screen's size"))
            .sensitive(false)
            .build();
        detect.update_property(&[gtk4::accessible::Property::Label(&i18n(
            "Use the main screen's size",
        ))]);
        if button {
            self.row.add_suffix(&detect);
            let me = Rc::clone(self);
            detect.connect_clicked(move |b| {
                if let Some(main) = me.main.get() {
                    me.set(main);
                } else {
                    crate::widgets::toast::show(
                        b,
                        &i18n("The main screen's size could not be read"),
                    );
                }
            });
        }
        let me = Rc::clone(self);
        glib::spawn_future_local(async move {
            let main = gio::spawn_blocking(screen::primary_size)
                .await
                .ok()
                .flatten();
            detect.set_sensitive(true);
            if let Some(main) = main {
                me.main.set(Some(main));
                me.rebuild(me.choice());
                detect.set_tooltip_text(Some(
                    &i18n("Use the main screen's size (%s)")
                        .replace("%s", &format!("{} × {}", main.0, main.1)),
                ));
            }
        });
    }
}
