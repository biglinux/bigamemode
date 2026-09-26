//! Turbo Mode: the master switch.
//!
//! ```text
//! Turbo OFF = BiGame-mode does not intervene in games.
//! Turbo ON  = BiGame-mode may detect games and apply optimizations.
//! ```
//!
//! falcond is a separate service that applies a profile to every game it
//! matches, whatever else is set, so Turbo switches falcond itself, not only
//! the Booster plan. It owns the whole flow, in a fixed order:
//!
//! ```text
//! ON:  conflicts noted → falcond profile set corrected → falcond enabled and
//!      started (verified by systemd and falcond's own status) → Booster's
//!      global plan (evidence-gated, owner-aware) → report
//! OFF: falcond stopped and disabled (it restores any game profile it holds)
//!      → Booster's journal restored → report
//! ```
//!
//! OFF undoes in reverse order on purpose. falcond's snapshot of a running
//! game is taken *after* Booster's changes, so it must be put back first;
//! restoring Booster first and then stopping falcond would write Booster's
//! values back as falcond's "baseline".
//!
//! **Turbo's state is falcond's state as systemd reports it**, not a flag of
//! ours. There is one source of truth, and it cannot say "off" while falcond
//! is running. On a machine without falcond, the Booster journal decides.

use std::path::PathBuf;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::booster::BoosterEngine;
use crate::booster::plan::Skipped;
use crate::booster::report::Report as BoosterReport;
use crate::capabilities::Capabilities;
use crate::hardware::{Chassis, Hardware};
use crate::text::{Arg, N_, Text};

/// The unit Turbo switches.
pub const BACKEND_UNIT: &str = "falcond.service";

/// Where the pre-ownership record is published by the helper.
pub const OWNERSHIP_RECORD: &str = "/var/lib/bigame-mode/game-backend.json";

/// Whether Turbo is on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum State {
    /// Nothing intervenes in games.
    Off,
    /// falcond runs (or, without falcond, Booster changes are in force).
    On,
}

impl State {}

/// Read Turbo's state from the systems that hold it.
///
/// # Errors
/// Returns an error if systemd cannot be reached.
pub async fn state() -> Result<State> {
    let connection = zbus::Connection::system().await?;
    let unit = crate::systemd::unit_state(&connection, BACKEND_UNIT).await?;
    let on = if unit.is_installed() {
        unit.is_active()
    } else {
        BoosterEngine::is_active()
    };
    Ok(if on { State::On } else { State::Off })
}

/// Blocking variant, for the UI's worker threads.
///
/// # Errors
/// Returns an error if systemd cannot be reached.
pub fn state_blocking() -> Result<State> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(state())
}

/// Put back Booster changes left in force while Turbo is off.
///
/// With falcond installed, Turbo's state is falcond's unit, and Booster's
/// changes are undone by Turbo off. When that did not happen — falcond was
/// stopped from outside, Turbo off failed half-way, the application was
/// killed between the two — every surface says Off while the changes stay.
/// Called when the application starts. Returns how many knobs were restored.
///
/// # Errors
/// Returns an error if systemd or the journal cannot be read.
pub async fn reconcile() -> Result<usize> {
    let connection = zbus::Connection::system().await?;
    let unit = crate::systemd::unit_state(&connection, BACKEND_UNIT).await?;
    // Without falcond the journal *is* Turbo's state; nothing to reconcile.
    if !unit.is_installed() || unit.is_active() || !BoosterEngine::is_active() {
        return Ok(0);
    }
    tracing::info!(target: "turbo", "Turbo is off but Booster changes are in force; restoring them");
    let outcomes = BoosterEngine::deactivate().await?;
    Ok(outcomes.iter().filter(|o| o.status.is_ok()).count())
}

/// Blocking variant of [`reconcile`], for the application's startup thread.
///
/// # Errors
/// As [`reconcile`].
pub fn reconcile_blocking() -> Result<usize> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(reconcile())
}

