//! Baseline capture and exact restoration.
//!
//! The rule this module enforces: **restore what was there, never what we
//! assume was there.** Turning off by writing a fixed `balanced` would silently
//! demote a machine resting in `performance` or a laptop resting in
//! `power-saver`; a snapshot removes the guess entirely.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::knob::{Knob, Verification};

/// The value a knob held before Booster touched it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Captured {
    /// The knob this refers to.
    pub knob: Knob,
    /// Its value at capture time. `None` means the knob existed but could not
    /// be read — in which case it must never be written either, because there
    /// would be no way back.
    pub value: Option<String>,
}

/// System state recorded immediately before any change.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Snapshot {
    /// Unix seconds when the snapshot was taken.
    pub taken_at: u64,
    /// Captured values, keyed by [`Knob::id`] so lookups are stable across
    /// versions and orderings.
    pub entries: BTreeMap<String, Captured>,
}

impl Snapshot {
    /// Capture the current value of every knob in `knobs`.
    #[must_use]
    pub fn capture(knobs: &[Knob]) -> Self {
        let mut entries = BTreeMap::new();
        for knob in knobs {
            entries.insert(
                knob.id(),
                Captured {
                    knob: knob.clone(),
                    value: knob.read(),
                },
            );
        }
        Self {
            taken_at: crate::unix_now(),
            entries,
        }
    }

    /// The recorded value for a knob, if it was captured and readable.
    #[must_use]
    pub fn value_of(&self, knob: &Knob) -> Option<&str> {
        self.entries.get(&knob.id())?.value.as_deref()
    }

    /// True when we hold a restorable value for this knob.
    ///
    /// The planner's rule, which it applies through [`Self::value_of`]: **a
    /// knob we cannot restore is a knob we must not touch.**
    #[cfg(test)]
    #[must_use]
    pub fn is_restorable(&self, knob: &Knob) -> bool {
        self.value_of(knob).is_some()
    }

    /// Restore only the knobs named in `knob_ids`, in reverse order.
    ///
    /// **Only knobs that were actually changed may be restored.** A snapshot
    /// deliberately captures more than the plan touches, because a broad
    /// baseline makes for a better report — but writing back a knob we never
    /// wrote is not a restoration, it is a new change, and it fails in exactly
    /// the ways a new change can.
    ///
    /// For example, `cpu_epp` captured as `power` on a machine in
    /// `power-saver` but never planned: writing it back is refused, because
    /// with the governor at `performance` the only accepted EPP is
    /// `performance` — and needless, since restoring the power profile it
    /// depends on already puts it back.
    ///
    /// Knobs are restored in reverse application order, the usual
    /// transactional discipline, so a knob is put back before whatever was
    /// changed on top of it.
    pub async fn restore_applied(&self, knob_ids: &[String]) -> Vec<RestoreOutcome> {
        let mut out = Vec::new();
        for id in knob_ids.iter().rev() {
            let Some(entry) = self.entries.get(id) else {
                continue;
            };
            out.push(self.restore_one(entry).await);
        }
        out
    }

    /// Restoration is best-effort per knob and never stops early: one knob
    /// failing must not strand the rest of the system in Booster state. Every
    /// outcome is reported so the caller can tell the user exactly what could
    /// not be put back.
    async fn restore_one(&self, entry: &Captured) -> RestoreOutcome {
        {
            let Some(want) = entry.value.as_deref() else {
                return RestoreOutcome {
                    knob: entry.knob.clone(),
                    target: String::new(),
                    status: RestoreStatus::Failed {
                        error: "no baseline was captured for this knob".into(),
                    },
                };
            };
            let current = entry.knob.read();
            if current.as_deref() == Some(want) {
                return RestoreOutcome {
                    knob: entry.knob.clone(),
                    target: want.to_owned(),
                    status: RestoreStatus::AlreadyCorrect,
                };
            }
            let status = match entry.knob.write(want).await {
                Ok(()) => match entry.knob.verify(want) {
                    Verification::Confirmed => RestoreStatus::Restored,
                    Verification::Mismatch { actual } => RestoreStatus::Failed {
                        error: format!("wrote {want:?} but the knob reads {actual:?}"),
                    },
                    Verification::Unreadable => RestoreStatus::Failed {
                        error: "value could not be read back".into(),
                    },
                },
                Err(e) => RestoreStatus::Failed {
                    error: format!("{e:#}"),
                },
            };
            RestoreOutcome {
                knob: entry.knob.clone(),
                target: want.to_owned(),
                status,
            }
        }
    }
}

/// What happened when one knob was restored.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RestoreOutcome {
    /// The knob.
    pub knob: Knob,
    /// The value it was being restored to.
    pub target: String,
    /// Result.
    pub status: RestoreStatus,
}

/// Result of restoring one knob.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum RestoreStatus {
    /// Written back and verified.
    Restored,
    /// Already held the captured value; nothing was written.
    AlreadyCorrect,
    /// Could not be put back. The system is left in a known-reported state
    /// rather than a silently wrong one.
    Failed {
        /// Why.
        error: String,
    },
}

