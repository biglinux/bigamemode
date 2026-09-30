//! The launchers games are started through — Steam, Heroic, Lutris, and
//! Flatpak for a game that is an application of its own.
//!
//! Two jobs:
//!
//! * **Starting a game through its launcher** ([`Start`]): `steam -applaunch`,
//!   a `heroic://launch` link, `lutris:rungame/<slug>`, `flatpak run`. The
//!   launcher starts the game in a process tree of its own, so the game gets
//!   its falcond profile — falcond matches the game's process wherever it
//!   comes from — but none of Big Game Mode's launch settings, which wrap a
//!   process Big Game Mode starts itself.
//! * **Opening a launcher again** ([`behind_the_session`], [`reopen`]). A
//!   launcher keeps the environment it started with, and its games inherit
//!   that, so a Turbo preset set in the session afterwards reaches them only
//!   once the launcher is opened again. Whether a launcher is behind is read
//!   from its own process (`/proc/<pid>/environ`), not assumed. It is closed
//!   the way it closes itself, never while it runs a game — closing Heroic or
//!   Lutris can take the game with it — and started again as a unit of the
//!   user's systemd manager, as the desktop's menu starts applications, so it
//!   inherits the session's environment rather than Big Game Mode's.
//!
//! Every command is an argument vector, and every id that goes into one —
//! Steam's app id, Heroic's app name and runner, a Lutris slug, a Flatpak
//! application id — is checked against a strict character set first, so none
//! can become an option or reach outside the link it is part of.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};

use crate::error::UserError;
use crate::games::{DetectedGame, LauncherRef, Source};
use crate::text::N_;

/// Heroic's Flatpak application id.
pub const HEROIC_FLATPAK: &str = "com.heroicgameslauncher.hgl";
/// Lutris's Flatpak application id.
pub const LUTRIS_FLATPAK: &str = "net.lutris.Lutris";

/// The runners a `heroic://launch` link accepts (Heroic 2.22's `RUNNERS`).
const HEROIC_RUNNERS: &[&str] = &["legendary", "gog", "nile", "sideload"];

/// Electron runs as plain Node.js with this set (VS Code sets it for what
/// it starts), and Heroic then fails to open.
const ELECTRON_AS_NODE: &str = "ELECTRON_RUN_AS_NODE";

/// A game launcher, as it is installed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Launcher {
    /// The Steam client.
    Steam,
    /// Heroic Games Launcher.
    Heroic {
        /// The Flatpak rather than a native package.
        flatpak: bool,
    },
    /// Lutris.
    Lutris {
        /// The Flatpak rather than a native package.
        flatpak: bool,
    },
}

impl Launcher {
    /// Its name, as people know it.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Steam => "Steam",
            Self::Heroic { .. } => "Heroic",
            Self::Lutris { .. } => "Lutris",
        }
    }

    /// Its Flatpak application id, when it is the Flatpak.
    #[must_use]
    pub fn flatpak_id(self) -> Option<&'static str> {
        match self {
            Self::Heroic { flatpak: true } => Some(HEROIC_FLATPAK),
            Self::Lutris { flatpak: true } => Some(LUTRIS_FLATPAK),
            _ => None,
        }
    }

    /// The word its systemd units are named with.
    fn unit(self) -> &'static str {
        match self {
            Self::Steam => "steam",
            Self::Heroic { .. } => "heroic",
            Self::Lutris { .. } => "lutris",
        }
    }

    /// The command that opens it, `program` naming a native one (its name
    /// on the `PATH`, or the file it runs from), then `extra`.
    fn command(self, program: &str, extra: &[String]) -> Vec<String> {
        let mut argv: Vec<String> = match self {
            Self::Heroic { flatpak: true } => {
                vec![
                    "flatpak".into(),
                    "run".into(),
                    "--command=heroic-run".into(),
                    HEROIC_FLATPAK.into(),
                ]
            }
            Self::Lutris { flatpak: true } => {
                vec!["flatpak".into(), "run".into(), LUTRIS_FLATPAK.into()]
            }
            _ => vec![program.to_owned()],
        };
        argv.extend_from_slice(extra);
        argv
    }

    /// The name a native one is started by.
    fn program(self) -> &'static str {
        match self {
            Self::Steam => "steam",
            Self::Heroic { .. } => "heroic",
            Self::Lutris { .. } => "lutris",
        }
    }

    /// Whether this machine can start it.
    fn installed(self) -> bool {
        match self.flatpak_id() {
            Some(id) => flatpak_installed(id),
            None => crate::capabilities::which(self.program()).is_some(),
        }
    }

    /// The names of its own processes: the launcher and what it keeps
    /// running beside it while no game runs. Anything else it started is a
    /// game, or a task of its own (a download, a cloud save) — either way,
    /// closing it now could cut it short.
    fn machinery(self) -> &'static [&'static str] {
        match self {
            Self::Steam => &[],
            // Electron's helpers (crash reporter, Zypak's sandbox), and
            // Flatpak's own: bwrap, its D-Bus proxy, and the two `cat`
            // processes `flatpak run` leaves in the sandbox.
            Self::Heroic { .. } => &[
                "heroic",
                "heroic-run",
                "bwrap",
                "cat",
                "xdg-dbus-proxy",
                "zypak-helper",
                "zypak-sandbox",
                "chrome_crashpad",
            ],
            Self::Lutris { .. } => &["lutris", "bwrap", "xdg-dbus-proxy"],
        }
    }
}

// ── Checking ids ─────────────────────────────────────────────────────────────

/// A Steam app id: digits only.
fn valid_steam_id(id: &str) -> bool {
    (1..=10).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_digit())
}

