//! Whether AI Graphics is actually working in a running game — from what the
//! game has loaded and what `OptiScaler` wrote, never from what was saved.
//!
//! "Active" is claimed only when `OptiScaler`'s own log, written since this
//! game process started, says it created an upscaler. A configuration that
//! was saved but never loaded is "Configured"; a DLL that is loaded but has
//! not been asked to upscale (the game's upscaler is off) is "Loaded".

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use serde::Serialize;

use super::manifest::Manifest;
use super::optiscaler::{LogFindings, read_log};
use super::transaction::{FileState, verify};

/// How long a game may run before a missing DLL counts as "not detected".
const GRACE: Duration = Duration::from_secs(45);

/// What AI Graphics is doing for a game right now.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case", tag = "state")]
pub enum Status {
    /// BiGame-mode has placed nothing in this game.
    NotInstalled,
    /// Installed and intact; the game is not running.
    Configured,
    /// Installed, but files are missing or were changed since (a game update,
    /// another tool) — Repair can put missing ones back.
    FilesChanged {
        /// Files that are not as placed.
        files: Vec<PathBuf>,
    },
    /// The game has just started; `OptiScaler` has not reported yet.
    Starting,
    /// Loaded into the game, but no upscaler created yet: the game's own
    /// upscaler (the input) is off in its menu.
    Loaded {
        /// `OptiScaler` version from its log.
        version: Option<String>,
    },
    /// Running and upscaling.
    Active {
        /// The backend it created (`fsr31`, `xess`, …).
        upscaler: String,
        /// `OptiScaler` version.
        version: Option<String>,
        /// The FSR 4 decision line, when `OptiScaler` logged one.
        fsr4: Option<String>,
        /// `Some(3)` when the log proves an FSR backend runs FSR 3.1 (it
        /// never proves FSR 4 — see [`LogFindings::fsr_generation`]).
        fsr_generation: Option<u8>,
    },
    /// The game has been running for a while and the DLL is not in it.
    NotDetected,
    /// `OptiScaler` loaded and reported a failure.
    Failed {
        /// Its error lines.
        errors: Vec<String>,
    },
}

/// Paths of the files a process has mapped, from `/proc/<pid>/maps` text.
///
/// The path is everything after the fifth field: game folders often have
/// spaces in their names (`Shadow of the Tomb Raider/dxgi.dll`), so the line
/// cannot simply be split on whitespace.
#[must_use]
pub fn mapped_paths(maps: &str) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = maps
        .lines()
        .filter_map(|line| {
            let mut rest = line;
            for _ in 0..5 {
                let t = rest.trim_start();
                let end = t.find(char::is_whitespace)?;
                rest = &t[end..];
            }
            let p = rest.trim();
            p.starts_with('/').then(|| PathBuf::from(p))
        })
        .collect();
    out.sort();
    out.dedup();
    out
}

/// The status for a game.
///
/// `running` is the game's pid, the install folder's executable directory
/// (absolute) and how long the process has been running. `maps` reads a
/// process's `/proc/<pid>/maps`; `log` reads `OptiScaler.log` if it was
/// written after the process started. Both are passed in so the logic can be
/// tested without a game.
#[must_use]
pub fn status(
    manifest: Option<&Manifest>,
    running: Option<(u32, &Path, Duration)>,
    maps: &dyn Fn(u32) -> Option<String>,
    log: &dyn Fn(&Path, Duration) -> Option<String>,
) -> Status {
    let Some(m) = manifest else {
        return Status::NotInstalled;
    };
    let changed: Vec<PathBuf> = verify(m)
        .into_iter()
        .filter(|(_, s)| *s != FileState::Intact)
        .map(|(p, _)| p)
        .collect();
    let Some((pid, exe_dir, age)) = running else {
        return if changed.is_empty() {
            Status::Configured
        } else {
            Status::FilesChanged { files: changed }
        };
    };
    let proxy = m
        .entries
        .iter()
        .find(|e| {
            e.kind == super::manifest::FileKind::Binary && {
                let name = e
                    .path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_ascii_lowercase());
                name.is_some_and(|n| super::scan::PROXY_SLOTS.contains(&n.as_str()))
            }
        })
        .map(|e| m.install_root.join(&e.path));
    let loaded = proxy
        .as_ref()
        .is_some_and(|p| maps(pid).is_some_and(|text| mapped_paths(&text).iter().any(|q| q == p)));
    let findings: Option<LogFindings> = log(exe_dir, age).map(|t| read_log(&t));
    if let Some(f) = &findings {
        if !f.errors.is_empty() {
            return Status::Failed {
                errors: f.errors.clone(),
            };
        }
        if let Some(up) = f.current_upscaler() {
            return Status::Active {
                upscaler: up.to_owned(),
                version: f.version.clone(),
                fsr4: f.fsr4.clone(),
                fsr_generation: f.fsr_generation(),
            };
        }
    }
    if loaded || findings.as_ref().is_some_and(|f| f.version.is_some()) {
        return Status::Loaded {
            version: findings.and_then(|f| f.version),
        };
    }
    if age < GRACE {
        Status::Starting
    } else {
        Status::NotDetected
    }
}

