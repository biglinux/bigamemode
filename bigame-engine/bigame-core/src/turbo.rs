//! Turbo Mode: the master switch.
//!
//! ```text
//! Turbo OFF = BiGame-mode does not intervene in games.
//! Turbo ON  = BiGame-mode may detect games and apply optimizations.
//! ```
//!
//! Before this module, Turbo ran the Booster planner and nothing else, while
//! falcond — a separate, always-on service — applied a profile to every game
//! whatever Turbo said. On the reference machine the Booster plan was empty,
//! so the product's headline control did nothing (docs/14-TURBO-AUDIT.md).
//!
//! Now Turbo owns the whole flow, in a fixed order:
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

impl State {
    /// Whether Turbo is on.
    #[must_use]
    pub fn is_on(&self) -> bool {
        *self == Self::On
    }
}

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
    /// A Booster knob, by its title.
    Knob(String),
}

/// One line of the report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Item {
    /// What it is about.
    pub kind: Kind,
    /// Where it goes.
    pub section: Section,
    /// The component that owns this state.
    pub owner: String,
    /// What happened, and why — in English; the UI titles items by `kind`.
    pub detail: String,
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

    fn push(&mut self, kind: Kind, section: Section, owner: &str, detail: impl Into<String>) {
        self.items.push(Item {
            kind,
            section,
            owner: owner.to_owned(),
            detail: detail.into(),
        });
    }
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
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
        at: now(),
        items: Vec::new(),
    };

    if caps.gamemode {
        report.push(
            Kind::GameMode,
            Section::ConflictAvoided,
            "falcond",
            "installed but not used: falcond already owns per-game performance \
             state, and two controllers would each snapshot and restore the same \
             power profile",
        );
    }

    if caps.falcond_installed {
        enable_backend(&hardware, &mut report, &mut progress).await;
    } else {
        report.push(
            Kind::GameBackend,
            Section::Unavailable,
            "falcond",
            "falcond is not installed, so there are no per-game profiles; only \
             global settings can be applied",
        );
    }

    let why = caps.sched_ext.switchable().describe();
    if let Some(why) = why {
        report.push(
            Kind::Scheduler,
            Section::Unavailable,
            "falcond",
            format!("{why}, so game profiles that ask for a scheduler cannot set one"),
        );
    }

    let engine = BoosterEngine::detect();
    match engine.activate(|p| progress(Step::Booster(p))).await {
        Ok(booster) => absorb_booster(&booster, &mut report),
        Err(e) => report.push(
            Kind::Knob("Booster".into()),
            Section::Failed,
            "Booster",
            format!("{e:#}"),
        ),
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
                Ok(()) if crate::config::read().is_ok_and(|c| c.profile_mode == fixed) => report.push(
                    Kind::ProfileSet,
                    Section::Verified,
                    "BiGame-mode",
                    format!(
                        "{before} → desktop: the handheld profiles run games in power-saving mode, \
                         and this machine is not a handheld"
                    ),
                ),
                Ok(()) => report.push(
                    Kind::ProfileSet,
                    Section::Failed,
                    "BiGame-mode",
                    "the configuration was written but reads back unchanged",
                ),
                Err(e) => report.push(
                    Kind::ProfileSet,
                    Section::Failed,
                    "BiGame-mode",
                    format!("{e:#}"),
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
                Some(s) => format!(
                    "running, {} profiles loaded ({} set); applies a profile to each game as it starts",
                    s.loaded_profiles,
                    if s.profile_mode == "none" {
                        "desktop"
                    } else {
                        &s.profile_mode
                    },
                ),
                None => {
                    "running (systemd reports it active; falcond has not published its status yet)"
                        .into()
                }
            };
            report.push(Kind::GameBackend, Section::Verified, "falcond", detail);
        }
        Ok(other) => report.push(
            Kind::GameBackend,
            Section::Failed,
            "falcond",
            format!("systemd reports it {other}"),
        ),
        Err(e) => report.push(
            Kind::GameBackend,
            Section::Failed,
            "falcond",
            format!("{e:#}"),
        ),
    }
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
        report.push(
            Kind::Knob(change.knob.title()),
            section,
            "Booster",
            change.summary(),
        );
    }
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
            Skipped::Unsupported { knob, detail } => {
                // The scheduler already has its own item from falcond's side.
                if knob.contains("sched-ext") {
                    continue;
                }
                (knob, Section::Unavailable, "Booster", detail.clone())
            }
            Skipped::AlreadyOptimal { knob, value } => (
                knob,
                Section::Skipped,
                "Booster",
                format!("already {value}"),
            ),
            Skipped::NotBeneficial { knob, detail } => {
                if knob.contains("sched-ext") {
                    continue;
                }
                (knob, Section::Skipped, "Booster", detail.clone())
            }
            Skipped::MeasuredHarmful { knob, detail } => (
                knob,
                Section::Skipped,
                "Booster",
                format!("measured slower — {detail}"),
            ),
            Skipped::NotRestorable { knob } => (
                knob,
                Section::Skipped,
                "Booster",
                "its current value could not be read, so it could not be restored".to_owned(),
            ),
        };
        report.push(Kind::Knob(knob.clone()), section, owner, detail);
    }
}

