//! The Optimization Report.
//!
//! What Turbo did, grouped by what happened to each thing it considered:
//! applied and verified, left to the component that owns it, skipped,
//! unavailable, a conflict avoided, or failed. Every row says who owns the
//! state, and its ⓘ says what the thing is.
//!
//! **What was changed** and **what got faster** are different claims: changes
//! are listed with their verification, and no speed is claimed here. Measure
//! the difference, in a game card's menu, is what measures a game.

use adw::prelude::*;
use libadwaita as adw;

use bigame_core::turbo::{Item, Kind, Report, Section};

use crate::i18n::{i18n, tr};
use crate::widgets::info;

/// The report's groups, in the order they are read.
const SECTIONS: &[Section] = &[
    Section::Verified,
    Section::ManagedPerGame,
    Section::Restored,
    Section::ConflictAvoided,
    Section::Failed,
    Section::Skipped,
    Section::Unavailable,
];

fn section_title(section: Section) -> String {
    match section {
        Section::Verified => i18n("Applied and verified"),
        Section::Restored => i18n("Put back"),
        Section::ManagedPerGame => i18n("Managed per game"),
        Section::Skipped => i18n("Skipped"),
        Section::Unavailable => i18n("Not available on this machine"),
        Section::ConflictAvoided => i18n("Conflicts avoided"),
        Section::Failed => i18n("Did not take effect"),
    }
}

fn section_description(section: Section) -> Option<String> {
    Some(match section {
        Section::Verified => i18n("Each was read back from the system after it was changed."),
        Section::ManagedPerGame => i18n(
            "Applied while each game runs by the component that owns it, and restored when the game exits.",
        ),
        Section::Skipped => i18n("Considered and deliberately left alone."),
        Section::ConflictAvoided => i18n(
            "Another tool that would control the same settings. Only one controller is used for each setting.",
        ),
        _ => return None,
    })
}

fn section_icon(section: Section) -> &'static str {
    match section {
        Section::Verified | Section::Restored => "object-select-symbolic",
        Section::ManagedPerGame => "system-users-symbolic",
        Section::Skipped => "action-unavailable-symbolic",
        Section::Unavailable => "window-close-symbolic",
        Section::ConflictAvoided => "dialog-warning-symbolic",
        Section::Failed => "dialog-error-symbolic",
    }
}

fn kind_title(kind: &Kind) -> String {
    match kind {
        Kind::GameBackend => i18n("Per-game optimization (falcond)"),
        Kind::ProfileSet => i18n("falcond profile set"),
        Kind::GameMode => i18n("Feral GameMode"),
        Kind::Scheduler => i18n("sched-ext scheduler"),
        // Only reports saved before titles were translatable get here: the
        // title is in English, and the GPU knob carries its card.
        Kind::Knob(name) => match name
            .strip_prefix("GPU power level (")
            .and_then(|rest| rest.strip_suffix(')'))
        {
            Some(card) => i18n("GPU power level (%s)").replace("%s", card),
            None => i18n(name),
        },
    }
}

fn kind_explanation(kind: &Kind) -> String {
    match kind {
        Kind::GameBackend => i18n(
            "falcond applies a profile to each game while it runs and restores everything when it exits. Turbo turns it on and off: with Turbo off it does not run at all.",
        ),
        Kind::ProfileSet => i18n(
            "falcond ships separate profile sets for desktops, handhelds and home-theatre PCs. The handheld set runs games in power-saving mode, which is the wrong set for a desktop.",
        ),
        Kind::GameMode => i18n(
            "Feral GameMode does the same job as falcond. Running both would make two controllers save and restore the same settings, and the one that restores last writes the other's changes back as if they were the original. Only falcond is used.",
        ),
        Kind::Scheduler => i18n(
            "sched-ext lets a scheduler loaded at runtime replace the kernel's CPU scheduler. falcond switches it per game when a profile asks for one, which needs the scx_loader service.",
        ),
        Kind::Knob(_) => i18n(
            "A system-wide setting BiGame-mode's own planner considers. It is applied only if nothing else owns it and it is known to help.",
        ),
    }
}

fn item_title(item: &Item) -> String {
    item.title
        .as_ref()
        .map_or_else(|| kind_title(&item.kind), tr)
}

fn item_detail(item: &Item) -> String {
    item.text.as_ref().map_or_else(|| item.detail.clone(), tr)
}

/// The component named as the owner. Component names are shown as they are,
/// except Booster, which names BiGame-mode's own planner.
fn owner_label(owner: &str) -> String {
    match owner {
        "Booster" => i18n("Booster"),
        other => other.to_owned(),
    }
}

