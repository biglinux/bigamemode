//! Background load that competes with a game — and, when the user asks,
//! closing or pausing one of those programs.
//!
//! Nothing here acts on its own. Deprioritising background tasks during a
//! match is mechanically easy; what is hard is deciding *which* process, and
//! being right. A compile that the user is deliberately running overnight, a
//! video export they are waiting on, a browser playing the music they are
//! listening to — each looks exactly like "background load" from `/proc`, and
//! silently slowing any of them down is a worse outcome than a few lost
//! frames. So this reports what is competing and says what it is, and the
//! user decides, one program at a time, after a confirmation.
//!
//! What the user may act on is decided here, by one pure rule
//! ([`protection`]), and it is deliberately narrow: only a program of the
//! user's own that is neither part of the system, the desktop session, the
//! running game nor Big Game Mode. Everything else is shown without a button.
//! The rule is checked again at the moment of acting, against a process
//! pinned by a pidfd, so a pid reused in between is never signalled.
//!
//! The actions are the gentlest ones that do the job, and nothing is
//! escalated:
//!
//! - **Close** sends `SIGTERM` — the request a window's close button or a
//!   logout makes. A program may save, ask, or ignore it; Big Game Mode never
//!   follows up with `SIGKILL`.
//! - **Pause** sends `SIGSTOP`, **Resume** `SIGCONT`. What Big Game Mode paused
//!   is remembered in `$XDG_STATE_HOME/bigame-mode/paused.json`, resumed
//!   when Big Game Mode quits, and resumed at its next start if it was killed
//!   in between — a program is never left frozen by a crash.
//!
//! Only processes owned by the current user are considered. Another user's
//! work is not ours to describe, and no privileged action exists here at all.

use std::collections::{HashMap, HashSet};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::error::UserError;
use crate::running::GameIdentity;
use crate::text::N_;

/// What kind of work a process is doing, when it can be recognised.
///
/// Used to explain the entry to the user. A category is never a licence to
/// act on it: whether a process may be closed or paused is [`protection`]'s
/// decision alone.
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
    /// Part of the desktop session: compositor, shell, audio, portals.
    Desktop,
    /// Big Game Mode itself.
    ThisApp,
    /// Recognised as nothing in particular.
    Other,
}

