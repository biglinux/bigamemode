//! Background load that competes with a game.
//!
//! This module **observes and reports**. It does not renice, suspend or kill
//! anything, and that is a decision rather than an omission.
//!
//! Deprioritising background tasks during a match is mechanically easy:
//! lowering the niceness of a process you own needs no privileges and is
//! trivially reversible. What is hard is deciding *which*
//! process, and being right. A compile that the user is deliberately running
//! overnight, a video export they are waiting on, a browser playing the music
//! they are listening to — each looks exactly like "background load" from
//! `/proc`, and silently slowing any of them down is a worse outcome than a few
//! lost frames.
//!
//! So this reports what is competing and says what it is, and the user decides.
//! An automatic version would need to be opt-in per application, and would need
//! the same snapshot-and-restore discipline as every other change this project
//! makes — which is exactly why it is not bolted on here.
//!
//! Only processes owned by the current user are considered. Another user's
//! work is not ours to describe, and system daemons are not something a gaming
//! utility should be editorialising about.

use std::collections::HashMap;
use std::time::Duration;

/// What kind of work a process is doing, when it can be recognised.
///
/// Used only to explain the entry to the user — a category is never a licence
/// to act on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Indexes files in the background; typically safe to pause.
    Indexer,
    /// Compiling or building.
    Compiler,
    /// Backup or file synchronisation.
    Sync,
    /// Web browser.
    Browser,
    /// Virtual machine or container runtime.
    Virtualisation,
    /// Media encoding or transcoding.
    Media,
    /// Another game, or a game launcher.
    Gaming,
    /// Recognised as nothing in particular.
    Other,
}

impl Kind {
    /// One line explaining what this is, for someone deciding what to close.
    #[must_use]
    pub fn describe(self) -> &'static str {
        match self {
            Self::Indexer => "Indexes files in the background. Usually safe to pause.",
            Self::Compiler => "A build or compile. Will finish sooner if left alone.",
            Self::Sync => "Backup or file sync. Usually safe to pause.",
            Self::Browser => "A web browser. Tabs playing video or running scripts cost the most.",
            Self::Virtualisation => {
                "A virtual machine or container. Closing it may interrupt work."
            }
            Self::Media => "Encoding or transcoding media.",
            Self::Gaming => "Another game or a game launcher.",
            Self::Other => "Unrecognised.",
        }
    }
}

/// Process-name fragments that identify a category.
const SIGNATURES: &[(&str, Kind)] = &[
    ("baloo", Kind::Indexer),
    ("tracker-", Kind::Indexer),
    ("updatedb", Kind::Indexer),
    ("mlocate", Kind::Indexer),
    ("cargo", Kind::Compiler),
    ("rustc", Kind::Compiler),
    ("cc1", Kind::Compiler),
    ("gcc", Kind::Compiler),
    ("clang", Kind::Compiler),
    ("make", Kind::Compiler),
    ("ninja", Kind::Compiler),
    ("javac", Kind::Compiler),
    ("gradle", Kind::Compiler),
    ("rsync", Kind::Sync),
    ("borg", Kind::Sync),
    ("restic", Kind::Sync),
    ("syncthing", Kind::Sync),
    ("dropbox", Kind::Sync),
    ("nextcloud", Kind::Sync),
    ("insync", Kind::Sync),
    ("timeshift", Kind::Sync),
    ("firefox", Kind::Browser),
    ("chrome", Kind::Browser),
    ("chromium", Kind::Browser),
    ("brave", Kind::Browser),
    ("vivaldi", Kind::Browser),
    ("opera", Kind::Browser),
    ("qemu", Kind::Virtualisation),
    ("virtualbox", Kind::Virtualisation),
    ("vboxheadless", Kind::Virtualisation),
    ("dockerd", Kind::Virtualisation),
    ("containerd", Kind::Virtualisation),
    ("podman", Kind::Virtualisation),
    ("ffmpeg", Kind::Media),
    ("handbrake", Kind::Media),
    ("obs", Kind::Media),
    ("kdenlive", Kind::Media),
    ("steam", Kind::Gaming),
    ("lutris", Kind::Gaming),
    ("heroic", Kind::Gaming),
    ("wine", Kind::Gaming),
];

