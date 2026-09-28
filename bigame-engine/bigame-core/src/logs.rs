//! Everything worth reading about a game session, from one place.
//!
//! Most of it is in the journal, and one `journalctl -o json` call reads it
//! all ([`JOURNAL_MATCHES`] lists every source, and why it is there): falcond,
//! the helper, the UI, power profiles, sched-ext, Gamescope, Polkit, crashes,
//! the kernel's DRM and GPU drivers — and Steam's output stream, which is
//! where every Steam game's Proton, Gamescope, `MangoHud`, vkBasalt, lsfg-vk
//! and `OptiScaler` lines land. The journal's kernel records are readable by a
//! normal user where `dmesg` usually is not (`kernel.dmesg_restrict`).
//!
//! Two things are only in files ([`Reader`]): Steam's own record of each
//! game launch and exit (`logs/console_log.txt`), and `OptiScaler.log` in
//! the games AI Graphics installed it in. Each is read from where the last
//! read stopped, never more than [`FILE_TAIL`] at a time.
//!
//! Refreshes are incremental: the last journal entry's cursor is kept, and
//! the next read asks only for what came after it.
//!
//! Severity comes from the journal's own `PRIORITY` first, then from the
//! message for tools that log everything at one level (falcond writes
//! `warning(dbus):` inside info-priority records).
//!
//! BiGame-mode's own output reaches the journal whichever way it was started
//! ([`JournalSink`]): launched from a menu as a systemd scope, from a
//! terminal, or at login, its standard output may go anywhere or nowhere.

use std::collections::{HashMap, HashSet};
use std::io::{Read as _, Seek as _};
use std::path::{Component, Path, PathBuf};

use serde::Serialize;

/// Where an entry came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub enum Source {
    /// BiGame-mode's UI.
    BiGame,
    /// BiGame-mode's privileged helper.
    Helper,
    /// falcond.
    Falcond,
    /// The kernel: DRM and GPU drivers, sched-ext.
    Kernel,
    /// Gamescope.
    Gamescope,
    /// `scx_loader` and the schedulers it runs.
    Scheduler,
    /// power-profiles-daemon, or tuned in its place.
    PowerProfiles,
    /// Polkit, and `pkexec` running the installs BiGame-mode offers.
    Polkit,
    /// The Steam client: game launches and exits.
    Steam,
    /// Proton, Wine and the Steam Linux Runtime around them.
    Wine,
    /// `MangoHud`.
    MangoHud,
    /// vkBasalt.
    VkBasalt,
    /// lsfg-vk (frame generation).
    Lsfg,
    /// `OptiScaler`, as AI Graphics installed it.
    OptiScaler,
    /// Anything else matched.
    Other,
}

impl Source {
    /// A short label for the log view.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::BiGame => "bigame",
            Self::Helper => "helper",
            Self::Falcond => "falcond",
            Self::Kernel => "kernel",
            Self::Gamescope => "gamescope",
            Self::Scheduler => "scx",
            Self::PowerProfiles => "power",
            Self::Polkit => "polkit",
            Self::Steam => "steam",
            Self::Wine => "proton",
            Self::MangoHud => "mangohud",
            Self::VkBasalt => "vkbasalt",
            Self::Lsfg => "lsfg-vk",
            Self::OptiScaler => "optiscaler",
            Self::Other => "system",
        }
    }
}

/// The width of the widest [`Source::label`], for aligned columns.
pub const LABEL_WIDTH: usize = 10;

/// How much an entry matters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub enum Level {
    /// Verbose detail.
    Debug,
    /// Ordinary.
    Info,
    /// Something was confirmed: applied, verified, restored.
    Success,
    /// Worth a look.
    Warning,
    /// Something failed.
    Error,
}

impl Level {
    /// The fixed-width label shown before the message.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Debug => "DEBUG",
            Self::Info => "INFO ",
            Self::Success => "OK   ",
            Self::Warning => "WARN ",
            Self::Error => "ERROR",
        }
    }
}

/// One log line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Entry {
    /// Microseconds since the epoch.
    pub time_us: u64,
    /// Where it came from.
    pub source: Source,
    /// How much it matters.
    pub level: Level,
    /// The message, with ANSI colour codes removed.
    pub message: String,
}

// ── The journal ─────────────────────────────────────────────────────────────

/// Kernel lines worth showing: graphics drivers and the scheduler.
///
/// `NVRM` is how the proprietary NVIDIA driver signs its lines, and its GPU
/// errors (`NVRM: Xid (PCI:0000:01:00): 32, pid=…, name=SOTTR.exe`) carry
/// none of the other words.
const KERNEL_KEYWORDS: &[&str] = &[
    "amdgpu",
    "radeon",
    "nvidia",
    "nvrm",
    "nouveau",
    "i915",
    " xe ",
    "drm",
    "gpu",
    "sched_ext",
    "scx",
];

/// Every journal source, OR-ed. `_SYSTEMD_UNIT` is the unit a process runs
/// in; `UNIT` is what systemd itself says about a unit (started, stopped,
/// failed, restarted) — a crash of a dependency is often only there.
///
/// Left out on purpose: `NetworkManager` and systemd-resolved (BiGame-mode
/// never configures the network; the DNS comparison reads
/// `/etc/resolv.conf` and sends queries), screensaver inhibition (falcond
/// does it, and its calls log under falcond's unit), and Heroic or Lutris
/// (BiGame-mode hands them a launch URL; they keep their own logs).
pub const JOURNAL_MATCHES: &[&str] = &[
    // falcond: profiles, Turbo, the scheduler and idle inhibition.
    "_SYSTEMD_UNIT=falcond.service",
    "UNIT=falcond.service",
    // BiGame-mode's privileged helper.
    "_SYSTEMD_UNIT=bigame-daemon.service",
    "UNIT=bigame-daemon.service",
    // BiGame-mode's UI: its own records, GTK's warnings, and the output of
    // games it starts itself.
    "SYSLOG_IDENTIFIER=bigame-ui",
    // sched-ext: scx_loader and the schedulers it runs (scx_lavd, … log
    // under its unit); scx.service is the older, standalone service.
    "_SYSTEMD_UNIT=scx_loader.service",
    "UNIT=scx_loader.service",
    "_SYSTEMD_UNIT=scx.service",
    // Power profiles, under its own unit or a distribution's wrapper (BigLinux
    // runs it as power-profiles-daemon-biglinux-start.service), so also by
    // process name, which journald cuts to 15 characters; tuned in its place.
    "_SYSTEMD_UNIT=power-profiles-daemon.service",
    "_COMM=power-profiles-",
    "_SYSTEMD_UNIT=tuned.service",
    "_SYSTEMD_UNIT=tuned-ppd.service",
    // Gamescope started on its own rather than by Steam.
    "SYSLOG_IDENTIFIER=gamescope",
    "_COMM=gamescope-wl",
    // Steam's output, which its srt-logger writes here: everything its games
    // print. Only the lines of the components above are kept (see
    // `component_of`); the client's own chatter runs to thousands a day.
    "SYSLOG_IDENTIFIER=steam",
    // Polkit deciding on the helper's actions, and pkexec running the
    // package installs BiGame-mode offers.
    "SYSLOG_IDENTIFIER=polkitd",
    "SYSLOG_IDENTIFIER=pkexec",
    // Crashes, recorded by systemd-coredump.
    "COREDUMP_COMM=bigame-ui",
    "COREDUMP_COMM=bigame-daemon",
    "COREDUMP_COMM=falcond",
    "COREDUMP_COMM=gamescope",
    "COREDUMP_COMM=gamescope-wl",
    // The kernel, kept by keyword (KERNEL_KEYWORDS): journald matches fields
    // exactly and cannot select by message without `--grep`, which scans
    // everything.
    "_TRANSPORT=kernel",
];

