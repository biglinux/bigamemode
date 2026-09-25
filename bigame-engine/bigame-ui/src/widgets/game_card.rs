//! A game card: cover art, title, launcher badge, profile status, actions.
//!
//! Every card is the same size, whatever it shows. The poster is a fixed
//! 2:3 box; the cover is painted *over* it rather than inside it, so a
//! texture's size never reaches the layout, and a placeholder, a portrait
//! cover and a landscape capsule all occupy exactly the same pixels. The
//! actions are overlaid on the poster too, and revealed by opacity, so
//! hovering a card moves nothing around it. Titles take one line and
//! ellipsise; the full title is the tooltip. The grid therefore only ever
//! changes its number of columns.
//!
//! Two other choices are deliberate:
//!
//! * hovering *or focusing* a card reveals its actions. A pointer-only
//!   affordance is unreachable by keyboard, so the reveal is driven by focus
//!   as well, and the card is a focusable widget in the tab order.
//! * colours come from the libadwaita palette rather than fixed hexes, so the
//!   grid follows the desktop's light/dark and accent settings instead of
//!   imposing its own.
//!
//! Cover images are decoded off the main thread, cropped to the poster at the
//! display's scale factor, and cached process-wide. A library of a few
//! hundred titles would otherwise stall the first frame for as long as it
//! takes to decode a few hundred JPEGs.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use adw::prelude::*;
use gtk4::{gdk, gio, glib};
use libadwaita as adw;

use crate::i18n::i18n;

/// Poster size, in logical pixels: 2:3, matching Steam's `library_600x900`
/// artwork. Wide enough for the actions row in any language the interface
/// ships, narrow enough for three columns in the default window.
pub const POSTER_WIDTH: i32 = 176;
/// See [`POSTER_WIDTH`].
pub const POSTER_HEIGHT: i32 = 264;

/// What a card shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// Title as the launcher names it.
    pub title: String,
    /// Process name a profile is, or would be, keyed on.
    pub key: String,
    /// Launcher label, e.g. `Steam`.
    pub source: String,
    /// Cover art on disk, if a launcher cached one.
    pub cover: Option<PathBuf>,
    /// Icon name to stand in for a missing cover, from the application menu.
    pub icon: Option<String>,
    /// Whether a profile already exists for [`Entry::key`].
    pub has_profile: bool,
    /// Whether that profile ships with the system rather than being the user's.
    pub system_profile: bool,
    /// The profile's file stem, when there is one: what opens and deletes it.
    pub profile_stem: Option<String>,
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
    /// Steam id. `None` when the launcher records no install folder.
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

