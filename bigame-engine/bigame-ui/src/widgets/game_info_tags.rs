//! The game information on a card: two tags over the cover (how well the
//! game runs on Linux, its anti-cheat) and a line of players under the
//! title, and what a tag says when it is opened.
//!
//! The tags take no room of their own: they sit over the cover's top
//! corner, under the launcher badge, where the actions (at the bottom, on
//! hover and focus) never reach; the players line is always there, empty or
//! not. Nothing a source says can move a card. A tag shows only what is known — a rating, a
//! listed status — and its colour is never all it says: each has an icon
//! and a word, an accessible name and a tooltip, and opens a popover that
//! explains it, says where it comes from and when, and links to the source.

use adw::prelude::*;
use gtk4::glib;
use libadwaita as adw;

use bigame_core::game_info::anticheat::{Match, Status};
use bigame_core::game_info::protondb::{Rating, Tier};
use bigame_core::game_info::{AntiCheat, GameRef, Info, Linux, Snapshot};

use crate::i18n::{i18n, labelled};
use crate::settings::Settings;

/// "4 minutes ago", for a time `at` seen at `now`.
#[must_use]
pub fn ago(at: u64, now: u64) -> String {
    let secs = now.saturating_sub(at);
    let n = |v: u64| usize::try_from(v).unwrap_or(usize::MAX);
    match secs {
        0..60 => i18n("just now"),
        60..3600 => crate::i18n::ni18n("%n minute ago", "%n minutes ago", n(secs / 60)),
        3600..86_400 => crate::i18n::ni18n("%n hour ago", "%n hours ago", n(secs / 3600)),
        _ => crate::i18n::ni18n("%n day ago", "%n days ago", n(secs / 86_400)),
    }
}

/// "31,351 playing now", grouped as the locale groups digits.
#[must_use]
pub fn playing_now(count: u64) -> String {
    let n = usize::try_from(count).unwrap_or(usize::MAX);
    crate::i18n::ni18n("%n playing now", "%n playing now", n).replacen(
        &n.to_string(),
        &bigame_core::text::grouped(count),
        1,
    )
}

/// "31,351 playing on Steam": the players of a game's Steam version, for a
/// game another launcher installed.
#[must_use]
pub fn playing_on_steam(count: u64) -> String {
    let n = usize::try_from(count).unwrap_or(usize::MAX);
    crate::i18n::ni18n("%n playing on Steam", "%n playing on Steam", n).replacen(
        &n.to_string(),
        &bigame_core::text::grouped(count),
        1,
    )
}

/// The icon and CSS class of a rating.
#[must_use]
pub fn tier_look(tier: Tier) -> (&'static str, &'static str) {
    match tier {
        Tier::Native => ("bigame-linux-symbolic", "tier-native"),
        Tier::Platinum => ("bigame-medal-symbolic", "tier-platinum"),
        Tier::Gold => ("bigame-medal-symbolic", "tier-gold"),
        Tier::Silver => ("bigame-medal-symbolic", "tier-silver"),
        Tier::Bronze => ("bigame-medal-symbolic", "tier-bronze"),
        Tier::Borked => ("action-unavailable-symbolic", "tier-borked"),
    }
}

/// The CSS class of an anti-cheat status.
#[must_use]
pub fn status_css(status: Status) -> &'static str {
    match status {
        Status::Supported => "ac-supported",
        Status::Running => "ac-running",
        Status::Planned => "ac-planned",
        Status::Broken => "ac-broken",
        Status::Denied => "ac-denied",
    }
}

/// The rating a card shows: Native when the copy installed is a Linux build
/// or Steam lists one, else `ProtonDB`'s tier; none when there is no rating
/// to show.
#[must_use]
pub fn shown_tier(info: &Info) -> Option<Tier> {
    match &info.linux {
        Linux::Native | Linux::LinuxBuild => Some(Tier::Native),
        Linux::Rating {
            rating: Rating::Summary(s),
            ..
        } => s.tier,
        _ => None,
    }
}

/// One tag: an icon and a word, in a flat button.
struct Tag {
    button: gtk4::Button,
    icon: gtk4::Image,
    label: gtk4::Label,
    /// The look class it has now, to take off when it changes.
    look: std::cell::RefCell<Option<&'static str>>,
}