/// The fields read from each record; the rest of a record (dozens of
/// `_CAP_*`, `_CMDLINE`, …) is not asked for.
const JOURNAL_FIELDS: &str =
    "MESSAGE,PRIORITY,SYSLOG_IDENTIFIER,_SYSTEMD_UNIT,UNIT,_COMM,_TRANSPORT,COREDUMP_COMM";

/// The `journalctl` arguments for the last `lines` records of every source,
/// after `after_cursor` if given.
#[must_use]
pub fn journal_args(lines: u32, after_cursor: Option<&str>) -> Vec<String> {
    let mut args: Vec<String> = vec![
        "--no-pager".into(),
        "--output=json".into(),
        format!("--output-fields={JOURNAL_FIELDS}"),
        "--since=-12h".into(),
        format!("--lines={lines}"),
    ];
    if let Some(cursor) = after_cursor {
        args.push(format!("--after-cursor={cursor}"));
    }
    for (i, m) in JOURNAL_MATCHES.iter().enumerate() {
        if i > 0 {
            args.push("+".into());
        }
        args.push((*m).into());
    }
    args
}

/// Remove ANSI escape sequences (tracing colours its output).
#[must_use]
pub fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            if chars.peek() == Some(&'[') {
                chars.next();
                for d in chars.by_ref() {
                    if d.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
            continue;
        }
        out.push(c);
    }
    out
}

/// A line written by `tracing` — BiGame-mode's own — carries its level:
/// `  INFO target: message`, after an RFC 3339 timestamp in builds that wrote
/// one. That level is the truth; the wording is not: `INFO turbo: turbo on
/// verified=1 … failed=0` contains "failed" and is not an error.
///
/// Returns the level and the message without the prefix.
#[must_use]
pub fn tracing_level(message: &str) -> Option<(Level, &str)> {
    let mut rest = message.trim_start();
    if let Some((first, after)) = rest.split_once(' ') {
        let timestamp = first.len() >= 20
            && first.as_bytes()[0].is_ascii_digit()
            && first.contains('T')
            && first.ends_with('Z');
        if timestamp {
            rest = after.trim_start();
        }
    }
    let (word, after) = rest.split_once(' ')?;
    let level = match word {
        "TRACE" | "DEBUG" => Level::Debug,
        "INFO" => Level::Info,
        "WARN" => Level::Warning,
        "ERROR" => Level::Error,
        _ => return None,
    };
    Some((level, after.trim_start()))
}

/// Lines that say nothing about BiGame-mode or the game, shown as debug.
///
/// gvfs, inside the UI, reports each volume monitor the system has masked or
/// not installed, every time GIO starts, with "failed" in the wording.
/// Wine's `fixme:` lines are notes to Wine's developers. falcond's `sudo`
/// calls open and close a PAM session each time.
const NOISE: &[&str] = &[
    "for remote volume monitor with dbus name",
    ":fixme:",
    "pam_unix(",
];

/// The severity of a message, from the journal priority and its wording.
#[must_use]
pub fn classify(priority: Option<u8>, message: &str) -> Level {
    let lower = message.to_ascii_lowercase();
    let has = |words: &[&str]| words.iter().any(|w| lower.contains(w));
    if priority.is_none_or(|p| p > 3) && has(NOISE) {
        return Level::Debug;
    }
    // An Xid is the NVIDIA driver reporting a GPU error (a channel fault, a
    // lost context, a hung engine); the kernel logs it at warning priority.
    if lower.contains("nvrm: xid") {
        return Level::Error;
    }
    if priority.is_some_and(|p| p <= 3)
        || has(&[
            " error", "error(", "error:", "[error]", ":err:", " err:", "failed", "failure",
            "panic", "critical", "fatal",
        ])
        || lower.starts_with("error")
    {
        return Level::Error;
    }
    if priority == Some(4)
        || has(&[
            "warning", " warn", "warn(", "warn:", "[warn", ":warn:", "alert", "denied",
        ])
    {
        return Level::Warning;
    }
    if has(&[
        "verified",
        "restored",
        "succeeded",
        "success",
        " ok",
        "activating profile",
        "game backend switched",
        "applied and verified",
    ]) {
        return Level::Success;
    }
    if priority == Some(7) || has(&["debug"]) {
        return Level::Debug;
    }
    Level::Info
}

/// The component a line of a game's output comes from, when it says.
///
/// Games inherit the output of whatever started them — Steam, or
/// BiGame-mode — so Gamescope, the Vulkan layers and Proton all write into
/// that one stream, each with its own prefix (`[gamescope]`, `[MANGOHUD]`,
/// `vkBasalt info:`, `wineserver:`, `pressure-vessel-wrap[…]`).
#[must_use]
pub fn component_of(message: &str) -> Option<Source> {
    let lower = message.to_ascii_lowercase();
    let has = |words: &[&str]| words.iter().any(|w| lower.contains(w));
    if has(&["mangohud", "mangoapp"]) {
        Some(Source::MangoHud)
    } else if has(&["vkbasalt"]) {
        Some(Source::VkBasalt)
    } else if has(&["lsfg"]) {
        Some(Source::Lsfg)
    } else if has(&["optiscaler"]) {
        Some(Source::OptiScaler)
    } else if has(&["gamescope"]) {
        Some(Source::Gamescope)
    } else if has(&[
        "wine",
        "proton",
        "pressure-vessel",
        "vkd3d",
        "dxvk",
        "ntsync",
        "fsync",
        "esync",
        ":err:",
        ":warn:",
        ":fixme:",
    ]) {
        Some(Source::Wine)
    } else {
        None
    }
}

fn unit_source(unit: &str) -> Option<Source> {
    Some(match unit {
        "falcond.service" => Source::Falcond,
        "bigame-daemon.service" => Source::Helper,
        "scx_loader.service" | "scx.service" => Source::Scheduler,
        "power-profiles-daemon.service" | "tuned.service" | "tuned-ppd.service" => {
            Source::PowerProfiles
        }
        _ => return None,
    })
}

