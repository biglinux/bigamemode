//! A game that closed with an error, told apart from one the player quit.
//!
//! Big Game Mode is not the game's parent, so it never sees an exit status.
//! What a crash leaves behind can be read, though, and only what was left
//! while the game ran counts — an old report never makes a new crash:
//!
//! - the game's own crash report, written the moment it crashed: `REDengine`'s
//!   `ReportQueue` (Cyberpunk 2077), Unreal Engine's `Saved/Crashes`, Unity's
//!   `Crashes` under the prefix's `Temp`;
//! - a crash reporter still open when the game is gone (Unreal's
//!   `CrashReportClient.exe`, `REDengine`'s `CrashReporter.exe`), or Wine's
//!   debugger attached to it;
//! - systemd-coredump's record of the game's process killed by a signal.
//!
//! A crash is said with what was in the game while it ran ([`Extra`]),
//! read from the game itself ([`Session`]) and never from settings alone:
//! a Vulkan layer counts only when it was mapped in the game. Seen on the
//! reference desktop (2026-10-18): Cyberpunk 2077 stopped after 25 minutes on
//! an access violation in its own scripts, with the Pretty preset's vkBasalt
//! and FSR 4 in it. Nothing says those caused it, and nothing says they did
//! not; what is in the player's hands is to leave them out of that one game
//! and see ([`leave_out`]), which is offered rather than done.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::running::GameIdentity;
use crate::text::{N_, Text};

/// Something in the game that the game itself did not bring: put there by
/// Big Game Mode, or by a choice made in it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Extra {
    /// vkBasalt's post-processing layer.
    VkBasalt,
    /// Proton's FSR 4 in place of the game's FSR 3.1.
    Fsr4Upgrade,
    /// Wine's fullscreen FSR upscaling.
    WineFsr,
    /// `MangoHud`'s overlay.
    MangoHud,
    /// lsfg-vk's frame generation.
    FrameGeneration,
    /// `OptiScaler`, from AI Graphics.
    OptiScaler,
    /// Gamescope around the game.
    Gamescope,
}

impl Extra {
    /// Its name, translatable.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::VkBasalt => N_("vkBasalt sharpening"),
            Self::Fsr4Upgrade => N_("FSR 4 in place of the game's FSR 3.1"),
            Self::WineFsr => N_("Wine FSR"),
            Self::MangoHud => N_("MangoHud overlay"),
            Self::FrameGeneration => N_("lsfg-vk frame generation"),
            Self::OptiScaler => N_("OptiScaler (AI Graphics)"),
            Self::Gamescope => N_("Gamescope"),
        }
    }

    /// Whether [`leave_out`] can keep it out of one game: a switch in the
    /// game's own launch settings that wins over the session's. The others
    /// are the game's own choices, each changed where it was made (Profiles,
    /// AI Graphics).
    #[must_use]
    pub fn can_leave_out(self) -> bool {
        matches!(self, Self::VkBasalt | Self::Fsr4Upgrade | Self::WineFsr)
    }
}

/// What was read of a game while it ran, for [`examine`] once it is gone.
#[derive(Debug, Clone)]
pub struct Session {
    /// The game's title.
    pub game: String,
    /// Its process name: a profile's key.
    pub process: String,
    /// The game's process.
    pub pid: u32,
    started: SystemTime,
    /// The Wine prefix: the folder that holds `drive_c`.
    prefix: Option<PathBuf>,
    install: Option<PathBuf>,
    /// A Windows game, under Proton or Wine.
    windows: bool,
    extras: BTreeSet<Extra>,
    /// The layers were read from the game's memory map ([`Self::look_inside`]),
    /// which replaces what its environment only asked for.
    looked_inside: bool,
}

impl Session {
    /// Start one for `game`, which has just been found running: when it
    /// started, its prefix, and what its environment asks for.
    #[must_use]
    pub fn begin(game: &GameIdentity) -> Self {
        let started = crate::running::running_for(game.pid)
            .and_then(|age| SystemTime::now().checked_sub(Duration::from_secs(age)))
            .unwrap_or_else(SystemTime::now);
        let environ = std::fs::read(format!("/proc/{}/environ", game.pid)).unwrap_or_default();
        let env = Environ(&environ);
        let prefix = game
            .compatdata_path
            .as_deref()
            .and_then(wine_prefix)
            .or_else(|| {
                env.get("WINEPREFIX")
                    .map(PathBuf::from)
                    .as_deref()
                    .and_then(wine_prefix)
            })
            .or_else(|| {
                env.get("STEAM_COMPAT_DATA_PATH")
                    .map(PathBuf::from)
                    .as_deref()
                    .and_then(wine_prefix)
            });
        Self {
            game: game.display_name.clone(),
            process: game.process_name.clone(),
            pid: game.pid,
            started,
            prefix,
            install: game.install_path.clone(),
            windows: game.runtime != crate::running::Runtime::Native,
            extras: env.extras(),
            looked_inside: false,
        }
    }

