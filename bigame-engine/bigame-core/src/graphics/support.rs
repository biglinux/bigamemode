//! A support report for one game's AI Graphics: a zip with what was found,
//! what was planned, what was placed, what the running game loaded, the
//! diagnosis, and the logs that say what happened — and nothing else.
//!
//! Every text file in it has the home folder, user name and host name
//! replaced ([`crate::logs::redact`]). Environment variables, Steam's
//! configuration, credentials and the user's own files are never read for
//! it.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use super::{Analysis, Target, manifest::Manifest, state_dir};

/// Lines of a log kept in the report: the end, which is the current run.
const LOG_TAIL: usize = 3000;
/// At most this much of a log is read for its last [`LOG_TAIL`] lines.
const LOG_BYTES: u64 = 4 * 1024 * 1024;

fn tail(text: &str, lines: usize) -> String {
    let all: Vec<&str> = text.lines().collect();
    all[all.len().saturating_sub(lines)..].join("\n")
}

fn safe_name(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    s.split('-')
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

/// A plain file's text, never through a link: a link planted in a game's
/// folder must not pull another of the user's files into a report they
/// share.
fn read_plain(path: &Path) -> std::io::Result<String> {
    read_plain_tail(path, u64::MAX)
}

/// [`read_plain`], keeping only the last `max` bytes: a debug log can grow
/// to gigabytes, and a report needs its end.
fn read_plain_tail(path: &Path, max: u64) -> std::io::Result<String> {
    use std::io::{Read as _, Seek as _};
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)?;
    if !f.metadata()?.is_file() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("{} is not a plain file", path.display()),
        ));
    }
    let len = f.metadata()?.len();
    if len > max {
        f.seek(std::io::SeekFrom::Start(len - max))?;
    }
    let mut bytes = Vec::new();
    f.take(max).read_to_end(&mut bytes)?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// The identifiers masked in every file.
struct Masks {
    home: String,
    user: String,
    host: String,
}

impl Masks {
    fn here() -> Self {
        Self {
            home: std::env::var("HOME").unwrap_or_default(),
            user: std::env::var("USER").unwrap_or_default(),
            host: std::fs::read_to_string("/etc/hostname")
                .unwrap_or_default()
                .trim()
                .to_owned(),
        }
    }

    fn apply(&self, text: &str) -> String {
        crate::logs::redact(text, &self.home, &self.user, &self.host)
    }
}