/// Recognise a process by name.
#[must_use]
pub fn classify(comm: &str) -> Kind {
    let lower = comm.to_ascii_lowercase();
    SIGNATURES
        .iter()
        .find(|(needle, _)| lower.contains(needle))
        .map_or(Kind::Other, |(_, kind)| *kind)
}

/// A process using enough CPU to matter.
#[derive(Debug, Clone, PartialEq)]
pub struct BusyProcess {
    /// Process id.
    pub pid: u32,
    /// Name from `/proc/<pid>/comm`.
    pub name: String,
    /// Share of one CPU, as a percentage. 200 means two cores fully used.
    pub cpu_percent: f64,
    /// Resident memory, in mebibytes.
    pub memory_mib: u64,
    /// What kind of work this looks like.
    pub kind: Kind,
}

/// How much of one CPU a process must use before it is worth mentioning.
///
/// Low enough to catch a steady background task, high enough that an idle
/// desktop reports nothing. A list that always has entries teaches people to
/// ignore it.
pub const BUSY_THRESHOLD_PERCENT: f64 = 5.0;

/// Sample CPU usage over `window` and report the current user's busy processes.
///
/// Two passes are needed because `/proc/<pid>/stat` reports cumulative CPU
/// time; a single reading gives the average since the process started, which
/// for a long-lived browser says nothing about what it is doing now.
#[must_use]
pub fn busy_processes(window: Duration) -> Vec<BusyProcess> {
    let uid = current_uid();
    let first = sample_cpu_times(uid);
    if first.is_empty() {
        return Vec::new();
    }
    std::thread::sleep(window);
    let second = sample_cpu_times(uid);

    let ticks_per_second = clock_ticks();
    let elapsed = window.as_secs_f64();
    if elapsed <= 0.0 || ticks_per_second <= 0.0 {
        return Vec::new();
    }

    let mut out: Vec<BusyProcess> = second
        .into_iter()
        .filter_map(|(pid, after)| {
            let before = first.get(&pid)?;
            let delta = after.ticks.saturating_sub(before.ticks);
            #[allow(clippy::cast_precision_loss)]
            let percent = (delta as f64 / ticks_per_second) / elapsed * 100.0;
            (percent >= BUSY_THRESHOLD_PERCENT).then(|| BusyProcess {
                pid,
                kind: classify(&after.name),
                name: after.name,
                cpu_percent: percent,
                memory_mib: after.rss_bytes / 1_048_576,
            })
        })
        .collect();

    out.sort_by(|a, b| b.cpu_percent.total_cmp(&a.cpu_percent));
    out.truncate(12);
    out
}

struct Sample {
    name: String,
    ticks: u64,
    rss_bytes: u64,
}

fn sample_cpu_times(uid: u32) -> HashMap<u32, Sample> {
    let mut out = HashMap::new();
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return out;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(pid) = name.to_string_lossy().parse::<u32>().ok() else {
            continue;
        };
        // Another user's work is not ours to describe.
        if process_uid(&entry.path()) != Some(uid) {
            continue;
        }
        let Some(sample) = read_sample(&entry.path()) else {
            continue;
        };
        out.insert(pid, sample);
    }
    out
}

fn process_uid(proc_dir: &std::path::Path) -> Option<u32> {
    let status = std::fs::read_to_string(proc_dir.join("status")).ok()?;
    status.lines().find_map(|line| {
        let rest = line.strip_prefix("Uid:")?;
        rest.split_whitespace().next()?.parse().ok()
    })
}

fn read_sample(proc_dir: &std::path::Path) -> Option<Sample> {
    let stat = std::fs::read_to_string(proc_dir.join("stat")).ok()?;
    // The comm field is parenthesised and may itself contain spaces and
    // parentheses, so fields are counted from after the final ')'.
    let close = stat.rfind(')')?;
    let fields: Vec<&str> = stat[close + 1..].split_whitespace().collect();
    // Indices count from the state field (proc(5) field 3): utime (field 14)
    // is 11, stime (15) is 12, rss (24) is 21.
    let utime: u64 = fields.get(11)?.parse().ok()?;
    let stime: u64 = fields.get(12)?.parse().ok()?;
    let rss_pages: u64 = fields.get(21)?.parse().ok()?;

    let name = std::fs::read_to_string(proc_dir.join("comm"))
        .ok()?
        .trim()
        .to_owned();
    if name.is_empty() {
        return None;
    }

    Some(Sample {
        name,
        ticks: utime + stime,
        rss_bytes: rss_pages.saturating_mul(page_size()),
    })
}

