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
    let _ = writeln!(readme, "BiGame-mode AI Graphics report");
    let _ = writeln!(readme, "Game: {} ({})", target.name, target.process);
    let _ = writeln!(readme, "BiGame-mode: {}", env!("CARGO_PKG_VERSION"));
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
    {
        if let Ok(maps) = std::fs::read_to_string(format!("/proc/{}/maps", g.pid)) {
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
    }
    files.push(("plan.json".into(), serde_json::to_string_pretty(&a.plan)?));
    files.push((
        "status.json".into(),
        serde_json::to_string_pretty(&a.status)?,
    ));
    let state = state_dir();
    let key = target.key();
    if let Some(m) = Manifest::load(&state, &key)? {
        files.push(("manifest.json".into(), serde_json::to_string_pretty(&m)?));
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
    if let Ok(ini) = std::fs::read_to_string(exe_dir.join("OptiScaler.ini")) {
        files.push(("OptiScaler.ini".into(), ini));
    }
    let log = std::fs::read_to_string(exe_dir.join("OptiScaler.log"))
        .or_else(|_| std::fs::read_to_string(state.join(&key).join("last-run/OptiScaler.log")));
    if let Ok(log) = log {
        files.push(("OptiScaler.log".into(), tail(&log, LOG_TAIL)));
    }
    if let Ok(log) = std::fs::read_to_string(exe_dir.join(super::external::LOG)) {
        files.push((super::external::LOG.into(), tail(&log, LOG_TAIL)));
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
}