impl Tag {
    fn new() -> Self {
        let icon = gtk4::Image::new();
        icon.set_pixel_size(12);
        let label = gtk4::Label::new(None);
        let content = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
        content.append(&icon);
        content.append(&label);
        let button = gtk4::Button::builder()
            .child(&content)
            .css_classes(["game-card-tag", "caption"])
            .visible(false)
            .build();
        Self {
            button,
            icon,
            label,
            look: std::cell::RefCell::new(None),
        }
    }

    fn set(&self, icon: &str, text: &str, look: &'static str, stale: bool, tooltip: &str) {
        self.icon.set_icon_name(Some(icon));
        self.label.set_label(text);
        if let Some(old) = self.look.replace(Some(look)) {
            self.button.remove_css_class(old);
        }
        self.button.add_css_class(look);
        if stale {
            self.button.add_css_class("stale");
        } else {
            self.button.remove_css_class("stale");
        }
        self.button.set_tooltip_text(Some(tooltip));
        self.button
            .update_property(&[gtk4::accessible::Property::Label(tooltip)]);
        self.button.set_visible(true);
    }
}

/// A card's game information.
#[derive(Clone)]
pub struct Badges(std::rc::Rc<Inner>);

struct Inner {
    game: GameRef,
    title: String,
    linux: Tag,
    anticheat: Tag,
    players: gtk4::Label,
}

impl Badges {
    /// The tags for the card of `game` (`title`), in `tags`, and its
    /// `players` line.
    #[must_use]
    pub fn new(game: &GameRef, title: &str, tags: &gtk4::Box, players: &gtk4::Label) -> Self {
        let inner = Inner {
            game: game.clone(),
            title: title.to_owned(),
            linux: Tag::new(),
            anticheat: Tag::new(),
            players: players.clone(),
        };
        tags.append(&inner.linux.button);
        tags.append(&inner.anticheat.button);
        let me = Self(std::rc::Rc::new(inner));
        for (button, which) in [
            (&me.0.linux.button, Which::Linux),
            (&me.0.anticheat.button, Which::AntiCheat),
        ] {
            let weak = std::rc::Rc::downgrade(&me.0);
            button.connect_clicked(move |b| {
                if let Some(inner) = weak.upgrade() {
                    open(b, &inner, which);
                }
            });
        }
        me
    }

    /// Show what `snap` knows of the game, as the settings `s` allow.
    pub fn show(&self, snap: &Snapshot, s: &Settings, now: u64) {
        let info = snap.info(&self.0.game, now);
        let i = &self.0;
        match shown_tier(&info).filter(|_| s.show_linux_rating) {
            Some(tier) => {
                let (icon, look) = tier_look(tier);
                let stale = matches!(info.linux, Linux::Rating { stale: true, .. });
                let tooltip = match (&info.linux, &info.steam_version) {
                    (Linux::LinuxBuild, _) => {
                        i18n("A Linux build: it runs here without Proton or Wine")
                    }
                    (Linux::Native, _) => labelled("Steam", tier.name()),
                    (_, Some(v)) => format!(
                        "{} · {}",
                        labelled("ProtonDB", tier.name()),
                        labelled(&i18n("Steam version"), &v.name)
                    ),
                    (_, None) => labelled("ProtonDB", tier.name()),
                };
                i.linux.set(icon, tier.name(), look, stale, &tooltip);
            }
            None => i.linux.button.set_visible(false),
        }
        match &info.anticheat {
            AntiCheat::Listed { game, .. } if s.show_anticheat => {
                let status = game.status.name().to_owned();
                i.anticheat.set(
                    "bigame-shield-symbolic",
                    &status,
                    status_css(game.status),
                    false,
                    &labelled(&i18n("Anti-cheat"), &status),
                );
            }
            _ => i.anticheat.button.set_visible(false),
        }
        let players = info
            .players
            .as_ref()
            .and_then(|p| p.now)
            .filter(|_| s.show_players)
            .map(|(n, _)| {
                if info.steam_version.is_some() {
                    playing_on_steam(n)
                } else {
                    playing_now(n)
                }
            })
            .unwrap_or_default();
        i.players.set_label(&players);
    }
}

#[derive(Clone, Copy)]
enum Which {
    Linux,
    AntiCheat,
}