impl Kind {
    /// One line explaining what this is, for someone deciding what to close;
    /// marked for translation.
    #[must_use]
    pub fn describe(self) -> &'static str {
        match self {
            Self::Indexer => N_("Indexes files in the background. Usually safe to pause."),
            Self::Compiler => N_("A build or compile. Will finish sooner if left alone."),
            Self::Sync => N_("Backup or file sync. Usually safe to pause."),
            Self::Browser => {
                N_("A web browser. Tabs playing video or running scripts cost the most.")
            }
            Self::Virtualisation => {
                N_("A virtual machine or container. Closing it may interrupt work.")
            }
            Self::Media => N_("Encoding or transcoding media."),
            Self::Gaming => N_("Another game or a game launcher."),
            Self::Desktop => {
                N_("Part of your desktop session. Big Game Mode will not close or pause it.")
            }
            Self::ThisApp => N_(
                "Big Game Mode itself. Details reads the system every second while it is on screen; minimised, it costs almost nothing.",
            ),
            Self::Other => N_("Unrecognised."),
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

/// Programs a desktop session is made of, by `comm` (lowercased; at most 15
/// characters, so long names appear cut). `name*` matches a prefix, `*name*`
/// anywhere in the name, anything else exactly.
///
/// Closing any of these ends or breaks the session — the compositor takes
/// every window with it, the audio server every sound — and pausing one
/// freezes the desktop until it is resumed, which the user may then have no
/// way to do. The list errs wide: a program missing from it is still refused
/// when systemd placed it among the session's services ([`cgroup_place`]).
const DESKTOP: &[&str] = &[
    // Service managers, the session bus and what hangs off it.
    "systemd*",
    "(sd-pam)",
    "dbus*",
    "at-spi*",
    "gvfs*",
    "dconf-service",
    "xdg-*",
    "flatpak-*",
    "bwrap",
    "p11-kit-*",
    // Keys, secrets and authentication agents.
    "*polkit*",
    "gpg-agent",
    "ssh-agent",
    "scdaemon",
    "pinentry*",
    "gnome-keyring*",
    "kwalletd*",
    "ksecretd",
    // Display servers and compositors.
    "x",
    "xorg",
    "xwayland",
    "kwin*",
    "mutter*",
    "gnome-shell*",
    "gamescope*",
    "sway*",
    "hypr*",
    "labwc",
    "wayfire",
    "river",
    "niri",
    "picom",
    "xfwm4",
    "muffin",
    "marco",
    "openbox",
    // KDE Plasma.
    "plasma*",
    "startplasma*",
    "ksmserver",
    "kded*",
    "kglobalaccel*",
    "kactivitymanage*",
    "kscreen*",
    "krunner",
    "baloorunner",
    "kaccess",
    "kiod*",
    "kio-fuse",
    "xembedsniproxy",
    "gmenudbusmenupr*",
    "org_kde_powerde*",
    "powerdevil",
    "xsettingsd",
    // GNOME, Xfce, Cinnamon, MATE, LXQt, Budgie, COSMIC.
    "gnome-session*",
    "gsd-*",
    "goa-*",
    "evolution-*",
    "xfce4-*",
    "xfdesktop",
    "xfsettingsd",
    "cinnamon*",
    "csd-*",
    "mate-*",
    "lxqt-*",
    "lxsession",
    "budgie-*",
    "cosmic-*",
    // Panels, notifications, input methods, screen lockers.
    "waybar",
    "mako",
    "dunst",
    "swaync*",
    "ibus*",
    "fcitx*",
    "kscreenlocker*",
    "xscreensaver*",
    "light-locker",
    // Sound: pausing any of these silences everything, the game included.
    "pipewire*",
    "wireplumber",
    "pulseaudio",
    "jackd*",
    "jackdbus",
    "easyeffects",
    "jamesdsp",
    "pulseeffects",
    "speech-dispatch*",
    "sd_*",
    "orca",
    // Network and devices.
    "networkmanager",
    "nm-*",
    "wpa_supplicant",
    "iwd",
    "bluetoothd",
    "obexd",
    // Logging in and gaining privileges.
    "sddm*",
    "gdm*",
    "lightdm*",
    "login",
    "agetty",
    "sudo",
    "su",
    "pkexec",
    "doas",
    "run0",
];

/// Big Game Mode, what it drives while a game runs, and the shells that may be
/// hosting either. Never offered, whoever owns them.
const ECOSYSTEM: &[&str] = &[
    "bigame*",
    "falcond",
    "gamemoded",
    "mangoapp",
    "scx_*",
    "power-profiles-*",
    "tuned*",
    // Steam's launch plumbing: the reaper, the Steam Linux Runtime container
    // and its logger. Closing Steam is the way to stop them.
    "reaper",
    "pv-*",
    "srt-*",
    "pressure-vessel*",
    "steam-runtime-*",
    // A busy shell is running a script whose parent may be a terminal the
    // user is typing in, or Big Game Mode's own launch wrapper.
    "bash",
    "sh",
    "zsh",
    "fish",
    "dash",
    "ksh",
    "tcsh",
    "csh",
    "nu",
];

/// Whether `name` (lowercased) matches one of `patterns`.
fn listed(patterns: &[&str], name: &str) -> bool {
    patterns.iter().any(|p| {
        if let Some(inner) = p.strip_prefix('*').and_then(|p| p.strip_suffix('*')) {
            name.contains(inner)
        } else if let Some(prefix) = p.strip_suffix('*') {
            name.starts_with(prefix)
        } else {
            name == *p
        }
    })
}

/// Recognise a process by name.
#[must_use]
pub fn classify(comm: &str) -> Kind {
    let lower = comm.to_ascii_lowercase();
    if lower == "bigame-ui" {
        return Kind::ThisApp;
    }
    if listed(DESKTOP, &lower) {
        return Kind::Desktop;
    }
    SIGNATURES
        .iter()
        .find(|(needle, _)| lower.contains(needle))
        .map_or(Kind::Other, |(_, kind)| *kind)
}

// ── What may be touched ─────────────────────────────────────────────────────

/// What the rule needs to know about one process, from `/proc`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Facts {
    /// Process id.
    pub pid: u32,
    /// Parent process id.
    pub ppid: u32,
    /// Real, effective, saved and filesystem user ids (`Uid:` in `status`).
    pub uids: Vec<u32>,
    /// `/proc/<pid>/comm`.
    pub name: String,
    /// The basename of `argv[0]`; empty for a kernel thread.
    pub argv0: String,
    /// The state letter from `stat` (`R`, `S`, `T`, `Z`, …).
    pub state: char,
    /// The kernel's `PF_KTHREAD` flag.
    pub kernel_thread: bool,
    /// The cgroup v2 path (the `0::` line of `/proc/<pid>/cgroup`).
    pub cgroup: String,
    /// When it started, in clock ticks since boot. With the pid, this is the
    /// process's identity: a pid can be reused, the pair cannot.
    pub start_ticks: u64,
}

/// Why a process gets no Close or Pause button.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Protection {
    /// Owned by another user, root included — or running with another
    /// user's privileges.
    NotYours,
    /// A kernel thread, the init process, or something with no command line.
    Kernel,
    /// Already exited.
    Exited,
    /// Big Game Mode, a program it drives, or a shell.
    ThisApp,
    /// Started Big Game Mode or was started by it, or belongs to the running
    /// game: its tree, what launched it, what it launched.
    Related,
    /// A launcher or runtime a game depends on, while a game runs.
    GameMachinery,
    /// A part of the desktop session, by name.
    Desktop,
    /// A service of the user's session (systemd placed it there).
    SessionService,
    /// A system service or container that happens to run as this user.
    SystemService,
}

/// Where systemd placed a process, from its cgroup path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Place {
    /// A program the user launched: `app.slice/app-*`.
    App,
    /// A background service of the session (`background.slice`): file
    /// indexers, but also FUSE mounts and Plasma's helpers, which other
    /// programs wait on.
    Background,
    /// A service of the session: `session.slice`, the user manager itself,
    /// D-Bus-activated services and portals.
    Session,
    /// Outside the user's manager: system services, containers, VMs.
    System,
    /// No systemd layout to go by (a login-session scope, cgroup v1, a system
    /// without systemd); the name rules decide alone.
    Unknown,
}

/// Where systemd placed a process with cgroup `path`.
#[must_use]
pub fn cgroup_place(path: &str) -> Place {
    let path = path.trim();
    if path.is_empty() || path == "/" {
        return Place::Unknown;
    }
    if ["/system.slice", "/machine.slice", "/init.scope"]
        .iter()
        .any(|p| path.starts_with(p))
    {
        return Place::System;
    }
    let Some(rest) = path.strip_prefix("/user.slice/") else {
        return Place::Unknown;
    };
    // Directly in user.slice (a rootless container's pause process, say) is
    // not the user's session.
    let (user_slice, rest) = rest.split_once('/').unwrap_or((rest, ""));
    if !user_slice.starts_with("user-") {
        return Place::System;
    }
    let (manager, rest) = rest.split_once('/').unwrap_or((rest, ""));
    if !(manager.starts_with("user@") && manager.ends_with(".service")) {
        // session-N.scope: what the display manager started. On desktops that
        // do not launch applications as systemd units, the user's programs
        // live here too, so it proves nothing either way.
        return Place::Unknown;
    }
    let (slice, rest) = rest.split_once('/').unwrap_or((rest, ""));
    let unit = rest.split('/').next().unwrap_or("");
    match slice {
        "app.slice" if unit.starts_with("app-") => Place::App,
        "background.slice" if !unit.is_empty() => Place::Background,
        _ => Place::Session,
    }
}

/// What must never be touched beyond what a process says about itself:
/// Big Game Mode's own lineage and the running game's.
#[derive(Debug, Clone, Default)]
pub struct Guard {
    uid: u32,
    own_pid: u32,
    related: HashSet<u32>,
    game_running: bool,
}