fn item_row(item: &Item) -> adw::ActionRow {
    let title = item_title(item);
    let owner_name = owner_label(&item.owner);
    // Plain text: details carry process names, paths and error messages,
    // any of which can contain `&` or `<`.
    let row = adw::ActionRow::builder()
        .title(&title)
        .subtitle(item_detail(item))
        .subtitle_lines(3)
        .use_markup(false)
        .build();
    let icon = gtk4::Image::from_icon_name(section_icon(item.section));
    icon.add_css_class("dim-label");
    row.add_prefix(&icon);
    let owner = gtk4::Label::new(Some(&owner_name));
    owner.add_css_class("caption");
    owner.add_css_class("dim-label");
    row.add_suffix(&owner);
    row.add_suffix(&info::button(
        &title,
        &format!(
            "{}\n\n{}: {}\n{}: {}",
            kind_explanation(&item.kind),
            i18n("Controlled by"),
            owner_name,
            i18n("Outcome"),
            section_title(item.section)
        ),
    ));
    row
}

/// What is in force right now, read live — not recalled from the report.
fn live_group() -> Option<adw::PreferencesGroup> {
    let game = crate::game_watch::current()?;
    let group = adw::PreferencesGroup::new();
    group.set_title(&format!("{} · {}", i18n("Right now"), game.display_name));

    let status = bigame_core::status::read();
    let profile = status
        .as_ref()
        .and_then(|s| s.active_profile.clone())
        .map_or_else(
            || i18n("none — Turbo is off or falcond has not matched it"),
            |p| {
                if p == "Proton" {
                    i18n("falcond's general Proton profile")
                } else {
                    p
                }
            },
        );
    let power = bigame_core::dbus::power_profile_get().unwrap_or_else(|| i18n("unknown"));
    let rows = [
        (i18n("Profile"), profile),
        (i18n("Process"), game.process_name.clone()),
        (i18n("Graphics"), game.graphics.label().to_owned()),
        (i18n("Power profile"), power),
        (
            i18n("Screen blanking"),
            if status.as_ref().is_some_and(|s| s.screensaver_inhibited) {
                i18n("held off while the game runs")
            } else {
                i18n("not held off")
            },
        ),
        (
            i18n("Scheduler"),
            status
                .as_ref()
                .map(|s| s.current_scx.clone())
                .filter(|s| !s.is_empty() && s != "(None)")
                .unwrap_or_else(|| i18n("kernel default")),
        ),
    ];
    for (title, value) in rows {
        let row = adw::ActionRow::builder()
            .title(title)
            .subtitle(value)
            .use_markup(false)
            .build();
        group.add(&row);
    }
    Some(group)
}

/// Build a page presenting `report`.
#[must_use]
pub fn build(report: &Report) -> gtk4::Widget {
    let page = adw::PreferencesPage::new();

    let head = adw::PreferencesGroup::new();
    head.set_title(&if report.turned_on {
        i18n("Optimization Report")
    } else {
        i18n("Turbo turned off")
    });
    head.set_description(Some(&if report.turned_on {
        i18n("What Turbo did when it was turned on, and why.")
    } else {
        i18n("What was put back when Turbo was turned off.")
    }));
    page.add(&head);

    if let Some(live) = live_group() {
        page.add(&live);
    }

    for section in SECTIONS {
        let items: Vec<&Item> = report
            .items
            .iter()
            .filter(|i| i.section == *section)
            .collect();
        if items.is_empty() {
            continue;
        }
        let group = adw::PreferencesGroup::new();
        group.set_title(&section_title(*section));
        if let Some(d) = section_description(*section) {
            group.set_description(Some(&d));
        }
        for item in items {
            group.add(&item_row(item));
        }
        page.add(&group);
    }

    page.upcast()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_section_has_a_title_and_an_icon() {
        for section in SECTIONS {
            assert!(!section_title(*section).is_empty());
            assert!(!section_icon(*section).is_empty());
        }
    }

    #[test]
    fn every_kind_explains_itself() {
        for kind in [
            Kind::GameBackend,
            Kind::ProfileSet,
            Kind::GameMode,
            Kind::Scheduler,
            Kind::Knob("Power profile".into()),
        ] {
            assert!(!kind_title(&kind).is_empty());
            assert!(kind_explanation(&kind).len() > 40, "{kind:?}");
        }
    }

    #[test]
    fn rows_prefer_the_translatable_sentences_and_fall_back_to_english() {
        use bigame_core::text::Text;
        let old = Item {
            kind: Kind::Knob("GPU power level (card1)".into()),
            section: Section::Skipped,
            owner: "Booster".into(),
            detail: "left to the driver".into(),
            text: None,
            title: None,
        };
        // No catalogue is loaded in tests, so gettext returns the English.
        assert_eq!(item_title(&old), "GPU power level (card1)");
        assert_eq!(item_detail(&old), "left to the driver");
        let new = Item {
            text: Some(Text::raw("from the sentence")),
            title: Some(Text::raw("Titled")),
            ..old
        };
        assert_eq!(item_title(&new), "Titled");
        assert_eq!(item_detail(&new), "from the sentence");
        assert_eq!(owner_label("falcond"), "falcond");
    }
}
