//! A game card: cover art, title, launcher badge, profile status, actions.
//!
//! Reinterprets the reference design (a 2:3 poster grid where hovering a card
//! reveals its actions) in GTK4 and libadwaita rather than transplanting its
//! markup. Two things change in translation and both are deliberate:
//!
//! * the reference reveals actions on hover only. A pointer-only affordance is
//!   unreachable by keyboard, so here the same revealer is driven by focus as
//!   well, and the card is a focusable widget in the tab order.
//! * colours come from the libadwaita palette rather than the reference's
//!   fixed hexes, so the grid follows the desktop's light/dark and accent
//!   settings instead of imposing its own.
//!
//! Cover images are decoded off the main thread and cached process-wide. A
//! library of a few hundred titles would otherwise stall the first frame for
//! as long as it takes to decode a few hundred JPEGs.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

use adw::prelude::*;
use gtk4::{gdk, gio, glib};
use libadwaita as adw;

use crate::i18n::i18n;

/// Poster aspect ratio, matching the reference grid and Steam's own
/// `library_600x900` artwork.
const COVER_RATIO: f64 = 2.0 / 3.0;

/// Nominal cover width. The flow box scales cards around this.
const COVER_WIDTH: i32 = 160;

/// What a card shows.
#[derive(Debug, Clone)]
pub struct Entry {
    /// Title as the launcher names it.
    pub title: String,
    /// Process name a profile is keyed on.
    pub key: String,
    /// Launcher label, e.g. `Steam`.
    pub source: String,
    /// Cover art on disk, if a launcher cached one.
    pub cover: Option<PathBuf>,
    /// Whether a profile already exists for [`Entry::key`].
    pub has_profile: bool,
    /// Whether that profile ships with the system rather than being the user's.
    pub system_profile: bool,
    /// A command that starts this game directly, when one exists.
    ///
    /// `None` for anything that needs a launcher — Steam titles in particular.
    /// Features that need a handle on the game's own process are offered only
    /// when this is `Some`.
    pub launch_command: Option<Vec<String>>,
    /// Whether the key came from a real executable rather than the title.
    ///
    /// A title-keyed profile cannot match a process, so the card says so
    /// instead of quietly offering to create one that will do nothing.
    pub key_is_verified: bool,
    /// Where AI Graphics would work on this game: its install folder and
    /// Steam id. `None` for a profile with no installed game behind it.
    pub target: Option<bigame_core::graphics::Target>,
}

impl Entry {
    /// Status line under the title.
    #[must_use]
    pub fn status(&self) -> String {
        if !self.has_profile {
            return self.source.clone();
        }
        if self.system_profile {
            format!("{} · {}", self.source, i18n("Built-in profile"))
        } else {
            format!("{} · {}", self.source, i18n("Custom profile"))
        }
    }

    /// CSS class for the status dot, or `None` when there is no profile.
    #[must_use]
    pub fn status_css(&self) -> Option<&'static str> {
        if !self.has_profile {
            None
        } else if self.system_profile {
            Some("profile-dot-builtin")
        } else {
            Some("profile-dot-custom")
        }
    }
}

/// Process-wide texture cache, keyed by file path.
///
/// Cards are rebuilt whenever the list refreshes, so without this every refresh
/// would re-decode every cover.
type CoverCache = Rc<RefCell<HashMap<PathBuf, gdk::Texture>>>;

thread_local! {
    static COVERS: CoverCache = Rc::new(RefCell::new(HashMap::new()));
}