impl RestoreStatus {
    /// True when the knob now holds its captured value.
    #[must_use]
    pub fn is_ok(&self) -> bool {
        matches!(self, Self::Restored | Self::AlreadyCorrect)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot_of(pairs: &[(Knob, Option<&str>)]) -> Snapshot {
        Snapshot {
            taken_at: 1_700_000_000,
            entries: pairs
                .iter()
                .map(|(k, v)| {
                    (
                        k.id(),
                        Captured {
                            knob: k.clone(),
                            value: v.map(str::to_owned),
                        },
                    )
                })
                .collect(),
        }
    }

    #[test]
    fn records_the_value_that_was_actually_there() {
        // A machine resting in `performance` is restored to `performance`,
        // not to a hardcoded `balanced`.
        let snap = snapshot_of(&[(Knob::PowerProfile, Some("performance"))]);
        assert_eq!(snap.value_of(&Knob::PowerProfile), Some("performance"));
        assert_ne!(snap.value_of(&Knob::PowerProfile), Some("balanced"));
    }

    #[test]
    fn a_power_saver_baseline_is_preserved_too() {
        let snap = snapshot_of(&[(Knob::PowerProfile, Some("power-saver"))]);
        assert_eq!(snap.value_of(&Knob::PowerProfile), Some("power-saver"));
    }

    #[test]
    fn unreadable_knobs_are_not_restorable_and_must_not_be_touched() {
        let snap = snapshot_of(&[(Knob::CpuEpp, None)]);
        assert!(!snap.is_restorable(&Knob::CpuEpp));
        assert_eq!(snap.value_of(&Knob::CpuEpp), None);
    }

    #[test]
    fn unknown_knobs_are_not_restorable() {
        let snap = snapshot_of(&[(Knob::PowerProfile, Some("balanced"))]);
        assert!(!snap.is_restorable(&Knob::CpuGovernor));
    }

    #[test]
    fn survives_the_journal_round_trip() {
        // A snapshot is useless if it cannot be reloaded after a crash.
        let snap = snapshot_of(&[
            (Knob::PowerProfile, Some("performance")),
            (Knob::CpuGovernor, Some("powersave")),
            (
                Knob::GpuDpmLevel {
                    card: "card1".into(),
                },
                Some("auto"),
            ),
            (Knob::CpuEpp, None),
        ]);
        let json = serde_json::to_string(&snap).unwrap();
        let back: Snapshot = serde_json::from_str(&json).unwrap();
        assert_eq!(back.taken_at, snap.taken_at);
        assert_eq!(back.entries.len(), 4);
        assert_eq!(back.value_of(&Knob::PowerProfile), Some("performance"));
        assert_eq!(
            back.value_of(&Knob::GpuDpmLevel {
                card: "card1".into()
            }),
            Some("auto")
        );
        assert!(!back.is_restorable(&Knob::CpuEpp));
    }

    #[test]
    fn capture_reads_the_live_machine() {
        let snap = Snapshot::capture(&[Knob::PowerProfile, Knob::CpuGovernor]);
        assert_eq!(snap.entries.len(), 2);
        assert!(snap.taken_at > 0);
    }

    #[tokio::test]
    async fn restore_touches_only_the_knobs_that_were_applied() {
        // A snapshot captures broadly so the report can be informative, but
        // rollback must write back only what was actually changed. Restoring
        // an untouched knob is a new change, not a restoration.
        let snap = snapshot_of(&[
            (Knob::PowerProfile, Some("power-saver")),
            (Knob::CpuEpp, Some("power")),
            (
                Knob::GpuDpmLevel {
                    card: "card999".into(),
                },
                Some("auto"),
            ),
        ]);

        // Nothing was applied, so nothing may be written.
        assert!(snap.restore_applied(&[]).await.is_empty());

        // Only the named knob is considered.
        let only_gpu = snap
            .restore_applied(&[Knob::GpuDpmLevel {
                card: "card999".into(),
            }
            .id()])
            .await;
        assert_eq!(only_gpu.len(), 1);
        assert_eq!(only_gpu[0].knob.id(), "gpu_dpm_level:card999");
    }

    #[tokio::test]
    async fn restore_runs_in_reverse_application_order() {
        // Knobs depend on one another — a power profile drives the governor and
        // the EPP — so the last thing changed is the first thing put back.
        let snap = snapshot_of(&[
            (
                Knob::GpuDpmLevel {
                    card: "card999".into(),
                },
                Some("auto"),
            ),
            (
                Knob::GpuDpmLevel {
                    card: "card998".into(),
                },
                Some("auto"),
            ),
        ]);
        let applied = vec![
            Knob::GpuDpmLevel {
                card: "card999".into(),
            }
            .id(),
            Knob::GpuDpmLevel {
                card: "card998".into(),
            }
            .id(),
        ];
        let order: Vec<String> = snap
            .restore_applied(&applied)
            .await
            .iter()
            .map(|o| o.knob.id())
            .collect();
        assert_eq!(
            order,
            vec!["gpu_dpm_level:card998", "gpu_dpm_level:card999"]
        );
    }

    #[tokio::test]
    async fn restore_ignores_ids_that_were_never_captured() {
        let snap = snapshot_of(&[(Knob::PowerProfile, Some("balanced"))]);
        assert!(
            snap.restore_applied(&["cpu_governor".to_owned()])
                .await
                .is_empty()
        );
    }

    #[test]
    fn restore_status_classification() {
        assert!(RestoreStatus::Restored.is_ok());
        assert!(RestoreStatus::AlreadyCorrect.is_ok());
        assert!(!RestoreStatus::Failed { error: "x".into() }.is_ok());
    }
}
