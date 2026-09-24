//! The Booster report.
//!
//! This view exists to make one distinction impossible to blur: **what was
//! changed** and **what got faster** are different claims, and only the first
//! one is something an activation can demonstrate on its own.
//!
//! So changes are listed with their verification result, and the performance
//! section says "not measured" unless a benchmark actually ran. Saying nothing
//! is better than saying "+20% FPS" on the strength of a successful sysfs
//! write, and it is what earns the numbers credibility when there are numbers
//! to show.

use adw::prelude::*;
use libadwaita as adw;

use bigame_core::booster::knob::Verification;
use bigame_core::booster::plan::Skipped;
use bigame_core::booster::report::{AppliedChange, Report};

use crate::i18n::i18n;

/// Build a page presenting `report`.
#[must_use]
pub fn build(report: &Report) -> gtk4::Widget {
    let page = adw::PreferencesPage::new();

    // ── Headline ────────────────────────────────────────────────────────
    let summary = adw::PreferencesGroup::new();
    summary.set_title(&report.headline());
    if !report.machine.is_empty() {
        summary.set_description(Some(&report.machine));
    }
    page.add(&summary);

    // ── What changed ────────────────────────────────────────────────────
    if !report.applied.is_empty() {
        let group = adw::PreferencesGroup::new();
        group.set_title(&i18n("Changes"));
        group.set_description(Some(&i18n(
            "Each change was read back from the system after being written.",
        )));
        for change in &report.applied {
            group.add(&change_row(change));
        }
        page.add(&group);
    }

    // ── Contention ──────────────────────────────────────────────────────
    // A write that was accepted but did not stick almost always means a second
    // process is managing the same knob. That is worth calling out on its own,
    // because it is a configuration problem rather than a transient failure.
    let contended = report.contended();
    if !contended.is_empty() {
        let group = adw::PreferencesGroup::new();
        group.set_title(&i18n("Something else is changing these settings"));
        group.set_description(Some(&i18n(
            "These were written successfully but the system reported a different \
             value afterwards, which usually means another service is managing them.",
        )));
        for change in contended {
            let row = adw::ActionRow::builder()
                .title(change.knob.title())
                .subtitle(match &change.verification {
                    Verification::Mismatch { actual } => {
                        format!(
                            "{} {} — {} {}",
                            i18n("Requested"),
                            change.to,
                            i18n("now reads"),
                            actual
                        )
                    }
                    _ => String::new(),
                })
                .build();
            row.add_prefix(&icon("dialog-warning-symbolic", "warning"));
            group.add(&row);
        }
        page.add(&group);
    }

    // ── Performance ─────────────────────────────────────────────────────
    let perf = adw::PreferencesGroup::new();
    perf.set_title(&i18n("Performance"));
    let perf_row = adw::ActionRow::builder()
        .title(report.performance_claim())
        .build();
    if report.measurements.is_empty() {
        perf_row.set_subtitle(&i18n(
            "Applying a setting proves the system changed. It does not prove a \
             game runs faster — that needs a before-and-after benchmark.",
        ));
        perf_row.add_prefix(&icon("dialog-information-symbolic", "dim-label"));
    } else {
        perf_row.add_prefix(&icon("emblem-ok-symbolic", "success"));
    }
    perf.add(&perf_row);
    page.add(&perf);

    // ── Considered and skipped ──────────────────────────────────────────
    if !report.skipped.is_empty() {
        let group = adw::PreferencesGroup::new();
        group.set_title(&i18n("Considered but not applied"));
        group.set_description(Some(&i18n(
            "Showing these is how you can tell the plan was reasoned about \
             rather than guessed.",
        )));
        for skipped in &report.skipped {
            group.add(&skipped_row(skipped));
        }
        page.add(&group);
    }

    page.upcast()
}

fn change_row(change: &AppliedChange) -> adw::ActionRow {
    let row = adw::ActionRow::builder()
        .title(change.summary())
        .subtitle(&change.rationale)
        .build();

    let (icon_name, css, status) = if let Some(error) = &change.error {
        ("dialog-error-symbolic", "error", error.clone())
    } else {
        match &change.verification {
            Verification::Confirmed => ("emblem-ok-symbolic", "success", i18n("Verified")),
            Verification::Mismatch { actual } => (
                "dialog-warning-symbolic",
                "warning",
                format!("{} {actual}", i18n("System reports")),
            ),
            Verification::Unreadable => (
                "dialog-question-symbolic",
                "warning",
                i18n("Could not be read back"),
            ),
        }
    };

    row.add_prefix(&icon(icon_name, css));
    let badge = gtk4::Label::new(Some(&status));
    badge.add_css_class("caption");
    badge.add_css_class(css);
    badge.set_wrap(true);
    badge.set_max_width_chars(28);
    badge.set_valign(gtk4::Align::Center);
    row.add_suffix(&badge);
    row
}

fn skipped_row(skipped: &Skipped) -> adw::ActionRow {
    let (title, detail, icon_name) = match skipped {
        Skipped::Unsupported { knob, detail } => {
            (knob.clone(), detail.clone(), "action-unavailable-symbolic")
        }
        Skipped::AlreadyOptimal { knob, value } => (
            knob.clone(),
            format!("{} {value}", i18n("Already set to")),
            "emblem-ok-symbolic",
        ),
        Skipped::NotBeneficial { knob, detail } => {
            (knob.clone(), detail.clone(), "dialog-information-symbolic")
        }
        // Given its own icon and wording: this is the only skip reason backed
        // by a measurement on this machine, and it deserves to read differently
        // from "the hardware does not support it".
        Skipped::MeasuredHarmful { knob, detail } => (
            knob.clone(),
            format!("{} — {detail}", i18n("Measured slower on this machine")),
            "speedometer-symbolic",
        ),
        Skipped::NotRestorable { knob } => (
            knob.clone(),
            i18n(
                "Skipped because the current value could not be read, so it could not be restored",
            ),
            "dialog-warning-symbolic",
        ),
    };

    let row = adw::ActionRow::builder()
        .title(title)
        .subtitle(detail)
        .build();
    row.add_prefix(&icon(icon_name, "dim-label"));
    row
}

fn icon(name: &str, css: &str) -> gtk4::Image {
    let image = gtk4::Image::from_icon_name(name);
    image.add_css_class(css);
    image
}

#[cfg(test)]
mod tests {
    use bigame_core::booster::knob::{Knob, Verification};
    use bigame_core::booster::report::Report;

    #[test]
    fn a_report_with_no_benchmark_never_claims_a_speedup() {
        let report = Report {
            machine: "bench".into(),
            applied: vec![bigame_core::booster::report::AppliedChange {
                knob: Knob::PowerProfile,
                from: "balanced".into(),
                to: "performance".into(),
                rationale: "because".into(),
                verification: Verification::Confirmed,
                error: None,
            }],
            skipped: Vec::new(),
            measurements: Vec::new(),
        };
        // This is the text the view puts in the Performance group.
        let claim = report.performance_claim();
        assert_eq!(claim, "Performance impact not measured");
        assert!(!claim.contains('%'));
        assert!(!claim.to_lowercase().contains("faster"));
    }
}
