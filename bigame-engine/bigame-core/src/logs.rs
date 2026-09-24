//! Everything worth reading about a game session, from one place: the journal.
//!
//! Every component involved already logs there — falcond, the helper, the UI,
//! power-profiles-daemon, `scx_loader`, Polkit, Gamescope, and the kernel's
//! DRM and GPU drivers — so one `journalctl -o json` call reads them all.
//! The Logs page previously ran three `journalctl` processes (one a
//! `--grep` over the whole journal) and `dmesg` every five seconds, forever,
//! whether or not it was on screen; `dmesg` usually fails for a normal user
//! anyway (`kernel.dmesg_restrict`), and the journal's kernel records do not.
//!
//! Refreshes are incremental: the last entry's cursor is kept, and the next
//! read asks only for what came after it.
//!
//! Severity comes from the journal's own `PRIORITY` first, then from the
//! message for tools that log everything at one level (falcond writes
//! `warning(dbus):` inside info-priority records).

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
    /// `scx_loader`.
    Scheduler,
    /// power-profiles-daemon.
    PowerProfiles,
    /// Polkit.
    Polkit,
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
            Self::Other => "system",
        }
    }
}

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

/// Kernel lines worth showing: graphics drivers and the scheduler.
const KERNEL_KEYWORDS: &[&str] = &[
    "amdgpu",
    "radeon",
    "nvidia",
    "nouveau",
    "i915",
    " xe ",
    "drm",
    "gpu",
    "sched_ext",
    "scx",
];

/// The journal matches for every source, OR-ed with `+`.
///
/// Kernel records are included whole and filtered by [`KERNEL_KEYWORDS`]
/// here, because journald matches fields exactly and cannot select by
/// message content without `--grep`, which scans everything.
#[must_use]
pub fn journal_args(lines: u32, after_cursor: Option<&str>) -> Vec<String> {
    let mut args: Vec<String> = vec![
        "--no-pager".into(),
        "--output=json".into(),
        "--since=-12h".into(),
        format!("--lines={lines}"),
    ];
    if let Some(cursor) = after_cursor {
        args.push(format!("--after-cursor={cursor}"));
    }
    let matches = [
        "_SYSTEMD_UNIT=falcond.service",
        "_SYSTEMD_UNIT=bigame-daemon.service",
        "SYSLOG_IDENTIFIER=bigame-ui",
        "_SYSTEMD_UNIT=scx_loader.service",
        "_SYSTEMD_UNIT=power-profiles-daemon.service",
        "SYSLOG_IDENTIFIER=gamescope",
        "SYSLOG_IDENTIFIER=polkitd",
        "_TRANSPORT=kernel",
    ];
    for (i, m) in matches.iter().enumerate() {
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

/// The severity of a message, from the journal priority and its wording.
#[must_use]
pub fn classify(priority: Option<u8>, message: &str) -> Level {
    let lower = message.to_ascii_lowercase();
    let has = |words: &[&str]| words.iter().any(|w| lower.contains(w));
    if priority.is_some_and(|p| p <= 3)
        || has(&[
            " error", "error(", "error:", "failed", "failure", "panic", "critical", "fatal",
        ])
        || lower.starts_with("error")
    {
        return Level::Error;
    }
    if priority == Some(4) || has(&["warning", " warn", "warn(", "warn:", "alert", "denied"]) {
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

fn source_of(record: &serde_json::Value) -> Source {
    let field = |k: &str| {
        record
            .get(k)
            .and_then(serde_json::Value::as_str)
            .unwrap_or("")
    };
    match (
        field("_SYSTEMD_UNIT"),
        field("SYSLOG_IDENTIFIER"),
        field("_TRANSPORT"),
    ) {
        (_, _, "kernel") => Source::Kernel,
        ("falcond.service", _, _) => Source::Falcond,
        ("bigame-daemon.service", _, _) => Source::Helper,
        (_, "bigame-ui", _) => Source::BiGame,
        ("scx_loader.service", _, _) => Source::Scheduler,
        ("power-profiles-daemon.service", _, _) => Source::PowerProfiles,
        (_, "gamescope", _) => Source::Gamescope,
        (_, "polkitd", _) => Source::Polkit,
        _ => Source::Other,
    }
}

/// Parse `journalctl -o json` output, one record per line.
///
/// Returns the entries worth showing and the cursor of the last record read,
/// for the next incremental read. Polkit records are kept only when they
/// concern BiGame-mode; kernel records only when they concern graphics or the
/// scheduler.
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
        let source = source_of(&record);
        let lower = message.to_ascii_lowercase();
        if source == Source::Kernel && !KERNEL_KEYWORDS.iter().any(|k| lower.contains(k)) {
            continue;
        }
        if source == Source::Polkit && !lower.contains("biglinux") && !lower.contains("bigame") {
            continue;
        }
        let priority = record
            .get("PRIORITY")
            .and_then(serde_json::Value::as_str)
            .and_then(|p| p.parse().ok());
        let time_us = record
            .get("__REALTIME_TIMESTAMP")
            .and_then(serde_json::Value::as_str)
            .and_then(|t| t.parse().ok())
            .unwrap_or(0);
        entries.push(Entry {
            time_us,
            source,
            level: classify(priority, &message),
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
    fn falcond_warnings_inside_info_records_are_warnings() {
        // What falcond actually wrote on the reference machine, at priority 6.
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
    fn the_query_ors_every_source_and_reads_incrementally() {
        let args = journal_args(200, Some("s=abc"));
        assert!(args.contains(&"--after-cursor=s=abc".to_owned()));
        assert!(args.contains(&"_TRANSPORT=kernel".to_owned()));
        let pluses = args.iter().filter(|a| *a == "+").count();
        let matches = args
            .iter()
            .filter(|a| a.contains('=') && !a.starts_with("--"))
            .count();
        assert_eq!(pluses, matches - 1);
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