/// Whether BiGame-mode has taken charge of falcond, and since when.
#[must_use]
pub fn owned_since() -> Option<u64> {
    let text = std::fs::read_to_string(OWNERSHIP_RECORD).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    value.get("taken_at")?.as_u64()
}

// ── Report ───────────────────────────────────────────────────────────────────

/// Which part of the report an item belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Section {
    /// Changed, and read back to confirm.
    Verified,
    /// Put back to what it was.
    Restored,
    /// Managed per game by another component, which this one leaves alone.
    ManagedPerGame,
    /// Considered and deliberately not applied.
    Skipped,
    /// Not possible on this machine.
    Unavailable,
    /// A second controller of the same state, deliberately not used.
    ConflictAvoided,
    /// Attempted and did not take effect.
    Failed,
}

/// What an item is about, so the UI can title it in the user's language.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "name")]
pub enum Kind {
    /// falcond's per-game optimization as a whole.
    GameBackend,
    /// Which of falcond's profile sets is loaded.
    ProfileSet,
    /// Feral `GameMode`.
    GameMode,
    /// The sched-ext scheduler.
    Scheduler,
    /// A Booster knob, by its title in English; [`Item::title`] carries it
    /// translatable.
    Knob(String),
}

/// One line of the report.
///
/// `kind` and `detail` stay in English next to the translatable `title` and
/// `text`, so a report written by this build still reads in an older one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Item {
    /// What it is about.
    pub kind: Kind,
    /// Where it goes.
    pub section: Section,
    /// The component that owns this state.
    pub owner: String,
    /// What happened, and why — in English.
    pub detail: String,
    /// `detail` as a translatable sentence; absent in reports written before
    /// the report was translatable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<Text>,
    /// A knob's title, translatable, for [`Kind::Knob`]; absent in older
    /// reports and for the other kinds, which the UI titles itself.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<Text>,
}

/// What a Turbo transition did.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Report {
    /// Whether this report is of turning Turbo on.
    pub turned_on: bool,
    /// Unix time.
    pub at: u64,
    /// Everything that was considered, in order.
    pub items: Vec<Item>,
}

impl Report {
    /// How many items are in `section`.
    #[must_use]
    pub fn count(&self, section: Section) -> usize {
        self.items.iter().filter(|i| i.section == section).count()
    }

    /// Where the last report is kept for the report view.
    #[must_use]
    pub fn path() -> Option<PathBuf> {
        let state = std::env::var_os("XDG_STATE_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state")))?;
        Some(state.join("bigame-mode").join("turbo-report.json"))
    }

    /// The last report written.
    #[must_use]
    pub fn load_last() -> Option<Self> {
        let text = std::fs::read_to_string(Self::path()?).ok()?;
        serde_json::from_str(&text).ok()
    }

    fn save(&self) {
        let Some(path) = Self::path() else { return };
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(json) = serde_json::to_vec_pretty(self) {
            let _ = std::fs::write(path, json);
        }
    }

    fn push(&mut self, kind: Kind, section: Section, owner: &str, text: Text) {
        self.items.push(Item {
            kind,
            section,
            owner: owner.to_owned(),
            detail: text.english(),
            text: Some(text),
            title: None,
        });
    }

    fn push_knob(&mut self, title: Text, section: Section, owner: &str, text: Text) {
        self.items.push(Item {
            kind: Kind::Knob(title.english()),
            section,
            owner: owner.to_owned(),
            detail: text.english(),
            text: Some(text),
            title: Some(title),
        });
    }
}

/// The row for a Booster that could not run at all.
fn booster_failed(report: &mut Report, error: &anyhow::Error) {
    report.push_knob(
        Text::plain(N_("Booster")),
        Section::Failed,
        "Booster",
        Text::raw(format!("{error:#}")),
    );
}

// ── Progress ─────────────────────────────────────────────────────────────────

