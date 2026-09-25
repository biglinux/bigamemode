//! Crash-recovery journal.
//!
//! Booster Mode changes system state that outlives the process that changed it.
//! If the UI is killed, the session ends or the machine loses power mid-apply,
//! something has to know what the machine looked like beforehand. That is this
//! file.
//!
//! Two properties matter more than anything else here:
//!
//! * **Atomicity.** The journal is written to a temporary file in the same
//!   directory and then `rename(2)`d into place, so a reader never observes a
//!   half-written record. A torn journal would be worse than none at all.
//! * **Boot awareness.** Every record carries the kernel's boot id. sysfs knobs
//!   (governor, DPM level, V-Cache) reset themselves at boot, so a journal from
//!   a previous boot must not be replayed against them — only knobs whose state
//!   genuinely survives a reboot are worth restoring from a stale record.

use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use super::plan::Plan;
use super::snapshot::Snapshot;

/// Journal format version. Bumped when the on-disk shape changes; records from
/// an unknown version are discarded rather than misinterpreted.
const FORMAT: u32 = 1;

/// A persisted Booster activation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Journal {
    /// On-disk format version.
    pub format: u32,
    /// `/proc/sys/kernel/random/boot_id` at write time.
    pub boot_id: String,
    /// Unix seconds when Booster was activated.
    pub activated_at: u64,
    /// The state to return to.
    pub snapshot: Snapshot,
    /// What was planned.
    pub plan: Plan,
    /// Knob ids that were written and verified, so a recovery pass knows which
    /// ones actually need undoing.
    pub applied: Vec<String>,
}

impl Journal {
    /// Where the journal lives.
    ///
    /// `$XDG_STATE_HOME` (not the runtime dir) because the record must survive
    /// the session that wrote it — a crash during logout is precisely when
    /// recovery is needed.
    #[must_use]
    pub fn path() -> PathBuf {
        crate::paths::state_home()
            .join("bigame-mode")
            .join("booster-journal.json")
    }

    /// Current boot id, or an empty string if unreadable.
    #[must_use]
    pub fn current_boot_id() -> String {
        std::fs::read_to_string("/proc/sys/kernel/random/boot_id")
            .map(|s| s.trim().to_owned())
            .unwrap_or_default()
    }

    /// Create a record for a fresh activation.
    #[must_use]
    pub fn new(snapshot: Snapshot, plan: Plan) -> Self {
        Self {
            format: FORMAT,
            boot_id: Self::current_boot_id(),
            activated_at: crate::unix_now(),
            snapshot,
            plan,
            applied: Vec::new(),
        }
    }

    /// Whether this record was written during the currently running boot.
    #[must_use]
    pub fn is_current_boot(&self) -> bool {
        !self.boot_id.is_empty() && self.boot_id == Self::current_boot_id()
    }

    /// Persist to the default location.
    ///
    /// # Errors
    /// Returns an error if the journal cannot be written.
    pub fn save(&self) -> Result<()> {
        self.save_to(&Self::path())
    }