fn source_of(record: &serde_json::Value) -> Source {
    let field = |k: &str| {
        record
            .get(k)
            .and_then(serde_json::Value::as_str)
            .unwrap_or("")
    };
    if field("_TRANSPORT") == "kernel" {
        return Source::Kernel;
    }
    match field("COREDUMP_COMM") {
        "" => {}
        "bigame-ui" => return Source::BiGame,
        "bigame-daemon" => return Source::Helper,
        "falcond" => return Source::Falcond,
        c if c.starts_with("gamescope") => return Source::Gamescope,
        _ => return Source::Other,
    }
    if let Some(source) = [field("_SYSTEMD_UNIT"), field("UNIT")]
        .into_iter()
        .find_map(unit_source)
    {
        return source;
    }
    match (field("SYSLOG_IDENTIFIER"), field("_COMM")) {
        ("bigame-ui", _) => Source::BiGame,
        ("gamescope", _) | (_, "gamescope-wl") => Source::Gamescope,
        ("steam", _) => Source::Steam,
        ("polkitd" | "pkexec", _) => Source::Polkit,
        (_, "power-profiles-") => Source::PowerProfiles,
        _ => Source::Other,
    }
}

/// Where a record belongs once its message is known, or `None` to drop it.
///
/// Polkit and pkexec records are kept only when they concern BiGame-mode;
/// kernel records only when they concern graphics or the scheduler; Steam's
/// stream only for the components that write into it. A line in the UI's
/// stream that is not the UI's own (`tracing` wrote it) is attributed the
/// same way, since a game BiGame-mode starts writes there.
fn keep(source: Source, message: &str) -> Option<Source> {
    let lower = message.to_ascii_lowercase();
    match source {
        Source::Kernel => KERNEL_KEYWORDS
            .iter()
            .any(|k| lower.contains(k))
            .then_some(source),
        Source::Polkit => (lower.contains("biglinux")
            || lower.contains("bigame")
            // The install BiGame-mode offers (app.rs), run through pkexec.
            || lower.contains("pacman -s --needed --noconfirm"))
        .then_some(source),
        Source::Steam => component_of(message),
        Source::BiGame if tracing_level(message).is_none() => {
            Some(component_of(message).unwrap_or(source))
        }
        _ => Some(source),
    }
}

/// Parse `journalctl -o json` output, one record per line.
///
/// Returns the entries worth showing ([`keep`]) and the cursor of the last
/// record read, for the next incremental read.
#[must_use]
pub fn parse_journal(output: &str) -> (Vec<Entry>, Option<String>) {
    let mut entries = Vec::new();
    let mut cursor = None;
    for line in output.lines() {
        let Ok(record) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if let Some(c) = record.get("__CURSOR").and_then(serde_json::Value::as_str) {
            cursor = Some(c.to_owned());
        }
        // MESSAGE is a string, or an array of bytes when it is not UTF-8.
        let message = match record.get("MESSAGE") {
            Some(serde_json::Value::String(s)) => s.clone(),
            Some(serde_json::Value::Array(bytes)) => String::from_utf8_lossy(
                &bytes
                    .iter()
                    .filter_map(|b| b.as_u64().and_then(|b| u8::try_from(b).ok()))
                    .collect::<Vec<u8>>(),
            )
            .into_owned(),
            _ => continue,
        };
        let message = strip_ansi(&message);
        let Some(source) = keep(source_of(&record), &message) else {
            continue;
        };
        let priority = record
            .get("PRIORITY")
            .and_then(serde_json::Value::as_str)
            .and_then(|p| p.parse().ok());
        let time_us = record
            .get("__REALTIME_TIMESTAMP")
            .and_then(serde_json::Value::as_str)
            .and_then(|t| t.parse().ok())
            .unwrap_or(0);
        let (level, message) = match tracing_level(&message) {
            // An ordinary line can still report a confirmation.
            Some((Level::Info, text)) => {
                let level = if classify(None, text) == Level::Success {
                    Level::Success
                } else {
                    Level::Info
                };
                (level, text.to_owned())
            }
            Some((level, text)) => (level, text.to_owned()),
            None => (classify(priority, &message), message),
        };
        entries.push(Entry {
            time_us,
            source,
            level,
            message,
        });
    }
    (entries, cursor)
}

/// Read what the journal has, after `cursor` if given.
///
/// One `journalctl` process per call.
///
/// # Errors
/// Returns an error if `journalctl` cannot be run.
pub fn read(lines: u32, cursor: Option<&str>) -> anyhow::Result<(Vec<Entry>, Option<String>)> {
    let output = std::process::Command::new("journalctl")
        .args(journal_args(lines, cursor))
        .output()?;
    Ok(parse_journal(&String::from_utf8_lossy(&output.stdout)))
}

// ── Log files ───────────────────────────────────────────────────────────────

/// The most read from one file at a time. A first read shows the end of the
/// file; later reads continue from where the last one stopped.
pub const FILE_TAIL: u64 = 64 * 1024;

/// A file not written to in this long is not read: the journal read covers
/// the same twelve hours.
const FILE_MAX_AGE: std::time::Duration = std::time::Duration::from_secs(12 * 3600);

/// How a log file is written.
#[derive(Debug, Clone, PartialEq, Eq)]
enum FileFormat {
    /// Steam's `console_log.txt`: `[YYYY-MM-DD HH:MM:SS] text`, local time.
    SteamConsole,
    /// `OptiScaler.log`: `[HH:MM:SS.ffffff] [W] text`, local time, no date;
    /// for the game named.
    OptiScaler(String),
}

/// The log files to read now: Steam's console log in every Steam root, and
/// the `OptiScaler.log` of every game AI Graphics installed `OptiScaler` in —
/// in the game's folder, or the copy AI Graphics kept when it removed it.
fn log_files() -> Vec<(PathBuf, FileFormat)> {
    let home = crate::paths::home_dir();
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for root in [
        home.join(".local/share/Steam"),
        home.join(".steam/steam"),
        home.join(".var/app/com.valvesoftware.Steam/.local/share/Steam"),
    ] {
        // ~/.steam/steam is normally a link to ~/.local/share/Steam.
        if let Ok(path) = root.join("logs/console_log.txt").canonicalize() {
            if seen.insert(path.clone()) {
                out.push((path, FileFormat::SteamConsole));
            }
        }
    }
    let state = crate::graphics::state_dir();
    let Ok(dirs) = std::fs::read_dir(&state) else {
        return out;
    };
    for dir in dirs.flatten() {
        let key = dir.file_name().to_string_lossy().into_owned();
        let Ok(Some(m)) = crate::graphics::manifest::Manifest::load(&state, &key) else {
            continue;
        };
        let title = m.title.clone().or(m.process.clone()).unwrap_or(key.clone());
        for generated in &m.generated {
            // Only a log, only inside the game's folder.
            let safe = generated
                .components()
                .all(|c| matches!(c, Component::Normal(_)));
            let is_log = generated
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("log"));
            if !safe || !is_log {
                continue;
            }
            let live = m.install_root.join(generated);
            let kept = state.join(&key).join("last-run").join(generated);
            let path = if live.is_file() { live } else { kept };
            if path.is_file() && seen.insert(path.clone()) {
                out.push((path, FileFormat::OptiScaler(title.clone())));
            }
        }
    }
    out
}