/// A stage of a transition, for the UI to show while it happens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// Reading hardware and tools.
    Detecting,
    /// Correcting which of falcond's profile sets it loads.
    ConfiguringProfiles,
    /// Starting or stopping per-game optimization.
    SwitchingBackend,
    /// Running the Booster's global plan.
    Booster(crate::booster::Progress),
    /// Putting Booster's changes back.
    Restoring,
}

// ── Transitions ──────────────────────────────────────────────────────────────

/// The profile set falcond should load on this machine, if the current one
/// is clearly wrong.
///
/// Narrow on purpose: only the one mismatch with an unambiguous answer is
/// corrected. falcond's `handheld` profiles run games in power-saving mode
/// (`perf=false`, `scx_lavd` with the `power` property); on anything that is
/// not a handheld that is the wrong set. `htpc` is left alone — a desktop may
/// well be one — and so is anything a person chose on a real handheld.
#[must_use]
pub fn corrected_profile_mode(chassis: Chassis, current: &str) -> Option<&'static str> {
    (current == "handheld" && matches!(chassis, Chassis::Desktop | Chassis::Laptop))
        .then_some("none")
}

/// Turn Turbo on.
///
/// # Errors
/// Returns an error only if nothing could be attempted — the helper
/// unreachable before any change. Anything that fails part-way is an item in
/// the report, not an error, so the user sees what did take effect.
pub async fn turn_on<F: FnMut(Step)>(mut progress: F) -> Result<Report> {
    progress(Step::Detecting);
    let hardware = Hardware::detect();
    let caps = Capabilities::detect();
    let mut report = Report {
        turned_on: true,
        at: crate::unix_now(),
        items: Vec::new(),
    };

    if caps.gamemode {
        report.push(
            Kind::GameMode,
            Section::ConflictAvoided,
            "falcond",
            Text::plain(N_(
                "installed but not used: falcond already owns per-game performance \
                 state, and two controllers would each snapshot and restore the same \
                 power profile",
            )),
        );
    }

    if caps.falcond_installed {
        enable_backend(&hardware, &mut report, &mut progress).await;
    } else {
        report.push(
            Kind::GameBackend,
            Section::Unavailable,
            "falcond",
            Text::plain(N_(
                "falcond is not installed, so there are no per-game profiles; only \
                 global settings can be applied",
            )),
        );
    }

    let why = caps.sched_ext.switchable().describe();
    if let Some(why) = why {
        report.push(
            Kind::Scheduler,
            Section::Unavailable,
            "falcond",
            Text::with(
                N_("%s, so game profiles that ask for a scheduler cannot set one"),
                [Text::raw(why)],
            ),
        );
    }

    let engine = BoosterEngine::detect();
    match engine.activate(|p| progress(Step::Booster(p))).await {
        Ok(booster) => absorb_booster(&booster, &mut report),
        Err(e) => booster_failed(&mut report, &e),
    }

    report.save();
    tracing::info!(
        target: "turbo",
        verified = report.count(Section::Verified),
        per_game = report.count(Section::ManagedPerGame),
        skipped = report.count(Section::Skipped),
        failed = report.count(Section::Failed),
        "turbo on"
    );
    Ok(report)
}

