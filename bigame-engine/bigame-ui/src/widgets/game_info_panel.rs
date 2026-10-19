//! A game's compatibility, on its page: how well it runs on Linux, its
//! anti-cheat, and who plays it on Steam, each with where it comes from,
//! when, and a link to the source.
//!
//! Three cards in a row (one under the other on a narrow window), filled
//! from what is known and again whenever more arrives. A count Steam keeps
//! is said as Steam's; the highest count Big Game Mode read itself is said
//! as that, with how many readings over how long, and "not enough history"
//! until there are enough. A chart of the readings appears only once there
//! are readings to draw.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use gtk4::glib;
use libadwaita as adw;

use bigame_core::game_info::{GameRef, Peak, Snapshot};

use crate::i18n::{i18n, labelled, ni18n};
use crate::widgets::game_info_tags::{self as tags, Said};
use crate::widgets::layout;

/// The panel of one game.
struct Panel {
    game: GameRef,
    groups: [adw::PreferencesGroup; 3],
    rows: RefCell<Vec<(usize, gtk4::Widget)>>,
}

/// Add the section and its three cards for `game` to `page`.
pub fn add(page: &layout::Page, game: &GameRef) {
    page.section(&i18n("Game Compatibility"));
    let groups = [
        layout::group(&i18n("Linux Compatibility"), "bigame-linux-symbolic"),
        layout::group(&i18n("Anti-Cheat"), "bigame-shield-symbolic"),
        layout::group(&i18n("Steam Activity"), "system-users-symbolic"),
    ];
    let update = gtk4::Button::builder()
        .icon_name("view-refresh-symbolic")
        .tooltip_text(i18n("Update Game Information"))
        .css_classes(["flat", "circular"])
        .valign(gtk4::Align::Center)
        .build();
    update.update_property(&[gtk4::accessible::Property::Label(&i18n(
        "Update Game Information",
    ))]);
    groups[2].set_header_suffix(Some(&update));
    let widgets: Vec<gtk4::Widget> = groups.iter().map(|g| g.clone().upcast()).collect();
    let refs: Vec<&gtk4::Widget> = widgets.iter().collect();
    page.row(&refs);
    let panel = Rc::new(Panel {
        game: game.clone(),
        groups,
        rows: RefCell::new(Vec::new()),
    });
    panel.fill(&crate::game_info::snapshot());
    // Alive as long as its cards are: the page keeps only the widgets, and
    // the information watch only a weak reference.
    {
        let keep = Rc::clone(&panel);
        panel.groups[0].connect_destroy(move |_| {
            let _ = &keep;
        });
    }
    {
        let game = game.clone();
        update.connect_clicked(move |_| crate::game_info::request(vec![game.clone()], true));
    }
    let weak = Rc::downgrade(&panel);
    crate::game_info::subscribe(move |snap, _| {
        let Some(p) = weak.upgrade() else {
            return glib::ControlFlow::Break;
        };
        p.fill(snap);
        glib::ControlFlow::Continue
    });
    // Asked for once when the page opens, if a pass may run.
    crate::game_info::request(vec![game.clone()], false);
}

impl Panel {
    fn add(&self, group: usize, widget: &impl IsA<gtk4::Widget>) {
        self.groups[group].add(widget);
        self.rows
            .borrow_mut()
            .push((group, widget.clone().upcast()));
    }

    fn fill(&self, snap: &Snapshot) {
        for (g, w) in self.rows.borrow_mut().drain(..) {
            self.groups[g].remove(&w);
        }
        let now = bigame_core::unix_now();
        let info = snap.info(&self.game, now);
        let linux_icon = tags::shown_tier(&info).map(tags::tier_look);
        self.said(0, &tags::linux_text(&info, now), linux_icon);
        let ac_icon = match &info.anticheat {
            bigame_core::game_info::AntiCheat::Listed { game, .. } => {
                Some(("bigame-shield-symbolic", tags::status_css(game.status)))
            }
            _ => None,
        };
        self.said(1, &tags::anticheat_text(&info, now), ac_icon);
        self.players(&info, now);
    }

    /// Rows for what `said` says, in group `group`: the headline with
    /// `look`'s icon and colour, the details, the caution and the link.
    fn said(&self, group: usize, said: &Said, look: Option<(&str, &str)>) {
        let mut lines = said.lines.iter();
        if let Some(head) = lines.next() {
            let row = adw::ActionRow::builder()
                .title(head)
                .subtitle(lines.next().cloned().unwrap_or_default())
                .subtitle_lines(4)
                .use_markup(false)
                .build();
            if let Some((icon, css)) = look {
                let img = gtk4::Image::from_icon_name(icon);
                img.add_css_class("game-info-icon");
                img.add_css_class(css);
                row.add_prefix(&img);
            }
            self.add(group, &row);
        }
        let rest: Vec<&String> = lines.collect();
        if !rest.is_empty() {
            let detail = adw::ActionRow::builder()
                .title(
                    rest.iter()
                        .map(|s| s.as_str())
                        .collect::<Vec<_>>()
                        .join("\n"),
                )
                .title_lines(12)
                .use_markup(false)
                .css_classes(["game-info-details"])
                .build();
            self.add(group, &detail);
        }
        if let Some(note) = &said.note {
            let row = adw::ActionRow::builder()
                .subtitle(note)
                .subtitle_lines(4)
                .use_markup(false)
                .build();
            row.add_prefix(&gtk4::Image::from_icon_name("dialog-information-symbolic"));
            self.add(group, &row);
        }
        if let Some((label, uri)) = &said.link {
            self.add(group, &link_row(label, uri));
        }
    }

