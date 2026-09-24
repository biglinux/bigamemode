//! The Optimization Report.
//!
//! What Turbo did, grouped by what happened to each thing it considered:
//! applied and verified, left to the component that owns it, skipped,
//! unavailable, a conflict avoided, or failed. Every row says who owns the
//! state, and its ⓘ says what the thing is.
//!
//! One distinction is kept impossible to blur: **what was changed** and **what
//! got faster** are different claims. Changes are listed with their
//! verification; speed is claimed only in "Measured on this machine", and only
//! for what a benchmark here actually measured.

use adw::prelude::*;
use libadwaita as adw;

use bigame_core::benchmark::calibration::Calibration;
use bigame_core::benchmark::result::Verdict;
use bigame_core::turbo::{Item, Kind, Report, Section};

use crate::i18n::i18n;
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
        Section::Verified | Section::Restored => "emblem-ok-symbolic",
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
        Kind::Knob(name) => i18n(name),
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
            "A system-wide setting BiGame-mode's own planner considers. It is applied only if nothing else owns it and nothing measured on this machine says it is slower.",
        ),
    }
}

fn item_row(item: &Item) -> adw::ActionRow {
    let title = kind_title(&item.kind);
    // Plain text: details carry process names, paths and error messages,
    // any of which can contain `&` or `<`.
    let row = adw::ActionRow::builder()
        .title(&title)
        .subtitle(&item.detail)
        .subtitle_lines(3)
        .use_markup(false)
        .build();
    let icon = gtk4::Image::from_icon_name(section_icon(item.section));
    icon.add_css_class("dim-label");
    row.add_prefix(&icon);
    let owner = gtk4::Label::new(Some(&item.owner));
    owner.add_css_class("caption");
    owner.add_css_class("dim-label");
    row.add_suffix(&owner);
    row.add_suffix(&info::button(
        &title,
        &format!(
            "{}\n\n{}: {}\n{}: {}",
            kind_explanation(&item.kind),
            i18n("Controlled by"),
            item.owner,
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

/// What benchmarks on this machine found, with their numbers.
fn measured_group() -> Option<adw::PreferencesGroup> {
    let hw = bigame_core::hardware::Hardware::detect();
    let fingerprint = bigame_core::inventory::fingerprint(&hw);
    let calibration = Calibration::load(&Calibration::default_path()?, &fingerprint).ok()??;
    if calibration.findings.is_empty() {
        return None;
    }
    let group = adw::PreferencesGroup::new();
    group.set_title(&i18n("Measured on this machine"));
    group.set_description(Some(&i18n(
        "Only these are performance claims: each comes from alternating benchmark runs, judged against their own run-to-run variation.",
    )));
    for finding in calibration.findings.values() {
        let verdict = match finding.verdict {
            Verdict::Improvement => i18n("faster"),
            Verdict::Regression => i18n("slower — not applied"),
            Verdict::WithinNoise => i18n("no difference above noise"),
            Verdict::Inconclusive => i18n("inconclusive"),
        };
        let row = adw::ActionRow::builder()
            .title(format!("{} · {verdict}", finding.knob))
            .subtitle(format!("{:+.1}% · {}", finding.delta_pct, finding.workload))
            .use_markup(false)
            .build();
        row.add_suffix(&info::button(&finding.knob, &finding.rationale));
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

    if let Some(measured) = measured_group() {
        page.add(&measured);
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
}