    /// Persist atomically to `path`, with owner-only permissions.
    ///
    /// # Errors
    /// Returns an error if the directory cannot be created or the file cannot
    /// be written or renamed into place.
    pub fn save_to(&self, path: &std::path::Path) -> Result<()> {
        let dir = path
            .parent()
            .context("journal path has no parent directory")?;
        std::fs::create_dir_all(dir)
            .with_context(|| format!("create state dir: {}", dir.display()))?;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700)).ok();

        let json = serde_json::to_vec_pretty(self).context("serialize journal")?;

        // Same directory, so the rename is atomic on the same filesystem.
        // `.pid` keeps two concurrent writers from sharing a temp file.
        let tmp = path.with_extension(format!("tmp.{}", std::process::id()));
        {
            let mut f = std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(0o600)
                .open(&tmp)
                .with_context(|| format!("open temp journal: {}", tmp.display()))?;
            f.write_all(&json).context("write journal")?;
            // Durability before the rename, so a power cut cannot leave the
            // renamed file pointing at unflushed data.
            f.sync_all().context("sync journal")?;
        }
        std::fs::rename(&tmp, path)
            .with_context(|| format!("rename {} -> {}", tmp.display(), path.display()))?;
        Ok(())
    }

    /// Load the journal from the default location.
    ///
    /// # Errors
    /// Returns an error only for unexpected I/O failures.
    pub fn load() -> Result<Option<Self>> {
        Self::load_from(&Self::path())
    }

    /// Load a journal from `path`, if one exists and is intelligible.
    ///
    /// Returns `Ok(None)` for "no journal" and for "a journal we cannot trust",
    /// which are the same thing from the caller's point of view. A corrupt
    /// record is removed so it cannot keep failing forever.
    ///
    /// # Errors
    /// Returns an error only for unexpected I/O failures.
    pub fn load_from(path: &std::path::Path) -> Result<Option<Self>> {
        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e).context("read journal"),
        };
        match serde_json::from_str::<Self>(&content) {
            Ok(j) if j.format == FORMAT => Ok(Some(j)),
            Ok(j) => {
                tracing::warn!(
                    target: "booster",
                    found = j.format,
                    expected = FORMAT,
                    "discarding journal written by an incompatible version"
                );
                Self::clear_at(path);
                Ok(None)
            }
            Err(e) => {
                tracing::warn!(target: "booster", error = %e, "discarding corrupt journal");
                Self::clear_at(path);
                Ok(None)
            }
        }
    }

    /// Remove the journal from the default location. Booster is no longer active.
    pub fn clear() {
        Self::clear_at(&Self::path());
    }

    /// Remove the journal at `path`.
    pub fn clear_at(path: &std::path::Path) {
        let _ = std::fs::remove_file(path);
    }

    /// Record that a knob was applied and verified.
    pub fn mark_applied(&mut self, knob_id: String) {
        if !self.applied.contains(&knob_id) {
            self.applied.push(knob_id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::booster::knob::Knob;
    use crate::booster::snapshot::Captured;

    /// A private journal path per test.
    ///
    /// These tests deliberately do **not** touch `XDG_STATE_HOME`: environment
    /// variables are process-global, and `cargo test` runs tests in parallel
    /// threads, so mutating one races every other test in the binary. Passing
    /// the path explicitly keeps each test hermetic — which is also why
    /// `save_to`/`load_from` exist as public API.
    struct Fixture(PathBuf);

    impl Fixture {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "bigame_journal_{tag}_{}_{:?}",
                std::process::id(),
                std::thread::current().id()
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir.join("booster-journal.json"))
        }

        fn path(&self) -> &std::path::Path {
            &self.0
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            if let Some(dir) = self.0.parent() {
                let _ = std::fs::remove_dir_all(dir);
            }
        }
    }

    fn sample_snapshot() -> Snapshot {
        let mut entries = std::collections::BTreeMap::new();
        entries.insert(
            Knob::PowerProfile.id(),
            Captured {
                knob: Knob::PowerProfile,
                value: Some("performance".into()),
            },
        );
        Snapshot {
            taken_at: 1_700_000_000,
            entries,
        }
    }

    #[test]
    fn save_then_load_round_trips() {
        let f = Fixture::new("roundtrip");
        let mut j = Journal::new(sample_snapshot(), Plan::default());
        j.mark_applied("power_profile".into());
        j.save_to(f.path()).unwrap();

        let back = Journal::load_from(f.path())
            .unwrap()
            .expect("journal should exist");
        assert_eq!(back.format, FORMAT);
        assert_eq!(back.applied, vec!["power_profile".to_owned()]);
        assert_eq!(
            back.snapshot.value_of(&Knob::PowerProfile),
            Some("performance")
        );
    }

    #[test]
    fn absent_journal_is_not_an_error() {
        let f = Fixture::new("absent");
        assert!(Journal::load_from(f.path()).unwrap().is_none());
    }

    #[test]
    fn corrupt_journal_is_discarded_not_fatal() {
        let f = Fixture::new("corrupt");
        std::fs::write(f.path(), b"{ this is not json").unwrap();

        assert!(Journal::load_from(f.path()).unwrap().is_none());
        // and it is removed, so it cannot keep failing on every start
        assert!(!f.path().exists());
    }

    #[test]
    fn journal_from_an_incompatible_format_is_discarded() {
        let f = Fixture::new("format");
        std::fs::write(
            f.path(),
            br#"{"format":999,"boot_id":"x","activated_at":0,
                 "snapshot":{"taken_at":0,"entries":{}},
                 "plan":{"changes":[],"skipped":[]},"applied":[]}"#,
        )
        .unwrap();
        assert!(Journal::load_from(f.path()).unwrap().is_none());
        assert!(!f.path().exists());
    }

    #[test]
    fn default_path_lives_under_the_state_directory() {
        // The record must survive the session that wrote it, so it belongs in
        // XDG_STATE_HOME rather than the runtime dir, which is wiped on logout.
        let p = Journal::path();
        assert!(p.ends_with("bigame-mode/booster-journal.json"), "{p:?}");
    }

    #[test]
    fn records_the_boot_it_was_written_in() {
        let j = Journal::new(sample_snapshot(), Plan::default());
        // On a real Linux host this is populated and matches.
        if !Journal::current_boot_id().is_empty() {
            assert!(j.is_current_boot());
        }

        // A record from a previous boot must be recognised as stale, because
        // sysfs knobs have already reset themselves by now.
        let stale = Journal {
            boot_id: "00000000-0000-0000-0000-000000000000".into(),
            ..j
        };
        assert!(!stale.is_current_boot());
    }

    #[test]
    fn an_empty_boot_id_is_never_treated_as_current() {
        let j = Journal {
            boot_id: String::new(),
            ..Journal::new(sample_snapshot(), Plan::default())
        };
        assert!(!j.is_current_boot());
    }

    #[test]
    fn saved_journal_is_owner_only() {
        let f = Fixture::new("perms");
        Journal::new(sample_snapshot(), Plan::default())
            .save_to(f.path())
            .unwrap();
        let mode = std::fs::metadata(f.path()).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "journal must not be world-readable");
    }

    #[test]
    fn save_leaves_no_temp_files_behind() {
        let f = Fixture::new("tmp");
        Journal::new(sample_snapshot(), Plan::default())
            .save_to(f.path())
            .unwrap();
        let leftovers: Vec<_> = std::fs::read_dir(f.path().parent().unwrap())
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.contains("tmp"))
            .collect();
        assert!(leftovers.is_empty(), "temp files left: {leftovers:?}");
    }

    #[test]
    fn save_overwrites_an_existing_record_atomically() {
        let f = Fixture::new("overwrite");
        let mut first = Journal::new(sample_snapshot(), Plan::default());
        first.mark_applied("cpu_governor".into());
        first.save_to(f.path()).unwrap();

        let second = Journal::new(sample_snapshot(), Plan::default());
        second.save_to(f.path()).unwrap();

        let back = Journal::load_from(f.path()).unwrap().unwrap();
        assert!(
            back.applied.is_empty(),
            "second write must fully replace the first"
        );
    }

    #[test]
    fn clear_removes_the_record() {
        let f = Fixture::new("clear");
        Journal::new(sample_snapshot(), Plan::default())
            .save_to(f.path())
            .unwrap();
        assert!(Journal::load_from(f.path()).unwrap().is_some());
        Journal::clear_at(f.path());
        assert!(Journal::load_from(f.path()).unwrap().is_none());
    }

    #[test]
    fn clearing_an_absent_record_is_harmless() {
        let f = Fixture::new("clear_absent");
        Journal::clear_at(f.path());
        Journal::clear_at(f.path());
    }

    #[test]
    fn mark_applied_is_idempotent() {
        let mut j = Journal::new(sample_snapshot(), Plan::default());
        j.mark_applied("cpu_governor".into());
        j.mark_applied("cpu_governor".into());
        assert_eq!(j.applied, vec!["cpu_governor".to_owned()]);
    }
}