    /// The Steam card: now, the day's peak, the readings.
    fn players(&self, info: &bigame_core::game_info::Info, now: u64) {
        let (Some(id), Some(p)) = (info.steam, info.players.as_ref()) else {
            let row = adw::ActionRow::builder()
                .title(i18n("Data unavailable"))
                .subtitle(i18n(
                    "Steam counts the players of Steam games; this one is not from Steam.",
                ))
                .subtitle_lines(3)
                .build();
            self.add(2, &row);
            return;
        };
        let now_row = adw::ActionRow::builder()
            .title(i18n("Players Now"))
            .use_markup(false)
            .subtitle_lines(3)
            .build();
        match p.now {
            Some((count, at)) => {
                now_row.add_suffix(&value_label(&bigame_core::text::grouped(count)));
                let mut sub = labelled(&i18n("Last Updated"), &tags::ago(at, now));
                if p.stale {
                    sub = format!("{sub} · {}", i18n("Cached data"));
                }
                now_row.set_subtitle(&sub);
            }
            None if p.not_counted => {
                now_row.set_subtitle(&i18n("Steam keeps no count for this game."));
            }
            None => now_row.set_subtitle(&i18n("Data unavailable")),
        }
        self.add(2, &now_row);

        let peak = adw::ActionRow::builder()
            .use_markup(false)
            .subtitle_lines(4)
            .build();
        match p.peak {
            Peak::SteamToday { peak: n, at } => {
                peak.set_title(&i18n("Peak today"));
                peak.add_suffix(&value_label(&bigame_core::text::grouped(n)));
                peak.set_subtitle(
                    &i18n("Steam's Most Played chart, %s").replace("%s", &tags::ago(at, now)),
                );
            }
            Peak::Observed(o) => {
                peak.set_title(&i18n("Observed Peak"));
                peak.add_suffix(&value_label(&bigame_core::text::grouped(o.peak)));
                peak.set_subtitle(&observed_text(o.samples, o.span));
            }
            Peak::NotEnough(o) => {
                peak.set_title(&i18n("Insufficient History"));
                let (samples, span) = o.map_or((0, 0), |o| (o.samples, o.span));
                peak.set_subtitle(&format!(
                    "{} {}",
                    observed_text(samples, span),
                    i18n("A day's peak needs readings over at least half a day.")
                ));
            }
        }
        self.add(2, &peak);

        // A chart only of real readings, two at least, over the last day.
        let day: Vec<f64> = p
            .samples
            .iter()
            .filter(|(t, _)| now.saturating_sub(*t) <= 24 * 3600)
            .map(|(_, c)| {
                #[allow(clippy::cast_precision_loss)]
                let v = *c as f64;
                v
            })
            .collect();
        if day.len() >= 2 {
            let top = day.iter().copied().fold(1.0, f64::max);
            let chart = crate::widgets::sparkline::chart(
                crate::widgets::sparkline::Scale::AtLeast(top),
                48,
            );
            for v in &day {
                chart.push(*v);
            }
            chart.area.set_margin_start(12);
            chart.area.set_margin_end(12);
            chart.area.set_margin_bottom(6);
            chart
                .area
                .set_tooltip_text(Some(&i18n("Counts read by Big Game Mode, last 24 hours")));
            self.add(2, &chart.area);
        }
        self.add(
            2,
            &link_row(
                &i18n("View on SteamDB"),
                &bigame_core::game_info::players::steamdb_url(id),
            ),
        );
    }
}

/// "The highest of 8 counts Big Game Mode read over 3 hours; not a record
/// of Steam's."
fn observed_text(samples: usize, span: u64) -> String {
    let hours = usize::try_from(span / 3600).unwrap_or(usize::MAX);
    format!(
        "{} {}",
        ni18n(
            "The highest of %n count Big Game Mode read",
            "The highest of %n counts Big Game Mode read",
            samples
        ),
        ni18n(
            "over %n hour; not a record of Steam's.",
            "over %n hours; not a record of Steam's.",
            hours
        )
    )
}

/// A number, as the row's value.
fn value_label(text: &str) -> gtk4::Label {
    let l = gtk4::Label::new(Some(text));
    l.add_css_class("title-4");
    l.add_css_class("numeric");
    l.set_valign(gtk4::Align::Center);
    l
}

/// A row that opens `uri` in the browser.
fn link_row(label: &str, uri: &str) -> adw::ActionRow {
    let row = adw::ActionRow::builder()
        .title(label)
        .subtitle(host(uri))
        .activatable(true)
        .use_markup(false)
        .build();
    row.add_suffix(&gtk4::Image::from_icon_name("adw-external-link-symbolic"));
    let uri = uri.to_owned();
    row.connect_activated(move |r| {
        let window = r.root().and_downcast::<gtk4::Window>();
        gtk4::UriLauncher::new(&uri).launch(window.as_ref(), gtk4::gio::Cancellable::NONE, |_| {});
    });
    row
}

/// The host of an address, to say where a link goes.
fn host(uri: &str) -> String {
    uri.split("://")
        .nth(1)
        .and_then(|rest| rest.split('/').next())
        .unwrap_or(uri)
        .to_owned()
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_link_says_where_it_goes() {
        assert_eq!(
            super::host("https://www.protondb.com/app/1091500"),
            "www.protondb.com"
        );
        assert_eq!(super::host("nothing"), "nothing");
    }
}