/// What is new in `path` since `offset`: whole lines only, at most
/// [`FILE_TAIL`] bytes, and the offset to continue from.
///
/// A file shorter than `offset` was rewritten or rotated, and is read from
/// its end again. A line still being written waits for the next read.
fn read_new(path: &Path, offset: Option<u64>) -> Option<(String, u64)> {
    let mut file = std::fs::File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    let from = offset.filter(|&o| o <= len).unwrap_or(0);
    let start = from.max(len.saturating_sub(FILE_TAIL));
    if start >= len {
        return Some((String::new(), len));
    }
    file.seek(std::io::SeekFrom::Start(start)).ok()?;
    let mut buf = Vec::new();
    file.take(len - start).read_to_end(&mut buf).ok()?;
    // Cut into the middle of a line: that line starts before what was read.
    let skip = if start > from || (offset.is_none() && start > 0) {
        buf.iter()
            .position(|&b| b == b'\n')
            .map_or(buf.len(), |i| i + 1)
    } else {
        0
    };
    let end = buf[skip..]
        .iter()
        .rposition(|&b| b == b'\n')
        .map_or(skip, |i| skip + i + 1);
    let text = String::from_utf8_lossy(&buf[skip..end]).into_owned();
    Some((text, start + end as u64))
}

/// A calendar date: year, month, day.
type Date = (i32, u32, u32);
/// A time of day: hours, minutes, seconds.
type Clock = (u32, u32, u32);

/// Local calendar time to seconds since the epoch.
fn local_epoch(date: Date, time: Clock) -> Option<i64> {
    // SAFETY: an all-zero `tm` is a valid value; mktime reads and normalises
    // only the struct it is given.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    tm.tm_year = date.0 - 1900;
    tm.tm_mon = i32::try_from(date.1).ok()? - 1;
    tm.tm_mday = i32::try_from(date.2).ok()?;
    tm.tm_hour = i32::try_from(time.0).ok()?;
    tm.tm_min = i32::try_from(time.1).ok()?;
    tm.tm_sec = i32::try_from(time.2).ok()?;
    // Let the time zone database decide whether summer time applied.
    tm.tm_isdst = -1;
    // SAFETY: `tm` is a valid, exclusively borrowed struct.
    let t = unsafe { libc::mktime(&raw mut tm) };
    (t != -1).then_some(t)
}

/// The local calendar date and time of day of `epoch`.
fn local_date(epoch: i64) -> Option<(Date, Clock)> {
    // SAFETY: an all-zero `tm` is a valid value for localtime_r to overwrite.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    let t: libc::time_t = epoch;
    // SAFETY: both pointers are to valid, exclusively borrowed values.
    if unsafe { libc::localtime_r(&raw const t, &raw mut tm) }.is_null() {
        return None;
    }
    Some((
        (
            tm.tm_year + 1900,
            u32::try_from(tm.tm_mon + 1).ok()?,
            u32::try_from(tm.tm_mday).ok()?,
        ),
        (
            u32::try_from(tm.tm_hour).ok()?,
            u32::try_from(tm.tm_min).ok()?,
            u32::try_from(tm.tm_sec).ok()?,
        ),
    ))
}

fn micros(epoch: i64, fraction_us: u64) -> u64 {
    u64::try_from(epoch).unwrap_or(0) * 1_000_000 + fraction_us
}

/// `[YYYY-MM-DD HH:MM:SS] text` → its date, time and text.
fn steam_console_line(line: &str) -> Option<(Date, Clock, &str)> {
    let rest = line.strip_prefix('[')?;
    let (stamp, text) = rest.split_once("] ")?;
    let (date, time) = stamp.split_once(' ')?;
    let mut d = date.split('-');
    let date = (
        d.next()?.parse().ok()?,
        d.next()?.parse().ok()?,
        d.next()?.parse().ok()?,
    );
    let mut t = time.split(':');
    let time = (
        t.next()?.parse().ok()?,
        t.next()?.parse().ok()?,
        t.next()?.parse().ok()?,
    );
    Some((date, time, text))
}

/// The lines of Steam's console log that say what happened to a game: the
/// command it was started with, the launch's progress, and its exit.
fn steam_console_worth_showing(text: &str) -> bool {
    text.starts_with("Game process added")
        || text.starts_with("Game process removed")
        || (text.starts_with("GameAction [")
            && ["changed task to", "failed", "error"]
                .iter()
                .any(|w| text.contains(w)))
}

fn parse_steam_console(text: &str) -> Vec<Entry> {
    text.lines()
        .filter_map(|line| {
            let (date, time, text) = steam_console_line(line)?;
            if !steam_console_worth_showing(text) {
                return None;
            }
            Some(Entry {
                time_us: micros(local_epoch(date, time)?, 0),
                source: Source::Steam,
                level: classify(None, text),
                message: text.to_owned(),
            })
        })
        .collect()
}

/// `[HH:MM:SS.ffffff] [W] text` → its time of day, microseconds, level
/// letter and text.
fn optiscaler_line(line: &str) -> Option<(Clock, u64, char, &str)> {
    let rest = line.strip_prefix('[')?;
    let (stamp, rest) = rest.split_once("] [")?;
    let (level, text) = rest.split_once(']')?;
    let level = level.chars().next()?;
    let (hms, fraction) = stamp.split_once('.').unwrap_or((stamp, "0"));
    let mut t = hms.split(':');
    let time = (
        t.next()?.parse().ok()?,
        t.next()?.parse().ok()?,
        t.next()?.parse().ok()?,
    );
    // Six digits are microseconds; fewer are scaled up, more cut.
    let digits: String = fraction.chars().take(6).collect();
    let scale = 10u64.pow(u32::try_from(6 - digits.len()).ok()?);
    let micros = digits.parse::<u64>().ok()? * scale;
    Some((time, micros, level, text.trim()))
}

/// `OptiScaler` prints its banner at warning level; it is not a warning.
fn optiscaler_banner(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.chars().all(|c| c == '-')
        || [
            "optiscaler v",
            "freely downloadable",
            "github :",
            "nexus  :",
            "scammed",
            "multiplayer",
        ]
        .iter()
        .any(|w| lower.contains(w))
}

/// How many lines that differ only in their numbers one read keeps.
const REPEATS: usize = 3;

/// A message with its numbers taken out, to tell repeats apart from news.
fn shape(message: &str) -> String {
    message.chars().filter(|c| !c.is_ascii_digit()).collect()
}

/// `OptiScaler.log` for `title`, written up to `modified` (seconds since the
/// epoch). The log has times but no dates; a line whose time is later than
/// the file's last change was written the day before.
fn parse_optiscaler(text: &str, title: &str, modified: i64) -> Vec<Entry> {
    let Some((date, last)) = local_date(modified) else {
        return Vec::new();
    };
    let seconds = |t: (u32, u32, u32)| t.0 * 3600 + t.1 * 60 + t.2;
    let mut out: Vec<Entry> = Vec::new();
    let mut shapes: HashMap<String, usize> = HashMap::new();
    for line in text.lines() {
        let Some((time, fraction, letter, message)) = optiscaler_line(line) else {
            continue;
        };
        let level = match letter {
            'E' | 'C' => Level::Error,
            'W' if optiscaler_banner(message) => Level::Info,
            'W' => Level::Warning,
            'I' => Level::Info,
            // Debug and trace: at those levels the log runs to megabytes.
            _ => continue,
        };
        if message.is_empty() {
            continue;
        }
        let Some(mut epoch) = local_epoch(date, time) else {
            continue;
        };
        if seconds(time) > seconds(last) + 60 {
            epoch -= 86_400;
        }
        // Per-frame warnings repeat for minutes, differing only in a frame
        // number; a few say it.
        let seen = shapes.entry(shape(message)).or_insert(0);
        *seen += 1;
        if *seen > REPEATS {
            continue;
        }
        let message = format!("{title}: {message}");
        out.push(Entry {
            time_us: micros(epoch, fraction),
            source: Source::OptiScaler,
            level,
            message,
        });
    }
    out
}

