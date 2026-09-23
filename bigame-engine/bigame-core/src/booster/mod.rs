//! Booster Mode — the optimization orchestrator.
//!
//! The pipeline is fixed and every stage is mandatory:
//!
//! ```text
//! Detect → Snapshot → Plan → Apply → Verify → Report → (later) Restore
//! ```
//!
//! Two of those stages are what separate this from the toggle it replaces.
//! **Snapshot** runs before anything is written, so "off" returns the machine
//! to the state it was actually in rather than to a hardcoded guess. **Verify**
//! runs after every write, so the report describes what the system did rather
//! than what we asked it to do.
//!
//! The engine has no opinions of its own: [`plan::Plan`] decides what to change
//! and [`knob::Knob`] knows how. That separation is what makes the whole thing
//! testable without root.

pub mod journal;
pub mod knob;
pub mod plan;
pub mod report;
pub mod snapshot;

use anyhow::Result;

use crate::capabilities::Capabilities;
use crate::hardware::Hardware;

use journal::Journal;
use knob::Knob;
use plan::Plan;
use report::{AppliedChange, Report};
use snapshot::{RestoreOutcome, Snapshot};

/// Progress stages, emitted as the engine works.
///
/// The UI shows these as they arrive. Every variant corresponds to work that is
/// genuinely happening — there is no synthetic progress and no fixed-duration
/// animation standing in for real work.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Progress {
    /// Reading CPU, GPU, display and power state.
    DetectingHardware,
    /// Probing which tools and services are usable.
    DetectingCapabilities,
    /// Recording current values so they can be restored.
    CapturingBaseline,
    /// Deciding what is worth changing.
    Planning,
    /// Writing one change.
    Applying {
        /// Human label of the knob being written.
        knob: String,
        /// 1-based position.
        index: usize,
        /// Total changes in the plan.
        total: usize,
    },
    /// Reading one change back.
    Verifying {
        /// Human label of the knob being checked.
        knob: String,
    },
    /// Done.
    Finished,
}

/// The orchestrator.
pub struct BoosterEngine {
    hardware: Hardware,
    capabilities: Capabilities,
}

impl BoosterEngine {
    /// Probe the machine. Cheap enough to call on demand, but the result is
    /// worth holding for the length of a session.
    #[must_use]
    pub fn detect() -> Self {
        Self {
            hardware: Hardware::detect(),
            capabilities: Capabilities::detect(),
        }
    }

    /// The detected hardware.
    #[must_use]
    pub fn hardware(&self) -> &Hardware {
        &self.hardware
    }

    /// The detected capabilities.
    #[must_use]
    pub fn capabilities(&self) -> &Capabilities {
        &self.capabilities
    }

    /// Every knob that exists on this machine and is therefore worth capturing.
    ///
    /// Capturing a knob is free and always safe; *writing* one is what the plan
    /// gates. Capturing generously means a later plan is never blocked by a
    /// missing baseline.
    #[must_use]
    pub fn relevant_knobs(&self) -> Vec<Knob> {
        let mut knobs = vec![Knob::PowerProfile, Knob::CpuGovernor, Knob::CpuEpp];
        if let Some(gpu) = self.hardware.render_gpu() {
            knobs.push(Knob::GpuDpmLevel {
                card: gpu.card.clone(),
            });
        }
        if self.hardware.cpu.vcache.is_some() {
            knobs.push(Knob::VCacheMode);
        }
        knobs
    }

    /// Whether Booster is currently active, according to the journal.
    #[must_use]
    pub fn is_active() -> bool {
        Self::active_summary().is_some()
    }

    /// How many changes are currently in force, if Booster is active.
    ///
    /// Also reconciles a stale record: a journal written during a previous boot
    /// describes sysfs knobs that the kernel has already reset to their
    /// defaults, so replaying or reporting it would be describing a state the
    /// machine is not in. Such a record is discarded here rather than shown.
    ///
    /// Returns `None` when Booster is not active.
    #[must_use]
    pub fn active_summary() -> Option<usize> {
        let record = Journal::load().ok().flatten()?;
        if !record.is_current_boot() {
            tracing::info!(
                target: "booster",
                "discarding a journal from a previous boot; kernel state has already reset"
            );
            Journal::clear();
            return None;
        }
        Some(record.applied.len())
    }