impl Guard {
    /// The guard for this moment: Big Game Mode as it runs, and `game` if one
    /// is running.
    #[must_use]
    pub fn new(game: Option<&GameIdentity>) -> Self {
        let game_pids: Vec<u32> = game
            .map(|g| {
                let mut pids: Vec<u32> = g.tree.iter().map(|(pid, _)| *pid).collect();
                pids.push(g.pid);
                pids
            })
            .unwrap_or_default();
        Self::from_table(
            current_uid(),
            std::process::id(),
            &parent_table(),
            &game_pids,
        )
    }

    /// The guard for a process table (`pid → ppid`): `own_pid` and every
    /// process in `game`, each with its ancestors and descendants.
    ///
    /// Ancestors matter as much as descendants: the terminal or launcher a
    /// game was started from takes the game with it when it closes, and the
    /// Steam client is the parent of every Steam game's reaper.
    #[allow(clippy::similar_names)] // pid and ppid are what /proc calls them
    #[must_use]
    pub fn from_table<S: std::hash::BuildHasher>(
        uid: u32,
        own_pid: u32,
        parents: &HashMap<u32, u32, S>,
        game: &[u32],
    ) -> Self {
        let mut children: HashMap<u32, Vec<u32>> = HashMap::new();
        for (&pid, &ppid) in parents {
            children.entry(ppid).or_default().push(pid);
        }
        let mut related = HashSet::new();
        for &root in std::iter::once(&own_pid).chain(game) {
            // Up, bounded: a table read while processes come and go can
            // briefly hold a cycle.
            let mut pid = root;
            for _ in 0..1024 {
                related.insert(pid);
                match parents.get(&pid) {
                    Some(&ppid) if ppid > 1 && !related.contains(&ppid) => pid = ppid,
                    _ => break,
                }
            }
            // Down.
            let mut queue = vec![root];
            while let Some(pid) = queue.pop() {
                for &child in children.get(&pid).into_iter().flatten() {
                    if related.insert(child) {
                        queue.push(child);
                    }
                }
            }
        }
        Self {
            uid,
            own_pid,
            related,
            game_running: !game.is_empty(),
        }
    }
}

/// Why `p` must not be closed or paused, or `None` if the user may.
///
/// Conservative by construction: every test that can refuse, refuses, and a
/// process is offered only when it passes all of them — the user's own, a
/// user program (not the kernel, the session, the system, Big Game Mode or
/// anything it drives), unrelated to Big Game Mode and to the running game.
#[must_use]
pub fn protection(p: &Facts, guard: &Guard) -> Option<Protection> {
    // A Big Game Mode running as root could signal anything; that is exactly
    // what must not be possible, so it may signal nothing.
    if guard.uid == 0 || p.uids.is_empty() || p.uids.iter().any(|&u| u != guard.uid) {
        return Some(Protection::NotYours);
    }
    if p.kernel_thread || p.pid <= 1 || p.ppid == 0 || p.ppid == 2 || p.argv0.is_empty() {
        return Some(Protection::Kernel);
    }
    if matches!(p.state, 'Z' | 'X' | 'x') {
        return Some(Protection::Exited);
    }
    let name = p.name.to_ascii_lowercase();
    let argv0 = p.argv0.to_ascii_lowercase();
    if p.pid == guard.own_pid
        || classify(&name) == Kind::ThisApp
        || listed(ECOSYSTEM, &name)
        || listed(ECOSYSTEM, &argv0)
    {
        return Some(Protection::ThisApp);
    }
    if guard.related.contains(&p.pid) {
        return Some(Protection::Related);
    }
    if guard.game_running
        && (classify(&name) == Kind::Gaming
            || crate::running::is_infrastructure(&name)
            || crate::running::is_infrastructure(&argv0))
    {
        return Some(Protection::GameMachinery);
    }
    if listed(DESKTOP, &name) || listed(DESKTOP, &argv0) {
        return Some(Protection::Desktop);
    }
    match cgroup_place(&p.cgroup) {
        // Of the session's background services only an indexer is nobody's
        // dependency: pausing a FUSE mount or a Plasma helper would hang every
        // program that touches it.
        Place::Background if classify(&name) == Kind::Indexer => None,
        Place::Session | Place::Background => Some(Protection::SessionService),
        Place::System => Some(Protection::SystemService),
        Place::App | Place::Unknown => None,
    }
}

// ── Measuring ───────────────────────────────────────────────────────────────

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
    /// When it started, for [`Target`].
    pub start_ticks: u64,
    /// Whether the user may close or pause it: [`protection`] found nothing.
    pub controllable: bool,
}

impl BusyProcess {
    /// What to name when acting on this process.
    #[must_use]
    pub fn target(&self) -> Target {
        Target {
            pid: self.pid,
            start_ticks: self.start_ticks,
            name: self.name.clone(),
        }
    }
}

/// How much of one CPU a process must use before it is worth mentioning.
///
/// Low enough to catch a steady background task, high enough that an idle
/// desktop reports nothing. A list that always has entries teaches people to
/// ignore it.
pub const BUSY_THRESHOLD_PERCENT: f64 = 5.0;

/// Sample CPU usage over `window` and report the current user's busy
/// processes, each with whether it may be closed or paused while `game` runs.
///
/// Two passes are needed because `/proc/<pid>/stat` reports cumulative CPU
/// time; a single reading gives the average since the process started, which
/// for a long-lived browser says nothing about what it is doing now.
#[must_use]
pub fn busy_processes(window: Duration, game: Option<&GameIdentity>) -> Vec<BusyProcess> {
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
            // The same pid with another start time is another process.
            if before.start_ticks != after.start_ticks {
                return None;
            }
            let delta = after.ticks.saturating_sub(before.ticks);
            #[allow(clippy::cast_precision_loss)]
            let percent = (delta as f64 / ticks_per_second) / elapsed * 100.0;
            (percent >= BUSY_THRESHOLD_PERCENT).then(|| BusyProcess {
                pid,
                kind: classify(&after.name),
                name: after.name,
                cpu_percent: percent,
                memory_mib: after.rss_bytes / 1_048_576,
                start_ticks: after.start_ticks,
                controllable: false,
            })
        })
        .collect();

    out.sort_by(|a, b| b.cpu_percent.total_cmp(&a.cpu_percent));
    out.truncate(12);

    // The rule reads more of each process than sampling does; only the few
    // that are listed are worth it.
    let guard = Guard::new(game);
    for process in &mut out {
        process.controllable = read_facts(process.pid).is_some_and(|f| {
            f.start_ticks == process.start_ticks && protection(&f, &guard).is_none()
        });
    }
    out
}