/// Process-wide texture cache, keyed by file path and display scale.
///
/// Cards are rebuilt whenever the library changes, so without this every
/// rebuild would re-decode every cover.
type CoverCache = Rc<RefCell<HashMap<(PathBuf, i32), gdk::Texture>>>;

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
    // ── Actions, overlaid on the poster ─────────────────────────────────
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
    actions.add_css_class("game-card-actions");
    actions.set_halign(gtk4::Align::Fill);
    actions.set_valign(gtk4::Align::End);
    // Hidden actions must not swallow the clicks meant for the card.
    actions.set_can_target(false);
    actions.append(&primary);
    actions.append(&menu);

    let poster = build_poster(entry, &actions);

    // ── Text ────────────────────────────────────────────────────────────
    let title = gtk4::Label::builder()
        .label(&entry.title)
        .ellipsize(gtk4::pango::EllipsizeMode::End)
        .single_line_mode(true)
        .xalign(0.0)
        .max_width_chars(1) // let ellipsize do the work at the card's width
        .tooltip_text(&entry.title)
        .build();
    title.add_css_class("heading");

    let status = gtk4::Label::builder()
        .label(entry.status())
        .ellipsize(gtk4::pango::EllipsizeMode::End)
        .single_line_mode(true)
        .xalign(0.0)
        .max_width_chars(1)
        .build();
    status.add_css_class("caption");
    status.add_css_class("dim-label");

    let text = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    text.append(&title);
    text.append(&status);

    let card = gtk4::Box::new(gtk4::Orientation::Vertical, 6);
    card.add_css_class("game-card");
    // The poster fixes the width; the flow box's cell may be wider, and the
    // card sits centred in it rather than stretching.
    card.set_halign(gtk4::Align::Center);
    card.set_valign(gtk4::Align::Start);
    card.append(&poster);
    card.append(&text);

    // The card itself is focusable so keyboard users can reach every title,
    // and focus reveals the same actions a pointer hover does.
    card.set_focusable(true);
    card.set_can_focus(true);
    card.update_property(&[
        gtk4::accessible::Property::Label(&entry.title),
        gtk4::accessible::Property::Description(&entry.status()),
    ]);

    let reveal = {
        let actions = actions.clone();
        move |shown: bool| {
            if shown {
                actions.add_css_class("revealed");
            } else {
                actions.remove_css_class("revealed");
            }
            actions.set_can_target(shown);
        }
    };
    {
        let reveal = reveal.clone();
        let motion = gtk4::EventControllerMotion::new();
        let r_enter = reveal.clone();
        motion.connect_enter(move |_, _, _| r_enter(true));
        motion.connect_leave(move |c| {
            // Keep the actions up while something inside the card has focus,
            // otherwise tabbing into a button would hide the button.
            let focused = c
                .widget()
                .and_then(|w| w.root())
                .and_then(|r| r.focus())
                .is_some_and(|f| c.widget().is_some_and(|w| f.is_ancestor(&w) || f == w));
            if !focused {
                reveal(false);
            }
        });
        card.add_controller(motion);
    }
    {
        let focus = gtk4::EventControllerFocus::new();
        let r_in = reveal.clone();
        focus.connect_enter(move |_| r_in(true));
        focus.connect_leave(move |_| reveal(false));
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

/// The fixed-size poster: placeholder underneath, cover painted over it, and
/// the badge, profile dot, warning and `actions` on top.
///
/// The placeholder box is the only child that takes part in layout. The
/// picture is an overlay child, which `GtkOverlay` allocates to the main
/// child's size and leaves out of its measurement, so whatever the texture's
/// dimensions, the poster stays [`POSTER_WIDTH`] × [`POSTER_HEIGHT`].
fn build_poster(entry: &Entry, actions: &gtk4::Box) -> gtk4::Widget {
    let placeholder = gtk4::Image::from_icon_name(
        entry
            .icon
            .as_deref()
            .unwrap_or("applications-games-symbolic"),
    );
    placeholder.set_pixel_size(if entry.icon.is_some() { 64 } else { 48 });
    placeholder.add_css_class("dim-label");
    placeholder.set_halign(gtk4::Align::Center);
    placeholder.set_valign(gtk4::Align::Center);
    placeholder.set_hexpand(true);
    placeholder.set_vexpand(true);

    let frame = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    frame.add_css_class("game-cover-frame");
    frame.set_size_request(POSTER_WIDTH, POSTER_HEIGHT);
    frame.set_halign(gtk4::Align::Center);
    frame.set_valign(gtk4::Align::Start);
    frame.append(&placeholder);

    let picture = gtk4::Picture::builder()
        .content_fit(gtk4::ContentFit::Cover)
        .can_shrink(true)
        .halign(gtk4::Align::Fill)
        .valign(gtk4::Align::Fill)
        .visible(false)
        .build();
    picture.add_css_class("game-cover");

    let overlay = gtk4::Overlay::new();
    overlay.set_child(Some(&frame));
    overlay.set_halign(gtk4::Align::Center);
    overlay.add_overlay(&picture);
    overlay.set_measure_overlay(&picture, false);
    overlay.set_clip_overlay(&picture, true);

    if let Some(path) = entry.cover.clone() {
        load_cover_async(path, &picture);
    }

    // Launcher badge and, when the key is a guess, a warning: top-left.
    let corner = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    corner.set_halign(gtk4::Align::Start);
    corner.set_valign(gtk4::Align::Start);
    corner.set_margin_start(6);
    corner.set_margin_top(6);
    let badge = gtk4::Label::new(Some(&entry.source));
    badge.add_css_class("caption");
    badge.add_css_class("game-card-badge");
    corner.append(&badge);
    // A key that cannot match a process is worth saying out loud rather than
    // discovering later when the profile never activates.
    if !entry.key_is_verified {
        let warn = gtk4::Image::from_icon_name("dialog-warning-symbolic");
        warn.add_css_class("warning");
        warn.set_tooltip_text(Some(&i18n(
            "No game executable was found, so this profile may not match the running game.",
        )));
        corner.append(&warn);
    }
    overlay.add_overlay(&corner);

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

    overlay.add_overlay(actions);
    overlay.set_clip_overlay(actions, true);

    overlay.upcast()
}

/// Decode a cover off the main thread, cropped to the poster, then show it.
///
/// Decoding to the poster's size at the display's scale factor is what keeps
/// covers sharp and memory bounded; cropping to 2:3 there is what keeps a
/// landscape capsule from being squashed or letterboxed.
fn load_cover_async(path: PathBuf, picture: &gtk4::Picture) {
    let scale = picture.scale_factor().max(1);
    if let Some(texture) = COVERS.with(|c| c.borrow().get(&(path.clone(), scale)).cloned()) {
        picture.set_paintable(Some(&texture));
        picture.set_visible(true);
        return;
    }

    let picture = picture.clone();
    glib::spawn_future_local(async move {
        let load_path = path.clone();
        // Decoding is the expensive part; gio's thread pool keeps it off the
        // main loop so the grid stays responsive while covers stream in.
        let texture = gio::spawn_blocking(move || {
            decode_cover(&load_path, POSTER_WIDTH * scale, POSTER_HEIGHT * scale)
        })
        .await
        .ok()
        .flatten();

        if let Some(texture) = texture {
            COVERS.with(|c| {
                c.borrow_mut().insert((path, scale), texture.clone());
            });
            picture.set_paintable(Some(&texture));
            picture.set_visible(true);
        }
    });
}

/// The image at `path` scaled to cover `width` × `height` and cropped to it,
/// centred.
fn decode_cover(path: &Path, width: i32, height: i32) -> Option<gdk::Texture> {
    use gtk4::gdk_pixbuf::Pixbuf;
    // The header only, to know the aspect ratio before decoding.
    let (_, source_w, source_h) = Pixbuf::file_info(path)?;
    if source_w <= 0 || source_h <= 0 {
        return None;
    }
    let scale = f64::max(
        f64::from(width) / f64::from(source_w),
        f64::from(height) / f64::from(source_h),
    );
    #[allow(clippy::cast_possible_truncation)]
    let scaled = |n: i32| (f64::from(n) * scale).ceil() as i32;
    let pixbuf = Pixbuf::from_file_at_scale(
        path,
        scaled(source_w).max(width),
        scaled(source_h).max(height),
        false,
    )
    .ok()?;
    let crop_w = width.min(pixbuf.width());
    let crop_h = height.min(pixbuf.height());
    let cropped = pixbuf.new_subpixbuf(
        (pixbuf.width() - crop_w) / 2,
        (pixbuf.height() - crop_h) / 2,
        crop_w,
        crop_h,
    );
    Some(gdk::Texture::for_pixbuf(&cropped))
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
            icon: None,
            launch_command: None,
            has_profile,
            system_profile: system,
            profile_stem: has_profile.then(|| "PioneerGame.exe".to_owned()),
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
    fn the_poster_is_a_steam_portrait() {
        // Steam ships library_600x900, which is exactly 2:3.
        assert_eq!(POSTER_WIDTH * 3, POSTER_HEIGHT * 2);
    }
}