async fn enable_backend<F: FnMut(Step)>(
    hardware: &Hardware,
    report: &mut Report,
    progress: &mut F,
) {
    // Profile set first, so falcond starts with the right one rather than
    // loading the wrong set and reloading.
    if let Ok(mut config) = crate::config::read() {
        if let Some(fixed) = corrected_profile_mode(hardware.chassis, &config.profile_mode) {
            progress(Step::ConfiguringProfiles);
            let before = config.profile_mode.clone();
            config.profile_mode = fixed.to_owned();
            match crate::config::write(&config).await {
                Ok(()) if crate::config::read().is_ok_and(|c| c.profile_mode == fixed) => report
                    .push(
                        Kind::ProfileSet,
                        Section::Verified,
                        "BiGame-mode",
                        Text::with(
                            N_(
                                "%s → %s: the handheld profiles run games in power-saving mode, \
                            and this machine is not a handheld",
                            ),
                            [Arg::Raw(before), Arg::Text(desktop_set())],
                        ),
                    ),
                Ok(()) => report.push(
                    Kind::ProfileSet,
                    Section::Failed,
                    "BiGame-mode",
                    Text::plain(N_("the configuration was written but reads back unchanged")),
                ),
                Err(e) => report.push(
                    Kind::ProfileSet,
                    Section::Failed,
                    "BiGame-mode",
                    crate::error::describe(&e),
                ),
            }
        }
    }

    progress(Step::SwitchingBackend);
    let started = std::time::SystemTime::now();
    let result = async {
        let proxy = crate::dbus_client::daemon_proxy().await?;
        anyhow::Ok(proxy.set_game_backend(true).await?)
    }
    .await;
    match result {
        Ok(reached) if reached == "active" => {
            // systemd says it runs; falcond's own status says it has loaded.
            let status = wait_for_fresh_status(started).await;
            let detail = match status {
                Some(s) => Text::with(
                    N_(
                        "running, %s profiles loaded (%s set); applies a profile to each game as it starts",
                    ),
                    [
                        Arg::Raw(s.loaded_profiles.to_string()),
                        // falcond's own name for its sets, except the one it
                        // calls `none`, which is the desktop set.
                        if s.profile_mode == "none" {
                            Arg::Text(desktop_set())
                        } else {
                            Arg::Raw(s.profile_mode)
                        },
                    ],
                ),
                None => Text::plain(N_(
                    "running (systemd reports it active; falcond has not published its status yet)",
                )),
            };
            report.push(Kind::GameBackend, Section::Verified, "falcond", detail);
        }
        Ok(other) => report.push(
            Kind::GameBackend,
            Section::Failed,
            "falcond",
            systemd_reports(other),
        ),
        Err(e) => report.push(
            Kind::GameBackend,
            Section::Failed,
            "falcond",
            crate::error::describe(&e),
        ),
    }
}

/// falcond's profile set for desktops, which its configuration calls `none`.
fn desktop_set() -> Text {
    Text::plain(N_("desktop"))
}

fn systemd_reports(state: String) -> Text {
    Text::with(N_("systemd reports it %s"), [state])
}