fn clock_ticks() -> f64 {
    // SAFETY: sysconf only reads a static configuration value.
    let ticks = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
    if ticks > 0 {
        #[allow(clippy::cast_precision_loss)]
        let t = ticks as f64;
        t
    } else {
        100.0
    }
}

fn page_size() -> u64 {
    // SAFETY: sysconf only reads a static configuration value.
    let size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    u64::try_from(size).unwrap_or(4096)
}

fn current_uid() -> u32 {
    // SAFETY: getuid cannot fail and takes no arguments.
    unsafe { libc::getuid() }
}

/// Whether process `pid` has `key` in its environment. Only the name is
/// compared; no value is kept.
#[must_use]
pub fn env_has_key(pid: u32, key: &str) -> bool {
    let Ok(bytes) = std::fs::read(format!("/proc/{pid}/environ")) else {
        return false;
    };
    let prefix = format!("{key}=");
    bytes
        .split(|b| *b == 0)
        .any(|entry| entry.starts_with(prefix.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_background_work_is_recognised() {
        assert_eq!(classify("baloo_file"), Kind::Indexer);
        assert_eq!(classify("tracker-miner-fs-3"), Kind::Indexer);
        assert_eq!(classify("cargo"), Kind::Compiler);
        assert_eq!(classify("rustc"), Kind::Compiler);
        assert_eq!(classify("syncthing"), Kind::Sync);
        assert_eq!(classify("firefox"), Kind::Browser);
        assert_eq!(classify("chrome"), Kind::Browser);
        assert_eq!(classify("qemu-system-x86_64"), Kind::Virtualisation);
        assert_eq!(classify("ffmpeg"), Kind::Media);
        assert_eq!(classify("steam"), Kind::Gaming);
    }

    #[test]
    fn matching_is_case_insensitive() {
        assert_eq!(classify("Firefox"), Kind::Browser);
        assert_eq!(classify("HandBrakeCLI"), Kind::Media);
    }

    #[test]
    fn an_unknown_process_is_not_guessed_at() {
        assert_eq!(classify("my-own-program"), Kind::Other);
        assert_eq!(classify(""), Kind::Other);
    }

    #[test]
    fn every_kind_explains_itself() {
        for kind in [
            Kind::Indexer,
            Kind::Compiler,
            Kind::Sync,
            Kind::Browser,
            Kind::Virtualisation,
            Kind::Media,
            Kind::Gaming,
            Kind::Other,
        ] {
            assert!(!kind.describe().is_empty());
        }
    }

    #[test]
    fn sampling_this_machine_reports_only_plausible_entries() {
        // A short window keeps the test quick; it is long enough that a busy
        // process registers and an idle one does not.
        let busy = busy_processes(Duration::from_millis(400));
        assert!(busy.len() <= 12);
        for process in &busy {
            assert!(process.pid > 0);
            assert!(!process.name.is_empty());
            assert!(
                process.cpu_percent >= BUSY_THRESHOLD_PERCENT,
                "{} reported below the threshold",
                process.name
            );
            // A single process cannot use more than every core.
            let cores = f64::from(u32::try_from(num_cpus()).unwrap_or(1));
            assert!(
                process.cpu_percent <= cores * 100.0 + 50.0,
                "{} reported {}%, which exceeds this machine",
                process.name,
                process.cpu_percent
            );
        }
        // Sorted by CPU, busiest first.
        for pair in busy.windows(2) {
            assert!(pair[0].cpu_percent >= pair[1].cpu_percent);
        }
    }

    fn num_cpus() -> usize {
        std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get)
    }

    #[test]
    fn only_the_current_users_processes_are_considered() {
        // PID 1 is root-owned; it must never appear for a non-root user.
        if current_uid() == 0 {
            return;
        }
        let busy = busy_processes(Duration::from_millis(300));
        assert!(!busy.iter().any(|p| p.pid == 1));
    }
}