/// Open the popover of a tag: what it means, where it comes from, when,
/// and a link to the source.
fn open(button: &gtk4::Button, inner: &Inner, which: Which) {
    let now = bigame_core::unix_now();
    let info = crate::game_info::snapshot().info(&inner.game, now);
    let content = match which {
        Which::Linux => linux_text(&info, now),
        Which::AntiCheat => anticheat_text(&info, now),
    };
    let body = gtk4::Box::new(gtk4::Orientation::Vertical, 6);
    body.set_margin_top(6);
    body.set_margin_bottom(6);
    body.set_margin_start(6);
    body.set_margin_end(6);
    body.set_width_request(260);
    let heading = gtk4::Label::builder()
        .label(&inner.title)
        .css_classes(["heading"])
        .xalign(0.0)
        .wrap(true)
        .build();
    body.append(&heading);
    for (i, line) in content.lines.iter().enumerate() {
        let l = gtk4::Label::builder()
            .label(line)
            .xalign(0.0)
            .wrap(true)
            .max_width_chars(36)
            .build();
        if i == 0 {
            l.add_css_class("title-4");
        } else {
            l.add_css_class("caption");
        }
        body.append(&l);
    }
    if let Some(note) = &content.note {
        let l = gtk4::Label::builder()
            .label(note)
            .xalign(0.0)
            .wrap(true)
            .max_width_chars(36)
            .css_classes(["caption", "dim-label"])
            .build();
        body.append(&l);
    }
    if let Some((label, uri)) = &content.link {
        let link = gtk4::LinkButton::with_label(uri, label);
        link.set_halign(gtk4::Align::Start);
        body.append(&link);
    }
    let popover = gtk4::Popover::builder().child(&body).build();
    popover.set_parent(button);
    popover.connect_closed(|p| {
        let p = p.clone();
        glib::idle_add_local_once(move || p.unparent());
    });
    popover.popup();
}

/// What a popover or the details page says.
pub struct Said {
    /// The first is the headline; the rest, the details.
    pub lines: Vec<String>,
    /// A caution, dimmed.
    pub note: Option<String>,
    /// The source's page: its label and address.
    pub link: Option<(String, String)>,
}

/// How well the game runs, in words.
#[must_use]
pub fn linux_text(info: &Info, now: u64) -> Said {
    let mut lines = Vec::new();
    let link = info
        .steam
        .filter(|_| info.linux != Linux::LinuxBuild)
        .map(|id| {
            (
                i18n("View on ProtonDB"),
                bigame_core::game_info::protondb::page_url(id),
            )
        });
    match &info.linux {
        Linux::LinuxBuild => {
            lines.push(Tier::Native.name().to_owned());
            lines.push(i18n(
                "This copy is a Linux build: it runs here without Proton or Wine.",
            ));
            lines.push(i18n("Source: the game's files on this computer"));
        }
        Linux::Native => {
            lines.push(Tier::Native.name().to_owned());
            lines.push(i18n(Tier::Native.meaning()));
            lines.push(i18n("Source: Steam's store information on this computer"));
        }
        Linux::Rating { rating, at, stale } => {
            match rating {
                Rating::Summary(s) => {
                    if let Some(t) = s.tier {
                        lines.push(t.name().to_owned());
                        lines.push(i18n(t.meaning()));
                    } else {
                        lines.push(i18n("Not rated yet"));
                        lines.push(i18n("Too few reports for ProtonDB to rate it."));
                    }
                    if let Some(trending) = s.trending.filter(|t| Some(*t) != s.tier) {
                        lines.push(labelled(&i18n("Recent reports"), trending.name()));
                    }
                    lines.push(crate::i18n::ni18n(
                        "%n report",
                        "%n reports",
                        s.reports as usize,
                    ));
                }
                Rating::NoReports => {
                    lines.push(i18n("No reports yet"));
                    lines.push(i18n("Nobody has reported on this game on ProtonDB."));
                }
            }
            lines.push(fetched("ProtonDB", *at, *stale, now));
        }
        Linux::NotFetched => lines.push(i18n("Compatibility unknown")),
        Linux::Unavailable => {
            lines.push(i18n("Data unavailable"));
            lines.push(i18n(
                "ProtonDB could not be reached; it is asked again later.",
            ));
        }
        Linux::NoSteamId => {
            lines.push(i18n("Compatibility unknown"));
            lines.push(i18n(
                "ProtonDB rates Steam games, and no Steam version of this game was found by its title.",
            ));
        }
    }
    if let Some(v) = &info.steam_version {
        lines.push(labelled(&i18n("Steam version"), &v.name));
        lines.push(i18n(
            "Said of its Steam version, found by its title: the copy from another store is the same game, though its launcher and updates may differ.",
        ));
    }
    Said {
        lines,
        note: Some(i18n(
            "ProtonDB's ratings are players' reports, not a promise: a game's update can change them.",
        )),
        link,
    }
}