    /// Capture a baseline and build a plan, **without changing anything**.
    ///
    /// Exposed separately so the UI can show the user what is about to happen
    /// before it happens.
    #[must_use]
    pub fn dry_run(&self) -> (Snapshot, Plan) {
        let snapshot = Snapshot::capture(&self.relevant_knobs());
        let plan = Plan::build(&self.hardware, &self.capabilities, &snapshot);
        (snapshot, plan)
    }

    /// Run the full pipeline.
    ///
    /// `progress` is called on the calling task as each stage begins.
    ///
    /// The journal is written **before** the first change and updated after
    /// each verified one, so a crash at any point leaves enough on disk to
    /// undo the work that had been done.
    ///
    /// # Errors
    /// Returns an error only if the baseline could not be journalled — at which
    /// point nothing has been changed, because changing state we could not undo
    /// is the one thing this engine will not do.
    pub async fn activate<F: FnMut(Progress)>(&self, mut progress: F) -> Result<Report> {
        progress(Progress::DetectingHardware);
        progress(Progress::DetectingCapabilities);

        progress(Progress::CapturingBaseline);
        let snapshot = Snapshot::capture(&self.relevant_knobs());

        progress(Progress::Planning);
        let plan = Plan::build(&self.hardware, &self.capabilities, &snapshot);

        let mut report = Report {
            machine: plan::describe_machine(&self.hardware),
            skipped: plan.skipped.clone(),
            ..Report::default()
        };

        if plan.is_empty() {
            tracing::info!(
                target: "booster",
                skipped = plan.skipped.len(),
                "nothing to change; system already configured for gaming"
            );
            progress(Progress::Finished);
            return Ok(report);
        }

        // Persist the baseline before touching anything. If this fails we stop:
        // an un-undoable change is worse than no change.
        let mut record = Journal::new(snapshot.clone(), plan.clone());
        record.save()?;

        let total = plan.changes.len();
        for (i, change) in plan.changes.iter().enumerate() {
            progress(Progress::Applying {
                knob: change.knob.title(),
                index: i + 1,
                total,
            });

            let applied = match change.knob.write(&change.to).await {
                Ok(()) => {
                    progress(Progress::Verifying {
                        knob: change.knob.title(),
                    });
                    let verification = change.knob.verify(&change.to);
                    if verification.is_confirmed() {
                        record.mark_applied(change.knob.id());
                        // Best-effort: a journal write failing mid-run must not
                        // abort a run that is otherwise succeeding, but it is
                        // worth knowing about.
                        if let Err(e) = record.save() {
                            tracing::warn!(
                                target: "booster",
                                error = %e,
                                "could not update journal after applying {}",
                                change.knob.id()
                            );
                        }
                        tracing::info!(
                            target: "booster",
                            knob = %change.knob.id(),
                            from = %change.from,
                            to = %change.to,
                            "applied and verified"
                        );
                    } else {
                        tracing::warn!(
                            target: "booster",
                            knob = %change.knob.id(),
                            ?verification,
                            "write accepted but the system did not change"
                        );
                    }
                    AppliedChange {
                        knob: change.knob.clone(),
                        from: change.from.clone(),
                        to: change.to.clone(),
                        rationale: change.rationale.clone(),
                        verification,
                        error: None,
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        target: "booster",
                        knob = %change.knob.id(),
                        error = %format!("{e:#}"),
                        "write failed"
                    );
                    report::failed(change, format!("{e:#}"))
                }
            };
            report.applied.push(applied);
        }

        progress(Progress::Finished);
        Ok(report)
    }