struct Sample {
    name: String,
    ticks: u64,
    rss_bytes: u64,
    start_ticks: u64,
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
        if process_uids(&entry.path()).and_then(|u| u.first().copied()) != Some(uid) {
            continue;
        }
        let Some(sample) = read_sample(&entry.path()) else {
            continue;
        };
        out.insert(pid, sample);
    }
    out
}

/// Real, effective, saved and filesystem uids.
fn process_uids(proc_dir: &Path) -> Option<Vec<u32>> {
    let status = std::fs::read_to_string(proc_dir.join("status")).ok()?;
    status.lines().find_map(|line| {
        let rest = line.strip_prefix("Uid:")?;
        rest.split_whitespace().map(|u| u.parse().ok()).collect()
    })
}

/// The fields of `/proc/<pid>/stat` after the command name.
///
/// The comm field is parenthesised and may itself contain spaces and
/// parentheses, so fields are counted from after the final ')'. Indices count
/// from the state field (proc(5) field 3): ppid (4) is 1, flags (9) is 6,
/// utime (14) is 11, stime (15) is 12, starttime (22) is 19, rss (24) is 21.
fn stat_fields(stat: &str) -> Option<Vec<&str>> {
    let close = stat.rfind(')')?;
    Some(stat[close + 1..].split_whitespace().collect())
}

fn read_sample(proc_dir: &Path) -> Option<Sample> {
    let stat = std::fs::read_to_string(proc_dir.join("stat")).ok()?;
    let fields = stat_fields(&stat)?;
    let utime: u64 = fields.get(11)?.parse().ok()?;
    let stime: u64 = fields.get(12)?.parse().ok()?;
    let start_ticks: u64 = fields.get(19)?.parse().ok()?;
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
        start_ticks,
    })
}

/// The kernel's flag for its own threads (`include/linux/sched.h`).
const PF_KTHREAD: u64 = 0x0020_0000;

/// Everything [`protection`] needs about `pid`, or `None` if it is gone.
#[allow(clippy::similar_names)] // pid and ppid are what /proc calls them
#[must_use]
pub fn read_facts(pid: u32) -> Option<Facts> {
    let dir = PathBuf::from(format!("/proc/{pid}"));
    let stat = std::fs::read_to_string(dir.join("stat")).ok()?;
    let fields = stat_fields(&stat)?;
    let state = fields.first()?.chars().next()?;
    let ppid: u32 = fields.get(1)?.parse().ok()?;
    let flags: u64 = fields.get(6)?.parse().ok()?;
    let start_ticks: u64 = fields.get(19)?.parse().ok()?;
    let uids = process_uids(&dir)?;
    let name = std::fs::read_to_string(dir.join("comm"))
        .map(|s| s.trim().to_owned())
        .unwrap_or_default();
    let cmdline = std::fs::read(dir.join("cmdline")).unwrap_or_default();
    let argv0 = cmdline
        .split(|b| *b == 0)
        .next()
        .map(|a| crate::running::falcond_name(&String::from_utf8_lossy(a)).to_owned())
        .unwrap_or_default();
    let cgroup = std::fs::read_to_string(dir.join("cgroup"))
        .ok()
        .and_then(|text| {
            text.lines()
                .find_map(|l| l.strip_prefix("0::"))
                .map(str::to_owned)
        })
        .unwrap_or_default();
    Some(Facts {
        pid,
        ppid,
        uids,
        name,
        argv0,
        state,
        kernel_thread: flags & PF_KTHREAD != 0,
        cgroup,
        start_ticks,
    })
}

/// `pid → ppid` for every process on the machine, whoever owns it: a
/// lineage runs through other users' processes (a display manager, the
/// user manager's root parent).
#[allow(clippy::similar_names)] // pid and ppid are what /proc calls them
fn parent_table() -> HashMap<u32, u32> {
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return HashMap::new();
    };
    entries
        .flatten()
        .filter_map(|entry| {
            let pid: u32 = entry.file_name().to_string_lossy().parse().ok()?;
            let stat = std::fs::read_to_string(entry.path().join("stat")).ok()?;
            let ppid: u32 = stat_fields(&stat)?.get(1)?.parse().ok()?;
            Some((pid, ppid))
        })
        .collect()
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

// ── Acting ──────────────────────────────────────────────────────────────────

/// One process, as it was when it was listed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Target {
    /// Process id.
    pub pid: u32,
    /// Start time, in clock ticks since boot: a pid reused by another
    /// program has another.
    pub start_ticks: u64,
    /// Its name, for messages.
    pub name: String,
}

/// Ask `target` to quit with `SIGTERM`, if [`protection`] still allows it.
///
/// Only asks: a program may save, ask the user, or ignore the request, and
/// nothing here follows up with `SIGKILL`. A program Big Game Mode had paused
/// is resumed after the request, since a stopped process cannot act on it.
///
/// # Errors
/// Returns a [`UserError`] if the process has exited, is protected now, or
/// the system refuses the signal.
pub fn close(target: &Target, guard: &Guard) -> Result<()> {
    let was_stopped = signal(target, libc::SIGTERM, |facts| allowed(target, facts, guard))?;
    if was_stopped {
        // Best effort: it is already asked to quit; resuming only lets it.
        let _ = signal(target, libc::SIGCONT, |facts| owned(target, facts));
    }
    forget_paused(target);
    tracing::info!(target: "processes", pid = target.pid, name = %target.name, "asked to close (SIGTERM)");
    Ok(())
}