/// What goes into the report, as `(file name, contents)` — separated from
/// writing the zip so the redaction can be tested.
// One file after another; splitting it would hide what the report holds.
#[allow(clippy::too_many_lines)]
fn contents(target: &Target, a: &Analysis, masks: &Masks) -> Result<Vec<(String, String)>> {
    let mut files = Vec::new();
    let mut readme = String::new();
    let _ = writeln!(readme, "Big Game Mode AI Graphics report");
    let _ = writeln!(readme, "Game: {} ({})", target.name, target.process);
    let _ = writeln!(readme, "Big Game Mode: {}", env!("CARGO_PKG_VERSION"));
    let _ = writeln!(readme, "Plan: {} [{:?}]", a.plan.summary, a.plan.standing);
    let _ = writeln!(readme, "Status: {:?}", a.status);
    let _ = writeln!(
        readme,
        "\nHome folder, user and host names are replaced in every file."
    );
    files.push(("README.txt".into(), readme));
    // The machine, as the diagnostics page describes it: no host name, user
    // or address is in it.
    let hw = crate::hardware::Hardware::detect();
    let (gpus, render) = super::report::gpu_infos(&hw, None);
    let mut system = String::new();
    let _ = writeln!(system, "kernel: {}", hw.kernel);
    let _ = writeln!(system, "cpu: {}", hw.cpu.model);
    for (i, g) in gpus.iter().enumerate() {
        let _ = writeln!(
            system,
            "gpu: {} · {} · {} · {}{}",
            g.card,
            g.name,
            g.family().label(),
            g.userspace.clone().unwrap_or_else(|| g.driver.clone()),
            if Some(i) == render {
                " · games render here"
            } else {
                ""
            }
        );
    }
    files.push(("system.txt".into(), system));
    files.push((
        "report.json".into(),
        serde_json::to_string_pretty(&a.report)?,
    ));
    files.push((
        "diagnose.txt".into(),
        super::diagnose::render(&super::diagnose::diagnose(a)),
    ));
    if let Some(p) = &a.report.proton {
        let mut proton = String::new();
        let _ = writeln!(proton, "tool: {}", p.tool.clone().unwrap_or_default());
        let _ = writeln!(proton, "prefix: {}", p.prefix.display());
        let _ = writeln!(
            proton,
            "windows: {}",
            p.windows_version.clone().unwrap_or_default()
        );
        let _ = writeln!(
            proton,
            "fsr4_provider (amdxcffx64.dll): {}",
            p.fsr4_provider
        );
        let _ = writeln!(proton, "hip_runtime (amdhip64_7.dll): {}", p.hip_runtime);
        files.push(("proton.txt".into(), proton));
    }
    let mut conflicts = String::new();
    for r in &a.plan.problems {
        let _ = writeln!(
            conflicts,
            "{:?}: {} + {} — {}",
            r.verdict,
            r.a.label(),
            r.b.label(),
            r.why
        );
    }
    files.push(("conflicts.txt".into(), conflicts));
    files.push((
        "neural.json".into(),
        serde_json::to_string_pretty(&a.neural)?,
    ));
    // What the running game has mapped: which DLLs it really loaded, and
    // through which translation layer. Only library paths, nothing else.
    if let Some(g) =
        crate::running::detect().filter(|g| g.process_name.eq_ignore_ascii_case(&target.process))
        && let Ok(maps) = std::fs::read_to_string(format!("/proc/{}/maps", g.pid))
    {
        let mut modules = String::new();
        for p in super::runtime::mapped_paths(&maps) {
            let name = p.to_string_lossy();
            if name.ends_with(".dll")
                || name.ends_with(".so")
                || name.contains(".so.")
                || name.contains("/proton")
                || name.contains("/Proton")
            {
                let _ = writeln!(modules, "{name}");
            }
        }
        files.push(("loaded-modules.txt".into(), modules));
    }
    files.push(("plan.json".into(), serde_json::to_string_pretty(&a.plan)?));
    files.push((
        "status.json".into(),
        serde_json::to_string_pretty(&a.status)?,
    ));
    let state = state_dir();
    let key = target.key();
    match Manifest::load(&state, &key) {
        Ok(Some(m)) => files.push(("manifest.json".into(), serde_json::to_string_pretty(&m)?)),
        Ok(None) => {}
        // A record that does not load is what the report is for: as it is.
        Err(e) => {
            let raw = read_plain(&Manifest::path(&state, &key)).unwrap_or_default();
            files.push(("manifest.json".into(), raw));
            files.push(("manifest-error.txt".into(), format!("{e:#}\n")));
        }
    }
    let exe_dir = a
        .report
        .executable
        .as_ref()
        .and_then(|e| e.parent())
        .map_or_else(
            || target.install_root.clone(),
            |d| target.install_root.join(d),
        );
    if let Ok(ini) = read_plain(&exe_dir.join("OptiScaler.ini")) {
        files.push(("OptiScaler.ini".into(), ini));
    }
    let log = read_plain_tail(&exe_dir.join("OptiScaler.log"), LOG_BYTES)
        .or_else(|_| read_plain_tail(&state.join(&key).join("last-run/OptiScaler.log"), LOG_BYTES));
    if let Ok(log) = log {
        files.push(("OptiScaler.log".into(), tail(&log, LOG_TAIL)));
    }
    if let Ok(log) = read_plain_tail(&exe_dir.join(super::external::LOG), LOG_BYTES) {
        files.push((super::external::LOG.into(), tail(&log, LOG_TAIL)));
    }
    // Copies kept after a removal: originals of files another program
    // changed since, and configurations with the user's edits.
    let mut kept = String::new();
    for k in super::transaction::kept(&state)
        .into_iter()
        .filter(|k| k.game_key == key)
    {
        let _ = writeln!(kept, "{} → {}", k.file.display(), k.copy.display());
    }
    if !kept.is_empty() {
        files.push(("kept-copies.txt".into(), kept));
    }
    if let Ok((entries, _)) = crate::logs::read(600, None) {
        let mut journal = String::new();
        for e in entries {
            let _ = writeln!(journal, "{:?} {:?} {}", e.source, e.level, e.message);
        }
        files.push(("bigame-mode-journal.txt".into(), journal));
    }
    Ok(files
        .into_iter()
        .map(|(name, text)| (name, masks.apply(&text)))
        .collect())
}