/// falcond's status, once it has been rewritten after `since`.
async fn wait_for_fresh_status(
    since: std::time::SystemTime,
) -> Option<crate::status::FalcondStatus> {
    for _ in 0..30 {
        let fresh = std::fs::metadata(crate::status::status_path())
            .and_then(|m| m.modified())
            .is_ok_and(|m| m >= since);
        if fresh {
            if let Some(s) = crate::status::read() {
                return Some(s);
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    None
}

fn absorb_booster(booster: &BoosterReport, report: &mut Report) {
    for change in &booster.applied {
        let section = if change.succeeded() {
            Section::Verified
        } else {
            Section::Failed
        };
        report.push_knob(
            change.knob.title_text(),
            section,
            "Booster",
            change.summary_text(),
        );
    }
    // The scheduler already has its own item from falcond's side.
    let scheduler = crate::booster::plan::scheduler_title();
    for skipped in &booster.skipped {
        let (knob, section, owner, detail) = match skipped {
            Skipped::OwnedBy {
                knob,
                owner,
                detail,
            } => (
                knob,
                Section::ManagedPerGame,
                owner.as_str(),
                detail.clone(),
            ),
            Skipped::Unsupported { knob, .. } | Skipped::NotBeneficial { knob, .. }
                if *knob == scheduler =>
            {
                continue;
            }
            Skipped::Unsupported { knob, detail } => {
                (knob, Section::Unavailable, "Booster", detail.clone())
            }
            Skipped::AlreadyOptimal { knob, value } => {
                (knob, Section::Skipped, "Booster", already(value))
            }
            Skipped::NotBeneficial { knob, detail } => {
                (knob, Section::Skipped, "Booster", detail.clone())
            }
            Skipped::NotRestorable { knob } => (
                knob,
                Section::Skipped,
                "Booster",
                Text::plain(N_(
                    "its current value could not be read, so it could not be restored",
                )),
            ),
        };
        report.push_knob(knob.clone(), section, owner, detail);
    }
}

fn already(value: &str) -> Text {
    Text::with(N_("already %s"), [value])
}

/// Turn Turbo off.
///
/// # Errors
/// Returns an error only if nothing could be attempted.
pub async fn turn_off<F: FnMut(Step)>(mut progress: F) -> Result<Report> {
    let caps = Capabilities::detect();
    let mut report = Report {
        turned_on: false,
        at: crate::unix_now(),
        items: Vec::new(),
    };

    // falcond first: its snapshot of a running game sits on top of Booster's.
    if caps.falcond_installed {
        progress(Step::SwitchingBackend);
        let result = async {
            let proxy = crate::dbus_client::daemon_proxy().await?;
            anyhow::Ok(proxy.set_game_backend(false).await?)
        }
        .await;
        match result {
            Ok(reached) if reached == "inactive" => report.push(
                Kind::GameBackend,
                Section::Restored,
                "falcond",
                Text::plain(N_(
                    "stopped and disabled; any game profile it held was put back as it stopped",
                )),
            ),
            Ok(other) => report.push(
                Kind::GameBackend,
                Section::Failed,
                "falcond",
                systemd_reports(other),
            ),
            Err(e) => report.push(
                Kind::GameBackend,
                Section::Failed,
                "falcond",
                crate::error::describe(&e),
            ),
        }
    }

    progress(Step::Restoring);
    match BoosterEngine::deactivate().await {
        Ok(outcomes) => {
            for outcome in outcomes {
                let ok = outcome.status.is_ok();
                report.push_knob(
                    outcome.knob.title_text(),
                    if ok {
                        Section::Restored
                    } else {
                        Section::Failed
                    },
                    "Booster",
                    match &outcome.status {
                        crate::booster::snapshot::RestoreStatus::Restored => {
                            Text::with(N_("restored to %s"), [&outcome.target])
                        }
                        crate::booster::snapshot::RestoreStatus::AlreadyCorrect => {
                            already(&outcome.target)
                        }
                        crate::booster::snapshot::RestoreStatus::Failed { error } => Text::with(
                            N_("could not restore %s: %s"),
                            [Arg::from(&outcome.target), Arg::Text(error.clone())],
                        ),
                    },
                );
            }
        }
        Err(e) => booster_failed(&mut report, &e),
    }

    report.save();
    tracing::info!(
        target: "turbo",
        restored = report.count(Section::Restored),
        failed = report.count(Section::Failed),
        "turbo off"
    );
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handheld_profiles_are_corrected_only_off_handhelds() {
        assert_eq!(
            corrected_profile_mode(Chassis::Desktop, "handheld"),
            Some("none")
        );
        assert_eq!(
            corrected_profile_mode(Chassis::Laptop, "handheld"),
            Some("none")
        );
        // A handheld keeps its handheld set, and nothing else is second-guessed.
        assert_eq!(corrected_profile_mode(Chassis::Handheld, "handheld"), None);
        assert_eq!(corrected_profile_mode(Chassis::Desktop, "htpc"), None);
        assert_eq!(corrected_profile_mode(Chassis::Desktop, "none"), None);
        // Unknown hardware is not guessed at.
        assert_eq!(corrected_profile_mode(Chassis::Unknown, "handheld"), None);
    }

    #[test]
    fn owned_knobs_are_reported_as_managed_per_game() {
        let booster = BoosterReport {
            skipped: vec![
                Skipped::OwnedBy {
                    knob: crate::booster::knob::Knob::PowerProfile.title_text(),
                    owner: "falcond".into(),
                    detail: Text::raw("per game"),
                },
                Skipped::NotBeneficial {
                    knob: crate::booster::knob::Knob::GpuDpmLevel {
                        card: "card1".into(),
                    }
                    .title_text(),
                    detail: Text::raw("left to the driver"),
                },
                Skipped::Unsupported {
                    knob: crate::booster::plan::scheduler_title(),
                    detail: Text::raw("scx_loader service is not running"),
                },
            ],
            ..BoosterReport::default()
        };
        let mut report = Report::default();
        absorb_booster(&booster, &mut report);
        assert_eq!(report.count(Section::ManagedPerGame), 1);
        assert_eq!(report.count(Section::Skipped), 1);
        // The scheduler is reported once, from falcond's side, not twice.
        assert_eq!(report.items.len(), 2);
        let dpm = &report.items[1];
        assert_eq!(dpm.detail, "left to the driver");
        // Older builds read the English kind; this one titles it translated.
        assert_eq!(dpm.kind, Kind::Knob("GPU power level (card1)".into()));
        let title = dpm.title.as_ref().unwrap();
        assert_eq!(title.template, "GPU power level (%s)");
        assert_eq!(title.args, ["card1"]);
    }

    #[test]
    fn the_report_round_trips_for_the_report_view() {
        let mut report = Report {
            turned_on: true,
            at: 1,
            items: Vec::new(),
        };
        report.push(
            Kind::GameBackend,
            Section::Verified,
            "falcond",
            Text::raw("running"),
        );
        report.push_knob(
            crate::booster::knob::Knob::PowerProfile.title_text(),
            Section::ManagedPerGame,
            "falcond",
            already("performance"),
        );
        let json = serde_json::to_string(&report).unwrap();
        let back: Report = serde_json::from_str(&json).unwrap();
        assert_eq!(back.items, report.items);
        assert_eq!(back.items[1].detail, "already performance");
        assert_eq!(back.items[1].kind, Kind::Knob("Power profile".into()));
    }

    #[test]
    fn a_report_saved_before_translation_still_loads() {
        // The format every earlier build wrote: English kind and detail only.
        // A parse failure would hide the last report without a word.
        let old = r#"{"turned_on":true,"at":1,"items":[
            {"kind":{"kind":"game_backend"},"section":"verified","owner":"falcond",
             "detail":"running"},
            {"kind":{"kind":"knob","name":"GPU power level (card1)"},"section":"skipped",
             "owner":"Booster","detail":"left to the driver"}]}"#;
        let report: Report = serde_json::from_str(old).unwrap();
        assert_eq!(report.items.len(), 2);
        assert_eq!(
            report.items[1].kind,
            Kind::Knob("GPU power level (card1)".into())
        );
        assert_eq!(report.items[1].detail, "left to the driver");
        assert!(
            report
                .items
                .iter()
                .all(|i| i.text.is_none() && i.title.is_none())
        );
    }

    #[test]
    fn a_report_in_the_translatable_format_loads() {
        let new = r#"{"turned_on":false,"at":2,"items":[
            {"kind":{"kind":"knob","name":"Power profile"},"section":"failed","owner":"Booster",
             "detail":"could not restore balanced: value could not be read back",
             "text":{"template":"could not restore %s: %s",
                     "args":["balanced",{"template":"value could not be read back"}]},
             "title":{"template":"Power profile"}}]}"#;
        let report: Report = serde_json::from_str(new).unwrap();
        let item = &report.items[0];
        assert_eq!(item.text.as_ref().unwrap().english(), item.detail);
        assert_eq!(item.title.as_ref().unwrap().english(), "Power profile");
        assert_eq!(item.kind, Kind::Knob("Power profile".into()));
    }
}