    /// Read which layers the game has really loaded, from its memory map: a
    /// game maps them once its graphics start, so this is for a minute or so
    /// into the game, and once.
    pub fn look_inside(&mut self, game: &GameIdentity) {
        if game.pid != self.pid || self.looked_inside {
            return;
        }
        let inside = crate::running::in_game(game);
        let layers = [
            (
                Extra::VkBasalt,
                inside.vkbasalt || inside.vkbasalt_in_gamescope,
            ),
            (Extra::MangoHud, inside.mangohud),
            (Extra::FrameGeneration, inside.frame_generation.is_some()),
            (Extra::Gamescope, inside.gamescope),
        ];
        for (extra, mapped) in layers {
            if mapped {
                self.extras.insert(extra);
            } else {
                self.extras.remove(&extra);
            }
        }
        self.looked_inside = true;
    }

    /// Whether [`Self::look_inside`] has run.
    #[must_use]
    pub fn looked_inside(&self) -> bool {
        self.looked_inside
    }
}

/// The Wine prefix at or under `dir`: a Proton `compatdata/<id>` holds it in
/// `pfx`.
fn wine_prefix(dir: &Path) -> Option<PathBuf> {
    [dir.join("pfx"), dir.to_path_buf()]
        .into_iter()
        .find(|p| p.join("drive_c").is_dir())
}

/// A process's environment block (`/proc/<pid>/environ`).
struct Environ<'a>(&'a [u8]);

impl Environ<'_> {
    fn get(&self, key: &str) -> Option<&str> {
        self.0
            .split(|b| *b == 0)
            .filter_map(|e| std::str::from_utf8(e).ok())
            .find_map(|e| e.strip_prefix(key)?.strip_prefix('='))
    }

    /// Set, and neither empty nor `0`.
    fn on(&self, key: &str) -> bool {
        self.get(key).is_some_and(|v| !v.is_empty() && v != "0")
    }

    /// What the environment asks the game to load. The layers are a guess
    /// until the memory map is read; the rest is read by Proton and Wine
    /// from here and nowhere else.
    fn extras(&self) -> BTreeSet<Extra> {
        let mut found = BTreeSet::new();
        // Valve's Proton upgrades only at `1`; GE-Proton at anything but `0`.
        if self.get("FSR4_UPGRADE") == Some("1") || self.on("PROTON_FSR4_UPGRADE") {
            found.insert(Extra::Fsr4Upgrade);
        }
        if self.on("WINE_FULLSCREEN_FSR") {
            found.insert(Extra::WineFsr);
        }
        // vkBasalt's layer turns on at exactly `1`, and its off switch wins.
        if self.get("ENABLE_VKBASALT") == Some("1") && self.get("DISABLE_VKBASALT") != Some("1") {
            found.insert(Extra::VkBasalt);
        }
        if self.on("MANGOHUD")
            || self
                .get("LD_PRELOAD")
                .is_some_and(|p| p.to_ascii_lowercase().contains("mangohud"))
        {
            found.insert(Extra::MangoHud);
        }
        if self.get("GAMESCOPE_WAYLAND_DISPLAY").is_some() {
            found.insert(Extra::Gamescope);
        }
        found
    }
}

/// A game that closed with an error.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Crash {
    /// The game's title.
    pub game: String,
    /// Its process name.
    pub process: String,
    /// When it was found gone, in seconds since the epoch.
    pub at: u64,
    /// How long it had run, in seconds.
    pub played_secs: u64,
    /// What showed it was a crash, translatable.
    pub evidence: Text,
    /// What the game itself said, as it said it (usually English).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// The game's own crash report, to open.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub report: Option<PathBuf>,
    /// What was in the game that it did not bring itself.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extras: Vec<Extra>,
}

impl Crash {
    /// The extras [`leave_out`] can keep out of this game.
    #[must_use]
    pub fn removable(&self) -> Vec<Extra> {
        self.extras
            .iter()
            .copied()
            .filter(|e| e.can_leave_out())
            .collect()
    }
}

/// How long after the game is gone a crash reporter's report may still be
/// written: `REDengine`'s took under a second on the reference desktop.
const LATE: Duration = Duration::from_secs(120);