/// Write the report for `target` into `dest_dir`, returning the zip's path
/// (`bigamemode-report-<game>-<unix time>.zip`).
///
/// # Errors
/// Returns an error if a file cannot be written or `bsdtar` fails.
pub fn write_report(target: &Target, a: &Analysis, dest_dir: &Path) -> Result<PathBuf> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let work = state_dir().join(target.key()).join(format!("report-{now}"));
    std::fs::create_dir_all(&work)?;
    let result = (|| -> Result<PathBuf> {
        for (name, text) in contents(target, a, &Masks::here())? {
            std::fs::write(work.join(&name), text)?;
        }
        std::fs::create_dir_all(dest_dir)?;
        let zip = dest_dir.join(format!(
            "bigamemode-report-{}-{now}.zip",
            safe_name(&target.name)
        ));
        let status = std::process::Command::new("bsdtar")
            .args(["--format", "zip", "-cf"])
            .arg(&zip)
            .arg("-C")
            .arg(&work)
            .arg(".")
            .status()
            .context("run bsdtar")?;
        anyhow::ensure!(status.success(), "bsdtar could not write {}", zip.display());
        Ok(zip)
    })();
    let _ = std::fs::remove_dir_all(&work);
    result
}

#[cfg(test)]
mod tests {

    #[test]
    fn a_log_is_read_from_its_end_and_never_through_a_link() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("OptiScaler.log");
        std::fs::write(&log, "old\nmiddle\nnew\n").unwrap();
        assert_eq!(read_plain_tail(&log, 4).unwrap(), "new\n");
        assert_eq!(read_plain_tail(&log, 1000).unwrap(), "old\nmiddle\nnew\n");
        let link = dir.path().join("link.log");
        std::os::unix::fs::symlink(&log, &link).unwrap();
        assert!(read_plain_tail(&link, 1000).is_err());
    }

    use super::*;

    #[test]
    fn identifiers_are_masked_and_the_log_is_its_end() {
        let m = Masks {
            home: "/home/ruscher".into(),
            user: "ruscher".into(),
            host: "ruscher-big".into(),
        };
        let t = m.apply("S:\\ /home/ruscher/Games/x ruscher@ruscher-big");
        assert!(!t.contains("ruscher"), "{t}");
        assert!(t.contains("~/Games/x"));
        let log: String = (0..10).fold(String::new(), |mut s, i| {
            let _ = writeln!(s, "{i}");
            s
        });
        assert_eq!(tail(&log, 3), "7\n8\n9");
        assert_eq!(
            safe_name("Shadow of the Tomb Raider!"),
            "Shadow-of-the-Tomb-Raider"
        );
    }

    #[test]
    fn a_link_in_the_game_folder_is_not_read_into_the_report() {
        let dir = tempfile::tempdir().unwrap();
        let secret = dir.path().join("secret.txt");
        std::fs::write(&secret, "private").unwrap();
        let game = dir.path().join("game");
        std::fs::create_dir(&game).unwrap();
        std::os::unix::fs::symlink(&secret, game.join("OptiScaler.log")).unwrap();
        assert!(read_plain(&game.join("OptiScaler.log")).is_err());
        std::fs::create_dir(game.join("OptiScaler.ini")).unwrap();
        assert!(read_plain(&game.join("OptiScaler.ini")).is_err());
        std::fs::write(game.join("dlssnr_on_amd.log"), "a log").unwrap();
        assert_eq!(
            read_plain(&game.join("dlssnr_on_amd.log")).unwrap(),
            "a log"
        );
    }
}
