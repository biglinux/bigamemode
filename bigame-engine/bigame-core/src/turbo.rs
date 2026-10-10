//! Turbo Mode: the master switch.
//!
//! ```text
//! Turbo OFF = Big Game Mode does not intervene in games.
//! Turbo ON  = Big Game Mode may detect games and apply optimizations.
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
//! A falcond that does not start leaves Turbo off, and then neither the
//! Booster nor the preset is applied: what a failed start left behind is
//! taken away instead, so an "off" never hides a frame cap.
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

use anyhow::{Context, Result};
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

/// Where the helper notes a hand-back, until Big Game Mode takes charge again.
pub const RELEASE_RECORD: &str = "/var/lib/bigame-mode/game-backend.released.json";

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
    let on = if backend_governs(&unit) {
        unit.is_active()
    } else {
        BoosterEngine::is_active()
    };
    Ok(if on { State::On } else { State::Off })
}

/// Whether Turbo's state is falcond's: it is installed, and not a build that
/// crashed on this processor. Otherwise Turbo is the Booster's, as without
/// falcond. Every surface that reads Turbo from falcond's unit asks this
/// first: an inactive falcond that does not govern is not a Turbo off.
#[must_use]
pub fn backend_governs(unit: &crate::systemd::UnitState) -> bool {
    unit.is_installed() && !crate::falcond_compat::crashes_here()
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
    if !backend_governs(&unit) || unit.is_active() || !BoosterEngine::is_active() {
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

/// Whether Big Game Mode has taken charge of falcond, and since when.
#[must_use]
pub fn owned_since() -> Option<u64> {
    let text = std::fs::read_to_string(OWNERSHIP_RECORD).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    value.get("taken_at")?.as_u64()
}

/// Who is in charge of falcond's service.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Control {
    /// falcond is not installed: there is nothing to manage.
    NotInstalled,
    /// Big Game Mode switches it with Turbo, since this Unix time; handing it
    /// back restores the state recorded then.
    Managed {
        /// When the state before Big Game Mode was recorded.
        since: u64,
    },
    /// It was handed back at this Unix time, and has been left alone since.
    HandedBack {
        /// When it was handed back.
        at: u64,
    },
    /// Big Game Mode has never changed it.
    NeverManaged,
}

impl Control {
    /// Whether taking control makes sense: falcond is there and Big Game Mode
    /// is not managing it.
    #[must_use]
    pub fn can_take(self) -> bool {
        matches!(self, Self::HandedBack { .. } | Self::NeverManaged)
    }
}

/// Who is in charge, from whether falcond is installed and the two records
/// the helper keeps. The ownership record wins: the helper removes the
/// release note whenever it records ownership again, and a note left over by
/// a failed removal must not hide that Big Game Mode manages falcond.
#[must_use]
pub fn control_from(installed: bool, owned: Option<u64>, released: Option<u64>) -> Control {
    match (installed, owned, released) {
        (false, _, _) => Control::NotInstalled,
        (true, Some(since), _) => Control::Managed { since },
        (true, None, Some(at)) => Control::HandedBack { at },
        (true, None, None) => Control::NeverManaged,
    }
}

/// When falcond was handed back, while Big Game Mode has not taken it again.
#[must_use]
pub fn released_at() -> Option<u64> {
    let text = std::fs::read_to_string(RELEASE_RECORD).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    value.get("released_at")?.as_u64()
}

/// Who is in charge of falcond now, for the UI's worker threads.
///
/// # Errors
/// Returns an error if systemd cannot be reached.
pub fn control_blocking() -> Result<Control> {
    let unit = crate::systemd::Reader::shared()
        .and_then(|r| r.unit_state(BACKEND_UNIT))
        .ok_or_else(|| anyhow::anyhow!("systemd could not be asked about {BACKEND_UNIT}"))?;
    Ok(control_from(
        unit.is_installed(),
        owned_since(),
        released_at(),
    ))
}

/// Take charge of falcond again after it was handed back (or before
/// Big Game Mode ever changed it).
///
/// Exactly what the first Turbo switch did: the helper records falcond's
/// state as it is now — so a later hand-back restores *this* — and the
/// service follows Turbo from then on. Turbo's state is falcond's state, so
/// the switch is asked for the state falcond is already in: a running falcond
/// stays running and is enabled; a stopped one stays stopped and is disabled,
/// which is what Turbo off means. Nothing is restarted, so a game's profile
/// is not torn down. Returns the control read back afterwards.
///
/// No method of its own on the helper: `SetGameBackend` already records
/// ownership before its first change, under the same Polkit action.
///
/// # Errors
/// Returns an error if falcond is not installed, the helper refuses or fails,
/// or no ownership record is there afterwards.
pub async fn take_back() -> Result<Control> {
    let connection = zbus::Connection::system().await?;
    let unit = crate::systemd::unit_state(&connection, BACKEND_UNIT).await?;
    if !unit.is_installed() {
        anyhow::bail!(crate::error::UserError::plain(N_(
            "falcond is not installed"
        )));
    }
    let want = if unit.is_active() {
        "active"
    } else {
        "inactive"
    };
    let proxy = crate::dbus_client::daemon_proxy().await?;
    let reached = proxy.set_game_backend(unit.is_active()).await?;
    if reached != want {
        anyhow::bail!(crate::error::UserError::with(
            N_("falcond is %s, not %s as it was"),
            [reached.as_str(), want]
        ));
    }
    let now = control_from(true, owned_since(), released_at());
    if !matches!(now, Control::Managed { .. }) {
        anyhow::bail!(crate::error::UserError::plain(N_(
            "The helper did not record falcond's state"
        )));
    }
    tracing::info!(target: "turbo", reached, "took control of falcond again");
    Ok(now)
}

/// Blocking variant of [`take_back`], for the UI's worker threads.
///
/// # Errors
/// As [`take_back`].
pub fn take_back_blocking() -> Result<Control> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(take_back())
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
    /// A knob's title, translatable, for [`Kind::Knob`]; for a falcond that
    /// did not start, what is wrong ("falcond is not compatible with this
    /// processor") when systemd says why. Absent in older reports and
    /// otherwise, where the UI titles the kind itself.
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
    // The preset chosen when Turbo was asked for, whatever changes meanwhile.
    let preset = crate::turbo_preset::chosen();
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

    let mut running = true;
    let mut backend = caps.falcond_usable;
    if caps.falcond_usable {
        // What runs before Turbo, so Turbo off can tell what falcond left.
        remember_scheduler(current_scheduler().await.as_deref());
        running = enable_backend(&hardware, &mut report, &mut progress).await;
        if !running && crate::falcond_compat::crashes_here() {
            // It crashed on this processor just now, and the helper
            // recorded it: Turbo goes on without it, as without falcond.
            tracing::warn!(target: "turbo", "falcond cannot run on this processor; Turbo goes on without it");
            push_cannot_run(&mut report);
            running = true;
            backend = false;
        }
    } else if caps.falcond_installed {
        push_cannot_run(&mut report);
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

    if !running {
        // Turbo's state is falcond's: it did not start, so Turbo is off, and
        // nothing of Turbo may be left in force behind an "off".
        hold_back(preset, true, &mut report);
        remove_preset(&mut report).await;
        restore_booster(&mut report).await;
        report.save();
        tracing::warn!(target: "turbo", "falcond did not start; Booster and the preset were not applied");
        return Ok(report);
    }

    let engine = BoosterEngine::detect();
    match engine.activate(|p| progress(Step::Booster(p))).await {
        Ok(booster) => absorb_booster(&booster, &mut report),
        Err(e) => booster_failed(&mut report, &e),
    }

    // Without falcond Turbo is on while the Booster's record is there; a
    // preset with no Turbo to take it away would stay for the session.
    if backend || BoosterEngine::is_active() {
        apply_preset(preset, &mut report).await;
    } else {
        hold_back(preset, false, &mut report);
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

/// A falcond build that crashed on this processor, in the report: Turbo goes
/// on without it.
fn push_cannot_run(report: &mut Report) {
    report.push(
        Kind::GameBackend,
        Section::Unavailable,
        "falcond",
        Text::plain(N_(
            "this falcond build crashed on this processor (illegal instruction), so it is \
             not started again until the package changes; Turbo applies the general \
             settings without per-game profiles",
        )),
    );
}

/// Turbo's parts that were not applied because Turbo did not come on: the
/// Booster when `booster`, and the preset.
fn hold_back(preset: crate::turbo_preset::Preset, booster: bool, report: &mut Report) {
    let why = || Text::plain(N_("not applied: Turbo did not come on"));
    if booster {
        report.push_knob(
            Text::plain(N_("Booster")),
            Section::Skipped,
            "Booster",
            why(),
        );
    }
    if preset != crate::turbo_preset::Preset::Standard {
        report.push_knob(
            Text::plain(N_("Turbo preset")),
            Section::Skipped,
            "Big Game Mode",
            why(),
        );
    }
}

/// Why falcond's service failed, explained, if systemd says it did.
async fn backend_failure() -> Option<BackendFailure> {
    let connection = zbus::Connection::system().await.ok()?;
    let unit = crate::systemd::unit_state(&connection, BACKEND_UNIT)
        .await
        .ok()?;
    let failure = unit.failure()?;
    Some(explain_backend_failure(
        &failure,
        &crate::isa::CpuIsa::detect(),
    ))
}

/// A falcond that did not start, in the report: why, when systemd says so.
async fn push_backend_failed(report: &mut Report, otherwise: Text) {
    match backend_failure().await {
        Some(f) => {
            tracing::warn!(target: "turbo", failure = %f.summary.english(), "falcond did not start");
            let detail = Text::with(N_("%s %s"), [Arg::Text(f.summary), Arg::Text(f.advice)]);
            report.items.push(Item {
                kind: Kind::GameBackend,
                section: Section::Failed,
                owner: "falcond".to_owned(),
                detail: detail.english(),
                text: Some(detail),
                title: Some(f.title),
            });
        }
        None => report.push(Kind::GameBackend, Section::Failed, "falcond", otherwise),
    }
}

/// Whether falcond runs, as systemd reports it now. A unit that cannot be
/// read is taken as not running: nothing of Turbo is applied on a guess.
async fn backend_active() -> bool {
    let read = async {
        let connection = zbus::Connection::system().await?;
        anyhow::Ok(crate::systemd::unit_state(&connection, BACKEND_UNIT).await?)
    }
    .await;
    match read {
        Ok(unit) => unit.is_active(),
        Err(e) => {
            tracing::warn!(target: "turbo", error = %format!("{e:#}"), "falcond's state could not be read");
            false
        }
    }
}

/// Enable and start falcond, and say so in the report. Returns whether it
/// runs afterwards: a start that reported an error may still have brought it
/// up, and one that did not leaves Turbo off.
async fn enable_backend<F: FnMut(Step)>(
    hardware: &Hardware,
    report: &mut Report,
    progress: &mut F,
) -> bool {
    // Profile set first, so falcond starts with the right one rather than
    // loading the wrong set and reloading.
    if let Ok(mut config) = crate::config::read()
        && let Some(fixed) = corrected_profile_mode(hardware.chassis, &config.profile_mode)
    {
        progress(Step::ConfiguringProfiles);
        let before = config.profile_mode.clone();
        config.profile_mode = fixed.to_owned();
        match crate::config::write(&config).await {
            Ok(()) if crate::config::read().is_ok_and(|c| c.profile_mode == fixed) => report.push(
                Kind::ProfileSet,
                Section::Verified,
                "Big Game Mode",
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
                "Big Game Mode",
                Text::plain(N_("the configuration was written but reads back unchanged")),
            ),
            Err(e) => report.push(
                Kind::ProfileSet,
                Section::Failed,
                "Big Game Mode",
                crate::error::describe(&e),
            ),
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
            // Still running after the wait for its status: the helper has
            // seen it stay up, and a crash since leaves Turbo off all the
            // same, before the Booster or a preset is applied.
            if !backend_active().await {
                push_backend_failed(report, systemd_reports("failed".into())).await;
                return false;
            }
            report.push(Kind::GameBackend, Section::Verified, "falcond", detail);
            true
        }
        Ok(other) => {
            push_backend_failed(report, systemd_reports(other)).await;
            backend_active().await
        }
        Err(e) => {
            push_backend_failed(report, crate::error::describe(&e)).await;
            backend_active().await
        }
    }
}

/// Put the chosen Turbo preset in force and say so in the report.
///
/// The session's environment is set through zbus's blocking API, which
/// must not run on the runtime's own thread ("Cannot start a runtime from
/// within a runtime", seen on the first real run): it goes to the blocking
/// pool.
async fn apply_preset(preset: crate::turbo_preset::Preset, report: &mut Report) {
    use crate::turbo_preset::Preset;
    let title = Text::plain(N_("Turbo preset"));
    if preset == Preset::Standard {
        // Nothing to lay over Tuning; a preset left from before goes.
        let cleared = tokio::task::spawn_blocking(crate::turbo_preset::deactivate).await;
        if let Ok(Err(e)) = cleared {
            tracing::warn!(error = %format!("{e:#}"), "could not clear a previous Turbo preset");
        }
        return;
    }
    let name = Text::plain(preset.label());
    let applied = tokio::task::spawn_blocking(move || crate::turbo_preset::activate(preset))
        .await
        .unwrap_or_else(|e| Err(anyhow::anyhow!("{e}")));
    match applied {
        Ok(_) if crate::steam::is_running() => report.push_knob(
            title,
            Section::Verified,
            "Big Game Mode",
            Text::with(
                N_(
                    "%s: in the session's environment. Steam was already open and keeps the environment it started with: close and reopen it for its games to get the preset",
                ),
                [Arg::Text(name)],
            ),
        ),
        // Not "every game from now on": a launcher opened from the
        // application menu gets the login's environment from the desktop
        // shell, not the session manager's. Home's launcher notice finds
        // one that does not have the preset.
        Ok(_) => report.push_knob(
            title,
            Section::Verified,
            "Big Game Mode",
            Text::with(
                N_(
                    "%s: in the session's environment. Games get it through a launcher that has it; Home offers to reopen one that does not",
                ),
                [Arg::Text(name)],
            ),
        ),
        Err(e) => report.push_knob(title, Section::Failed, "Big Game Mode", crate::error::describe(&e)),
    }
}

/// Take the Turbo preset away and say so in the report (on the blocking
/// pool, as [`apply_preset`]).
async fn remove_preset(report: &mut Report) {
    let preset = crate::turbo_preset::active();
    if preset == crate::turbo_preset::Preset::Standard {
        return;
    }
    let title = Text::plain(N_("Turbo preset"));
    let removed = tokio::task::spawn_blocking(crate::turbo_preset::deactivate)
        .await
        .unwrap_or_else(|e| Err(anyhow::anyhow!("{e}")));
    match removed {
        Ok(_) => report.push_knob(
            title,
            Section::Restored,
            "Big Game Mode",
            Text::with(
                N_("%s taken away: games get Tuning's settings again"),
                [Arg::Text(Text::plain(preset.label()))],
            ),
        ),
        Err(e) => report.push_knob(
            title,
            Section::Failed,
            "Big Game Mode",
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

/// A failed falcond, said for the person in front of it: a title, what
/// happened, and what to do about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackendFailure {
    /// A short title: what is wrong.
    pub title: Text,
    /// What happened, in one sentence.
    pub summary: Text,
    /// What to do.
    pub advice: Text,
    /// Whether turning Turbo off and on again can help. Not for an illegal
    /// instruction: the same binary stops on the same instruction again.
    pub retry_helps: bool,
}

/// Explain why falcond's service failed.
///
/// `SIGILL` is named a CPU incompatibility only with evidence: a processor
/// below x86-64-v3 (no AVX2/BMI2…), the level falcond 2.0.14 was built for
/// when it crashed this way on a Sandy Bridge. On a processor that has all of
/// v3 the same signal means something else (newer extensions, or a damaged
/// binary), and is said so.
#[must_use]
pub fn explain_backend_failure(
    failure: &crate::systemd::Failure,
    isa: &crate::isa::CpuIsa,
) -> BackendFailure {
    use crate::systemd::Failure;
    match failure {
        Failure::IllegalInstruction if isa.below_v3() => BackendFailure {
            title: Text::plain(N_("falcond is not compatible with this processor")),
            summary: Text::with(
                N_(
                    "falcond crashed on a CPU instruction this processor does not have \
                     (illegal instruction). The installed falcond package appears to be built \
                     for a newer x86-64 level: this processor is x86-64-v%s, without %s.",
                ),
                [
                    isa.level().unwrap_or(1).to_string(),
                    isa.missing_for_v3().join(", "),
                ],
            ),
            advice: Text::plain(N_(
                "Update or reinstall falcond with a build for this processor. Turning Turbo \
                 off and on again does not help: the same package crashes the same way.",
            )),
            retry_helps: false,
        },
        Failure::IllegalInstruction => BackendFailure {
            title: Text::plain(N_("falcond crashed on an illegal CPU instruction")),
            summary: Text::plain(N_(
                "falcond stopped on an instruction this processor rejected: a CPU \
                 instruction-set incompatibility, or a damaged falcond binary.",
            )),
            advice: Text::plain(N_(
                "Reinstall falcond. If it keeps happening, report it and include the Support report.",
            )),
            retry_helps: false,
        },
        Failure::Signal(_) => BackendFailure {
            title: Text::plain(N_("falcond crashed")),
            summary: Text::with(N_("falcond's service ended on %s."), [failure.describe()]),
            advice: Text::plain(N_(
                "Logs show why. Turning Turbo off and on again starts it again.",
            )),
            retry_helps: true,
        },
        Failure::ExitCode(_) | Failure::Other(_) => BackendFailure {
            title: Text::plain(N_("falcond failed")),
            summary: Text::with(N_("falcond's service failed (%s)."), [failure.describe()]),
            advice: Text::plain(N_(
                "Logs show why. Turning Turbo off and on again starts it again.",
            )),
            retry_helps: true,
        },
    }
}

/// falcond's status once it has been rewritten after `since`, or, if it is
/// not rewritten within three seconds, the one it last wrote.
async fn wait_for_fresh_status(
    since: std::time::SystemTime,
) -> Option<crate::status::FalcondStatus> {
    for _ in 0..30 {
        let fresh = std::fs::metadata(crate::status::status_path())
            .and_then(|m| m.modified())
            .is_ok_and(|m| m >= since);
        if fresh && let Some(s) = crate::status::read() {
            return Some(s);
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    // A falcond that was already running is not restarted by Turbo on, so it
    // has no reason to write its status again: what it last wrote is current.
    crate::status::read()
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

    if caps.falcond_installed {
        note_scheduler_left(&mut report).await;
    }

    progress(Step::Restoring);
    remove_preset(&mut report).await;
    restore_booster(&mut report).await;

    report.save();
    tracing::info!(
        target: "turbo",
        restored = report.count(Section::Restored),
        failed = report.count(Section::Failed),
        "turbo off"
    );
    Ok(report)
}

/// Put the Booster's journalled changes back and say so in the report.
async fn restore_booster(report: &mut Report) {
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
        Err(e) => booster_failed(report, &e),
    }
}

// ── The scheduler falcond leaves behind ─────────────────────────────────────

/// Where the scheduler loaded before Turbo on is noted.
fn scheduler_record() -> PathBuf {
    crate::paths::state_home()
        .join("bigame-mode")
        .join("scheduler-before-turbo.json")
}

/// Note the sched-ext scheduler loaded before Turbo on (`None`: none).
fn remember_scheduler(loaded: Option<&str>) {
    let path = scheduler_record();
    let written = (|| {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(
            &path,
            serde_json::to_vec(&serde_json::json!({ "loaded": loaded }))?,
        )?;
        anyhow::Ok(())
    })();
    if let Err(e) = written {
        tracing::warn!(target: "turbo", error = %format!("{e:#}"), "could not note the scheduler loaded before Turbo");
    }
}

/// What was noted at Turbo on about the sched-ext scheduler.
#[derive(Debug, Clone, PartialEq, Eq)]
enum SchedulerBefore {
    /// Nothing was noted (Turbo was switched on by an earlier build).
    Unknown,
    /// None was loaded.
    Nothing,
    /// This one was.
    Loaded(String),
}

/// The scheduler noted at Turbo on.
fn remembered_scheduler() -> SchedulerBefore {
    let Some(value) = std::fs::read_to_string(scheduler_record())
        .ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
    else {
        return SchedulerBefore::Unknown;
    };
    match value.get("loaded") {
        Some(serde_json::Value::String(name)) => SchedulerBefore::Loaded(name.clone()),
        Some(serde_json::Value::Null) => SchedulerBefore::Nothing,
        _ => SchedulerBefore::Unknown,
    }
}

/// The scheduler still loaded after falcond stopped that Turbo put there.
///
/// falcond (2.0.14, `daemon.zig`) loads the scheduler its general
/// configuration names when it starts, and on stopping only puts back what
/// a game's profile changed: the general one stays loaded through
/// `scx_loader`. One that ran before Turbo on is not Turbo's; with nothing
/// noted then, the general configuration's own is.
fn scheduler_left(
    before: &SchedulerBefore,
    after: Option<&str>,
    general: Option<&str>,
) -> Option<String> {
    let after = after?;
    let turbos = match before {
        SchedulerBefore::Loaded(name) => name != after,
        SchedulerBefore::Nothing => true,
        SchedulerBefore::Unknown => general == Some(after),
    };
    turbos.then(|| after.to_owned())
}

/// The sched-ext scheduler running or being started: `scx_loader`'s
/// `CurrentScheduler`, else what the kernel has attached.
///
/// The loader names a scheduler as soon as it is asked to start one; the
/// kernel shows it only once its BPF program is attached, a second or two
/// later. A Turbo off right after a Turbo on (checked: on at :23, off at
/// :24, lavd attached at :25) would otherwise find nothing loaded, and the
/// scheduler falcond leaves behind would go unreported.
async fn current_scheduler() -> Option<String> {
    match loader_scheduler().await {
        Some(name) => name,
        None => crate::running::loaded_scheduler(),
    }
}

/// `scx_loader`'s `CurrentScheduler`: `Some(None)` when it runs none,
/// `None` when the loader cannot be asked (not running: nothing starts a
/// scheduler then, and the kernel's answer stands).
async fn loader_scheduler() -> Option<Option<String>> {
    let connection = zbus::Connection::system().await.ok()?;
    let dbus = zbus::fdo::DBusProxy::new(&connection).await.ok()?;
    let name = zbus::names::BusName::try_from("org.scx.Loader").ok()?;
    // Asked only of a running loader: a property read would start it.
    if !dbus.name_has_owner(name).await.ok()? {
        return None;
    }
    let proxy = zbus::proxy::Builder::<zbus::Proxy<'_>>::new(&connection)
        .destination("org.scx.Loader")
        .ok()?
        .path("/org/scx/Loader")
        .ok()?
        .interface("org.scx.Loader")
        .ok()?
        .cache_properties(zbus::proxy::CacheProperties::No)
        .build()
        .await
        .ok()?;
    let current: String = proxy.get_property("CurrentScheduler").await.ok()?;
    Some(loader_name(&current))
}

/// `scx_lavd` → `lavd`; `unknown` (the loader's word for none) → `None`.
fn loader_name(current: &str) -> Option<String> {
    let name = current.trim();
    let name = name.strip_prefix("scx_").unwrap_or(name);
    (!name.is_empty() && name != "unknown").then(|| name.to_owned())
}

/// Report a scheduler falcond left loaded as it stopped.
async fn note_scheduler_left(report: &mut Report) {
    let before = remembered_scheduler();
    let general = crate::config::read()
        .ok()
        .map(|c| c.scx_sched)
        .filter(|s| !s.is_empty() && s != "none");
    let left = scheduler_left(
        &before,
        current_scheduler().await.as_deref(),
        general.as_deref(),
    );
    if let Some(name) = left {
        report.push(
            Kind::Scheduler,
            Section::Failed,
            "falcond",
            Text::with(
                N_(
                    "%s is still loaded after falcond stopped: falcond does not unload the \
                     scheduler its general configuration loads",
                ),
                [format!("scx_{name}")],
            ),
        );
    }
    let path = scheduler_record();
    if let Err(e) = std::fs::remove_file(&path)
        && e.kind() != std::io::ErrorKind::NotFound
    {
        tracing::warn!(target: "turbo", file = %path.display(), error = %e, "could not remove the scheduler note");
    }
}

// ── Turbo switched off from outside ─────────────────────────────────────────

/// Whether anything of Turbo may still be in force: falcond enabled or
/// running, a preset's record, or Booster changes. An error after a failed
/// Turbo on is then something to turn off, not only to retry.
#[must_use]
pub fn something_left(
    unit: Option<&crate::systemd::UnitState>,
    preset: bool,
    booster: bool,
) -> bool {
    let unit = unit.is_some_and(|u| {
        u.is_installed() && (u.is_active() || u.unit_file_state.starts_with("enabled"))
    });
    unit || preset || booster
}

/// [`something_left`] on this machine, for the UI's worker threads.
#[must_use]
pub fn something_left_blocking() -> bool {
    let unit = crate::systemd::Reader::shared().and_then(|r| r.unit_state(BACKEND_UNIT));
    something_left(
        unit.as_ref(),
        crate::turbo_preset::active() != crate::turbo_preset::Preset::Standard,
        BoosterEngine::is_active(),
    )
}

/// Take away what Turbo left in force after falcond stopped without Big
/// Game Mode asking: Settings → Hand back, `systemctl stop`, a crash. The
/// preset's variables and the Booster's changes stay otherwise, behind a
/// Turbo that every surface shows off. Nothing is done while falcond runs:
/// Turbo is on then. For the UI's worker threads.
///
/// # Errors
/// Returns an error if systemd cannot be asked about falcond, or the preset
/// or the Booster's changes could not be put back; both are attempted
/// either way.
pub fn tidy_up_blocking() -> Result<()> {
    let unit = crate::systemd::Reader::shared()
        .and_then(|r| r.unit_state(BACKEND_UNIT))
        .ok_or_else(|| anyhow::anyhow!("systemd could not be asked about {BACKEND_UNIT}"))?;
    if !backend_governs(&unit) || unit.is_active() {
        return Ok(());
    }
    let preset = crate::turbo_preset::resync(false);
    let booster = reconcile_blocking();
    if let Ok(n) = &booster
        && *n > 0
    {
        tracing::info!(target: "turbo", restored = n, "Booster changes put back after falcond stopped");
    }
    preset.context("take the Turbo preset away")?;
    booster.context("put the Booster's changes back")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_loaders_scheduler_name_is_the_kernels() {
        assert_eq!(loader_name("scx_lavd").as_deref(), Some("lavd"));
        assert_eq!(loader_name("unknown"), None);
        assert_eq!(loader_name(""), None);
        assert_eq!(loader_name("bpfland").as_deref(), Some("bpfland"));
    }

    #[test]
    fn who_controls_falcond_is_read_from_both_records() {
        assert_eq!(control_from(false, Some(1), None), Control::NotInstalled);
        assert_eq!(
            control_from(true, Some(10), None),
            Control::Managed { since: 10 }
        );
        // A release note left behind must not hide a newer ownership record.
        assert_eq!(
            control_from(true, Some(20), Some(15)),
            Control::Managed { since: 20 }
        );
        assert_eq!(
            control_from(true, None, Some(15)),
            Control::HandedBack { at: 15 }
        );
        assert_eq!(control_from(true, None, None), Control::NeverManaged);
        // Taking control is offered only where it means something.
        assert!(Control::HandedBack { at: 1 }.can_take());
        assert!(Control::NeverManaged.can_take());
        assert!(!Control::Managed { since: 1 }.can_take());
        assert!(!Control::NotInstalled.can_take());
    }

    #[test]
    fn the_scheduler_falcond_leaves_loaded_is_reported_only_when_turbo_put_it_there() {
        // lavd from falcond's general configuration, nothing loaded before.
        use SchedulerBefore::{Loaded, Nothing, Unknown};
        assert_eq!(
            scheduler_left(&Nothing, Some("lavd"), Some("lavd")),
            Some("lavd".into())
        );
        // Loaded before Turbo on: someone else's, left alone.
        assert_eq!(
            scheduler_left(&Loaded("lavd".into()), Some("lavd"), Some("lavd")),
            None
        );
        // Another one than before is Turbo's doing.
        assert_eq!(
            scheduler_left(&Loaded("bpfland".into()), Some("lavd"), None),
            Some("lavd".into())
        );
        // Nothing loaded: nothing to say.
        assert_eq!(scheduler_left(&Nothing, None, Some("lavd")), None);
        // Nothing noted at Turbo on: only the general configuration's own.
        assert_eq!(
            scheduler_left(&Unknown, Some("lavd"), Some("lavd")),
            Some("lavd".into())
        );
        assert_eq!(scheduler_left(&Unknown, Some("rusty"), Some("lavd")), None);
        assert_eq!(scheduler_left(&Unknown, Some("lavd"), None), None);
    }

    fn sandy_bridge() -> crate::isa::CpuIsa {
        crate::isa::CpuIsa {
            sse2: true,
            sse3: true,
            ssse3: true,
            sse4_1: true,
            sse4_2: true,
            popcnt: true,
            cmpxchg16b: true,
            avx: true,
            ..crate::isa::CpuIsa::default()
        }
    }

    #[test]
    fn an_illegal_instruction_on_an_older_cpu_is_called_an_incompatible_build() {
        use crate::systemd::Failure;
        let why = explain_backend_failure(&Failure::IllegalInstruction, &sandy_bridge());
        assert_eq!(
            why.title.english(),
            "falcond is not compatible with this processor"
        );
        let summary = why.summary.english();
        assert!(summary.contains("illegal instruction"), "{summary}");
        assert!(summary.contains("x86-64-v2"), "{summary}");
        assert!(
            summary.contains("BMI2") && summary.contains("AVX2"),
            "{summary}"
        );
        // Not "turn it off and on again": the same package fails the same way.
        assert!(!why.retry_helps);
        assert!(why.advice.english().contains("does not help"));
    }

    #[test]
    fn an_illegal_instruction_on_a_v3_cpu_blames_no_extension() {
        use crate::systemd::Failure;
        let modern = crate::isa::CpuIsa {
            avx2: true,
            bmi1: true,
            bmi2: true,
            fma: true,
            f16c: true,
            lzcnt: true,
            movbe: true,
            ..sandy_bridge()
        };
        let why = explain_backend_failure(&Failure::IllegalInstruction, &modern);
        assert!(why.title.english().contains("illegal CPU instruction"));
        assert!(!why.summary.english().contains("BMI2"));
        assert!(
            why.summary
                .english()
                .contains("instruction-set incompatibility")
        );
        assert!(!why.retry_helps);
    }

    #[test]
    fn other_falcond_failures_keep_the_restart_advice() {
        use crate::systemd::Failure;
        let crash = explain_backend_failure(&Failure::Signal(libc::SIGSEGV), &sandy_bridge());
        assert_eq!(crash.title.english(), "falcond crashed");
        assert!(crash.summary.english().contains("SIGSEGV"));
        assert!(crash.retry_helps);
        let exit = explain_backend_failure(&Failure::ExitCode(1), &sandy_bridge());
        assert!(exit.summary.english().contains("exit status 1"));
        assert!(exit.retry_helps);
    }

    #[test]
    fn an_error_with_something_left_offers_turning_it_off() {
        let unit = |file: &str, active: &str| crate::systemd::UnitState {
            unit_file_state: file.into(),
            active_state: active.into(),
            service: None,
        };
        // A failed start left falcond enabled: Turbo off disables it.
        assert!(something_left(
            Some(&unit("enabled", "failed")),
            false,
            false
        ));
        assert!(something_left(
            Some(&unit("disabled", "active")),
            false,
            false
        ));
        // A preset or Booster changes left in force.
        assert!(something_left(
            Some(&unit("disabled", "inactive")),
            true,
            false
        ));
        assert!(something_left(None, false, true));
        // Nothing at all: the error is only a Turbo that did not start.
        assert!(!something_left(
            Some(&unit("disabled", "failed")),
            false,
            false
        ));
        assert!(!something_left(
            Some(&unit("not-found", "inactive")),
            false,
            false
        ));
    }

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