/// Pause `target` with `SIGSTOP`, if [`protection`] still allows it, and
/// remember it so it can be resumed.
///
/// # Errors
/// Returns a [`UserError`] if the process has exited, is protected now, or
/// the system refuses the signal.
pub fn pause(target: &Target, guard: &Guard) -> Result<()> {
    signal(target, libc::SIGSTOP, |facts| allowed(target, facts, guard))?;
    with_paused(|list| {
        if !list.contains(target) {
            list.push(target.clone());
        }
    });
    tracing::info!(target: "processes", pid = target.pid, name = %target.name, "paused (SIGSTOP)");
    Ok(())
}

/// Resume `target` with `SIGCONT` and forget it.
///
/// No rule applies beyond ownership: resuming undoes a pause, and is always
/// the safe direction.
///
/// # Errors
/// Returns a [`UserError`] if the process has exited or the system refuses
/// the signal; an exited process is forgotten all the same.
pub fn resume(target: &Target) -> Result<()> {
    let result = signal(target, libc::SIGCONT, |facts| owned(target, facts));
    forget_paused(target);
    result?;
    tracing::info!(target: "processes", pid = target.pid, name = %target.name, "resumed (SIGCONT)");
    Ok(())
}

/// The processes Big Game Mode paused that are still paused. Those that exited
/// or were resumed from elsewhere are forgotten.
#[must_use]
pub fn paused() -> Vec<Target> {
    with_paused(|list| {
        list.retain(|t| {
            read_facts(t.pid)
                .is_some_and(|f| f.start_ticks == t.start_ticks && matches!(f.state, 'T' | 't'))
        });
        list.clone()
    })
}

/// Resume everything Big Game Mode paused — when it quits, and at its next
/// start if it was killed before it could. Returns how many were resumed.
pub fn resume_all() -> usize {
    let list = with_paused(std::mem::take);
    let mut resumed = 0;
    for target in &list {
        match signal(target, libc::SIGCONT, |facts| owned(target, facts)) {
            Ok(_) => {
                resumed += 1;
                tracing::info!(target: "processes", pid = target.pid, name = %target.name, "resumed (SIGCONT)");
            }
            Err(e) => {
                tracing::debug!(target: "processes", pid = target.pid, error = %e, "not resumed");
            }
        }
    }
    resumed
}

fn owned(target: &Target, facts: &Facts) -> Result<()> {
    if facts.uids.is_empty() || facts.uids.iter().any(|&u| u != current_uid()) {
        return Err(refused(target).into());
    }
    Ok(())
}

fn allowed(target: &Target, facts: &Facts, guard: &Guard) -> Result<()> {
    match protection(facts, guard) {
        None => Ok(()),
        Some(_) => Err(refused(target).into()),
    }
}

fn refused(target: &Target) -> UserError {
    UserError::with(
        N_(
            "%s is part of the system, the desktop, the game or Big Game Mode; Big Game Mode will not close or pause it.",
        ),
        [target.name.as_str()],
    )
}

fn gone(target: &Target) -> UserError {
    UserError::with(N_("%s has already exited."), [target.name.as_str()])
}

/// Send `sig` to `target` once `check` accepts what it is now. Returns
/// whether it was stopped (paused) before the signal.
///
/// The process is pinned with a pidfd first, then re-read and compared with
/// what was listed: from then on the signal can only reach that process, even
/// if it exits and its pid is reused. `kill` is the fallback on a kernel
/// without pidfds (before 5.3), with the start time still compared.
fn signal(target: &Target, sig: i32, check: impl Fn(&Facts) -> Result<()>) -> Result<bool> {
    let pid = libc::pid_t::try_from(target.pid).context("pid out of range")?;
    // SAFETY: pidfd_open takes a pid and flags and returns a new descriptor or
    // -1; nothing is passed by pointer.
    let raw = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) };
    let pidfd = if raw >= 0 {
        let fd = i32::try_from(raw).context("pidfd out of range")?;
        // SAFETY: `fd` was just returned by pidfd_open and is owned by nobody
        // else; OwnedFd closes it.
        Some(unsafe { OwnedFd::from_raw_fd(fd) })
    } else {
        let e = std::io::Error::last_os_error();
        match e.raw_os_error() {
            Some(libc::ESRCH) => return Err(gone(target).into()),
            Some(libc::ENOSYS) => None,
            _ => return Err(e).context("pidfd_open"),
        }
    };
    let facts = read_facts(target.pid)
        .filter(|f| f.start_ticks == target.start_ticks && !matches!(f.state, 'Z' | 'X' | 'x'))
        .ok_or_else(|| gone(target))?;
    check(&facts)?;
    let rc = match &pidfd {
        // SAFETY: a valid pidfd, a signal number, no siginfo (the kernel then
        // fills it in as kill(2) would), and no flags.
        Some(fd) => unsafe {
            libc::syscall(
                libc::SYS_pidfd_send_signal,
                fd.as_raw_fd(),
                sig,
                std::ptr::null::<libc::siginfo_t>(),
                0,
            )
        },
        // SAFETY: kill takes a pid and a signal number.
        None => i64::from(unsafe { libc::kill(pid, sig) }),
    };
    if rc != 0 {
        let e = std::io::Error::last_os_error();
        return match e.raw_os_error() {
            Some(libc::ESRCH) => Err(gone(target).into()),
            Some(libc::EPERM) => Err(UserError::with(
                N_("The system did not let Big Game Mode signal %s."),
                [target.name.as_str()],
            )
            .caused_by(e)
            .into()),
            _ => Err(e).context("send signal"),
        };
    }
    Ok(matches!(facts.state, 'T' | 't'))
}

// ── What Big Game Mode paused ─────────────────────────────────────────────────

/// Where the paused list is kept, so a crash cannot lose it.
fn paused_file() -> PathBuf {
    crate::paths::state_home().join("bigame-mode/paused.json")
}

/// The paused list: loaded from its file on first use, saved after every
/// change.
static PAUSED: Mutex<Option<Vec<Target>>> = Mutex::new(None);