/// A Heroic app name: Epic's hex ids and names, GOG's numbers, Amazon's
/// `amzn1.adg.product.…` and the sideloaded ids Heroic makes up. Every
/// character is one a URL carries as it is.
fn valid_heroic_app_name(name: &str) -> bool {
    (1..=128).contains(&name.len())
        && name
            .bytes()
            .next()
            .is_some_and(|b| b.is_ascii_alphanumeric())
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

/// A Lutris slug: lower-case letters, digits and dashes (Lutris's own
/// `slugify`), with dots and underscores allowed for older entries.
fn valid_lutris_slug(slug: &str) -> bool {
    (1..=128).contains(&slug.len())
        && slug
            .bytes()
            .next()
            .is_some_and(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        && slug.bytes().all(|b| {
            b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'-' | b'.' | b'_')
        })
}

/// A Flatpak application id: at least three dot-separated elements of
/// letters, digits, `_` and `-`, none starting with a digit or `-`
/// (Flatpak's own rules), at most 255 characters.
fn valid_flatpak_id(id: &str) -> bool {
    let elements: Vec<&str> = id.split('.').collect();
    id.len() <= 255
        && elements.len() >= 3
        && elements.iter().all(|e| {
            e.bytes()
                .next()
                .is_some_and(|b| b.is_ascii_alphabetic() || b == b'_')
                && e.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
        })
}

/// Whether Flatpak has `id` installed, system-wide or for this user.
fn flatpak_installed(id: &str) -> bool {
    crate::capabilities::which("flatpak").is_some()
        && [
            PathBuf::from("/var/lib/flatpak"),
            crate::paths::home_dir().join(".local/share/flatpak"),
        ]
        .iter()
        .any(|root| root.join("app").join(id).join("current").exists())
}

// ── Starting a game through its launcher ─────────────────────────────────────

/// A game's start through its launcher.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Start {
    /// Who starts it: the launcher's name, or `Flatpak`.
    pub by: &'static str,
    /// The word its systemd unit is named with.
    unit: &'static str,
    /// The command, an argument vector.
    pub argv: Vec<String>,
}

impl Start {
    /// How `game` is started through its launcher, when this machine has
    /// the launcher and the game's ids are well formed.
    #[must_use]
    pub fn for_game(game: &DetectedGame) -> Option<Self> {
        Self::for_game_with(game, Launcher::installed, flatpak_installed)
    }

    /// [`Self::for_game`], with what is installed said rather than read.
    fn for_game_with(
        game: &DetectedGame,
        has_launcher: impl Fn(Launcher) -> bool,
        has_flatpak: impl Fn(&str) -> bool,
    ) -> Option<Self> {
        let through = |launcher: Launcher, extra: Vec<String>| {
            has_launcher(launcher).then(|| Self {
                by: launcher.name(),
                unit: launcher.unit(),
                argv: launcher.command(launcher.program(), &extra),
            })
        };
        match (game.source, &game.launcher) {
            (Source::Steam, _) => {
                let id = game.app_id.as_deref().filter(|id| valid_steam_id(id))?;
                through(Launcher::Steam, vec!["-applaunch".into(), id.to_owned()])
            }
            (
                _,
                Some(
                    reference @ LauncherRef::Heroic {
                        app_name,
                        runner: Some(runner),
                        ..
                    },
                ),
            ) => {
                if !valid_heroic_app_name(app_name) || !HEROIC_RUNNERS.contains(runner) {
                    return None;
                }
                let launcher = Launcher::Heroic {
                    flatpak: reference.flatpak_id().is_some(),
                };
                through(
                    launcher,
                    vec![format!(
                        "heroic://launch?appName={app_name}&runner={runner}"
                    )],
                )
            }
            (_, Some(reference @ LauncherRef::Lutris { config_file })) => {
                let slug = lutris_slug(config_file)?;
                let launcher = Launcher::Lutris {
                    flatpak: reference.flatpak_id().is_some(),
                };
                through(launcher, vec![format!("lutris:rungame/{slug}")])
            }
            (Source::Flatpak, _) => {
                let id = game.app_id.as_deref().filter(|id| valid_flatpak_id(id))?;
                has_flatpak(id).then(|| Self {
                    by: "Flatpak",
                    unit: "flatpak",
                    argv: vec!["flatpak".into(), "run".into(), id.to_owned()],
                })
            }
            _ => None,
        }
    }

    /// Start it, as a unit of the user's systemd manager.
    ///
    /// # Errors
    /// Returns an error when the command cannot be started.
    pub fn spawn(&self) -> Result<()> {
        start_in_session(self.unit, &self.argv)
    }
}

/// The slug Lutris knows a game by: its configuration's file name,
/// `<slug>-<install time>.yml`, without the time.
fn lutris_slug(config_file: &Path) -> Option<String> {
    let stem = config_file.file_stem()?.to_str()?;
    let slug = crate::games::strip_numeric_suffix(stem);
    valid_lutris_slug(slug).then(|| slug.to_owned())
}

// ── Starting in the session ──────────────────────────────────────────────────

/// A unit name no other start can have taken: two starts in the same second
/// asked for the same name, and systemd refused the second.
fn unit_name(word: &str) -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    format!("app-{word}-bigame-{}-{nanos}", std::process::id())
}

/// The `systemd-run` command that starts `argv` as the unit `unit`.
///
/// * `Type=exec`: `systemd-run` answers once the program really runs, so a
///   missing one is an error here rather than a unit that failed unseen.
/// * `ExitType=cgroup`: the unit lasts while anything it started does. A
///   launcher that hands the request to its running copy and exits must not
///   take down what is left in its unit, as the default would.
/// * `UnsetEnvironment=ELECTRON_RUN_AS_NODE`: Heroic fails with it set.
fn systemd_run_argv(unit: &str, argv: &[String]) -> Vec<String> {
    let mut out: Vec<String> = [
        "--user",
        "--collect",
        "--quiet",
        "--property=Type=exec",
        "--property=ExitType=cgroup",
    ]
    .iter()
    .map(|s| (*s).to_owned())
    .collect();
    out.push(format!("--property=UnsetEnvironment={ELECTRON_AS_NODE}"));
    out.push(format!("--unit={unit}"));
    out.push("--".into());
    out.extend_from_slice(argv);
    out
}