/// Read `OptiScaler.log` in `exe_dir` if it was written after a process that
/// has been running for `age` started — an older log describes another run.
#[must_use]
pub fn fresh_log(exe_dir: &Path, age: Duration) -> Option<String> {
    let path = exe_dir.join("OptiScaler.log");
    let modified = std::fs::metadata(&path).ok()?.modified().ok()?;
    let started = SystemTime::now().checked_sub(age)?;
    // A little slack: the log's first line can be written in the same second.
    if modified + Duration::from_secs(2) < started {
        return None;
    }
    let mut text = std::fs::read_to_string(&path).ok()?;
    // The log can grow large at trace level; the last megabyte says what is
    // happening now.
    if text.len() > 1 << 20 {
        let cut = text.len() - (1 << 20);
        let cut = (cut..text.len())
            .find(|&i| text.is_char_boundary(i))
            .unwrap_or(cut);
        text = text.split_off(cut);
    }
    Some(text)
}

/// How long `pid` has been running, from `/proc`.
#[must_use]
pub fn process_age(pid: u32) -> Option<Duration> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // Field 22 (starttime) counts after the command name, which is in
    // parentheses and may contain spaces.
    let after = &stat[stat.rfind(')')? + 2..];
    let start_ticks: u64 = after.split_whitespace().nth(19)?.parse().ok()?;
    // SAFETY: sysconf has no preconditions.
    let hz = u64::try_from(unsafe { libc::sysconf(libc::_SC_CLK_TCK) })
        .ok()?
        .max(1);
    let uptime: f64 = std::fs::read_to_string("/proc/uptime")
        .ok()?
        .split_whitespace()
        .next()?
        .parse()
        .ok()?;
    #[allow(clippy::cast_precision_loss)]
    let started = start_ticks as f64 / hz as f64;
    Some(Duration::from_secs_f64((uptime - started).max(0.0)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graphics::manifest::{Entry, FileKind, SCHEMA, Source, State, sha256_bytes};

    fn installed(root: &Path) -> Manifest {
        std::fs::write(root.join("dxgi.dll"), b"ours").unwrap();
        Manifest {
            schema: SCHEMA,
            game_key: "k".into(),
            process: None,
            title: None,
            install_root: root.to_path_buf(),
            source: Source::default(),
            started_at: 0,
            state: State::Installed,
            entries: vec![Entry {
                path: "dxgi.dll".into(),
                sha256: sha256_bytes(b"ours"),
                kind: FileKind::Binary,
                replaced: None,
            }],
            created_dirs: vec![],
            generated: vec![],
            previous: None,
        }
    }

    const NO_MAPS: &dyn Fn(u32) -> Option<String> = &|_| None;
    const NO_LOG: &dyn Fn(&Path, Duration) -> Option<String> = &|_, _| None;

    #[test]
    fn maps_paths_with_spaces_are_read_whole() {
        let maps = "7f00-7f10 r--p 00000000 08:01 1234   /games/Shadow of the Tomb Raider/dxgi.dll\n\
                    7f10-7f20 r-xp 00001000 08:01 1234   /games/Shadow of the Tomb Raider/dxgi.dll\n\
                    7f20-7f30 rw-p 00000000 00:00 0 \n\
                    7f30-7f40 r--p 00000000 00:00 0      [heap]\n";
        assert_eq!(
            mapped_paths(maps),
            [PathBuf::from("/games/Shadow of the Tomb Raider/dxgi.dll")]
        );
    }

    #[test]
    fn not_running_is_configured_or_files_changed() {
        let dir = tempfile::tempdir().unwrap();
        let m = installed(dir.path());
        assert_eq!(status(None, None, NO_MAPS, NO_LOG), Status::NotInstalled);
        assert_eq!(status(Some(&m), None, NO_MAPS, NO_LOG), Status::Configured);
        std::fs::remove_file(dir.path().join("dxgi.dll")).unwrap();
        assert_eq!(
            status(Some(&m), None, NO_MAPS, NO_LOG),
            Status::FilesChanged {
                files: vec!["dxgi.dll".into()]
            }
        );
    }

    #[test]
    fn active_needs_the_log_to_say_an_upscaler_was_created() {
        let dir = tempfile::tempdir().unwrap();
        let m = installed(dir.path());
        let exe = dir.path().to_path_buf();
        let dll = dir.path().join("dxgi.dll").display().to_string();
        let maps = move |_| Some(format!("7f00-7f10 r-xp 0 08:01 1 {dll}\n"));
        let loaded_only =
            |_: &Path, _| Some("[1] [W] OptiScaler v0.9.4-final (x) loaded\n".to_owned());
        assert_eq!(
            status(
                Some(&m),
                Some((1, &exe, Duration::from_secs(90))),
                &maps,
                &loaded_only
            ),
            Status::Loaded {
                version: Some("0.9.4-final".into())
            }
        );
        let working = |_: &Path, _| {
            Some("[1] [W] OptiScaler v0.9.4-final (x) loaded\n[2] [I] f RDNA4: true, RDNA3: false, Fsr4Update: true\n[3] [I] f Creating new fsr31 upscaler\n".to_owned())
        };
        let Status::Active { upscaler, fsr4, .. } = status(
            Some(&m),
            Some((1, &exe, Duration::from_secs(90))),
            &maps,
            &working,
        ) else {
            panic!()
        };
        assert_eq!(upscaler, "fsr31");
        assert!(fsr4.unwrap().contains("Fsr4Update: true"));
    }

    #[test]
    fn a_failure_line_wins_and_absence_after_the_grace_period_is_not_detected() {
        let dir = tempfile::tempdir().unwrap();
        let m = installed(dir.path());
        let exe = dir.path().to_path_buf();
        let failed = |_: &Path, _| {
            Some("[1] [E] f can't load amd_fidelityfx_dx12.dll methods!\n[2] [I] f Creating new fsr31 upscaler\n".to_owned())
        };
        assert!(matches!(
            status(
                Some(&m),
                Some((1, &exe, Duration::from_secs(90))),
                NO_MAPS,
                &failed
            ),
            Status::Failed { .. }
        ));
        assert_eq!(
            status(
                Some(&m),
                Some((1, &exe, Duration::from_secs(5))),
                NO_MAPS,
                NO_LOG
            ),
            Status::Starting
        );
        assert_eq!(
            status(
                Some(&m),
                Some((1, &exe, Duration::from_secs(90))),
                NO_MAPS,
                NO_LOG
            ),
            Status::NotDetected
        );
    }

    #[test]
    fn a_log_from_an_earlier_run_is_not_read_as_this_one() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("OptiScaler.log"), "old").unwrap();
        // A process that started "now" has not written this log.
        let t = std::fs::File::options()
            .write(true)
            .open(dir.path().join("OptiScaler.log"))
            .unwrap();
        t.set_modified(SystemTime::now() - Duration::from_secs(3600))
            .unwrap();
        assert_eq!(fresh_log(dir.path(), Duration::from_secs(10)), None);
        assert_eq!(
            fresh_log(dir.path(), Duration::from_secs(7200)).as_deref(),
            Some("old")
        );
    }

    #[test]
    fn this_process_has_an_age() {
        let age = process_age(std::process::id()).unwrap();
        assert!(age < Duration::from_secs(24 * 3600));
    }
}
