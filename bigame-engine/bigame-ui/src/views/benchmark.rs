//! Benchmark Lab: what can be measured here, and what measurement found.
//!
//! Two questions, and the page is built around keeping them apart.
//!
//! **What can be measured here?** Not a yes/no. A workload can be absent, or
//! present but missing something it needs, or present and perfectly good but
//! only startable by a person sitting in front of it — several commercial games
//! have an excellent built-in benchmark reachable only from a menu. Each of
//! those leads somewhere different, so each gets its own wording and its own
//! row, rather than a checkmark that hides which one applies.
//!
//! **What did measurement find?** The calibration, shown as it was measured,
//! including the settings that turned out not to help and the ones that turned
//! out to hurt. A page that showed only the wins would be advertising rather
//! than evidence, and the whole point of measuring is to be able to tell the
//! difference.

use adw::prelude::*;
use libadwaita as adw;

use bigame_core::benchmark::calibration::Calibration;
use bigame_core::benchmark::provider::{self, Availability};
use bigame_core::benchmark::result::Verdict;
use bigame_core::{hardware::Hardware, inventory};

use crate::i18n::i18n;

/// Build the Benchmark Lab page.
#[must_use]
pub fn build() -> gtk4::Widget {
    let page = adw::PreferencesPage::new();
    page.add(&workloads_group());
    page.add(&calibration_group());
    page.add(&method_group());
    page.upcast()
}

/// Which workloads are usable on this machine, and why not when they are not.
fn workloads_group() -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::new();
    group.set_title(&i18n("Workloads"));
    group.set_description(Some(&i18n(
        "A benchmark that cannot run is listed with the reason, not hidden. \
         Simple loops such as glxgears confirm the driver is alive and are \
         never treated as evidence about game performance.",
    )));

    let providers = provider::all();
    if providers.is_empty() {
        group.add(&placeholder(&i18n(
            "No benchmark workload is known to this build.",
        )));
        return group;
    }

    let mut rows = Vec::new();
    for workload in &providers {
        let row = adw::ActionRow::new();
        row.set_title(workload.name());
        row.set_subtitle_lines(0);
        let label = gtk4::Label::new(None);
        label.add_css_class("dim-label");
        label.add_css_class("caption");
        row.add_suffix(&label);
        group.add(&row);
        rows.push((row, label));
    }

    // Availability is read again each time the page is shown: a fix the
    // page asked for (SuperTuxKart's vsync, an install) shows without
    // restarting the application.
    let refresh = move || {
        for (workload, (row, label)) in providers.iter().zip(&rows) {
            let (subtitle, badge) = availability_text(&workload.availability());
            row.set_subtitle(&subtitle);
            label.set_text(&badge);
        }
    };
    refresh();
    group.connect_map(move |_| refresh());
    group
}

fn availability_text(availability: &Availability) -> (String, String) {
    match availability {
        Availability::Ready => (i18n("Ready to run"), i18n("READY")),
        Availability::NotInstalled(what) => (
            // The install target is the actionable part, so it leads.
            format!("{}: {what}", i18n("Not installed")),
            i18n("MISSING"),
        ),
        Availability::MissingDependency(what) => (
            format!("{}: {what}", i18n("Cannot produce a usable measurement")),
            i18n("BLOCKED"),
        ),
        Availability::NeedsManualStart(what) => (
            format!("{}: {what}", i18n("Needs to be started by hand")),
            i18n("MANUAL"),
        ),
    }
}

/// What measurement concluded about each setting on this machine.
fn calibration_group() -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::new();
    group.set_title(&i18n("What measurement found on this machine"));

    let hardware = Hardware::detect();
    let fingerprint = inventory::fingerprint(&hardware);

    let calibration = Calibration::default_path()
        .and_then(|path| Calibration::load(&path, &fingerprint).ok().flatten());

    let Some(calibration) = calibration else {
        group.set_description(Some(&i18n(
            "Nothing has been measured on this hardware yet. Until it has, \
             settings are chosen from what the hardware reports it supports, \
             which is a weaker basis than a measurement.",
        )));
        group.add(&placeholder(&i18n("No calibration for this hardware.")));
        return group;
    };

    group.set_description(Some(&calibration.describe()));

    if calibration.findings.is_empty() {
        group.add(&placeholder(&i18n("No setting has been measured yet.")));
        return group;
    }

    // Harmful first. A setting measurement showed to hurt is the most
    // important thing on this page, and burying it under the wins would defeat
    // the purpose of having measured at all.
    let mut findings: Vec<_> = calibration.findings.values().collect();
    findings.sort_by_key(|f| match f.verdict {
        Verdict::Regression => 0,
        Verdict::Improvement => 1,
        Verdict::WithinNoise => 2,
        Verdict::Inconclusive => 3,
    });

    for finding in findings {
        let row = adw::ActionRow::new();
        row.set_title(&finding.knob);
        row.set_subtitle(&finding.rationale);
        row.set_subtitle_lines(0);

        let verdict = match finding.verdict {
            Verdict::Improvement => i18n("HELPS"),
            Verdict::Regression => i18n("HURTS"),
            Verdict::WithinNoise => i18n("NO CHANGE"),
            Verdict::Inconclusive => i18n("UNPROVEN"),
        };
        let label = gtk4::Label::new(Some(&verdict));
        label.add_css_class("caption");
        label.add_css_class(match finding.verdict {
            Verdict::Improvement => "success",
            Verdict::Regression => "error",
            _ => "dim-label",
        });
        row.add_suffix(&label);
        group.add(&row);
    }
    group
}

/// How to read any of the above.
fn method_group() -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::new();
    group.set_title(&i18n("How these results are decided"));

    for (title, body) in [
        (
            i18n("Runs alternate"),
            i18n(
                "Configurations are interleaved rather than run in blocks, so that \
                 the machine warming up over the session falls on both equally \
                 instead of penalising whichever ran last.",
            ),
        ),
        (
            i18n("The first run is discarded"),
            i18n(
                "A cold shader cache and a cold GPU make the first run unlike \
                 every run after it.",
            ),
        ),
        (
            i18n("A difference must clear two bars"),
            i18n(
                "It has to be larger than the variation between repeated runs of \
                 the same configuration, and it has to be statistically \
                 significant at 95%. Anything smaller is reported as no change — \
                 never as a small gain.",
            ),
        ),
    ] {
        let row = adw::ActionRow::new();
        row.set_title(&title);
        row.set_subtitle(&body);
        row.set_subtitle_lines(0);
        group.add(&row);
    }
    group
}

/// A row for an empty state, so absence is visible rather than blank.
fn placeholder(text: &str) -> adw::ActionRow {
    let row = adw::ActionRow::new();
    row.set_title(text);
    row.set_subtitle_lines(0);
    row.add_css_class("dim-label");
    row
}