/// Start `argv` as a unit of the user's systemd manager, named after
/// `word`, so it inherits the session's environment: how the desktop's menu
/// starts applications (KDE Plasma: `app-…@.service`). Without
/// `systemd-run`, it is started directly, without `ELECTRON_RUN_AS_NODE`.
pub(crate) fn start_in_session(word: &str, argv: &[String]) -> Result<()> {
    let program = argv.first().context("empty command")?;
    let status = std::process::Command::new("systemd-run")
        .args(systemd_run_argv(&unit_name(word), argv))
        .stdin(std::process::Stdio::null())
        .status();
    match status {
        Ok(status) => {
            anyhow::ensure!(
                status.success(),
                UserError::with(
                    N_("systemd could not start %s (%s)"),
                    [program.clone(), status.to_string()]
                )
            );
            Ok(())
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let mut cmd = std::process::Command::new(program);
            cmd.args(&argv[1..])
                .env_remove(ELECTRON_AS_NODE)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null());
            crate::launcher::in_own_process_group(&mut cmd);
            let mut child = cmd.spawn().with_context(|| format!("start {program}"))?;
            // Reaped, so an exited launcher does not stay a zombie.
            std::thread::spawn(move || {
                let _ = child.wait();
            });
            Ok(())
        }
        Err(e) => Err(e).context("start systemd-run"),
    }
}

// ── The launchers that are open ──────────────────────────────────────────────

/// What telling launchers apart needs to know about one process.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Proc {
    pid: u32,
    ppid: u32,
    /// The kernel's name for it (`/proc/<pid>/comm`), at most 15 bytes.
    comm: String,
    /// Its command line.
    args: Vec<String>,
    /// Its cgroup (`/proc/<pid>/cgroup`, the unified hierarchy's path).
    cgroup: String,
}

/// The current user's processes.
#[allow(clippy::similar_names)] // pid and ppid are what /proc calls them
fn snapshot() -> Vec<Proc> {
    use std::os::unix::fs::MetadataExt;
    // SAFETY: getuid cannot fail and has no side effects.
    let uid = unsafe { libc::getuid() };
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|entry| {
            let pid: u32 = entry.file_name().to_str()?.parse().ok()?;
            let dir = entry.path();
            if entry.metadata().ok()?.uid() != uid {
                return None;
            }
            let stat = std::fs::read_to_string(dir.join("stat")).ok()?;
            let (comm, rest) = stat.split_once(" (")?.1.rsplit_once(") ")?;
            let mut fields = rest.split_whitespace();
            if fields.next()? == "Z" {
                return None;
            }
            let ppid = fields.next()?.parse().ok()?;
            let args = std::fs::read(dir.join("cmdline"))
                .unwrap_or_default()
                .split(|b| *b == 0)
                .filter(|a| !a.is_empty())
                .map(|a| String::from_utf8_lossy(a).into_owned())
                .collect();
            let cgroup = std::fs::read_to_string(dir.join("cgroup"))
                .unwrap_or_default()
                .lines()
                .find_map(|l| l.strip_prefix("0::"))
                .unwrap_or_default()
                .to_owned();
            Some(Proc {
                pid,
                ppid,
                comm: comm.to_owned(),
                args,
                cgroup,
            })
        })
        .collect()
}

/// The Flatpak a cgroup belongs to: Flatpak puts every instance in a scope
/// `app-flatpak-<app id>-<number>.scope`, and a program it starts through
/// its portal (a game Heroic runs with umu) in another one of those.
fn flatpak_of(cgroup: &str) -> Option<&str> {
    let leaf = cgroup.rsplit('/').next()?;
    let rest = leaf.strip_prefix("app-flatpak-")?.strip_suffix(".scope")?;
    let (id, number) = rest.rsplit_once('-')?;
    number.bytes().all(|b| b.is_ascii_digit()).then_some(id)
}

/// A launcher that is open.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Open {
    launcher: Launcher,
    /// The process asked to close.
    main: u32,
    /// The process whose environment is the launcher's. Electron writes
    /// over its main process's, so Heroic's Flatpak is read from the
    /// `heroic-run` script that started it.
    env_from: u32,
    /// The main process's cgroup.
    cgroup: String,
}

/// Every launcher open in `procs`, once each.
fn find_open(procs: &[Proc]) -> Vec<Open> {
    let comm_of: HashMap<u32, &str> = procs.iter().map(|p| (p.pid, p.comm.as_str())).collect();
    let parent_is = |p: &Proc, name: &str| comm_of.get(&p.ppid) == Some(&name);
    // Electron's helpers are started with `--type=…`; the main process is not.
    let electron_main = |p: &Proc| !p.args.iter().any(|a| a.starts_with("--type="));
    let mut found: Vec<Open> = Vec::new();
    for p in procs {
        let flatpak = flatpak_of(&p.cgroup);
        let (launcher, main) = match p.comm.as_str() {
            "steam" if flatpak.is_none() && !parent_is(p, "steam") => (Launcher::Steam, p.pid),
            "heroic-run" if flatpak == Some(HEROIC_FLATPAK) => {
                let main = procs
                    .iter()
                    .find(|c| c.ppid == p.pid && c.comm == "heroic" && electron_main(c))
                    .map_or(p.pid, |c| c.pid);
                (Launcher::Heroic { flatpak: true }, main)
            }
            "heroic" if flatpak.is_none() && electron_main(p) && !parent_is(p, "heroic") => {
                (Launcher::Heroic { flatpak: false }, p.pid)
            }
            "lutris" if !parent_is(p, "lutris") => match flatpak {
                None => (Launcher::Lutris { flatpak: false }, p.pid),
                Some(LUTRIS_FLATPAK) => (Launcher::Lutris { flatpak: true }, p.pid),
                Some(_) => continue,
            },
            _ => continue,
        };
        if found.iter().any(|o| o.launcher == launcher) {
            continue;
        }
        found.push(Open {
            launcher,
            main,
            env_from: p.pid,
            cgroup: p.cgroup.clone(),
        });
    }
    found
}

