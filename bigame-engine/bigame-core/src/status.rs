//! Parse falcond's status file.
//!
//! The file uses a simple `KEY: VALUE` / `  KEY: VALUE` format with section
//! headers like `FEATURES:`, `CONFIG:`, `CURRENT_STATUS:`.
//!
//! # Where it lives, and why that matters
//!
//! falcond 2.0.2 hardcodes `/tmp/falcond_status` — the path is the only one in
//! its binary, and the `status_dir` setting some documentation mentions is not
//! implemented in this release. `/run/falcond`, which would be the correct
//! location, does not exist.
//!
//! `/tmp` is world-writable. falcond runs as root and creates the file, but if
//! it has not started yet any local user can create `/tmp/falcond_status`
//! first, or replace it with a symlink pointing somewhere else. Nothing here
//! writes the file, so there is no privilege escalation — but a spoofed status
//! would make the UI report a scheduler, a V-Cache mode and an active profile
//! that are not real, which is exactly the kind of confident falsehood this
//! project exists to stop.
//!
//! [`read`] therefore accepts the file only when it is a regular file owned by
//! root. Newer falcond releases (checked against upstream 2.0.14) also write
//! `/var/lib/falcond/status`, in a root-owned directory; that one is preferred
//! whenever it exists.

use std::collections::HashMap;
use std::os::unix::fs::MetadataExt;
use std::path::Path;

/// Where newer falcond releases publish, in a directory only root can write.
pub const STATE_STATUS_PATH: &str = "/var/lib/falcond/status";

/// Where falcond 2.x actually writes, in world-writable `/tmp`.
pub const STATUS_PATH: &str = "/tmp/falcond_status";

/// The status file to read, preferring falcond's state directory over `/tmp`.
#[must_use]
pub fn status_path() -> &'static Path {
    let state = Path::new(STATE_STATUS_PATH);
    if state.exists() {
        state
    } else {
        Path::new(STATUS_PATH)
    }
}

/// Whether `path` is a regular file owned by root.
///
/// `symlink_metadata` deliberately does not follow links: a symlink planted in
/// `/tmp` would otherwise report the metadata of whatever it points at.
#[must_use]
pub fn is_trustworthy(path: &Path) -> bool {
    match std::fs::symlink_metadata(path) {
        Ok(meta) => meta.is_file() && meta.uid() == 0,
        Err(_) => false,
    }
}

/// Parsed falcond daemon status.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FalcondStatus {
    /// Whether performance mode switching is available.
    pub performance_available: bool,
    /// Profile mode (none / handheld / htpc).
    pub profile_mode: String,
    /// Global `VCache` mode from config.
    pub config_vcache: String,
    /// Global SCX scheduler from config.
    pub config_scx: String,
    /// Number of loaded profiles.
    pub loaded_profiles: u32,
    /// Currently active profile name (if any).
    pub active_profile: Option<String>,
    /// Live: performance mode active/inactive.
    pub perf_mode_active: bool,
    /// Live: current `VCache` mode.
    pub current_vcache: String,
    /// Live: current SCX scheduler.
    pub current_scx: String,
    /// Live: screensaver inhibit active.
    pub screensaver_inhibited: bool,
    /// Whether falcond can protect a game's VRAM through the DMEM cgroup.
    /// `None` when this falcond does not report it — releases before DMEM
    /// support — which is different from "reported unavailable".
    pub dmem_cgroup: Option<bool>,
    /// The sched-ext schedulers falcond can switch to (`scx_lavd`, …), as it
    /// lists them itself.
    pub available_scx: Vec<String>,
}

/// Read and parse falcond's status.
///
/// Returns `None` when the file is absent, unreadable, or fails the ownership
/// check described in the module documentation.
#[must_use]
pub fn read() -> Option<FalcondStatus> {
    read_from(status_path())
}

/// Read and parse a specific status file, after checking it can be trusted.
///
/// Returns `None` rather than an error: from the caller's point of view "no
/// status" and "status we will not believe" lead to the same behaviour, and
/// treating a spoofed file as absent is the safe reading.
#[must_use]
pub fn read_from(path: &Path) -> Option<FalcondStatus> {
    let content = read_trusted(path);
    if content.is_none() && path.exists() {
        tracing::warn!(
            target: "security",
            path = %path.display(),
            "ignoring falcond status: not a root-owned regular file"
        );
    }
    content.as_deref().map(parse)
}