/// Reads the journal and the log files, each from where it last stopped.
///
/// Kept by the caller between reads, and `Send`, so it can go to a worker
/// thread and come back.
#[derive(Debug, Default)]
pub struct Reader {
    cursor: Option<String>,
    offsets: HashMap<PathBuf, u64>,
}

impl Reader {
    /// Everything new since the last read — the last `lines` journal records
    /// on the first — oldest first.
    ///
    /// # Errors
    /// Returns an error if `journalctl` cannot be run. A log file that
    /// cannot be read is skipped.
    pub fn read(&mut self, lines: u32) -> anyhow::Result<Vec<Entry>> {
        let (mut entries, cursor) = read(lines, self.cursor.as_deref())?;
        if cursor.is_some() {
            self.cursor = cursor;
        }
        for (path, format) in log_files() {
            let Ok(meta) = std::fs::metadata(&path) else {
                continue;
            };
            let modified = meta.modified().ok();
            let offset = self.offsets.get(&path).copied();
            let recent = modified
                .and_then(|m| m.elapsed().ok())
                .is_some_and(|age| age < FILE_MAX_AGE);
            if offset.is_none() && !recent {
                continue;
            }
            let Some((text, next)) = read_new(&path, offset) else {
                continue;
            };
            self.offsets.insert(path, next);
            if text.is_empty() {
                continue;
            }
            entries.extend(match &format {
                FileFormat::SteamConsole => parse_steam_console(&text),
                FileFormat::OptiScaler(title) => {
                    let modified = modified
                        .and_then(|m| m.duration_since(std::time::UNIX_EPOCH).ok())
                        .and_then(|d| i64::try_from(d.as_secs()).ok())
                        .unwrap_or(0);
                    parse_optiscaler(&text, title, modified)
                }
            });
        }
        // The files' lines interleave with the journal's.
        entries.sort_by_key(|e| e.time_us);
        Ok(entries)
    }
}

// ── Writing to the journal ──────────────────────────────────────────────────

/// journald's socket for its native protocol.
const JOURNAL_SOCKET: &str = "/run/systemd/journal/socket";

/// The most of one message sent; journald takes a datagram of a few hundred
/// kilobytes at most, and no log line of ours comes near this.
const MESSAGE_MAX: usize = 48 * 1024;

/// One record in journald's native protocol: `KEY=value` per line, or — for a
/// value with a newline in it — `KEY`, a newline, the value's length as a
/// 64-bit little-endian integer, the value, and a newline.
#[must_use]
pub fn native_record(fields: &[(&str, &str)]) -> Vec<u8> {
    let mut out = Vec::new();
    for (key, value) in fields {
        out.extend_from_slice(key.as_bytes());
        if value.contains('\n') {
            out.push(b'\n');
            out.extend_from_slice(&(value.len() as u64).to_le_bytes());
        } else {
            out.push(b'=');
        }
        out.extend_from_slice(value.as_bytes());
        out.push(b'\n');
    }
    out
}

/// Writes records straight to the journal, as `SYSLOG_IDENTIFIER` given.
///
/// Why not standard output: started from a menu entry the desktop launches
/// as a systemd service, it reaches the journal; launched as a scope (the
/// way most desktops start applications), from a file manager or at login
/// on a desktop without systemd integration, it goes to whatever started it
/// — often nowhere at all. Sent here, the Logs page and a support report see
/// it however BiGame-mode was started.
#[derive(Debug)]
pub struct JournalSink {
    socket: std::os::unix::net::UnixDatagram,
}

impl JournalSink {
    /// A sink, if this system has a journal to send to.
    #[must_use]
    pub fn open() -> Option<Self> {
        if !Path::new(JOURNAL_SOCKET).exists() {
            return None;
        }
        let socket = std::os::unix::net::UnixDatagram::unbound().ok()?;
        Some(Self { socket })
    }

    /// Send one message at syslog `priority` (3 error … 7 debug).
    ///
    /// # Errors
    /// Returns an error if journald does not take the datagram.
    pub fn send(&self, identifier: &str, priority: u8, message: &str) -> std::io::Result<()> {
        let mut end = message.len().min(MESSAGE_MAX);
        while !message.is_char_boundary(end) {
            end -= 1;
        }
        let priority = priority.to_string();
        let record = native_record(&[
            ("MESSAGE", &message[..end]),
            ("PRIORITY", &priority),
            ("SYSLOG_IDENTIFIER", identifier),
        ]);
        self.socket.send_to(&record, JOURNAL_SOCKET).map(|_| ())
    }
}

/// Whether standard output already goes to the journal.
///
/// systemd sets `JOURNAL_STREAM` to the device and inode of the stream it
/// connected, and the variable is inherited, so the descriptor is compared,
/// not just the variable read. The variable can also be wrong: KDE Plasma
/// starts an application as `app-…@….service` with the launcher's own
/// environment in `Environment=`, which overrides the one systemd sets, so
/// the application gets Plasma's `JOURNAL_STREAM` while its standard output
/// is a stream of its own, and every record was written twice. A descriptor
/// connected to journald's stdout socket is therefore the journal whatever the
/// variable says.
#[must_use]
pub fn stdout_is_journal() -> bool {
    connected_to_journal(libc::STDOUT_FILENO) || matches_journal_stream(libc::STDOUT_FILENO)
}

fn matches_journal_stream(fd: libc::c_int) -> bool {
    let Ok(stream) = std::env::var("JOURNAL_STREAM") else {
        return false;
    };
    let Some((dev, ino)) = stream.split_once(':') else {
        return false;
    };
    let (Ok(dev), Ok(ino)) = (dev.parse::<u64>(), ino.parse::<u64>()) else {
        return false;
    };
    // SAFETY: an all-zero `stat` is a valid value for fstat to overwrite.
    let mut st: libc::stat = unsafe { std::mem::zeroed() };
    // SAFETY: fstat on a descriptor number writes only into `st`.
    if unsafe { libc::fstat(fd, &raw mut st) } != 0 {
        return false;
    }
    st.st_dev == dev && st.st_ino == ino
}