/// Whether `open` is running a game, or a task of its own, right now.
///
/// Steam runs every game under a `reaper` that names it (`SteamLaunch
/// AppId=…`). For the others, a process counts as theirs when it is in the
/// launcher's Flatpak — its sandbox, or a scope its portal started — or,
/// for a native launcher, below it in the process tree or in its own
/// systemd unit; and as a game when it is not one of the launcher's own.
fn busy(open: &Open, procs: &[Proc]) -> bool {
    if open.launcher == Launcher::Steam {
        return procs
            .iter()
            .any(|p| p.comm == "reaper" && p.args.iter().any(|a| a == "SteamLaunch"));
    }
    let machinery = open.launcher.machinery();
    let game = |p: &Proc| p.pid != open.main && !machinery.contains(&p.comm.as_str());
    if let Some(id) = open.launcher.flatpak_id() {
        return procs
            .iter()
            .filter(|p| flatpak_of(&p.cgroup) == Some(id))
            .any(game);
    }
    // Its own unit, when the cgroup is named after it: a terminal's scope
    // holds whatever else was started there.
    let own_unit = open
        .cgroup
        .rsplit('/')
        .next()
        .is_some_and(|leaf| leaf.to_ascii_lowercase().contains(open.launcher.unit()));
    let mut below = vec![open.main];
    let mut i = 0;
    while let Some(&pid) = below.get(i) {
        below.extend(procs.iter().filter(|p| p.ppid == pid).map(|p| p.pid));
        i += 1;
    }
    procs
        .iter()
        .filter(|p| below.contains(&p.pid) || (own_unit && p.cgroup == open.cgroup))
        .any(game)
}

// ── Whether a launcher has the session's environment ─────────────────────────

/// A process's environment from `/proc/<pid>/environ`. `None` when it does
/// not read as one: a program that wrote over it (Electron does), or one
/// that could not be read.
fn parse_environ(raw: &[u8]) -> Option<HashMap<String, String>> {
    let env: HashMap<String, String> = raw
        .split(|b| *b == 0)
        .filter_map(|entry| {
            let entry = std::str::from_utf8(entry).ok()?;
            let (key, value) = entry.split_once('=')?;
            let valid = key
                .bytes()
                .next()
                .is_some_and(|b| b.is_ascii_alphabetic() || b == b'_')
                && key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_');
            valid.then(|| (key.to_owned(), value.to_owned()))
        })
        .collect();
    // Every login environment has these; a page of blanks does not.
    (env.contains_key("HOME") && env.contains_key("PATH")).then_some(env)
}

/// Whether a launcher whose environment is `env` gives its games something
/// else than `session` does. An environment that cannot be read is taken
/// as behind: offering to open a launcher again costs a click, a preset that
/// silently does not arrive costs the game.
fn behind<S: std::hash::BuildHasher>(
    env: Option<&HashMap<String, String, S>>,
    session: &HashMap<String, String, S>,
) -> bool {
    let Some(env) = env else {
        return true;
    };
    crate::video_config::in_force(|k| env.get(k).map(String::as_str))
        != crate::video_config::in_force(|k| session.get(k).map(String::as_str))
}

/// The launchers open now whose games would not get what the session holds
/// (Tuning's variables and the Turbo preset's) until they are opened again.
#[must_use]
pub fn behind_the_session() -> Vec<Launcher> {
    let session = crate::video_config::session_env(&crate::video_config::load());
    find_open(&snapshot())
        .into_iter()
        .filter(|open| {
            let env = std::fs::read(format!("/proc/{}/environ", open.env_from))
                .ok()
                .and_then(|raw| parse_environ(&raw));
            behind(env.as_ref(), &session)
        })
        .map(|open| open.launcher)
        .collect()
}

// ── Opening a launcher again ─────────────────────────────────────────────────

/// How long a launcher asked to close is given before it is forced
/// (a Flatpak) or the reopening gives up (a native one).
const CLOSE_TIMEOUT: Duration = Duration::from_secs(30);

/// Close `launcher` the way it closes itself and open it again in the
/// session's current environment. One that is not open is just opened.
///
/// # Errors
/// Returns an error, and leaves the launcher as it was, while it runs a
/// game; also when it does not close in time or cannot be started again.
pub fn reopen(launcher: Launcher) -> Result<()> {
    let procs = snapshot();
    let Some(open) = find_open(&procs)
        .into_iter()
        .find(|o| o.launcher == launcher)
    else {
        return start(launcher, None);
    };
    ensure_idle(&open, &procs)?;
    if launcher == Launcher::Steam {
        return crate::steam::restart_in_session();
    }
    let exe = close(&open)?;
    start(launcher, exe.as_deref())
}