/// Build a card for `entry`.
///
/// `on_activate` fires when the card or its primary button is activated;
/// `on_menu` fires for the overflow button.
#[must_use]
#[allow(clippy::too_many_lines)]
pub fn build<A, M>(entry: &Entry, on_activate: A, on_menu: M) -> gtk4::Widget
where
    A: Fn(&Entry) + 'static,
    M: Fn(&Entry, &gtk4::Widget) + 'static,
{
    let cover = build_cover(entry);

    let title = gtk4::Label::builder()
        .label(&entry.title)
        .ellipsize(gtk4::pango::EllipsizeMode::End)
        .xalign(0.0)
        .max_width_chars(1) // let ellipsize do the work at any card width
        .tooltip_text(&entry.title)
        .build();
    title.add_css_class("heading");

    let status = gtk4::Label::builder()
        .label(entry.status())
        .ellipsize(gtk4::pango::EllipsizeMode::End)
        .xalign(0.0)
        .max_width_chars(1)
        .build();
    status.add_css_class("caption");
    status.add_css_class("dim-label");

    let text = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    text.append(&title);
    text.append(&status);

    // ── Actions, revealed on hover or focus ─────────────────────────────
    let primary = gtk4::Button::builder()
        .label(if entry.has_profile {
            i18n("Edit")
        } else {
            i18n("Optimize")
        })
        .hexpand(true)
        .css_classes(["suggested-action"])
        .build();

    let menu = gtk4::Button::builder()
        .icon_name("view-more-symbolic")
        .tooltip_text(i18n("More options"))
        .build();
    menu.update_property(&[gtk4::accessible::Property::Label(&i18n("More options"))]);

    let actions = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    actions.set_margin_top(6);
    actions.append(&primary);
    actions.append(&menu);

    let revealer = gtk4::Revealer::builder()
        .transition_type(gtk4::RevealerTransitionType::SlideUp)
        .transition_duration(180)
        .child(&actions)
        .reveal_child(false)
        .build();

    let card = gtk4::Box::new(gtk4::Orientation::Vertical, 6);
    card.add_css_class("game-card");
    card.set_width_request(COVER_WIDTH);
    card.append(&cover);
    card.append(&text);
    card.append(&revealer);

    // The card itself is focusable so keyboard users can reach every title,
    // and focus reveals the same actions a pointer hover does.
    card.set_focusable(true);
    card.set_can_focus(true);
    card.update_property(&[
        gtk4::accessible::Property::Label(&entry.title),
        gtk4::accessible::Property::Description(&entry.status()),
    ]);

    {
        let revealer = revealer.clone();
        let motion = gtk4::EventControllerMotion::new();
        let r_enter = revealer.clone();
        motion.connect_enter(move |_, _, _| r_enter.set_reveal_child(true));
        motion.connect_leave(move |c| {
            // Keep the actions up while something inside the card has focus,
            // otherwise tabbing into a button would hide the button.
            let focused = c
                .widget()
                .and_then(|w| w.root())
                .and_then(|r| r.focus())
                .is_some_and(|f| c.widget().is_some_and(|w| f.is_ancestor(&w) || f == w));
            if !focused {
                revealer.set_reveal_child(false);
            }
        });
        card.add_controller(motion);
    }
    {
        let revealer = revealer.clone();
        let focus = gtk4::EventControllerFocus::new();
        let r_in = revealer.clone();
        focus.connect_enter(move |_| r_in.set_reveal_child(true));
        focus.connect_leave(move |_| revealer.set_reveal_child(false));
        card.add_controller(focus);
    }

    // ── Activation ──────────────────────────────────────────────────────
    let on_activate = Rc::new(on_activate);
    {
        let entry = entry.clone();
        let handler = Rc::clone(&on_activate);
        primary.connect_clicked(move |_| handler(&entry));
    }
    {
        let entry = entry.clone();
        let handler = Rc::clone(&on_activate);
        let click = gtk4::GestureClick::new();
        click.connect_released(move |_, n, _, _| {
            if n == 1 {
                handler(&entry);
            }
        });
        card.add_controller(click);
    }
    {
        // Enter and Space activate the focused card, as they would a button.
        let entry = entry.clone();
        let handler = Rc::clone(&on_activate);
        let keys = gtk4::EventControllerKey::new();
        keys.connect_key_pressed(move |_, key, _, _| {
            if matches!(key, gdk::Key::Return | gdk::Key::KP_Enter | gdk::Key::space) {
                handler(&entry);
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
        card.add_controller(keys);
    }
    {
        let entry = entry.clone();
        let menu_btn = menu.clone();
        menu.connect_clicked(move |_| on_menu(&entry, menu_btn.upcast_ref::<gtk4::Widget>()));
    }

    card.upcast()
}

/// Cover image with the launcher badge and profile dot overlaid.
fn build_cover(entry: &Entry) -> gtk4::Widget {
    let height = f64::from(COVER_WIDTH) / COVER_RATIO;
    #[allow(clippy::cast_possible_truncation)]
    let height = height.round() as i32;

    let picture = gtk4::Picture::builder()
        .content_fit(gtk4::ContentFit::Cover)
        .width_request(COVER_WIDTH)
        .height_request(height)
        .build();
    picture.add_css_class("game-cover");

    // Placeholder first, so the grid lays out immediately and never reflows
    // when images arrive.
    let placeholder = gtk4::Image::from_icon_name("applications-games-symbolic");
    placeholder.set_pixel_size(48);
    placeholder.add_css_class("dim-label");
    placeholder.set_width_request(COVER_WIDTH);
    placeholder.set_height_request(height);

    let stack = gtk4::Stack::new();
    stack.add_named(&placeholder, Some("placeholder"));
    stack.add_named(&picture, Some("cover"));
    stack.set_transition_type(gtk4::StackTransitionType::Crossfade);
    stack.set_transition_duration(150);
    stack.add_css_class("game-cover-frame");

    if let Some(path) = entry.cover.clone() {
        load_cover_async(path, &picture, &stack, height);
    }

    let overlay = gtk4::Overlay::new();
    overlay.set_child(Some(&stack));

    // Launcher badge, bottom-left, as in the reference.
    let badge = gtk4::Label::new(Some(&entry.source));
    badge.add_css_class("caption");
    badge.add_css_class("game-card-badge");
    badge.set_halign(gtk4::Align::Start);
    badge.set_valign(gtk4::Align::End);
    badge.set_margin_start(6);
    badge.set_margin_bottom(6);
    overlay.add_overlay(&badge);

    // Profile indicator, top-right.
    if let Some(css) = entry.status_css() {
        let dot = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
        dot.add_css_class("profile-dot");
        dot.add_css_class(css);
        dot.set_halign(gtk4::Align::End);
        dot.set_valign(gtk4::Align::Start);
        dot.set_margin_end(6);
        dot.set_margin_top(6);
        dot.set_tooltip_text(Some(&entry.status()));
        overlay.add_overlay(&dot);
    }

    // A key that cannot match a process is worth saying out loud rather than
    // discovering later when the profile never activates.
    if !entry.key_is_verified {
        let warn = gtk4::Image::from_icon_name("dialog-warning-symbolic");
        warn.add_css_class("warning");
        warn.set_halign(gtk4::Align::Start);
        warn.set_valign(gtk4::Align::Start);
        warn.set_margin_start(6);
        warn.set_margin_top(6);
        warn.set_tooltip_text(Some(&i18n(
            "No game executable was found, so this profile may not match the running game.",
        )));
        overlay.add_overlay(&warn);
    }

    overlay.upcast()
}

/// Decode and downscale a cover off the main thread, then swap it in.
///
/// Scaling at load time is not only a memory saving. `GtkPicture` reports its
/// texture's size as its natural size, so a full-resolution 600×900 cover makes
/// every card ask for 600 px of width and the flow box drops to two columns
/// regardless of how wide the window is. Decoding straight to the display size
/// is what lets the grid reflow properly.
fn load_cover_async(path: PathBuf, picture: &gtk4::Picture, stack: &gtk4::Stack, height: i32) {
    if let Some(texture) = COVERS.with(|c| c.borrow().get(&path).cloned()) {
        picture.set_paintable(Some(&texture));
        stack.set_visible_child_name("cover");
        return;
    }

    let picture = picture.clone();
    let stack = stack.clone();
    glib::spawn_future_local(async move {
        let load_path = path.clone();
        // Decoding is the expensive part; gio's thread pool keeps it off the
        // main loop so the grid stays responsive while covers stream in.
        let texture = gio::spawn_blocking(move || {
            // Scale to fill the card, cropping rather than letterboxing, which
            // is what ContentFit::Cover then does with the result.
            gtk4::gdk_pixbuf::Pixbuf::from_file_at_scale(&load_path, COVER_WIDTH, height, false)
                .ok()
                .map(|pixbuf| gdk::Texture::for_pixbuf(&pixbuf))
        })
        .await
        .ok()
        .flatten();

        if let Some(texture) = texture {
            COVERS.with(|c| {
                c.borrow_mut().insert(path, texture.clone());
            });
            picture.set_paintable(Some(&texture));
            stack.set_visible_child_name("cover");
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(has_profile: bool, system: bool) -> Entry {
        Entry {
            title: "ARC Raiders".into(),
            key: "PioneerGame.exe".into(),
            source: "Steam".into(),
            cover: None,
            launch_command: None,
            has_profile,
            system_profile: system,
            key_is_verified: true,
            target: None,
        }
    }

    #[test]
    fn status_distinguishes_no_profile_from_the_two_kinds_of_profile() {
        assert_eq!(entry(false, false).status(), "Steam");
        assert_eq!(entry(true, false).status(), "Steam · Custom profile");
        assert_eq!(entry(true, true).status(), "Steam · Built-in profile");
    }

    #[test]
    fn only_a_game_with_a_profile_gets_a_status_dot() {
        assert_eq!(entry(false, false).status_css(), None);
        assert_eq!(entry(true, false).status_css(), Some("profile-dot-custom"));
        assert_eq!(entry(true, true).status_css(), Some("profile-dot-builtin"));
    }

    #[test]
    fn the_poster_ratio_matches_steam_artwork() {
        // Steam ships library_600x900, which is exactly 2:3.
        assert!((COVER_RATIO - 600.0 / 900.0).abs() < f64::EPSILON);
    }
}