/// Whether `fd` is a Unix stream connected to journald's stdout socket.
fn connected_to_journal(fd: libc::c_int) -> bool {
    const STDOUT_SOCKET: &[u8] = b"/run/systemd/journal/stdout";
    // SAFETY: an all-zero `sockaddr_un` is a valid value for getpeername to
    // overwrite.
    let mut addr: libc::sockaddr_un = unsafe { std::mem::zeroed() };
    let Ok(mut len) = libc::socklen_t::try_from(std::mem::size_of::<libc::sockaddr_un>()) else {
        return false;
    };
    // SAFETY: getpeername writes at most `len` bytes into `addr` and stores
    // the length it wrote in `len`; a descriptor that is not a connected
    // socket makes it fail without writing.
    if unsafe { libc::getpeername(fd, (&raw mut addr).cast(), &raw mut len) } != 0 {
        return false;
    }
    if libc::c_int::from(addr.sun_family) != libc::AF_UNIX {
        return false;
    }
    let path: Vec<u8> = addr
        .sun_path
        .iter()
        .take_while(|c| **c != 0)
        .map(|c| c.to_ne_bytes()[0])
        .collect();
    path == STDOUT_SOCKET
}

/// Mask personal data in text meant to leave the machine.
///
/// The home directory becomes `~`, the user name `<user>`, and the host name
/// `<host>`. Applied to exports, not to what is shown on screen.
#[must_use]
pub fn redact(text: &str, home: &str, user: &str, host: &str) -> String {
    let mut out = text.to_owned();
    if !home.is_empty() {
        out = out.replace(home, "~");
    }
    for (value, mask) in [(host, "<host>"), (user, "<user>")] {
        // Short names would mask ordinary words; only mask what is specific.
        if value.len() >= 3 {
            out = out.replace(value, mask);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stream_connected_to_journald_is_the_journal_whatever_the_variable() {
        use std::os::fd::AsRawFd as _;
        // A descriptor that is not connected to journald is not.
        let (a, _b) = std::os::unix::net::UnixStream::pair().unwrap();
        assert!(!connected_to_journal(a.as_raw_fd()));
        // One that is, is: what an application started by KDE Plasma has on
        // standard output while its JOURNAL_STREAM names Plasma's stream.
        // Only where journald runs (not in a build chroot).
        if let Ok(stream) = std::os::unix::net::UnixStream::connect("/run/systemd/journal/stdout") {
            assert!(connected_to_journal(stream.as_raw_fd()));
        }
    }

    #[test]
    fn a_tracing_line_keeps_its_own_level() {
        let (level, text) =
            tracing_level(" INFO turbo: turbo on verified=1 per_game=1 skipped=0 failed=0")
                .unwrap();
        assert_eq!(level, Level::Info);
        assert_eq!(
            text,
            "turbo: turbo on verified=1 per_game=1 skipped=0 failed=0"
        );
        let (level, text) = tracing_level(
            "2026-09-24T15:03:02.350657Z  WARN bigame_daemon::polkit: denied by policy",
        )
        .unwrap();
        assert_eq!(level, Level::Warning);
        assert_eq!(text, "bigame_daemon::polkit: denied by policy");
        assert_eq!(tracing_level("ERROR x: y").unwrap().0, Level::Error);
        assert!(tracing_level("info(daemon): activating profile 'supertuxkart'").is_none());
        assert!(tracing_level("Failed to set text from markup").is_none());
    }

    #[test]
    fn falcond_warnings_inside_info_records_are_warnings() {
        // A real falcond line, logged at priority 6.
        assert_eq!(
            classify(
                Some(6),
                "warning(daemon): failed to switch scx scheduler: error.MethodCallFailed"
            ),
            Level::Error,
            "a failure is an error whatever priority it was logged at"
        );
        assert_eq!(
            classify(
                Some(6),
                "warning(dbus): D-Bus error: org.freedesktop.DBus.Error.ServiceUnknown"
            ),
            Level::Error
        );
        assert_eq!(
            classify(Some(6), "warning(config): no system.conf"),
            Level::Warning
        );
        assert_eq!(
            classify(
                Some(6),
                "info(daemon): activating profile 'Proton' (scx=lavd)"
            ),
            Level::Success
        );
        assert_eq!(
            classify(Some(6), "info(daemon): reloaded 11 profiles"),
            Level::Info
        );
        assert_eq!(classify(Some(3), "anything"), Level::Error);
        assert_eq!(classify(Some(4), "anything"), Level::Warning);
    }

    #[test]
    fn a_masked_gvfs_monitor_is_not_an_error() {
        // A real line, printed by GVfs inside bigame-ui at priority 6.
        let gvfs = "invoking IsSupported() failed for remote volume monitor with dbus name \
                    org.gtk.vfs.GPhoto2VolumeMonitor:: GDBus.Error:org.freedesktop.DBus.Error.\
                    NameHasNoOwner: Could not activate remote peer \
                    'org.gtk.vfs.GPhoto2VolumeMonitor': activation request failed: unit is \
                    masked (g-dbus-error-quark, 3)";
        assert_eq!(classify(Some(6), gvfs), Level::Debug);
        assert_eq!(
            classify(Some(3), gvfs),
            Level::Error,
            "the priority still wins"
        );
        assert_eq!(
            classify(Some(6), "thread 'main' panicked at src/x.rs:1:1"),
            Level::Error
        );
    }

    #[test]
    fn the_levels_of_game_components_are_read_from_their_own_prefixes() {
        // Real lines from Steam's stream.
        assert_eq!(
            classify(
                Some(6),
                "[gamescope] [Error] xdg_backend: Compositor released us but we were not acquired. Oh no."
            ),
            Level::Error
        );
        assert_eq!(
            classify(
                Some(6),
                "[2026-09-27 17:28:31.181] [MANGOHUD] [warning] [vulkan.cpp:1] Present mode is not supported"
            ),
            Level::Warning
        );
        assert_eq!(
            classify(
                Some(6),
                "0180:err:module:import_dll Library Qt5Pdf.dll not found"
            ),
            Level::Error
        );
        assert_eq!(
            classify(
                Some(6),
                "00d4:fixme:wineusb:add_usb_device Interface 1 has 2 alternate settings"
            ),
            Level::Debug
        );
        assert_eq!(
            classify(Some(6), "vkBasalt info: effects = cas"),
            Level::Info
        );
        assert_eq!(
            classify(
                Some(6),
                "pam_unix(sudo:session): session opened for user ruscher(uid=1000) by (uid=0)"
            ),
            Level::Debug
        );
    }

    #[test]
    fn a_games_output_is_attributed_to_the_component_that_wrote_it() {
        for (line, source) in [
            (
                "[gamescope] [Info]  wlserver: Running compositor on wayland display 'gamescope-0'",
                Source::Gamescope,
            ),
            (
                "[Gamescope WSI] Forcing on VK_EXT_swapchain_maintenance1.",
                Source::Gamescope,
            ),
            (
                "[2026-09-27 17:25:33.479] [MANGOHUD] [info] [gpu.cpp:90] Set renderD128 as active GPU",
                Source::MangoHud,
            ),
            (
                "vkBasalt info: config file: /home/u/.config/vkBasalt/vkBasalt.conf",
                Source::VkBasalt,
            ),
            ("lsfg-vk: frame generation enabled", Source::Lsfg),
            ("wineserver: NTSync up and running!", Source::Wine),
            (
                "pressure-vessel-wrap[1234]: W: Extra environment variable WAYLAND_DISPLAY set",
                Source::Wine,
            ),
        ] {
            assert_eq!(component_of(line), Some(source), "{line}");
        }
        // The client's own chatter belongs to none of them.
        assert_eq!(component_of("Manifest download: send request"), None);
    }

    #[test]
    fn an_nvidia_xid_is_shown_as_an_error() {
        // As the lab laptop's kernel logged them, both at warning priority.
        let journal = r#"{"__CURSOR":"s=1","__REALTIME_TIMESTAMP":"1790200000000000","_TRANSPORT":"kernel","PRIORITY":"4","MESSAGE":"NVRM: GPU at PCI:0000:01:00: GPU-dbe116b8-a325-0daf-c382-c87c94a4bf0b"}
{"__CURSOR":"s=2","__REALTIME_TIMESTAMP":"1790200000000001","_TRANSPORT":"kernel","PRIORITY":"4","MESSAGE":"NVRM: Xid (PCI:0000:01:00): 32, pid=177594, name=SOTTR.exe, channel 0x00000023 intr 00040000"}
"#;
        let (entries, _) = parse_journal(journal);
        assert_eq!(entries.len(), 2, "{entries:#?}");
        assert!(entries.iter().all(|e| e.source == Source::Kernel));
        assert_eq!(entries[0].level, Level::Warning);
        assert_eq!(entries[1].level, Level::Error);
    }

    #[test]
    fn records_are_attributed_and_filtered() {
        let journal = r#"{"__CURSOR":"s=1","__REALTIME_TIMESTAMP":"1790200000000000","_SYSTEMD_UNIT":"falcond.service","PRIORITY":"6","MESSAGE":"info(daemon): activating profile 'Proton'"}
{"__CURSOR":"s=2","__REALTIME_TIMESTAMP":"1790200000000001","_TRANSPORT":"kernel","PRIORITY":"4","MESSAGE":"amdgpu 0000:03:00.0: ring gfx timeout"}
{"__CURSOR":"s=3","__REALTIME_TIMESTAMP":"1790200000000002","_TRANSPORT":"kernel","PRIORITY":"6","MESSAGE":"usb 1-2: new device"}
{"__CURSOR":"s=4","__REALTIME_TIMESTAMP":"1790200000000003","SYSLOG_IDENTIFIER":"bigame-ui","PRIORITY":"6","MESSAGE":"\u001b[32m INFO\u001b[0m turbo on verified=1"}
{"__CURSOR":"s=5","__REALTIME_TIMESTAMP":"1790200000000004","SYSLOG_IDENTIFIER":"polkitd","PRIORITY":"5","MESSAGE":"Registered Authentication Agent for unix-session:2"}
"#;
        let (entries, cursor) = parse_journal(journal);
        assert_eq!(
            cursor.as_deref(),
            Some("s=5"),
            "the last record's cursor, even if filtered"
        );
        assert_eq!(entries.len(), 3, "{entries:#?}");
        assert_eq!(entries[0].source, Source::Falcond);
        assert_eq!(entries[1].source, Source::Kernel);
        assert_eq!(entries[1].level, Level::Warning);
        assert_eq!(entries[2].source, Source::BiGame);
        assert!(
            !entries[2].message.contains('\u{1b}'),
            "ANSI colours are stripped"
        );
    }

    #[test]
    fn steam_power_profiles_crashes_and_units_are_attributed() {
        let journal = r#"{"__CURSOR":"s=1","__REALTIME_TIMESTAMP":"1","SYSLOG_IDENTIFIER":"steam","_COMM":"srt-logger","PRIORITY":"6","MESSAGE":"[2026-09-27 18:29:57] Manifest download: send request"}
{"__CURSOR":"s=2","__REALTIME_TIMESTAMP":"2","SYSLOG_IDENTIFIER":"steam","_COMM":"srt-logger","PRIORITY":"6","MESSAGE":"vkBasalt info: effects = cas"}
{"__CURSOR":"s=3","__REALTIME_TIMESTAMP":"3","SYSLOG_IDENTIFIER":"power-profiles-daemon","_COMM":"power-profiles-","_SYSTEMD_UNIT":"power-profiles-daemon-biglinux-start.service","PRIORITY":"6","MESSAGE":"Switching to profile performance"}
{"__CURSOR":"s=4","__REALTIME_TIMESTAMP":"4","SYSLOG_IDENTIFIER":"systemd","_SYSTEMD_UNIT":"init.scope","UNIT":"falcond.service","PRIORITY":"3","MESSAGE":"falcond.service: Main process exited, code=dumped, status=11/SEGV"}
{"__CURSOR":"s=5","__REALTIME_TIMESTAMP":"5","SYSLOG_IDENTIFIER":"systemd-coredump","COREDUMP_COMM":"bigame-ui","PRIORITY":"2","MESSAGE":"Process 1234 (bigame-ui) of user 1000 dumped core."}
{"__CURSOR":"s=6","__REALTIME_TIMESTAMP":"6","SYSLOG_IDENTIFIER":"pkexec","PRIORITY":"5","MESSAGE":"ruscher: Executing command [USER=root] [COMMAND=/usr/bin/pacman -S --needed --noconfirm vkbasalt]"}
{"__CURSOR":"s=7","__REALTIME_TIMESTAMP":"7","SYSLOG_IDENTIFIER":"pkexec","PRIORITY":"5","MESSAGE":"ruscher: Executing command [USER=root] [COMMAND=/usr/bin/something-else]"}
{"__CURSOR":"s=8","__REALTIME_TIMESTAMP":"8","SYSLOG_IDENTIFIER":"bigame-ui","PRIORITY":"6","MESSAGE":"[gamescope] [Info]  vblank: Using timerfd."}
"#;
        let (entries, _) = parse_journal(journal);
        let got: Vec<(Source, Level)> = entries.iter().map(|e| (e.source, e.level)).collect();
        assert_eq!(
            got,
            [
                (Source::VkBasalt, Level::Info),
                (Source::PowerProfiles, Level::Info),
                (Source::Falcond, Level::Error),
                (Source::BiGame, Level::Error),
                (Source::Polkit, Level::Info),
                (Source::Gamescope, Level::Info),
            ],
            "{entries:#?}"
        );
    }

    #[test]
    fn the_query_ors_every_source_and_reads_incrementally() {
        let args = journal_args(200, Some("s=abc"));
        assert!(args.contains(&"--after-cursor=s=abc".to_owned()));
        assert!(args.contains(&"_TRANSPORT=kernel".to_owned()));
        assert!(args.contains(&"SYSLOG_IDENTIFIER=steam".to_owned()));
        assert!(
            args.iter()
                .any(|a| a.starts_with("--output-fields=MESSAGE,"))
        );
        let pluses = args.iter().filter(|a| *a == "+").count();
        let matches = args
            .iter()
            .filter(|a| a.contains('=') && !a.starts_with("--"))
            .count();
        assert_eq!(pluses, matches - 1);
        assert_eq!(matches, JOURNAL_MATCHES.len());
    }

    #[test]
    fn every_label_fits_the_column() {
        for source in [
            Source::BiGame,
            Source::Helper,
            Source::Falcond,
            Source::Kernel,
            Source::Gamescope,
            Source::Scheduler,
            Source::PowerProfiles,
            Source::Polkit,
            Source::Steam,
            Source::Wine,
            Source::MangoHud,
            Source::VkBasalt,
            Source::Lsfg,
            Source::OptiScaler,
            Source::Other,
        ] {
            assert!(source.label().len() <= LABEL_WIDTH, "{source:?}");
        }
    }

    #[test]
    fn steams_console_log_gives_launches_and_exits() {
        let text = "[2026-09-27 17:25:30] Game process added : AppID 750920 \"MANGOHUD=1 gamescope -W 3440 -- %command%\", ProcID 1499703, IP 0.0.0.0:0\n\
                    [2026-09-27 17:25:30] GameAction [AppID 750920, ActionID 2] : LaunchApp changed task to WaitingGameWindow with \"\"\n\
                    [2026-09-27 17:25:31] Loaded Config for Local Override Path for App ID 413080, Controller 0\n\
                    [2026-09-27 17:34:24] Game process removed: AppID 750920 \"MANGOHUD=1 gamescope\", ProcID 1499703\n\
                    not a line of Steam's\n";
        let entries = parse_steam_console(text);
        assert_eq!(entries.len(), 3, "{entries:#?}");
        assert!(entries.iter().all(|e| e.source == Source::Steam));
        assert!(entries[0].message.starts_with("Game process added"));
        assert!(entries[2].time_us > entries[0].time_us);
        assert_eq!(
            steam_console_line("[2026-09-27 17:25:30] x"),
            Some(((2026, 9, 27), (17, 25, 30), "x"))
        );
    }

    #[test]
    fn local_times_round_trip() {
        let epoch = local_epoch((2026, 9, 27), (17, 25, 30)).unwrap();
        assert_eq!(local_date(epoch), Some(((2026, 9, 27), (17, 25, 30))));
    }

    #[test]
    fn optiscaler_logs_keep_their_levels_and_drop_debug_and_floods() {
        let modified = local_epoch((2026, 9, 27), (16, 6, 5)).unwrap();
        let text = "[15:59:15.281895] [W] OptiScaler v0.9.4-final (7534ad0) loaded\n\
                    [15:59:15.281938] [W] ---------------------------------\n\
                    [15:59:15.281969] [W] DO NOT USE IN MULTIPLAYER GAMES\n\
                    [15:59:15.281974] [I] \n\
                    [15:59:15.281980] [D] Some debug detail\n\
                    [16:02:38.230586] [W] FSRFG_Dx12::DispatchCallback Dispatched with the same frame id! frameID: 23976\n\
                    [16:02:38.230588] [W] FSRFG_Dx12::DispatchCallback Dispatched with the same frame id! frameID: 23978\n\
                    [16:02:38.230589] [W] FSRFG_Dx12::DispatchCallback Dispatched with the same frame id! frameID: 23980\n\
                    [16:02:38.230590] [W] FSRFG_Dx12::DispatchCallback Dispatched with the same frame id! frameID: 23982\n\
                    [16:03:00.5] [E] Hooking failed\n\
                    [16:06:05.734852] [I] Unloading OptiScaler\n";
        let entries = parse_optiscaler(text, "SOTTR", modified);
        let got: Vec<(Level, &str)> = entries
            .iter()
            .map(|e| (e.level, e.message.as_str()))
            .collect();
        assert_eq!(
            got,
            [
                (
                    Level::Info,
                    "SOTTR: OptiScaler v0.9.4-final (7534ad0) loaded"
                ),
                (Level::Info, "SOTTR: ---------------------------------"),
                (Level::Info, "SOTTR: DO NOT USE IN MULTIPLAYER GAMES"),
                (
                    Level::Warning,
                    "SOTTR: FSRFG_Dx12::DispatchCallback Dispatched with the same frame id! frameID: 23976"
                ),
                (
                    Level::Warning,
                    "SOTTR: FSRFG_Dx12::DispatchCallback Dispatched with the same frame id! frameID: 23978"
                ),
                (
                    Level::Warning,
                    "SOTTR: FSRFG_Dx12::DispatchCallback Dispatched with the same frame id! frameID: 23980"
                ),
                (Level::Error, "SOTTR: Hooking failed"),
                (Level::Info, "SOTTR: Unloading OptiScaler"),
            ]
        );
        let first = local_epoch((2026, 9, 27), (15, 59, 15)).unwrap();
        assert_eq!(
            entries[0].time_us,
            u64::try_from(first).unwrap() * 1_000_000 + 281_895
        );
        assert_eq!(entries[6].time_us % 1_000_000, 500_000);
        // Written before midnight, last changed after it.
        let after_midnight = local_epoch((2026, 9, 28), (0, 10, 0)).unwrap();
        let late = parse_optiscaler("[23:50:00.000000] [E] x\n", "g", after_midnight);
        assert_eq!(
            late[0].time_us,
            u64::try_from(local_epoch((2026, 9, 27), (23, 50, 0)).unwrap()).unwrap() * 1_000_000
        );
    }

    #[test]
    fn files_are_read_in_whole_lines_from_where_the_last_read_stopped() {
        use std::io::Write as _;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("log.txt");
        std::fs::write(&path, "one\ntwo\nthr").unwrap();
        let (text, offset) = read_new(&path, None).unwrap();
        assert_eq!(text, "one\ntwo\n", "a line still being written waits");
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();
        file.write_all(b"ee\nfour\n").unwrap();
        let (text, offset) = read_new(&path, Some(offset)).unwrap();
        assert_eq!(text, "three\nfour\n");
        assert_eq!(read_new(&path, Some(offset)).unwrap().0, "");
        // Rewritten shorter: read again from the start.
        std::fs::write(&path, "new\n").unwrap();
        assert_eq!(read_new(&path, Some(offset)).unwrap().0, "new\n");
    }

    #[test]
    fn a_large_file_is_read_from_its_end_only() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("big.log");
        let line = "x".repeat(99) + "\n";
        let lines = usize::try_from(FILE_TAIL).unwrap() / line.len() * 3;
        std::fs::write(&path, line.repeat(lines)).unwrap();
        let (text, offset) = read_new(&path, None).unwrap();
        assert!(text.len() as u64 <= FILE_TAIL);
        assert!(text.lines().all(|l| l.len() == 99), "whole lines only");
        assert_eq!(offset, std::fs::metadata(&path).unwrap().len());
    }

    #[test]
    fn native_records_frame_multi_line_values() {
        assert_eq!(
            native_record(&[("MESSAGE", "hi"), ("PRIORITY", "6")]),
            b"MESSAGE=hi\nPRIORITY=6\n"
        );
        let mut want = b"MESSAGE\n".to_vec();
        want.extend_from_slice(&3u64.to_le_bytes());
        want.extend_from_slice(b"a\nb\n");
        assert_eq!(native_record(&[("MESSAGE", "a\nb")]), want);
    }

    #[test]
    fn exports_mask_personal_data() {
        let text = "saved /home/ruscher/.local/state/x by ruscher on ruscher-big";
        let out = redact(text, "/home/ruscher", "ruscher", "ruscher-big");
        assert_eq!(out, "saved ~/.local/state/x by <user> on <host>");
        // A two-letter user name would mask every word containing it.
        assert_eq!(redact("go to bo", "", "bo", ""), "go to bo");
    }
}