fn with_paused<R>(f: impl FnOnce(&mut Vec<Target>) -> R) -> R {
    let mut guard = PAUSED
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let path = paused_file();
    let list = guard.get_or_insert_with(|| load_paused(&path));
    let before = list.clone();
    let result = f(list);
    if *list != before {
        if let Err(e) = save_paused(&path, list) {
            tracing::warn!(target: "processes", error = %format!("{e:#}"), "could not save the paused list");
        }
    }
    result
}

fn forget_paused(target: &Target) {
    with_paused(|list| list.retain(|t| t != target));
}

fn load_paused(path: &Path) -> Vec<Target> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

fn save_paused(path: &Path, list: &[Target]) -> Result<()> {
    if list.is_empty() {
        return match std::fs::remove_file(path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
            _ => Ok(()),
        };
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec(list)?)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// Whether process `pid` has the switch `key` on in its environment: set,
/// and not empty or `0` (Big Game Mode turns a switch off in the session by
/// setting it to `0`). No value is kept.
#[must_use]
pub fn env_switch_on(pid: u32, key: &str) -> bool {
    let Ok(bytes) = std::fs::read(format!("/proc/{pid}/environ")) else {
        return false;
    };
    switch_on(&bytes, key)
}

fn switch_on(environ: &[u8], key: &str) -> bool {
    let prefix = format!("{key}=");
    environ.split(|b| *b == 0).any(|entry| {
        entry
            .strip_prefix(prefix.as_bytes())
            .is_some_and(|v| !v.is_empty() && v != b"0")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_switch_set_to_zero_is_off() {
        let env = b"PATH=/usr/bin\0WINE_FULLSCREEN_FSR=0\0ENABLE_VKBASALT=1\0EMPTY=\0";
        assert!(!switch_on(env, "WINE_FULLSCREEN_FSR"));
        assert!(switch_on(env, "ENABLE_VKBASALT"));
        assert!(!switch_on(env, "EMPTY"));
        assert!(!switch_on(env, "MISSING"));
    }

    #[test]
    fn known_background_work_is_recognised() {
        assert_eq!(classify("baloo_file"), Kind::Indexer);
        assert_eq!(classify("tracker-miner-fs-3"), Kind::Indexer);
        assert_eq!(classify("cargo"), Kind::Compiler);
        assert_eq!(classify("rustc"), Kind::Compiler);
        assert_eq!(classify("syncthing"), Kind::Sync);
        assert_eq!(classify("firefox"), Kind::Browser);
        assert_eq!(classify("chrome"), Kind::Browser);
        assert_eq!(classify("bigame-ui"), Kind::ThisApp);
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
            Kind::Desktop,
            Kind::ThisApp,
            Kind::Other,
        ] {
            assert!(!kind.describe().is_empty());
        }
    }

    #[test]
    fn sampling_this_machine_reports_only_plausible_entries() {
        // A short window keeps the test quick; it is long enough that a busy
        // process registers and an idle one does not.
        let busy = busy_processes(Duration::from_millis(400), None);
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
        let busy = busy_processes(Duration::from_millis(300), None);
        assert!(!busy.iter().any(|p| p.pid == 1));
    }

    // ── The rule ────────────────────────────────────────────────────────────

    const ME: u32 = 1000;

    fn app(pid: u32, name: &str, cgroup: &str) -> Facts {
        Facts {
            pid,
            ppid: 900,
            uids: vec![ME; 4],
            name: name.into(),
            argv0: name.into(),
            state: 'R',
            kernel_thread: false,
            cgroup: cgroup.into(),
            start_ticks: 12_345,
        }
    }

    const APP_SCOPE: &str =
        "/user.slice/user-1000.slice/user@1000.service/app.slice/app-firefox-1234.scope";

    fn nobody_running() -> Guard {
        Guard::from_table(ME, 4000, &HashMap::from([(4000, 900), (900, 1)]), &[])
    }

    #[test]
    fn a_program_of_the_users_own_may_be_closed_or_paused() {
        let guard = nobody_running();
        for (name, cgroup) in [
            ("firefox", APP_SCOPE),
            (
                "Telegram",
                "/user.slice/user-1000.slice/user@1000.service/app.slice/app-flatpak-org.telegram.desktop-2268265498.scope",
            ),
            (
                "baloo_file",
                "/user.slice/user-1000.slice/user@1000.service/background.slice/kde-baloo.service",
            ),
            (
                "code-insiders",
                "/user.slice/user-1000.slice/user@1000.service/app.slice/app-code\\x2dinsiders@1c51ab44.service",
            ),
            // No systemd layout to go by: the name rules decide.
            ("discord", "/user.slice/user-1000.slice/session-2.scope"),
            ("discord", "/"),
            ("discord", ""),
        ] {
            assert_eq!(
                protection(&app(3000, name, cgroup), &guard),
                None,
                "{name} in {cgroup}"
            );
        }
    }

    #[test]
    fn nothing_of_another_user_or_root_is_offered() {
        let guard = nobody_running();
        let mut root = app(3000, "firefox", APP_SCOPE);
        root.uids = vec![0; 4];
        assert_eq!(protection(&root, &guard), Some(Protection::NotYours));
        // Started by the user, running with root's privileges (setuid).
        let mut setuid = app(3000, "firefox", APP_SCOPE);
        setuid.uids = vec![ME, 0, 0, ME];
        assert_eq!(protection(&setuid, &guard), Some(Protection::NotYours));
        let mut unknown = app(3000, "firefox", APP_SCOPE);
        unknown.uids.clear();
        assert_eq!(protection(&unknown, &guard), Some(Protection::NotYours));
        // A Big Game Mode running as root may signal nothing at all.
        let as_root = Guard::from_table(0, 4000, &HashMap::new(), &[]);
        let mut roots_own = app(3000, "firefox", APP_SCOPE);
        roots_own.uids = vec![0; 4];
        assert_eq!(protection(&roots_own, &as_root), Some(Protection::NotYours));
    }

    #[test]
    fn kernel_threads_init_and_the_exited_are_never_offered() {
        let guard = nobody_running();
        let mut kthread = app(3000, "kworker/0:1", "/");
        kthread.kernel_thread = true;
        assert_eq!(protection(&kthread, &guard), Some(Protection::Kernel));
        let mut child_of_kthreadd = app(3000, "anything", "/");
        child_of_kthreadd.ppid = 2;
        assert_eq!(
            protection(&child_of_kthreadd, &guard),
            Some(Protection::Kernel)
        );
        assert_eq!(
            protection(&app(1, "systemd", "/init.scope"), &guard),
            Some(Protection::Kernel)
        );
        let mut no_cmdline = app(3000, "firefox", APP_SCOPE);
        no_cmdline.argv0.clear();
        assert_eq!(protection(&no_cmdline, &guard), Some(Protection::Kernel));
        let mut zombie = app(3000, "firefox", APP_SCOPE);
        zombie.state = 'Z';
        assert_eq!(protection(&zombie, &guard), Some(Protection::Exited));
    }

    #[test]
    fn bigame_mode_and_what_it_drives_are_never_offered() {
        let guard = nobody_running();
        for name in [
            "bigame-ui",
            "bigame-daemon",
            "falcond",
            "gamemoded",
            "mangoapp",
            "scx_lavd",
            "power-profiles-",
            "tuned-ppd",
            "reaper",
            "srt-bwrap",
            "pv-adverb",
            "steam-runtime-l",
            "bash",
            "sh",
            "zsh",
            "fish",
        ] {
            assert_eq!(
                protection(&app(3000, name, APP_SCOPE), &guard),
                Some(Protection::ThisApp),
                "{name}"
            );
        }
        // The running instance, whatever it is called.
        assert_eq!(
            protection(&app(4000, "renamed", APP_SCOPE), &guard),
            Some(Protection::ThisApp)
        );
        // argv[0] counts as much as comm.
        let mut disguised = app(3000, "worker", APP_SCOPE);
        disguised.argv0 = "falcond".into();
        assert_eq!(protection(&disguised, &guard), Some(Protection::ThisApp));
    }

    #[test]
    fn the_desktop_session_is_never_offered() {
        let guard = nobody_running();
        // Real names, as /proc/<pid>/comm spells them (15 characters at most).
        for name in [
            "kwin_wayland",
            "kwin_wayland_wr",
            "kwin_x11",
            "plasmashell",
            "ksmserver",
            "kded6",
            "Xwayland",
            "Xorg",
            "gnome-shell",
            "gnome-session-b",
            "gsd-power",
            "mutter-x11-fram",
            "xfwm4",
            "Hyprland",
            "sway",
            "gamescope-wl",
            "pipewire",
            "pipewire-pulse",
            "wireplumber",
            "pulseaudio",
            "dbus-daemon",
            "dbus-broker",
            "dbus-broker-lau",
            "systemd",
            "(sd-pam)",
            "polkit-kde-auth",
            "polkit-gnome-au",
            "lxpolkit",
            "xdg-desktop-por",
            "xdg-document-po",
            "xdg-permission-",
            "at-spi-bus-laun",
            "gvfsd",
            "NetworkManager",
            "nm-applet",
            "kwalletd6",
            "ksecretd",
            "gnome-keyring-d",
            "ssh-agent",
            "gpg-agent",
            "org_kde_powerde",
            "baloorunner",
            "xembedsniproxy",
            "ibus-daemon",
            "fcitx5",
            "speech-dispatch",
            "sddm-helper",
            "sudo",
            "pkexec",
        ] {
            assert_eq!(
                protection(&app(3000, name, APP_SCOPE), &guard),
                Some(Protection::Desktop),
                "{name}"
            );
        }
        assert_eq!(classify("kwin_wayland"), Kind::Desktop);
        assert_eq!(classify("pipewire"), Kind::Desktop);
    }

    #[test]
    fn session_and_system_services_are_refused_by_where_systemd_put_them() {
        let guard = nobody_running();
        let base = "/user.slice/user-1000.slice/user@1000.service";
        for (cgroup, why) in [
            (
                format!("{base}/session.slice/some-helper.service"),
                Protection::SessionService,
            ),
            (format!("{base}/init.scope"), Protection::SessionService),
            (
                format!("{base}/app.slice/dconf.service"),
                Protection::SessionService,
            ),
            (
                format!("{base}/app.slice/dbus-:1.2-org.kde.kdeconnect@0.service"),
                Protection::SessionService,
            ),
            (
                format!("{base}/background.slice/plasma-kactivitymanagerd.service"),
                Protection::SessionService,
            ),
            (
                format!("{base}/background.slice/kio-fuse.service"),
                Protection::SessionService,
            ),
            (
                format!("{base}/uresourced.service"),
                Protection::SessionService,
            ),
            (
                "/system.slice/docker-abb88c41.scope".to_owned(),
                Protection::SystemService,
            ),
            (
                "/machine.slice/libpod-1234.scope".to_owned(),
                Protection::SystemService,
            ),
            (
                "/user.slice/podman-pause-02aab77f.scope".to_owned(),
                Protection::SystemService,
            ),
        ] {
            assert_eq!(
                protection(&app(3000, "some-helper", &cgroup), &guard),
                Some(why),
                "{cgroup}"
            );
        }
    }

    #[test]
    fn the_game_and_everything_around_it_are_never_offered() {
        // systemd --user (1000) → steam (2000) → reaper (2100) → game (2200)
        // → its child (2300); steamwebhelper (2010) beside the reaper; a
        // browser (3000); Big Game Mode (4000) and a program it started (4100).
        let table = HashMap::from([
            (1000, 1),
            (2000, 1000),
            (2010, 2000),
            (2100, 2000),
            (2200, 2100),
            (2300, 2200),
            (2400, 1),
            (3000, 1000),
            (4000, 1000),
            (4100, 4000),
        ]);
        let guard = Guard::from_table(ME, 4000, &table, &[2100, 2200]);
        let check = |pid, name: &str| protection(&app(pid, name, APP_SCOPE), &guard);
        assert_eq!(check(2200, "SOTTR.exe"), Some(Protection::Related));
        assert_eq!(check(2300, "d3ddriverquery"), Some(Protection::Related));
        assert_eq!(check(2100, "container"), Some(Protection::Related));
        // What launched the game.
        assert_eq!(check(2000, "client"), Some(Protection::Related));
        // Steam's own processes beside the game, and Wine's daemon that was
        // re-parented away from it.
        assert_eq!(
            check(2010, "steamwebhelper"),
            Some(Protection::GameMachinery)
        );
        assert_eq!(check(2400, "wineserver"), Some(Protection::GameMachinery));
        // What Big Game Mode started.
        assert_eq!(check(4100, "journalctl"), Some(Protection::Related));
        // Unrelated to either.
        assert_eq!(check(3000, "firefox"), None);

        // With no game running, a launcher is the user's to close.
        let idle = Guard::from_table(ME, 4000, &table, &[]);
        assert_eq!(
            protection(&app(2010, "steamwebhelper", APP_SCOPE), &idle),
            None
        );
    }

    #[test]
    fn a_process_table_with_a_cycle_does_not_hang() {
        let table = HashMap::from([(5, 6), (6, 5), (4000, 5)]);
        let guard = Guard::from_table(ME, 4000, &table, &[5]);
        assert!(guard.related.contains(&6));
    }

    #[test]
    fn patterns_match_exactly_by_prefix_or_anywhere() {
        assert!(listed(&["x"], "x"));
        assert!(!listed(&["x"], "xterm"));
        assert!(listed(&["kwin*"], "kwin_x11"));
        assert!(!listed(&["kwin*"], "akwin"));
        assert!(listed(&["*polkit*"], "lxpolkit"));
        assert!(listed(&["*polkit*"], "polkit-gnome-au"));
    }

    #[test]
    fn systemd_places_are_read_from_the_cgroup_path() {
        assert_eq!(cgroup_place(APP_SCOPE), Place::App);
        assert_eq!(
            cgroup_place(
                "/user.slice/user-1000.slice/user@1000.service/app.slice/app-org.kde.dolphin-1554755.scope/tab(1554771).scope"
            ),
            Place::App
        );
        assert_eq!(
            cgroup_place(
                "/user.slice/user-1000.slice/user@1000.service/session.slice/pipewire.service"
            ),
            Place::Session
        );
        assert_eq!(cgroup_place("/init.scope"), Place::System);
        assert_eq!(
            cgroup_place("/user.slice/user-1000.slice/session-2.scope"),
            Place::Unknown
        );
        assert_eq!(cgroup_place(""), Place::Unknown);
        assert_eq!(
            cgroup_place(
                "/user.slice/user-1000.slice/user@1000.service/background.slice/kde-baloo.service"
            ),
            Place::Background
        );
    }

    // ── Acting ──────────────────────────────────────────────────────────────

    #[test]
    fn the_paused_list_survives_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bigame-mode/paused.json");
        assert!(load_paused(&path).is_empty());
        let list = vec![Target {
            pid: 42,
            start_ticks: 7,
            name: "firefox".into(),
        }];
        save_paused(&path, &list).unwrap();
        assert_eq!(load_paused(&path), list);
        // An empty list leaves no file behind.
        save_paused(&path, &[]).unwrap();
        assert!(!path.exists());
        save_paused(&path, &[]).unwrap();
    }

    /// A process of the test's own, so nothing real is ever signalled.
    struct Sleeper(std::process::Child);

    impl Sleeper {
        fn start() -> Self {
            Self(
                std::process::Command::new("sleep")
                    .arg("30")
                    .spawn()
                    .unwrap(),
            )
        }

        fn target(&self) -> Target {
            let facts = read_facts(self.0.id()).unwrap();
            Target {
                pid: facts.pid,
                start_ticks: facts.start_ticks,
                name: facts.name,
            }
        }

        fn state(&self) -> char {
            read_facts(self.0.id()).map_or('?', |f| f.state)
        }

        fn settle(&self, want: char) -> char {
            for _ in 0..100 {
                if self.state() == want {
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            self.state()
        }
    }

    impl Drop for Sleeper {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    #[test]
    fn signals_pause_resume_and_close_a_process_of_our_own() {
        let sleeper = Sleeper::start();
        let target = sleeper.target();
        signal(&target, libc::SIGSTOP, |_| Ok(())).unwrap();
        assert_eq!(sleeper.settle('T'), 'T');
        assert!(
            signal(&target, libc::SIGCONT, |_| Ok(())).unwrap(),
            "reports that it was stopped"
        );
        assert_eq!(sleeper.settle('S'), 'S');
        signal(&target, libc::SIGTERM, |_| Ok(())).unwrap();
        let mut child = sleeper;
        let status = child.0.wait().unwrap();
        assert!(!status.success(), "terminated by the signal");
        let err = signal(&target, libc::SIGTERM, |_| Ok(())).unwrap_err();
        assert!(err.to_string().contains("already exited"), "{err}");
    }

    #[test]
    fn a_reused_pid_or_a_refusal_sends_nothing() {
        let sleeper = Sleeper::start();
        let mut stale = sleeper.target();
        stale.start_ticks += 1;
        let err = signal(&stale, libc::SIGSTOP, |_| Ok(())).unwrap_err();
        assert!(err.to_string().contains("already exited"), "{err}");
        let target = sleeper.target();
        let err = signal(&target, libc::SIGSTOP, |_| Err(refused(&target).into())).unwrap_err();
        assert!(err.to_string().contains("will not close or pause"), "{err}");
        std::thread::sleep(Duration::from_millis(50));
        assert_ne!(sleeper.state(), 'T', "still running");
    }

    #[test]
    fn a_program_bigame_mode_started_is_refused_at_the_moment_of_acting() {
        if current_uid() == 0 {
            return;
        }
        let sleeper = Sleeper::start();
        let target = sleeper.target();
        let err = pause(&target, &Guard::new(None)).unwrap_err();
        assert!(err.to_string().contains("will not close or pause"), "{err}");
        let err = close(&target, &Guard::new(None)).unwrap_err();
        assert!(err.to_string().contains("will not close or pause"), "{err}");
        assert_ne!(sleeper.state(), 'T');
    }
}