/// The status file's text, if it is a root-owned regular file.
///
/// The check is made on the descriptor that is then read (`fstat`), so the
/// file cannot be swapped between check and read. The open neither follows a
/// symlink nor blocks: in `/tmp`, before falcond has created it, the name can
/// be anyone's, including a FIFO that would hang a blocking open for ever.
/// At most [`crate::watch::MAX_WATCHED_BYTES`] are read.
#[must_use]
pub fn read_trusted(path: &Path) -> Option<String> {
    use std::io::Read;
    use std::os::unix::fs::OpenOptionsExt;
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .ok()?;
    let meta = file.metadata().ok()?;
    if !meta.is_file() || meta.uid() != 0 {
        return None;
    }
    let mut content = String::new();
    file.take(crate::watch::MAX_WATCHED_BYTES)
        .read_to_string(&mut content)
        .ok()?;
    Some(content)
}

/// Parse status file content into structured data.
#[must_use]
pub fn parse(content: &str) -> FalcondStatus {
    let mut status = FalcondStatus::default();
    let mut section = "";

    // Pre-parse key-value pairs per section
    let mut kv: HashMap<(&str, &str), &str> = HashMap::new();

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        // Section header: "SECTION_NAME:"
        if !trimmed.starts_with(' ') && trimmed.ends_with(':') && !trimmed.contains(": ") {
            section = trimmed.trim_end_matches(':');
            continue;
        }

        // Top-level key: "KEY: VALUE"
        if !line.starts_with(' ') {
            if let Some((key, val)) = trimmed.split_once(": ") {
                kv.insert(("", key), val);
            }
            continue;
        }

        // Indented list item: "  - scx_lavd"
        if let Some(item) = trimmed.strip_prefix("- ") {
            if section == "AVAILABLE_SCX_SCHEDULERS" {
                status.available_scx.push(item.trim().to_owned());
            }
            continue;
        }

        // Indented key-value: "  Key: Value"
        if let Some((key, val)) = trimmed.split_once(": ") {
            kv.insert((section, key), val);
        }
    }

    // Map parsed values to struct fields
    if let Some(&v) = kv.get(&("FEATURES", "Performance Mode")) {
        status.performance_available = v == "Available";
    }
    if let Some(&v) = kv.get(&("FEATURES", "DMEM Cgroup")) {
        status.dmem_cgroup = Some(v == "Available");
    }
    if let Some(&v) = kv.get(&("CONFIG", "Profile Mode")) {
        v.clone_into(&mut status.profile_mode);
    }
    if let Some(&v) = kv.get(&("CONFIG", "Global VCache Mode")) {
        v.clone_into(&mut status.config_vcache);
    }
    if let Some(&v) = kv.get(&("CONFIG", "Global SCX Scheduler")) {
        v.clone_into(&mut status.config_scx);
    }
    if let Some(&v) = kv.get(&("", "LOADED_PROFILES")) {
        status.loaded_profiles = v.parse().unwrap_or(0);
    }
    if let Some(&v) = kv.get(&("", "ACTIVE_PROFILE")) {
        status.active_profile = if v == "None" {
            None
        } else {
            Some(v.to_owned())
        };
    }
    if let Some(&v) = kv.get(&("CURRENT_STATUS", "Performance Mode")) {
        status.perf_mode_active = v == "Active";
    }
    if let Some(&v) = kv.get(&("CURRENT_STATUS", "VCache Mode")) {
        v.clone_into(&mut status.current_vcache);
    }
    if let Some(&v) = kv.get(&("CURRENT_STATUS", "SCX Scheduler")) {
        v.clone_into(&mut status.current_scx);
    }
    if let Some(&v) = kv.get(&("CURRENT_STATUS", "Screensaver Inhibit")) {
        status.screensaver_inhibited = v == "Active";
    }

    status
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fifo_or_a_users_file_under_the_name_is_refused_without_blocking() {
        let dir = crate::tests::tempdir("status-trust");
        let fifo = dir.join("falcond_status");
        let name = std::ffi::CString::new(fifo.as_os_str().as_encoded_bytes()).unwrap();
        // SAFETY: mkfifo only creates the node named by a valid C string.
        assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o644) }, 0);
        assert_eq!(read_trusted(&fifo), None);
        let own = dir.join("owned_by_me");
        std::fs::write(&own, "CURRENT_STATUS:\n").unwrap();
        assert_eq!(read_trusted(&own), None, "not root-owned");
    }

    #[test]
    fn dmem_support_is_read_from_a_newer_falcond_and_unknown_from_an_older_one() {
        // falcond 2.0.14's documented status output.
        let newer = "FEATURES:\n  Performance Mode: Available\n  DMEM Cgroup: Available\n\nCONFIG:\n  Profile Mode: none\n";
        assert_eq!(parse(newer).dmem_cgroup, Some(true));
        // falcond 2.0.2 has no such line.
        let older =
            "FEATURES:\n  Performance Mode: Available\n\nCONFIG:\n  Profile Mode: handheld\n";
        assert_eq!(parse(older).dmem_cgroup, None);
    }

    #[test]
    fn parse_full_status() {
        let input = "\
FEATURES:
  Performance Mode: Available

CONFIG:
  Profile Mode: none
  Global VCache Mode: cache
  Global SCX Scheduler: bpfland

AVAILABLE_SCX_SCHEDULERS:
  - scx_bpfland
  - scx_lavd

LOADED_PROFILES: 5

ACTIVE_PROFILE: Cyberpunk2077.exe

QUEUED_PROFILES:
  (None)

RESTORE_STATE:
  SCX Scheduler: none (Mode: default)
  Power Profile: balanced

CURRENT_STATUS:
  Performance Mode: Active
  VCache Mode: cache
  SCX Scheduler: bpfland
  Screensaver Inhibit: Active
";
        let s = parse(input);
        assert!(s.performance_available);
        assert_eq!(s.profile_mode, "none");
        assert_eq!(s.config_vcache, "cache");
        assert_eq!(s.config_scx, "bpfland");
        assert_eq!(s.loaded_profiles, 5);
        assert_eq!(s.active_profile.as_deref(), Some("Cyberpunk2077.exe"));
        assert_eq!(s.available_scx, ["scx_bpfland", "scx_lavd"]);
        assert!(s.perf_mode_active);
        assert_eq!(s.current_vcache, "cache");
        assert_eq!(s.current_scx, "bpfland");
        assert!(s.screensaver_inhibited);
    }

    #[test]
    fn a_symlink_is_never_trusted() {
        // /tmp is world-writable: a symlink planted there would otherwise let
        // any local user choose what the UI reports as falcond's state.
        let dir = std::env::temp_dir().join(format!(
            "bigame_status_{}_{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let real = dir.join("real");
        std::fs::write(&real, "FEATURES:\n  Performance Mode: Available\n").unwrap();
        let link = dir.join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();

        assert!(!is_trustworthy(&link), "a symlink must not be trusted");
        assert_eq!(read_from(&link), None);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_file_owned_by_the_user_is_not_trusted() {
        // falcond runs as root; anything else wrote this.
        let dir = std::env::temp_dir().join(format!(
            "bigame_status_own_{}_{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("status");
        std::fs::write(&file, "ACTIVE_PROFILE: Cyberpunk2077.exe\n").unwrap();

        // Running as root in CI would legitimately own it; only assert the
        // non-root case, which is how the application actually runs.
        if unsafe { libc::geteuid() } != 0 {
            assert!(!is_trustworthy(&file));
            assert_eq!(read_from(&file), None);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_real_falcond_status_is_trusted_when_present() {
        // When falcond is running it owns /tmp/falcond_status as root, and
        // that file must be trusted.
        let path = Path::new(STATUS_PATH);
        if path.exists() {
            assert!(
                is_trustworthy(path),
                "falcond's own status should be trusted"
            );
            assert!(read().is_some());
        }
    }

    #[test]
    fn a_missing_file_is_simply_absent() {
        assert!(!is_trustworthy(Path::new("/nonexistent/falcond_status")));
        assert_eq!(read_from(Path::new("/nonexistent/falcond_status")), None);
    }

    #[test]
    fn parse_inactive_status() {
        let input = "\
FEATURES:
  Performance Mode: Unavailable

CONFIG:
  Profile Mode: handheld
  Global VCache Mode: none
  Global SCX Scheduler: none

LOADED_PROFILES: 0

ACTIVE_PROFILE: None
";
        let s = parse(input);
        assert!(!s.performance_available);
        assert_eq!(s.profile_mode, "handheld");
        assert_eq!(s.active_profile, None);
        assert_eq!(s.loaded_profiles, 0);
        assert!(!s.perf_mode_active);
    }
}