/// Look, once the game of `session` is gone (`ended`), at what a crash
/// would have left, and at how its process ended (`how`, from its pidfd:
/// [`crate::processes::ended`]). `None`: nothing says it crashed, and the
/// game was most likely quit.
#[must_use]
pub fn examine(
    session: &Session,
    ended: SystemTime,
    how: Option<crate::processes::Ended>,
) -> Option<Crash> {
    // A report from the second the game started counts; one from before
    // belongs to another run.
    let since = session
        .started
        .checked_sub(Duration::from_secs(2))
        .unwrap_or(session.started);
    let until = ended + LATE;
    let (evidence, reason, report) = if let Some(found) = report_in(
        session.prefix.as_deref(),
        session.install.as_deref(),
        since,
        until,
    ) {
        (found.evidence, found.reason, Some(found.dir))
    } else if let Some(reporter) = reporter_running(since) {
        (
            Text::with(
                N_("%s, the game's crash reporter, opened as it closed"),
                [reporter],
            ),
            None,
            None,
        )
    } else if let Some(exception) = how.filter(|_| session.windows).and_then(windows_exception) {
        (
            Text::with(
                N_("The game ended with the code Windows gives %s"),
                [exception],
            ),
            None,
            None,
        )
    } else if let Some(signal) = how.and_then(crash_signal) {
        (
            Text::with(
                N_("The game was ended by %s, the signal of a crash"),
                [signal],
            ),
            None,
            None,
        )
    } else if let Some(signal) = core_dump(session.pid, since) {
        (
            Text::with(
                N_("The game was stopped by %s (systemd-coredump)"),
                [signal],
            ),
            None,
            None,
        )
    } else {
        return None;
    };
    let mut extras = session.extras.clone();
    if crate::graphics::installed_processes(&crate::graphics::state_dir())
        .contains(&session.process.to_lowercase())
    {
        extras.insert(Extra::OptiScaler);
    }
    Some(Crash {
        game: session.game.clone(),
        process: session.process.clone(),
        at: secs(ended),
        played_secs: ended
            .duration_since(session.started)
            .unwrap_or_default()
            .as_secs(),
        evidence,
        reason,
        report,
        extras: extras.into_iter().collect(),
    })
}

fn secs(t: SystemTime) -> u64 {
    t.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()
}

/// A crash report found on disk.
#[derive(Debug, PartialEq, Eq)]
struct Found {
    evidence: Text,
    reason: Option<String>,
    dir: PathBuf,
}

/// Which engine writes reports where, under a Windows user's folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Engine {
    Red,
    Unreal,
    Unity,
}

impl Engine {
    fn evidence(self) -> Text {
        match self {
            Self::Red => Text::plain(N_("REDengine wrote a crash report")),
            Self::Unreal => Text::plain(N_("Unreal Engine wrote a crash report")),
            Self::Unity => Text::plain(N_("Unity wrote a crash report")),
        }
    }

    /// The folders, under one Windows user's folder, whose entries are one
    /// crash each.
    fn queues(self, user: &Path) -> Vec<PathBuf> {
        let local = user.join("AppData/Local");
        match self {
            Self::Red => vec![local.join("REDEngine/ReportQueue")],
            // `AppData/Local/<Project>/Saved/Crashes`.
            Self::Unreal => subdirs(&local)
                .into_iter()
                .map(|p| p.join("Saved/Crashes"))
                .collect(),
            // `AppData/Local/Temp/<Company>/<Product>/Crashes`.
            Self::Unity => subdirs(&local.join("Temp"))
                .into_iter()
                .flat_map(|company| subdirs(&company))
                .map(|p| p.join("Crashes"))
                .collect(),
        }
    }

    fn reason(self, dir: &Path) -> Option<String> {
        let text = match self {
            Self::Red => red_engine_reason(&read_small(&dir.join("stacktrace.txt"))?),
            Self::Unreal => unreal_reason(&read_small(&dir.join("CrashContext.runtime-xml"))?),
            Self::Unity => unity_reason(&read_small(&dir.join("error.log"))?),
        }?;
        Some(shorten(&text))
    }

    /// Whether the report in `dir` is of a crash. Unreal also writes one,
    /// and the game plays on, for an `Ensure` (a check that failed) and for
    /// a hang it recovered from.
    fn fatal(self, dir: &Path) -> bool {
        match self {
            Self::Unreal => read_small(&dir.join("CrashContext.runtime-xml"))
                .and_then(|xml| xml_field(&xml, "CrashType"))
                .is_none_or(|kind| !matches!(kind.as_str(), "Ensure" | "Hang")),
            Self::Red | Self::Unity => true,
        }
    }
}