/// Turn Turbo off.
///
/// # Errors
/// Returns an error only if nothing could be attempted.
pub async fn turn_off<F: FnMut(Step)>(mut progress: F) -> Result<Report> {
    let caps = Capabilities::detect();
    let mut report = Report {
        turned_on: false,
        at: now(),
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
                "stopped and disabled; any game profile it held was put back as it stopped",
            ),
            Ok(other) => report.push(
                Kind::GameBackend,
                Section::Failed,
                "falcond",
                format!("systemd reports it {other}"),
            ),
            Err(e) => report.push(
                Kind::GameBackend,
                Section::Failed,
                "falcond",
                format!("{e:#}"),
            ),
        }
    }

    progress(Step::Restoring);
    match BoosterEngine::deactivate().await {
        Ok(outcomes) => {
            for outcome in outcomes {
                let ok = outcome.status.is_ok();
                report.push(
                    Kind::Knob(outcome.knob.title()),
                    if ok {
                        Section::Restored
                    } else {
                        Section::Failed
                    },
                    "Booster",
                    match &outcome.status {
                        crate::booster::snapshot::RestoreStatus::Restored => {
                            format!("restored to {}", outcome.target)
                        }
                        crate::booster::snapshot::RestoreStatus::AlreadyCorrect => {
                            format!("already {}", outcome.target)
                        }
                        crate::booster::snapshot::RestoreStatus::Failed { error } => {
                            format!("could not restore {}: {error}", outcome.target)
                        }
                    },
                );
            }
        }
        Err(e) => report.push(
            Kind::Knob("Booster".into()),
            Section::Failed,
            "Booster",
            format!("{e:#}"),
        ),
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
                    knob: "Power profile".into(),
                    owner: "falcond".into(),
                    detail: "per game".into(),
                },
                Skipped::MeasuredHarmful {
                    knob: "GPU power level (card1)".into(),
                    detail: "8.0% slower".into(),
                },
                Skipped::Unsupported {
                    knob: "sched-ext scheduler".into(),
                    detail: "scx_loader service is not running".into(),
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
        assert!(dpm.detail.starts_with("measured slower"), "{}", dpm.detail);
    }

    #[test]
    fn the_report_round_trips_for_the_report_view() {
        let mut report = Report {
            turned_on: true,
            at: 1,
            items: Vec::new(),
        };
        report.push(Kind::GameBackend, Section::Verified, "falcond", "running");
        report.push(
            Kind::Knob("Power profile".into()),
            Section::ManagedPerGame,
            "falcond",
            "per game",
        );
        let json = serde_json::to_string(&report).unwrap();
        let back: Report = serde_json::from_str(&json).unwrap();
        assert_eq!(back.items, report.items);
    }
}