/// Run `f` with `launcher` closed, and open it again afterwards if it was
/// open. Heroic keeps a game's settings in memory and writes them back, so
/// a change made to its files while it runs would be lost. Closed as
/// [`reopen`] closes it, and never while it runs a game.
///
/// # Errors
/// Returns an error, and `f` has not run, while the launcher runs a game or
/// when it does not close in time; also when it cannot be opened again.
pub fn while_closed<T>(launcher: Launcher, f: impl FnOnce() -> T) -> Result<T> {
    if launcher == Launcher::Steam {
        return crate::steam::while_closed(f);
    }
    let procs = snapshot();
    let Some(open) = find_open(&procs)
        .into_iter()
        .find(|o| o.launcher == launcher)
    else {
        return Ok(f());
    };
    ensure_idle(&open, &procs)?;
    let exe = close(&open)?;
    let out = f();
    start(launcher, exe.as_deref())?;
    Ok(out)
}

/// Whether `launcher` is open now.
#[must_use]
pub fn is_open(launcher: Launcher) -> bool {
    find_open(&snapshot())
        .iter()
        .any(|o| o.launcher == launcher)
}

/// Close `open`, which runs no game, the way it closes itself, and wait
/// until it is gone. Returns the file a native one runs from, to start it
/// again from there when its name is not on the `PATH`.
fn close(open: &Open) -> Result<Option<std::path::PathBuf>> {
    let launcher = open.launcher;
    // A native launcher that is not on the PATH is started again from the
    // file it runs from now.
    let exe = std::fs::read_link(format!("/proc/{}/exe", open.main)).ok();
    ask_to_close(open);
    if !wait_closed(launcher, CLOSE_TIMEOUT) {
        let Some(id) = launcher.flatpak_id() else {
            anyhow::bail!(UserError::with(
                N_("%s did not close within half a minute. Close it and try again."),
                [launcher.name()]
            ));
        };
        // The last resort, and only while it still runs no game.
        let procs = snapshot();
        if let Some(open) = find_open(&procs)
            .into_iter()
            .find(|o| o.launcher == launcher)
        {
            ensure_idle(&open, &procs)?;
        }
        tracing::warn!(
            launcher = launcher.name(),
            "did not close when asked; stopping its Flatpak"
        );
        let _ = std::process::Command::new("flatpak")
            .args(["kill", id])
            .stdin(std::process::Stdio::null())
            .status();
        anyhow::ensure!(
            wait_closed(launcher, Duration::from_secs(10)),
            UserError::with(
                N_("%s did not close within half a minute. Close it and try again."),
                [launcher.name()]
            )
        );
    }
    Ok(exe)
}

/// Refuse while `launcher` is open and runs a game — before anything closes
/// it for a moment (Steam, to write launch options).
///
/// # Errors
/// Returns the reason when a game from it is running.
pub fn ensure_launcher_idle(launcher: Launcher) -> Result<()> {
    let procs = snapshot();
    match find_open(&procs)
        .into_iter()
        .find(|o| o.launcher == launcher)
    {
        Some(open) => ensure_idle(&open, &procs),
        None => Ok(()),
    }
}

/// Refuse while `open` runs a game.
fn ensure_idle(open: &Open, procs: &[Proc]) -> Result<()> {
    let name = open.launcher.name();
    anyhow::ensure!(
        !busy(open, procs),
        UserError::with(
            N_(
                "%s is running a game. Close the game first: closing %s now could close the game with it."
            ),
            [name, name]
        )
    );
    Ok(())
}

/// Ask `open` to close: Lutris through its own `quit` action, Heroic with
/// SIGTERM, which Electron takes as a request to quit.
fn ask_to_close(open: &Open) {
    if matches!(open.launcher, Launcher::Lutris { .. }) {
        match quit_lutris() {
            Ok(()) => return,
            Err(e) => {
                tracing::debug!(error = %format!("{e:#}"), "Lutris's quit action did not answer");
            }
        }
    }
    // Only the process that was found, if it is still that process.
    let comm = std::fs::read_to_string(format!("/proc/{}/comm", open.main)).unwrap_or_default();
    if ["heroic", "heroic-run", "lutris"].contains(&comm.trim()) {
        if let Ok(pid) = libc::pid_t::try_from(open.main) {
            // SAFETY: kill has no memory effects; the pid was checked above.
            unsafe {
                libc::kill(pid, libc::SIGTERM);
            }
        }
    }
}

/// Lutris's `app.quit` (Ctrl+Q), over its `GApplication` interface. Only to
/// a Lutris that holds its name: a call to a free name could start one.
fn quit_lutris() -> Result<()> {
    let conn = zbus::blocking::Connection::session().context("session bus")?;
    let owned = zbus::blocking::fdo::DBusProxy::new(&conn)?
        .name_has_owner(zbus::names::BusName::try_from(LUTRIS_FLATPAK)?)?;
    anyhow::ensure!(
        owned,
        "Lutris does not hold {LUTRIS_FLATPAK} on the session bus"
    );
    let empty: Vec<zbus::zvariant::Value<'_>> = Vec::new();
    let platform: HashMap<&str, zbus::zvariant::Value<'_>> = HashMap::new();
    conn.call_method(
        Some(LUTRIS_FLATPAK),
        "/net/lutris/Lutris",
        Some("org.gtk.Actions"),
        "Activate",
        &("quit", empty, platform),
    )
    .context("org.gtk.Actions.Activate quit")?;
    Ok(())
}

/// Wait up to `timeout` for `launcher` to be gone — a Flatpak's sandbox
/// included, so the new copy does not meet the old one on its way out.
fn wait_closed(launcher: Launcher, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        let procs = snapshot();
        let open = find_open(&procs).iter().any(|o| o.launcher == launcher)
            || launcher
                .flatpak_id()
                .is_some_and(|id| procs.iter().any(|p| flatpak_of(&p.cgroup) == Some(id)));
        if !open {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(500));
    }
}