/// The newest crash report under the prefix's users, or under the game's
/// own folder (an Unreal game run from where it is installed), written
/// between `since` and `until`.
fn report_in(
    prefix: Option<&Path>,
    install: Option<&Path>,
    since: SystemTime,
    until: SystemTime,
) -> Option<Found> {
    let mut queues: Vec<(Engine, PathBuf)> = Vec::new();
    if let Some(prefix) = prefix {
        for user in subdirs(&prefix.join("drive_c/users")) {
            for engine in [Engine::Red, Engine::Unreal, Engine::Unity] {
                queues.extend(engine.queues(&user).into_iter().map(|q| (engine, q)));
            }
        }
    }
    if let Some(install) = install {
        queues.extend(
            subdirs(install)
                .into_iter()
                .map(|p| (Engine::Unreal, p.join("Saved/Crashes"))),
        );
    }
    queues
        .into_iter()
        .flat_map(|(engine, queue)| {
            subdirs(&queue)
                .into_iter()
                .filter_map(move |dir| Some((modified(&dir)?, engine, dir)))
        })
        .filter(|(at, _, _)| *at >= since && *at <= until)
        .filter(|(_, engine, dir)| engine.fatal(dir))
        .max_by_key(|(at, _, _)| *at)
        .map(|(_, engine, dir)| Found {
            evidence: engine.evidence(),
            reason: engine.reason(&dir),
            dir,
        })
}

/// The folders in `dir`, none when it cannot be read. A prefix's user
/// folders hold a few dozen; this never descends further than asked.
fn subdirs(dir: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(dir)
        .map(|d| {
            d.flatten()
                .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
                .map(|e| e.path())
                .collect()
        })
        .unwrap_or_default()
}