/// "Source: `ProtonDB` · 4 minutes ago", said to be cached when it is old.
fn fetched(source: &str, at: u64, stale: bool, now: u64) -> String {
    let when = ago(at, now);
    let text = labelled(&i18n("Source"), &format!("{source} · {when}"));
    if stale {
        format!("{text} · {}", i18n("Cached data"))
    } else {
        text
    }
}

/// The anti-cheat, in words.
#[must_use]
pub fn anticheat_text(info: &Info, now: u64) -> Said {
    let mut lines = Vec::new();
    let mut link = None;
    let mut note = Some(i18n(
        "Are We Anti-Cheat Yet? is a community list: a status can change with an update of the game, and none is a promise that an account is safe.",
    ));
    match &info.anticheat {
        AntiCheat::Listed { game, how } => {
            lines.push(game.status.name().to_owned());
            lines.push(i18n(game.status.meaning()));
            if !game.anticheats.is_empty() {
                lines.push(labelled(&i18n("Anti-cheat"), &game.anticheats.join(", ")));
            }
            if let Some(changed) = game.changed.as_deref().and_then(date) {
                lines.push(labelled(&i18n("Last change in the list"), &changed));
            }
            for (text, _) in game.notes.iter().take(3) {
                lines.push(format!("• {text}"));
            }
            match how {
                Match::SteamIdAmbiguous => lines.push(i18n(
                    "The list has more than one game under this Steam id; this is the newest, and may be another edition.",
                )),
                Match::Title => lines.push(i18n(
                    "Found by its exact title: the list has no store id for this copy.",
                )),
                Match::SteamId | Match::SteamIdAndTitle => {}
            }
            if let Some(at) = info.anticheat_at {
                lines.push(labelled(
                    &i18n("Source"),
                    &format!("Are We Anti-Cheat Yet? · {}", ago(at, now)),
                ));
            }
            link = Some((i18n("View Anti-Cheat Details"), game.page_url()));
        }
        AntiCheat::NotListed => {
            lines.push(i18n("Not listed"));
            lines.push(i18n(
                "The list does not have this game: most games without anti-cheat are not in it.",
            ));
        }
        AntiCheat::Unavailable => {
            lines.push(i18n("Data unavailable"));
            lines.push(i18n("The anti-cheat list has not been downloaded yet."));
            note = None;
        }
    }
    Said { lines, note, link }
}

/// An ISO 8601 date from the list (`2025-07-16T20:26:00…`) in the user's
/// format; `None` when it is not one.
fn date(iso: &str) -> Option<String> {
    let day = iso.get(..10)?;
    let mut parts = day.split('-').map(|p| p.parse::<i32>().ok());
    let (y, m, d) = (parts.next()??, parts.next()??, parts.next()??);
    let dt = glib::DateTime::from_local(y, m, d, 12, 0, 0.0).ok()?;
    dt.format("%x").ok().map(|s| s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times_are_said_in_the_largest_whole_unit() {
        assert_eq!(ago(100, 130), "just now");
        assert_eq!(ago(0, 240), "4 minutes ago");
        assert_eq!(ago(0, 7200), "2 hours ago");
        assert_eq!(ago(0, 86_400 * 3), "3 days ago");
    }

    #[test]
    fn every_tier_and_status_has_a_look_that_is_not_only_a_colour() {
        for t in [
            Tier::Native,
            Tier::Platinum,
            Tier::Gold,
            Tier::Silver,
            Tier::Bronze,
            Tier::Borked,
        ] {
            let (icon, css) = tier_look(t);
            assert!(icon.ends_with("-symbolic") && css.starts_with("tier-"));
        }
        assert_eq!(status_css(Status::Denied), "ac-denied");
    }

    #[test]
    fn a_date_from_the_list_reads_and_garbage_does_not() {
        assert!(date("2025-07-16T20:26:00.0000000-04:00").is_some());
        assert_eq!(date("soon"), None);
        assert_eq!(date(""), None);
    }
}
