//! A falcond build that crashed on this processor, remembered until it changes.
//!
//! falcond 2.0.14 built for x86-64-v3 dies with `SIGILL` on an x86-64-v2
//! processor (Sandy Bridge, Ivy Bridge: AVX, no AVX2 or BMI2), the same way
//! at every start. Started again it crash-loops to systemd's start limit, and
//! while Turbo's state is falcond's, Turbo never comes on.
//!
//! When it dies that way the helper writes [`RECORD`]: the binary's size and
//! modification time, and the processor's x86-64 level. While both still
//! match, that falcond is not started again, and Turbo works without it as it
//! does when falcond is not installed: the Booster applies the general
//! settings, the power profile included. A package update or a rebuild
//! changes the binary, the record no longer matches, and falcond is tried
//! again; reinstalling the same build does not, and it would crash the same.

use std::path::Path;

use serde::{Deserialize, Serialize};

/// Where the helper keeps the record. In its systemd `StateDirectory`,
/// world-readable so the UI reads it too.
pub const RECORD: &str = "/var/lib/bigame-mode/game-backend.incompatible.json";

/// The binary falcond's service runs (`ExecStart`).
pub const BINARY: &str = "/usr/bin/falcond";

/// Which build of a binary: what a package update changes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Build {
    /// Size in bytes.
    pub size: u64,
    /// Modification time, Unix seconds. pacman sets it from the package, so
    /// it is the build's, not the installation's.
    pub modified: u64,
}

impl Build {
    /// The build at `path`, if it can be read.
    #[must_use]
    pub fn of(path: &Path) -> Option<Self> {
        let meta = std::fs::metadata(path).ok()?;
        let modified = meta
            .modified()
            .ok()?
            .duration_since(std::time::UNIX_EPOCH)
            .ok()?
            .as_secs();
        Some(Self {
            size: meta.len(),
            modified,
        })
    }
}

/// A falcond build that died on an illegal instruction on this processor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Incompatible {
    /// The build that crashed.
    pub binary: Build,
    /// The processor's x86-64 level then. The same disk in a newer machine
    /// is not held to an older machine's crash.
    pub cpu_level: Option<u8>,
    /// Unix time it was recorded.
    pub recorded_at: u64,
}

/// Remember that the build at `binary` crashed on this processor.
///
/// # Errors
/// Returns an error if the binary cannot be read or the record cannot be
/// written.
pub fn record_at(record: &Path, binary: &Path, cpu_level: Option<u8>) -> anyhow::Result<()> {
    let build =
        Build::of(binary).ok_or_else(|| anyhow::anyhow!("{} cannot be read", binary.display()))?;
    let entry = Incompatible {
        binary: build,
        cpu_level,
        recorded_at: crate::unix_now(),
    };
    std::fs::write(record, serde_json::to_string_pretty(&entry)?)?;
    Ok(())
}

/// [`record_at`] for falcond on this machine.
///
/// # Errors
/// As [`record_at`].
pub fn record() -> anyhow::Result<()> {
    record_at(
        Path::new(RECORD),
        Path::new(BINARY),
        crate::isa::CpuIsa::detect().level(),
    )
}

/// The record, when it still describes the build at `binary` on a processor
/// of `cpu_level`.
#[must_use]
pub fn matching_at(record: &Path, binary: &Path, cpu_level: Option<u8>) -> Option<Incompatible> {
    let text = std::fs::read_to_string(record).ok()?;
    let entry: Incompatible = serde_json::from_str(&text).ok()?;
    (Build::of(binary).as_ref() == Some(&entry.binary) && entry.cpu_level == cpu_level)
        .then_some(entry)
}

/// Whether the installed falcond is one that crashed on this processor.
#[must_use]
pub fn crashes_here() -> bool {
    matching_at(
        Path::new(RECORD),
        Path::new(BINARY),
        crate::isa::CpuIsa::detect().level(),
    )
    .is_some()
}

/// Forget the record: falcond ran.
pub fn forget() {
    match std::fs::remove_file(RECORD) {
        Ok(()) => tracing::info!("falcond runs on this processor; its crash record is gone"),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => tracing::warn!(error = %e, "falcond's crash record could not be removed"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set_modified(path: &Path, secs: u64) {
        let f = std::fs::File::options().write(true).open(path).unwrap();
        f.set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(secs))
            .unwrap();
    }

    #[test]
    fn the_build_that_crashed_is_recognised_until_it_changes() {
        let dir = tempfile::tempdir().unwrap();
        let record = dir.path().join("incompatible.json");
        let binary = dir.path().join("falcond");
        std::fs::write(&binary, b"x86-64-v3 build").unwrap();
        set_modified(&binary, 1_759_000_000);

        assert!(
            matching_at(&record, &binary, Some(2)).is_none(),
            "no record yet"
        );
        record_at(&record, &binary, Some(2)).unwrap();
        assert!(matching_at(&record, &binary, Some(2)).is_some());

        // Reinstalling the same package puts back the same build.
        std::fs::write(&binary, b"x86-64-v3 build").unwrap();
        set_modified(&binary, 1_759_000_000);
        assert!(matching_at(&record, &binary, Some(2)).is_some());

        // An update: another build, tried again.
        std::fs::write(&binary, b"baseline build!").unwrap();
        set_modified(&binary, 1_759_900_000);
        assert!(matching_at(&record, &binary, Some(2)).is_none());
    }

    #[test]
    fn another_processor_is_not_held_to_the_crash() {
        let dir = tempfile::tempdir().unwrap();
        let record = dir.path().join("incompatible.json");
        let binary = dir.path().join("falcond");
        std::fs::write(&binary, b"x86-64-v3 build").unwrap();
        record_at(&record, &binary, Some(2)).unwrap();
        assert!(matching_at(&record, &binary, Some(3)).is_none());
    }

    #[test]
    fn a_missing_binary_or_a_damaged_record_matches_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let record = dir.path().join("incompatible.json");
        let binary = dir.path().join("falcond");
        assert!(record_at(&record, &binary, Some(2)).is_err());
        std::fs::write(&binary, b"build").unwrap();
        std::fs::write(&record, b"{ not json").unwrap();
        assert!(matching_at(&record, &binary, Some(2)).is_none());
    }
}