fn modified(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// A report's text file, if it is one: a few kilobytes, never a dump.
fn read_small(path: &Path) -> Option<String> {
    use std::io::Read as _;
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .ok()?
        .take(64 * 1024)
        .read_to_end(&mut bytes)
        .ok()?;
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

/// `REDengine`'s `stacktrace.txt`: `Expression:` names the exception,
/// `Message:` says what it did.
fn red_engine_reason(text: &str) -> Option<String> {
    let field = |name: &str| {
        text.lines()
            .find_map(|l| l.trim().strip_prefix(name))
            .map(str::trim)
            .filter(|v| !v.is_empty() && *v != "<Unknown>")
    };
    match (
        field("Expression:"),
        field("Message:"),
        field("Error reason:"),
    ) {
        (Some(e), Some(m), _) => Some(format!("{e}: {m}")),
        (Some(one), None, _) | (None, Some(one), _) | (None, None, Some(one)) => {
            Some(one.to_owned())
        }
        _ => None,
    }
}

/// Unreal's `CrashContext.runtime-xml`: its `<ErrorMessage>`.
fn unreal_reason(xml: &str) -> Option<String> {
    xml_field(xml, "ErrorMessage")
}

/// The text of the first `<name>` element in `xml`, unescaped and trimmed;
/// `None` when it is missing or empty.
fn xml_field(xml: &str, name: &str) -> Option<String> {
    let open = format!("<{name}>");
    let start = xml.find(&open)? + open.len();
    let end = start + xml[start..].find(&format!("</{name}>"))?;
    let message = xml[start..end]
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&");
    let message = message.trim();
    (!message.is_empty()).then(|| message.to_owned())
}

/// Unity's `error.log`: the line that says what the player caused, else its
/// first.
fn unity_reason(text: &str) -> Option<String> {
    let lines = || text.lines().map(str::trim).filter(|l| !l.is_empty());
    lines()
        .find(|l| l.contains("caused an"))
        .or_else(|| lines().next())
        .map(str::to_owned)
}

/// One line, at most 300 characters.
fn shorten(text: &str) -> String {
    let line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if line.chars().count() <= 300 {
        return line;
    }
    let mut cut: String = line.chars().take(299).collect();
    cut.push('…');
    cut
}

/// Programs that start only when a game has crashed. Unity's
/// `UnityCrashHandler64.exe`, Chromium's `crashpad_handler` and `REDengine`'s
/// `REDEngineErrorReporter.exe` run beside the game from its start (the
/// last seen with Cyberpunk 2077 on the reference desktop), so they say
/// nothing; `REDengine`'s crash is in its report queue.
const REPORTERS: &[&str] = &[
    "crashreportclient.exe",
    "crashreporter.exe",
    "werfault.exe",
    "winedbg.exe",
    "bssndrpt64.exe",
    "bssndrpt.exe",
];

/// A crash reporter started since `since` and still running, by the name
/// it was started as (Wine's `comm` is cut at 15 characters).
fn reporter_running(since: SystemTime) -> Option<String> {
    let age_limit = SystemTime::now().duration_since(since).ok()?.as_secs();
    std::fs::read_dir("/proc")
        .ok()?
        .flatten()
        .filter_map(|e| e.file_name().to_str()?.parse::<u32>().ok())
        .find_map(|pid| {
            let cmdline = std::fs::read(format!("/proc/{pid}/cmdline")).ok()?;
            let argv0 = cmdline.split(|b| *b == 0).next()?;
            let argv0 = String::from_utf8_lossy(argv0);
            let name = argv0.rsplit(['/', '\\']).next()?.to_owned();
            if !REPORTERS.contains(&name.to_ascii_lowercase().as_str()) {
                return None;
            }
            let age = crate::running::running_for(pid)?;
            (age <= age_limit).then_some(name)
        })
}

/// The Windows exception a Wine game's exit status stands for, when it is
/// one a crash raises. Wine ends a process an unhandled exception stopped
/// with the exception's code, of which the exit status keeps the low byte:
/// checked on the reference desktop with a program reading 0x3F800010
/// under Proton Experimental (status 5; a normal exit, 0). Only codes no
/// game ends with on purpose count: a 1 or a 2 says nothing.
fn windows_exception(how: crate::processes::Ended) -> Option<&'static str> {
    let crate::processes::Ended::Exited(code) = how else {
        return None;
    };
    Some(match code {
        0x05 => "EXCEPTION_ACCESS_VIOLATION (0xC0000005)",
        0x06 => "EXCEPTION_IN_PAGE_ERROR (0xC0000006)",
        0x1D => "EXCEPTION_ILLEGAL_INSTRUCTION (0xC000001D)",
        0x94 => "EXCEPTION_INT_DIVIDE_BY_ZERO (0xC0000094)",
        0xFD => "EXCEPTION_STACK_OVERFLOW (0xC00000FD)",
        0x09 => "STATUS_STACK_BUFFER_OVERRUN (0xC0000409)",
        0x74 => "STATUS_HEAP_CORRUPTION (0xC0000374)",
        _ => return None,
    })
}

/// The name of the signal that ended a process, when it is one a crash
/// raises. `SIGKILL` and `SIGTERM` are someone stopping the game (a
/// launcher, the session ending), not the game failing. A Wine game that
/// fails ends with an exit status instead, which its report says more
/// about; only a Wine that fails itself, or a native game, gets here.
fn crash_signal(how: crate::processes::Ended) -> Option<String> {
    let crate::processes::Ended::Signal { signal, .. } = how else {
        return None;
    };
    let name = match signal {
        libc::SIGSEGV => "SIGSEGV",
        libc::SIGABRT => "SIGABRT",
        libc::SIGBUS => "SIGBUS",
        libc::SIGILL => "SIGILL",
        libc::SIGFPE => "SIGFPE",
        libc::SIGTRAP => "SIGTRAP",
        libc::SIGSYS => "SIGSYS",
        _ => return None,
    };
    Some(name.to_owned())
}

/// The signal that ended process `pid`, from systemd-coredump's record in
/// the journal since `since`.
fn core_dump(pid: u32, since: SystemTime) -> Option<String> {
    let out = std::process::Command::new("journalctl")
        .args([
            "--no-pager",
            "--output=json",
            "--output-fields=COREDUMP_SIGNAL_NAME,COREDUMP_SIGNAL",
            "--lines=1",
            &format!("--since=@{}", secs(since)),
            // systemd-coredump's "Process … dumped core" message.
            "MESSAGE_ID=fc2e22bc6ee647b6b90729ab34a250b1",
            &format!("COREDUMP_PID={pid}"),
        ])
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    signal_in(&String::from_utf8_lossy(&out.stdout))
}

fn signal_in(journal_json: &str) -> Option<String> {
    let record: serde_json::Value = serde_json::from_str(journal_json.lines().next()?).ok()?;
    record["COREDUMP_SIGNAL_NAME"]
        .as_str()
        .map(str::to_owned)
        .or_else(|| {
            record["COREDUMP_SIGNAL"]
                .as_str()
                .map(|n| format!("signal {n}"))
        })
}

// ── The record ──────────────────────────────────────────────────────────────

/// How many crashes are kept.
const KEPT: usize = 20;

fn record_path() -> PathBuf {
    crate::paths::state_home().join("bigame-mode/crashes.json")
}

/// The crashes recorded, newest first.
#[must_use]
pub fn recorded() -> Vec<Crash> {
    read_record(&record_path())
}

fn read_record(path: &Path) -> Vec<Crash> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

/// Keep `crash` with the others.
///
/// # Errors
/// Returns an error when the record cannot be written.
pub fn record(crash: &Crash) -> Result<()> {
    record_at(&record_path(), crash)
}

fn record_at(path: &Path, crash: &Crash) -> Result<()> {
    let mut all = read_record(path);
    all.insert(0, crash.clone());
    all.truncate(KEPT);
    crate::turbo_preset::write_atomic(path, &serde_json::to_string_pretty(&all)?)
}

/// How many times the game `process` crashed in the last `days` days
/// before `now`, by the record.
#[must_use]
pub fn crashes_of(process: &str, days: u64, now: SystemTime) -> usize {
    count_in(&recorded(), process, days, now)
}

fn count_in(all: &[Crash], process: &str, days: u64, now: SystemTime) -> usize {
    let after = secs(now).saturating_sub(days * 86_400);
    all.iter()
        .filter(|c| c.process == process && c.at >= after)
        .count()
}

// ── Leaving the extras out ──────────────────────────────────────────────────

/// What [`leave_out`] did.
#[derive(Debug)]
pub struct LeftOut {
    /// What was switched off in the game's launch settings.
    pub off: Vec<Extra>,
    /// The game's Steam launch options, when it is a Steam game.
    pub steam: Option<Result<crate::steam_gamescope::Applied>>,
    /// The game's settings in Heroic, when Heroic starts it.
    pub heroic: Option<Result<crate::heroic_launch::Applied>>,
}

/// Keep `extras` out of the game `process` from its next start: off in its
/// own launch settings, which win over the session's (a Turbo preset's), and
/// written where its launcher reads them. An open Steam keeps launch options
/// in memory, so it is closed and opened again around the write — never
/// while it runs a game. Those [`Extra::can_leave_out`] does not name are
/// left as they are.
///
/// # Errors
/// Returns an error when the game's settings cannot be read or saved; then
/// nothing was written anywhere.
pub fn leave_out(process: &str, extras: &[Extra]) -> Result<LeftOut> {
    let mut settings = crate::game_settings::load(process)?;
    let mut off = Vec::new();
    for extra in extras.iter().copied().filter(|e| e.can_leave_out()) {
        let launch = &mut settings.launch;
        match extra {
            Extra::VkBasalt => launch.vkbasalt = Some(false),
            Extra::Fsr4Upgrade => launch.fsr4_upgrade_off = true,
            Extra::WineFsr => launch.wine_fsr = Some(false),
            _ => continue,
        }
        off.push(extra);
    }
    if off.is_empty() {
        return Ok(LeftOut {
            off,
            steam: None,
            heroic: None,
        });
    }
    crate::game_settings::save(process, &settings)?;
    tracing::info!(game = %process, ?off, "extras left out of the game's next start");
    let steam = match crate::steam_gamescope::apply(
        process,
        crate::optimization::steam_wanted(process, None),
    ) {
        Ok(crate::steam_gamescope::Applied::SteamRunning) => {
            crate::launchers::while_closed(crate::launchers::Launcher::Steam, || {
                crate::steam_gamescope::apply(
                    process,
                    crate::optimization::steam_wanted(process, None),
                )
            })
            .and_then(|r| r)
        }
        other => other,
    };
    // Saved all the same: Turbo's launcher follow-up writes it once Steam is
    // closed (crate::turbo_preset::follow_owed).
    if !matches!(
        steam,
        Ok(crate::steam_gamescope::Applied::Written(_)
            | crate::steam_gamescope::Applied::TheirGamescope(_)
            | crate::steam_gamescope::Applied::Unchanged
            | crate::steam_gamescope::Applied::NotSteam)
    ) {
        crate::turbo_preset::mark_launchers_owed();
    }
    let steam = (!matches!(steam, Ok(crate::steam_gamescope::Applied::NotSteam))).then_some(steam);
    let heroic = match crate::optimization::apply_heroic(process) {
        Ok(crate::heroic_launch::Applied::HeroicRunning {
            launcher,
            game_running: false,
        }) => {
            crate::launchers::while_closed(launcher, || crate::optimization::apply_heroic(process))
                .and_then(|r| r)
        }
        other => other,
    };
    if matches!(
        heroic,
        Ok(crate::heroic_launch::Applied::HeroicRunning { .. }) | Err(_)
    ) {
        crate::turbo_preset::mark_launchers_owed();
    }
    let heroic =
        (!matches!(heroic, Ok(crate::heroic_launch::Applied::NotHeroic))).then_some(heroic);
    Ok(LeftOut { off, steam, heroic })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(prefix: Option<&Path>, started: SystemTime) -> Session {
        Session {
            game: "Cyberpunk 2077".into(),
            process: "Cyberpunk2077.exe".into(),
            pid: u32::MAX,
            started,
            prefix: prefix.map(Path::to_path_buf),
            install: None,
            windows: true,
            extras: BTreeSet::from([Extra::VkBasalt, Extra::Fsr4Upgrade]),
            looked_inside: true,
        }
    }

    /// The report Cyberpunk 2077 wrote on the reference desktop, 2026-10-18.
    const CYBERPUNK_STACKTRACE: &str = "Error reason: Unhandled exception\r\nExpression: EXCEPTION_ACCESS_VIOLATION (0xC0000005)\r\nMessage: The thread attempted to read inaccessible data at 0x3F800010.\r\nFile: <Unknown>(0)\r\n";

    fn red_report(prefix: &Path, name: &str) -> PathBuf {
        let dir = prefix
            .join("drive_c/users/steamuser/AppData/Local/REDEngine/ReportQueue")
            .join(name);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("stacktrace.txt"), CYBERPUNK_STACKTRACE).unwrap();
        dir
    }

    #[test]
    fn a_redengine_report_written_while_the_game_ran_is_a_crash_with_its_reason() {
        let d = tempfile::tempdir().unwrap();
        let started = SystemTime::now() - Duration::from_secs(1524);
        let dir = red_report(d.path(), "Cyberpunk2077-20261018-201525-1068-1140");
        let crash = examine(&session(Some(d.path()), started), SystemTime::now(), None).unwrap();
        assert_eq!(crash.report.as_deref(), Some(dir.as_path()));
        assert_eq!(crash.evidence.english(), "REDengine wrote a crash report");
        assert_eq!(
            crash.reason.as_deref(),
            Some(
                "EXCEPTION_ACCESS_VIOLATION (0xC0000005): The thread attempted to read inaccessible data at 0x3F800010."
            )
        );
        assert!((1523..=1526).contains(&crash.played_secs));
        assert_eq!(crash.removable(), [Extra::VkBasalt, Extra::Fsr4Upgrade]);
    }

    #[test]
    fn a_report_from_before_the_game_started_is_another_runs() {
        let d = tempfile::tempdir().unwrap();
        red_report(d.path(), "old");
        // Started after the report was written.
        let started = SystemTime::now() + Duration::from_secs(60);
        assert_eq!(
            report_in(Some(d.path()), None, started, started + LATE),
            None
        );
    }

    #[test]
    fn unreal_and_unity_reports_are_found_where_their_engines_write_them() {
        let d = tempfile::tempdir().unwrap();
        let local = d.path().join("drive_c/users/steamuser/AppData/Local");
        let ue = local.join("Bodycam/Saved/Crashes/UECC-Windows-1");
        std::fs::create_dir_all(&ue).unwrap();
        std::fs::write(
            ue.join("CrashContext.runtime-xml"),
            "<FGenericCrashContext><RuntimeProperties><ErrorMessage>Unhandled Exception: EXCEPTION_ACCESS_VIOLATION reading address 0x00000000 &amp; more</ErrorMessage></RuntimeProperties></FGenericCrashContext>",
        )
        .unwrap();
        let since = SystemTime::now() - Duration::from_secs(10);
        let found = report_in(Some(d.path()), None, since, since + LATE).unwrap();
        assert_eq!(
            found.evidence.english(),
            "Unreal Engine wrote a crash report"
        );
        assert_eq!(
            found.reason.as_deref(),
            Some(
                "Unhandled Exception: EXCEPTION_ACCESS_VIOLATION reading address 0x00000000 & more"
            )
        );
        // An Ensure is reported, and the game plays on.
        let ensure = local.join("Bodycam/Saved/Crashes/UECC-Windows-2");
        std::fs::create_dir_all(&ensure).unwrap();
        std::fs::write(
            ensure.join("CrashContext.runtime-xml"),
            "<RuntimeProperties><CrashType>Ensure</CrashType></RuntimeProperties>",
        )
        .unwrap();
        std::fs::remove_dir_all(&ue).unwrap();
        assert_eq!(report_in(Some(d.path()), None, since, since + LATE), None);
        std::fs::remove_dir_all(local.join("Bodycam")).unwrap();

        let unity = local.join("Temp/Studio/Game/Crashes/Crash_2026-10-18_201525");
        std::fs::create_dir_all(&unity).unwrap();
        std::fs::write(
            unity.join("error.log"),
            "\nGame by Studio [version: Unity 2022.3]\n\nUnityPlayer.dll caused an Access Violation (0xc0000005)\n",
        )
        .unwrap();
        let found = report_in(Some(d.path()), None, since, since + LATE).unwrap();
        assert_eq!(found.evidence.english(), "Unity wrote a crash report");
        assert_eq!(
            found.reason.as_deref(),
            Some("UnityPlayer.dll caused an Access Violation (0xc0000005)")
        );
    }

    #[test]
    fn a_game_that_left_nothing_was_quit() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(d.path().join("drive_c/users/steamuser")).unwrap();
        let started = SystemTime::now() - Duration::from_secs(60);
        assert_eq!(
            examine(&session(Some(d.path()), started), SystemTime::now(), None),
            None
        );
    }

    #[test]
    fn the_environment_says_what_proton_and_the_layers_were_asked_for() {
        let env = Environ(
            b"ENABLE_VKBASALT=1\0FSR4_UPGRADE=1\0WINE_FULLSCREEN_FSR=0\0LD_PRELOAD=/usr/lib/mangohud/libMangoHud_shim.so\0",
        );
        assert_eq!(
            env.extras(),
            BTreeSet::from([Extra::VkBasalt, Extra::Fsr4Upgrade, Extra::MangoHud])
        );
        // Valve's Proton reads FSR4_UPGRADE only at `1`; a game's own `0`
        // and vkBasalt's off switch win.
        let off = Environ(
            b"ENABLE_VKBASALT=1\0DISABLE_VKBASALT=1\0FSR4_UPGRADE=0\0PROTON_FSR4_UPGRADE=0\0",
        );
        assert!(off.extras().is_empty());
        assert!(
            Environ(b"PROTON_FSR4_UPGRADE=yes\0")
                .extras()
                .contains(&Extra::Fsr4Upgrade)
        );
        assert!(
            Environ(b"GAMESCOPE_WAYLAND_DISPLAY=gamescope-0\0")
                .extras()
                .contains(&Extra::Gamescope)
        );
    }

    #[test]
    fn a_crash_signal_is_a_crash_and_being_stopped_is_not() {
        use crate::processes::Ended;
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(d.path().join("drive_c/users/steamuser")).unwrap();
        let started = SystemTime::now() - Duration::from_secs(120);
        let s = session(Some(d.path()), started);
        let abort = Ended::Signal {
            signal: libc::SIGABRT,
            core: false,
        };
        let crash = examine(&s, SystemTime::now(), Some(abort)).unwrap();
        assert_eq!(
            crash.evidence.english(),
            "The game was ended by SIGABRT, the signal of a crash"
        );
        for quit in [
            Ended::Exited(0),
            Ended::Exited(1),
            Ended::Signal {
                signal: libc::SIGKILL,
                core: false,
            },
            Ended::Signal {
                signal: libc::SIGTERM,
                core: false,
            },
        ] {
            assert_eq!(examine(&s, SystemTime::now(), Some(quit)), None, "{quit:?}");
        }
        // Wine's exit for an unhandled access violation, in a Windows game
        // only: a native game's 5 is its own.
        let crash = examine(&s, SystemTime::now(), Some(Ended::Exited(5))).unwrap();
        assert_eq!(
            crash.evidence.english(),
            "The game ended with the code Windows gives EXCEPTION_ACCESS_VIOLATION (0xC0000005)"
        );
        let native = Session {
            windows: false,
            ..s
        };
        assert_eq!(
            examine(&native, SystemTime::now(), Some(Ended::Exited(5))),
            None
        );
    }

    #[test]
    fn a_proton_prefix_is_the_pfx_inside_compatdata() {
        let d = tempfile::tempdir().unwrap();
        assert_eq!(wine_prefix(d.path()), None);
        std::fs::create_dir_all(d.path().join("pfx/drive_c")).unwrap();
        assert_eq!(wine_prefix(d.path()), Some(d.path().join("pfx")));
        assert_eq!(
            wine_prefix(&d.path().join("pfx")),
            Some(d.path().join("pfx"))
        );
    }

    #[test]
    fn the_reasons_are_read_from_each_engines_words() {
        assert_eq!(
            red_engine_reason("Error reason: Unhandled exception\nFile: <Unknown>(0)\n").as_deref(),
            Some("Unhandled exception")
        );
        assert_eq!(red_engine_reason("nothing here"), None);
        assert_eq!(unreal_reason("<ErrorMessage>  </ErrorMessage>"), None);
        let long = "x ".repeat(400);
        assert_eq!(shorten(&long).chars().count(), 300);
        assert!(shorten(&long).ends_with('…'));
    }

    #[test]
    fn the_coredump_record_names_the_signal() {
        assert_eq!(
            signal_in("{\"COREDUMP_SIGNAL_NAME\":\"SIGSEGV\",\"COREDUMP_SIGNAL\":\"11\"}\n")
                .as_deref(),
            Some("SIGSEGV")
        );
        assert_eq!(
            signal_in("{\"COREDUMP_SIGNAL\":\"6\"}").as_deref(),
            Some("signal 6")
        );
        assert_eq!(signal_in(""), None);
    }

    #[test]
    fn the_record_keeps_the_newest_and_counts_a_games_crashes() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("crashes.json");
        let now = SystemTime::now();
        let crash = |process: &str, ago_days: u64| Crash {
            game: process.into(),
            process: process.into(),
            at: secs(now) - ago_days * 86_400,
            played_secs: 60,
            evidence: Text::plain(N_("REDengine wrote a crash report")),
            reason: None,
            report: None,
            extras: vec![Extra::VkBasalt],
        };
        for _ in 0..KEPT {
            record_at(&path, &crash("Old.exe", 30)).unwrap();
        }
        record_at(&path, &crash("Cyberpunk2077.exe", 3)).unwrap();
        record_at(&path, &crash("Cyberpunk2077.exe", 0)).unwrap();
        let all = read_record(&path);
        assert_eq!(all.len(), KEPT);
        assert_eq!(all[0].process, "Cyberpunk2077.exe");
        assert_eq!(count_in(&all, "Cyberpunk2077.exe", 7, now), 2);
        assert_eq!(count_in(&all, "Cyberpunk2077.exe", 1, now), 1);
        assert_eq!(count_in(&all, "Old.exe", 7, now), 0);
    }
}