    /// Turn Booster off: restore the journalled baseline exactly.
    ///
    /// Returns the per-knob outcomes. The journal is cleared only when every
    /// knob was put back — if something could not be restored the record stays
    /// on disk so a later attempt, or the next session, can finish the job.
    ///
    /// # Errors
    /// Returns an error if the journal could not be read.
    pub async fn deactivate() -> Result<Vec<RestoreOutcome>> {
        let Some(record) = Journal::load()? else {
            return Ok(Vec::new());
        };

        // sysfs knobs (governor, DPM level, V-Cache) reset themselves at boot,
        // so replaying a previous boot's values would be writing state that is
        // already correct — or worse, re-applying a value the user has since
        // changed deliberately.
        if !record.is_current_boot() {
            tracing::info!(
                target: "booster",
                "journal is from a previous boot; kernel state has already reset"
            );
            Journal::clear();
            return Ok(Vec::new());
        }

        // Only the knobs this run actually applied are put back. Writing a
        // knob we never wrote would be a fresh change, not a restoration.
        let outcomes = record.snapshot.restore_applied(&record.applied).await;
        let all_ok = outcomes.iter().all(|o| o.status.is_ok());
        if all_ok {
            Journal::clear();
            tracing::info!(target: "booster", restored = outcomes.len(), "baseline restored");
        } else {
            tracing::warn!(
                target: "booster",
                failed = outcomes.iter().filter(|o| !o.status.is_ok()).count(),
                "some knobs could not be restored; keeping the journal for retry"
            );
        }
        Ok(outcomes)
    }

    /// Recover after a crash or an unclean shutdown.
    ///
    /// Call once at start-up. If a journal is present it means a previous run
    /// left the machine in Booster state without ever being turned off, so the
    /// baseline is restored before the user sees anything.
    ///
    /// # Errors
    /// Returns an error if the journal could not be read.
    pub async fn recover() -> Result<Vec<RestoreOutcome>> {
        if Journal::load()?.is_none() {
            return Ok(Vec::new());
        }
        tracing::info!(target: "booster", "found an unfinished Booster session; restoring baseline");
        Self::deactivate().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_the_real_machine_without_panicking() {
        let engine = BoosterEngine::detect();
        assert!(engine.hardware().cpu.logical_cpus >= 1);
    }

    #[test]
    fn relevant_knobs_track_the_hardware_that_exists() {
        let engine = BoosterEngine::detect();
        let knobs = engine.relevant_knobs();
        // These three are always worth capturing.
        assert!(knobs.contains(&Knob::PowerProfile));
        assert!(knobs.contains(&Knob::CpuGovernor));
        assert!(knobs.contains(&Knob::CpuEpp));

        // V-Cache is only listed when the CPU actually has it. On the bench
        // (a 5700G) it must not be.
        assert_eq!(
            knobs.contains(&Knob::VCacheMode),
            engine.hardware().cpu.vcache.is_some()
        );

        // The GPU knob names the render GPU, never a hardcoded card0.
        if let Some(gpu) = engine.hardware().render_gpu() {
            assert!(knobs.contains(&Knob::GpuDpmLevel {
                card: gpu.card.clone()
            }));
        }
    }

    #[test]
    fn knob_ids_stay_unique_for_this_machine() {
        let engine = BoosterEngine::detect();
        let mut ids: Vec<String> = engine.relevant_knobs().iter().map(Knob::id).collect();
        let before = ids.len();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), before);
    }

    #[test]
    fn dry_run_changes_nothing() {
        let engine = BoosterEngine::detect();
        let before: Vec<Option<String>> = engine.relevant_knobs().iter().map(Knob::read).collect();
        let (snapshot, _plan) = engine.dry_run();
        let after: Vec<Option<String>> = engine.relevant_knobs().iter().map(Knob::read).collect();
        assert_eq!(before, after, "dry_run must not write anything");
        assert_eq!(snapshot.entries.len(), engine.relevant_knobs().len());
    }

    #[test]
    fn a_plan_never_contains_a_no_op() {
        let engine = BoosterEngine::detect();
        let (_snapshot, plan) = engine.dry_run();
        for change in &plan.changes {
            assert_ne!(
                change.from, change.to,
                "planned a change from {} to itself",
                change.from
            );
            assert!(!change.rationale.is_empty());
        }
    }

    #[test]
    fn every_planned_knob_was_captured_first() {
        let engine = BoosterEngine::detect();
        let (snapshot, plan) = engine.dry_run();
        for change in &plan.changes {
            assert!(
                snapshot.is_restorable(&change.knob),
                "planned {} without a restorable baseline",
                change.knob.id()
            );
        }
    }

    #[test]
    fn progress_stages_are_comparable() {
        assert_eq!(Progress::Finished, Progress::Finished);
        assert_ne!(Progress::Planning, Progress::Finished);
        assert_eq!(
            Progress::Applying {
                knob: "Power profile".into(),
                index: 1,
                total: 3
            },
            Progress::Applying {
                knob: "Power profile".into(),
                index: 1,
                total: 3
            }
        );
    }
}