/// Open `launcher` in the session; a native one from `exe` when its name is
/// not on the `PATH`.
fn start(launcher: Launcher, exe: Option<&Path>) -> Result<()> {
    let on_path = crate::capabilities::which(launcher.program()).is_some();
    let program = match exe.and_then(Path::to_str) {
        Some(exe) if !on_path && launcher.flatpak_id().is_none() && exe.starts_with('/') => exe,
        _ => launcher.program(),
    };
    start_in_session(launcher.unit(), &launcher.command(program, &[]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[allow(clippy::similar_names)] // pid and ppid are what /proc calls them
    fn proc(pid: u32, ppid: u32, comm: &str, args: &str, cgroup: &str) -> Proc {
        Proc {
            pid,
            ppid,
            comm: comm.to_owned(),
            args: args.split_whitespace().map(str::to_owned).collect(),
            cgroup: cgroup.to_owned(),
        }
    }

    const APPS: &str = "/user.slice/user-1000.slice/user@1000.service/app.slice";

    fn heroic_scope(n: u32) -> String {
        format!("{APPS}/app-flatpak-com.heroicgameslauncher.hgl-{n}.scope")
    }

    /// Heroic's Flatpak as the reference desktop had it, idle: the sandbox,
    /// the `heroic-run` script, Electron and its helpers (one set started
    /// through Zypak in a scope of its own), and Flatpak's own processes.
    fn heroic_idle() -> Vec<Proc> {
        let main = heroic_scope(3_718_150_323);
        let zypak = heroic_scope(281_242_882);
        vec![
            proc(1, 0, "systemd", "/usr/lib/systemd/systemd --user", ""),
            proc(10, 1, "bwrap", "bwrap --args 76 -- heroic-run", &main),
            proc(11, 10, "bwrap", "bwrap --args 76 -- heroic-run", &main),
            proc(12, 11, "heroic-run", "/bin/sh /app/bin/heroic-run", &main),
            proc(13, 12, "heroic", "/app/bin/heroic/heroic", &main),
            proc(
                14,
                13,
                "heroic",
                "/app/bin/heroic/heroic --type=zygote",
                &main,
            ),
            proc(
                15,
                14,
                "heroic",
                "/app/bin/heroic/heroic --type=gpu-process",
                &main,
            ),
            proc(16, 11, "cat", "cat", &main),
            proc(17, 1, "xdg-dbus-proxy", "xdg-dbus-proxy", &main),
            proc(
                18,
                2,
                "bwrap",
                "bwrap --args 72 -- /app/bin/zypak-helper",
                &zypak,
            ),
            proc(
                19,
                18,
                "heroic",
                "/app/bin/heroic/heroic --type=zygote",
                &zypak,
            ),
        ]
    }

    /// The same Heroic playing The Outer Worlds: umu's wrapper in the
    /// sandbox, the game itself in a scope Flatpak's portal made for it.
    fn heroic_playing() -> Vec<Proc> {
        let main = heroic_scope(3_718_150_323);
        let game = heroic_scope(1_804_029_661);
        let mut procs = heroic_idle();
        procs.extend([
            proc(
                30,
                11,
                "gamemoderun",
                "/bin/bash /app/bin/gamemoderun umu_run.py",
                &main,
            ),
            proc(31, 30, "python3", "python3 umu_run.py", &main),
            proc(40, 2, "bwrap", "bwrap --args 94 -- pv-adverb", &game),
            proc(41, 40, "pv-adverb", "pv-adverb", &game),
            proc(
                42,
                41,
                "TheOuterWorldsS",
                "S:\\Games\\TheOuterWorlds.exe",
                &game,
            ),
        ]);
        procs
    }

    #[test]
    fn heroics_flatpak_is_found_by_its_script_and_asked_to_close_through_electron() {
        let open = find_open(&heroic_idle());
        assert_eq!(open.len(), 1, "{open:?}");
        assert_eq!(open[0].launcher, Launcher::Heroic { flatpak: true });
        // SIGTERM goes to Electron's main process, the environment is read
        // from the script (Electron writes over its own).
        assert_eq!(open[0].main, 13);
        assert_eq!(open[0].env_from, 12);
    }

    #[test]
    fn a_game_heroic_started_keeps_heroic_open() {
        let procs = heroic_idle();
        assert!(!busy(&find_open(&procs)[0], &procs));
        let procs = heroic_playing();
        assert!(busy(&find_open(&procs)[0], &procs));
        // Only the game's scope left, the sandbox's wrapper gone: still a game.
        let procs: Vec<Proc> = heroic_playing()
            .into_iter()
            .filter(|p| !matches!(p.pid, 30 | 31))
            .collect();
        assert!(busy(&find_open(&procs)[0], &procs));
    }

    #[test]
    fn steam_is_busy_while_a_reaper_runs_a_game() {
        let steam_unit = format!("{APPS}/app-steam-bigame-1-2.service");
        let mut procs = vec![
            proc(
                50,
                1,
                "bash",
                "bash /home/u/.local/share/Steam/steam.sh",
                &steam_unit,
            ),
            proc(
                51,
                50,
                "steam",
                "/home/u/.local/share/Steam/ubuntu12_32/steam",
                &steam_unit,
            ),
            proc(52, 51, "steamwebhelper", "./steamwebhelper", &steam_unit),
        ];
        let open = find_open(&procs);
        assert_eq!(open.len(), 1);
        assert_eq!(open[0].launcher, Launcher::Steam);
        assert_eq!(open[0].main, 51);
        assert!(!busy(&open[0], &procs));
        procs.push(proc(
            53,
            51,
            "reaper",
            "/home/u/.local/share/Steam/ubuntu12_32/reaper SteamLaunch AppId=750920 -- x",
            &steam_unit,
        ));
        assert!(busy(&open[0], &procs));
    }

    #[test]
    fn a_native_launcher_counts_what_it_started_and_its_own_unit() {
        let unit = format!("{APPS}/app-net.lutris.Lutris@1234.service");
        let procs = vec![
            proc(60, 1, "lutris", "/usr/bin/python3 /usr/bin/lutris", &unit),
            proc(61, 60, "lutris-wrapper", "lutris-wrapper Celeste", &unit),
            proc(62, 61, "Celeste.bin.x86", "/g/Celeste.bin.x86_64", &unit),
        ];
        let open = find_open(&procs);
        assert_eq!(open[0].launcher, Launcher::Lutris { flatpak: false });
        assert!(busy(&open[0], &procs));
        // A game that left the tree (reparented to the manager) is still in
        // Lutris's unit.
        let reparented = vec![
            procs[0].clone(),
            proc(62, 1, "Celeste.bin.x86", "/g/Celeste.bin.x86_64", &unit),
        ];
        assert!(busy(&find_open(&reparented)[0], &reparented));
        assert!(!busy(&open[0], &procs[..1]));
        // Started from a terminal, the scope is the terminal's: what else runs
        // there is not Lutris's.
        let terminal = format!("{APPS}/app-org.kde.konsole@9.service");
        let procs = vec![
            proc(70, 1, "bash", "bash", &terminal),
            proc(
                71,
                70,
                "lutris",
                "/usr/bin/python3 /usr/bin/lutris",
                &terminal,
            ),
            proc(72, 70, "vim", "vim notes", &terminal),
        ];
        assert!(!busy(&find_open(&procs)[0], &procs));
    }

    #[test]
    fn native_heroic_is_its_electron_main_process() {
        let unit = format!("{APPS}/app-heroic@5.service");
        let procs = vec![
            proc(80, 1, "heroic", "/opt/Heroic/heroic", &unit),
            proc(81, 80, "heroic", "/opt/Heroic/heroic --type=zygote", &unit),
            proc(82, 80, "chrome_crashpad", "chrome_crashpad_handler", &unit),
        ];
        let open = find_open(&procs);
        assert_eq!(open.len(), 1);
        assert_eq!(open[0].launcher, Launcher::Heroic { flatpak: false });
        assert_eq!(open[0].main, 80);
        assert!(!busy(&open[0], &procs));
    }

    #[test]
    fn flatpak_scopes_name_their_application() {
        assert_eq!(
            flatpak_of(&heroic_scope(42)),
            Some("com.heroicgameslauncher.hgl")
        );
        assert_eq!(
            flatpak_of(&format!("{APPS}/app-steam-bigame-1-2.service")),
            None
        );
        assert_eq!(
            flatpak_of(&format!("{APPS}/app-flatpak-x.y.z-abc.scope")),
            None
        );
        assert_eq!(flatpak_of(""), None);
    }

    #[test]
    fn an_environment_is_read_only_when_it_is_one() {
        let env = parse_environ(b"HOME=/home/u\0PATH=/usr/bin\0WINE_FULLSCREEN_FSR=1\0A=b=c\0\0");
        let env = env.unwrap();
        assert_eq!(env["WINE_FULLSCREEN_FSR"], "1");
        assert_eq!(env["A"], "b=c");
        // Electron's main process: its environment area written over.
        let mut blanks = vec![b' '; 4000];
        blanks.extend_from_slice(b"\0LD_PRELOAD=x\0");
        assert!(parse_environ(&blanks).is_none());
        assert!(parse_environ(b"").is_none());
    }

    fn env(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect()
    }

    #[test]
    fn a_launcher_is_behind_only_when_its_games_would_get_something_else() {
        let session = env(&[
            ("WINE_FULLSCREEN_FSR", "1"),
            ("WINE_FULLSCREEN_FSR_MODE", "performance"),
            (
                "DXVK_CONFIG",
                "dxgi.maxFrameRate = 60; d3d9.maxFrameRate = 60",
            ),
            ("VKD3D_FRAME_RATE", "60"),
        ]);
        // Opened before the 60 FPS cap.
        let before = env(&[
            ("HOME", "/h"),
            ("WINE_FULLSCREEN_FSR", "1"),
            ("WINE_FULLSCREEN_FSR_MODE", "performance"),
        ]);
        assert!(behind(Some(&before), &session));
        // Opened after: the same, whatever else it has.
        let mut after = before.clone();
        after.extend(env(&[
            (
                "DXVK_CONFIG",
                "dxgi.maxFrameRate = 60; d3d9.maxFrameRate = 60",
            ),
            ("VKD3D_FRAME_RATE", "60"),
            ("ENABLE_VKBASALT", "0"),
            ("LANG", "pt_BR.UTF-8"),
        ]));
        assert!(!behind(Some(&after), &session));
        // A switch at 0 is a switch that is off, and its detail goes with it
        // (the reference desktop's Steam: ENABLE_VKBASALT=0 from the login).
        let off = env(&[("ENABLE_VKBASALT", "0"), ("WINE_FULLSCREEN_FSR_MODE", "x")]);
        assert!(!behind(Some(&off), &HashMap::new()));
        // Unreadable: offered, rather than a preset silently missing.
        assert!(behind(None, &session));
    }

    fn game(source: Source) -> DetectedGame {
        DetectedGame {
            name: "Game".into(),
            source,
            app_id: None,
            install_path: None,
            executables: Vec::new(),
            launch_file: None,
            cover: None,
            icon: None,
            launch_command: None,
            launcher: None,
        }
    }

    fn start(game: &DetectedGame) -> Option<Vec<String>> {
        Start::for_game_with(game, |_| true, |_| true).map(|s| s.argv)
    }

    #[test]
    fn each_launcher_gets_its_own_command() {
        let mut steam = game(Source::Steam);
        steam.app_id = Some("750920".into());
        assert_eq!(start(&steam).unwrap(), ["steam", "-applaunch", "750920"]);

        let mut heroic = game(Source::Heroic);
        heroic.launcher = Some(LauncherRef::Heroic {
            app_name: "cb3bf7ba89574a66ae3b795e039d4dbc".into(),
            config_dir: "/home/u/.var/app/com.heroicgameslauncher.hgl/config/heroic".into(),
            runner: Some("legendary"),
        });
        assert_eq!(
            start(&heroic).unwrap(),
            [
                "flatpak",
                "run",
                "--command=heroic-run",
                "com.heroicgameslauncher.hgl",
                "heroic://launch?appName=cb3bf7ba89574a66ae3b795e039d4dbc&runner=legendary",
            ]
        );
        heroic.launcher = Some(LauncherRef::Heroic {
            app_name: "1207658924".into(),
            config_dir: "/home/u/.config/heroic".into(),
            runner: Some("gog"),
        });
        assert_eq!(
            start(&heroic).unwrap(),
            ["heroic", "heroic://launch?appName=1207658924&runner=gog"]
        );

        let mut lutris = game(Source::Lutris);
        lutris.launcher = Some(LauncherRef::Lutris {
            config_file: "/home/u/.config/lutris/games/celeste-1700000000.yml".into(),
        });
        assert_eq!(
            start(&lutris).unwrap(),
            ["lutris", "lutris:rungame/celeste"]
        );
        lutris.launcher = Some(LauncherRef::Lutris {
            config_file:
                "/home/u/.var/app/net.lutris.Lutris/config/lutris/games/celeste-1700000000.yml"
                    .into(),
        });
        assert_eq!(
            start(&lutris).unwrap(),
            [
                "flatpak",
                "run",
                "net.lutris.Lutris",
                "lutris:rungame/celeste"
            ]
        );

        let mut flatpak = game(Source::Flatpak);
        flatpak.app_id = Some("net.supertuxkart.SuperTuxKart".into());
        assert_eq!(
            start(&flatpak).unwrap(),
            ["flatpak", "run", "net.supertuxkart.SuperTuxKart"]
        );
    }

    #[test]
    fn nothing_is_offered_without_the_launcher_or_a_well_formed_id() {
        let mut steam = game(Source::Steam);
        steam.app_id = Some("750920".into());
        assert!(Start::for_game_with(&steam, |_| false, |_| true).is_none());
        for bad in ["", "75 0920", "-shutdown", "750920;rm"] {
            steam.app_id = Some(bad.into());
            assert!(start(&steam).is_none(), "{bad:?}");
        }

        let mut heroic = game(Source::Heroic);
        for (name, runner) in [
            ("Fortnite&runner=gog", Some("legendary")),
            ("-x", Some("legendary")),
            ("a/b", Some("legendary")),
            ("Fortnite", Some("zoom")),
            ("Fortnite", None),
        ] {
            heroic.launcher = Some(LauncherRef::Heroic {
                app_name: name.into(),
                config_dir: "/home/u/.config/heroic".into(),
                runner,
            });
            assert!(start(&heroic).is_none(), "{name:?} {runner:?}");
        }

        let mut lutris = game(Source::Lutris);
        for file in ["/g/Celeste Game-1.yml", "/g/-rf-1.yml", "/g/.yml"] {
            lutris.launcher = Some(LauncherRef::Lutris {
                config_file: file.into(),
            });
            assert!(start(&lutris).is_none(), "{file:?}");
        }

        let mut flatpak = game(Source::Flatpak);
        for id in ["org.example", "--command=sh", "org.ex ample.X", "org.1x.Y"] {
            flatpak.app_id = Some(id.into());
            assert!(start(&flatpak).is_none(), "{id:?}");
        }
        flatpak.app_id = Some("net.supertuxkart.SuperTuxKart".into());
        assert!(Start::for_game_with(&flatpak, |_| true, |_| false).is_none());
        // A native game with neither is not a launcher's.
        assert!(start(&game(Source::Native)).is_none());
    }

    #[test]
    fn a_start_is_a_unit_of_its_own_in_the_session() {
        let argv = systemd_run_argv("heroic", &["flatpak".into(), "run".into()]);
        assert_eq!(argv[..3], ["--user", "--collect", "--quiet"]);
        assert!(argv.contains(&"--property=Type=exec".to_owned()));
        assert!(argv.contains(&"--property=UnsetEnvironment=ELECTRON_RUN_AS_NODE".to_owned()));
        // The command follows `--`, so nothing in it is read as an option.
        let sep = argv.iter().position(|a| a == "--").unwrap();
        assert_eq!(argv[sep + 1..], ["flatpak", "run"]);
        assert!(argv[sep - 1].starts_with("--unit=heroic"));
        let a = unit_name("heroic");
        assert!(a.starts_with(&format!("app-heroic-bigame-{}-", std::process::id())));
        assert_ne!(a, unit_name("heroic"), "two starts, two names");
    }

    #[test]
    fn reopening_starts_each_launcher_by_its_own_command() {
        assert_eq!(Launcher::Steam.command("steam", &[]), ["steam"]);
        assert_eq!(
            Launcher::Heroic { flatpak: true }.command("heroic", &[]),
            ["flatpak", "run", "--command=heroic-run", HEROIC_FLATPAK]
        );
        assert_eq!(
            Launcher::Heroic { flatpak: false }.command("/opt/Heroic/heroic", &[]),
            ["/opt/Heroic/heroic"]
        );
        assert_eq!(
            Launcher::Lutris { flatpak: true }.command("lutris", &[]),
            ["flatpak", "run", LUTRIS_FLATPAK]
        );
    }

    #[test]
    fn this_machines_launchers_are_read_without_panicking() {
        // Read only: nothing is closed or started.
        let procs = snapshot();
        for open in find_open(&procs) {
            let _ = busy(&open, &procs);
        }
    }
}
