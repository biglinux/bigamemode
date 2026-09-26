//! The game's own setting that `OptiScaler` needs switched on.
//!
//! `OptiScaler` takes over an upscaler the game already runs — its `XeSS`,
//! DLSS or FSR — so nothing happens until that upscaler is selected in the
//! game's menu. For a game whose game-list entry says where that setting is
//! kept (a value in its Proton prefix's registry), Apply switches it on when
//! it is off, and Restore puts back what was there, as it does with files.
//!
//! Only DWORD values under `HKEY_CURRENT_USER`, only in the game's own
//! prefix, and only while no process of that prefix runs: Wine keeps the
//! registry in memory and writes it back when its server exits, so a change
//! made while the server is up would be lost.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use super::optiscaler::Input;
use crate::error::UserError;
use crate::text::N_;

/// Where a game keeps the switch for one of its upscalers (game list).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InputSetting {
    /// Which of the game's upscalers it switches on.
    pub input: Input,
    /// The key under `HKEY_CURRENT_USER`, with single backslashes.
    pub registry: String,
    /// The DWORD value that selects the upscaler; 0 is off.
    pub value: String,
    /// What is written when it is off: the preset the entry was verified with.
    pub on: u32,
    /// Values of the game's other upscalers, set to 0 when this one is
    /// switched on: the game runs one at a time.
    #[serde(default)]
    pub exclusive: Vec<String>,
}

/// One value Apply changed, kept in the manifest for Restore.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SettingChange {
    /// The registry file (`<prefix>/user.reg`), absolute.
    pub file: PathBuf,
    /// The key, with single backslashes.
    pub key: String,
    /// The value name.
    pub value: String,
    /// What BiGame-mode wrote.
    pub set: u32,
    /// What was there before; `None` when the value did not exist.
    pub original: Option<u32>,
}

/// What Apply did with the game's setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Applied {
    /// Switched on; the change is in the manifest.
    TurnedOn(Input),
    /// Already on, at whatever preset the user chose: left alone.
    AlreadyOn(Input),
    /// Not written; the upscaler still has to be chosen in the game's menu.
    NotWritten(Input, Skip),
}

/// Why the setting was not written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Skip {
    /// The game has no Proton prefix (not a Steam game, or never started).
    NoPrefix,
    /// The game has not saved its settings in the prefix yet.
    NeverRan,
    /// A process of the prefix kept running.
    PrefixBusy,
    /// The registry could not be read or written.
    Failed,
}

/// What Restore did with one value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Restored {
    /// Put back as it was before Apply.
    PutBack(String),
    /// Changed since Apply (in the game's menu): left as it is now.
    LeftAsChanged(String),
}

/// How long to wait for the prefix's Wine server, which stays up a few
/// seconds after the game exits.
const SETTLE: Duration = Duration::from_secs(10);

/// Switch the game's upscaler on in `prefix`, when it is off.
///
/// Never fails: whatever stops the write is reported as [`Skip`], and the
/// game is then as it was. The changes returned belong in the manifest.
#[must_use]
pub fn switch_on(prefix: Option<&Path>, s: &InputSetting) -> (Applied, Vec<SettingChange>) {
    switch_on_with(prefix, s, &prefix_in_use, SETTLE)
}

fn switch_on_with(
    prefix: Option<&Path>,
    s: &InputSetting,
    in_use: &dyn Fn(&Path) -> bool,
    settle: Duration,
) -> (Applied, Vec<SettingChange>) {
    let skip = |why| (Applied::NotWritten(s.input, why), Vec::new());
    let Some(prefix) = prefix else {
        return skip(Skip::NoPrefix);
    };
    if !wait_until_free(prefix, in_use, settle) {
        return skip(Skip::PrefixBusy);
    }
    let file = prefix.join("user.reg");
    let Ok(text) = std::fs::read_to_string(&file) else {
        return skip(Skip::Failed);
    };
    match switch_text(&text, s, &file) {
        None => skip(Skip::NeverRan),
        Some(Switch::AlreadyOn) => (Applied::AlreadyOn(s.input), Vec::new()),
        Some(Switch::Write(new, changes)) => match write_atomic(&file, &new) {
            Ok(()) => {
                tracing::info!(target: "graphics", key = %s.registry, value = %s.value, set = s.on,
                    "the game's upscaler switched on in its settings");
                (Applied::TurnedOn(s.input), changes)
            }
            Err(e) => {
                tracing::warn!(target: "graphics", error = %format!("{e:#}"),
                    "the game's settings could not be written");
                skip(Skip::Failed)
            }
        },
    }
}

enum Switch {
    AlreadyOn,
    Write(String, Vec<SettingChange>),
}

/// The registry text with the upscaler switched on, or `None` when the key
/// is not there — the game has never saved its settings.
fn switch_text(text: &str, s: &InputSetting, file: &Path) -> Option<Switch> {
    if !reg::has_key(text, &s.registry) {
        return None;
    }
    let current = reg::get(text, &s.registry, &s.value);
    if current.is_some_and(|v| v != 0) {
        return Some(Switch::AlreadyOn);
    }
    let mut new = reg::set(text, &s.registry, &s.value, Some(s.on))?;
    let change = |value: &str, set, original| SettingChange {
        file: file.to_owned(),
        key: s.registry.clone(),
        value: value.to_owned(),
        set,
        original,
    };
    let mut changes = vec![change(&s.value, s.on, current)];
    for other in &s.exclusive {
        let was = reg::get(&new, &s.registry, other);
        if was.is_some_and(|v| v != 0) {
            new = reg::set(&new, &s.registry, other, Some(0))?;
            changes.push(change(other, 0, was));
        }
    }
    Some(Switch::Write(new, changes))
}

/// Put back what Apply changed. A value changed again since (in the game's
/// menu) is the user's choice and stays.
///
/// # Errors
/// Returns an error, having changed nothing, if a process of the prefix
/// keeps running or the registry cannot be read or written.
pub fn restore(changes: &[SettingChange]) -> Result<Vec<Restored>> {
    restore_with(changes, &prefix_in_use, SETTLE)
}

fn restore_with(
    changes: &[SettingChange],
    in_use: &dyn Fn(&Path) -> bool,
    settle: Duration,
) -> Result<Vec<Restored>> {
    let mut out = Vec::new();
    let mut files: Vec<&Path> = changes.iter().map(|c| c.file.as_path()).collect();
    files.dedup();
    for file in files {
        let prefix = file.parent().context("registry file without a folder")?;
        if !wait_until_free(prefix, in_use, settle) {
            bail!(UserError::plain(N_(
                "the game's Wine prefix is still in use; close the game completely and try again"
            )));
        }
        let mut text =
            std::fs::read_to_string(file).with_context(|| format!("read {}", file.display()))?;
        for c in changes.iter().filter(|c| c.file == file) {
            if reg::get(&text, &c.key, &c.value) == Some(c.set) {
                if let Some(t) = reg::set(&text, &c.key, &c.value, c.original) {
                    text = t;
                }
                out.push(Restored::PutBack(c.value.clone()));
            } else {
                out.push(Restored::LeftAsChanged(c.value.clone()));
            }
        }
        write_atomic(file, &text)?;
    }
    Ok(out)
}

fn wait_until_free(prefix: &Path, in_use: &dyn Fn(&Path) -> bool, settle: Duration) -> bool {
    let start = Instant::now();
    loop {
        if !in_use(prefix) {
            return true;
        }
        if start.elapsed() >= settle {
            return false;
        }
        std::thread::sleep(Duration::from_millis(500));
    }
}

/// Whether any process of this user runs in `prefix`: the game, its
/// launcher, or the Wine server that writes the registry back on exit.
#[must_use]
pub fn prefix_in_use(prefix: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    let want = normalize(prefix);
    let Ok(procs) = std::fs::read_dir("/proc") else {
        return false;
    };
    procs.flatten().any(|p| {
        // Another user's environment is unreadable, and not our prefix.
        std::fs::read(p.path().join("environ")).is_ok_and(|env| {
            env.split(|b| *b == 0).any(|var| {
                var.strip_prefix(b"WINEPREFIX=")
                    .is_some_and(|v| normalize(Path::new(std::ffi::OsStr::from_bytes(v))) == want)
            })
        })
    })
}

fn normalize(p: &Path) -> PathBuf {
    std::fs::canonicalize(p).unwrap_or_else(|_| p.components().collect())
}

/// Replace `file` atomically, keeping its permissions; never through a link.
fn write_atomic(file: &Path, text: &str) -> Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let meta = std::fs::symlink_metadata(file)?;
    if meta.file_type().is_symlink() {
        bail!(UserError::with(
            N_("%s is a symlink; refusing to replace it"),
            [file.display().to_string()]
        ));
    }
    let tmp = file.with_file_name(".user.reg.bigame-new");
    match std::fs::remove_file(&tmp) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e).with_context(|| format!("remove {}", tmp.display())),
    }
    {
        let mut out = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&tmp)
            .with_context(|| format!("create {}", tmp.display()))?;
        out.write_all(text.as_bytes())?;
        out.set_permissions(meta.permissions())?;
        out.sync_all()?;
    }
    std::fs::rename(&tmp, file)
        .with_context(|| format!("rename {} → {}", tmp.display(), file.display()))
}

/// DWORD values in a Wine registry file (`user.reg`), edited as text.
///
/// A key is a section, `[Software\\Vendor\\Game] 1790250876`, with its
/// backslashes doubled; a value is a line, `"Name"=dword:00000003`. Key and
/// value names compare without case, as in the registry. Everything else in
/// the file is left byte for byte as it was.
mod reg {
    /// The key a section header names, backslashes single.
    fn header_key(line: &str) -> Option<String> {
        let rest = line.strip_prefix('[')?;
        let end = rest.rfind(']')?;
        Some(rest[..end].replace("\\\\", "\\"))
    }

    /// The value name and data of a value line.
    fn value_line(line: &str) -> Option<(&str, &str)> {
        let rest = line.strip_prefix('"')?;
        let end = rest.find("\"=")?;
        Some((&rest[..end], &rest[end + 2..]))
    }

    /// The lines of `key`'s section: its header, and the end (exclusive).
    fn section(lines: &[&str], key: &str) -> Option<(usize, usize)> {
        let start = lines
            .iter()
            .position(|l| header_key(l).is_some_and(|k| k.eq_ignore_ascii_case(key)))?;
        let end = lines[start + 1..]
            .iter()
            .position(|l| l.starts_with('['))
            .map_or(lines.len(), |i| start + 1 + i);
        Some((start, end))
    }

    /// Whether the key is there: the game has saved its settings.
    pub fn has_key(text: &str, key: &str) -> bool {
        section(&text.split('\n').collect::<Vec<_>>(), key).is_some()
    }

    /// The value as a DWORD; `None` when the key or the value is not there,
    /// or the value is not a DWORD.
    pub fn get(text: &str, key: &str, value: &str) -> Option<u32> {
        let lines: Vec<&str> = text.split('\n').collect();
        let (start, end) = section(&lines, key)?;
        lines[start + 1..end].iter().find_map(|l| {
            let (name, data) = value_line(l)?;
            name.eq_ignore_ascii_case(value)
                .then(|| u32::from_str_radix(data.strip_prefix("dword:")?.trim(), 16).ok())?
        })
    }

    /// `text` with the value set to `data`, or removed for `None`; `None`
    /// when the key is not there.
    pub fn set(text: &str, key: &str, value: &str, data: Option<u32>) -> Option<String> {
        let mut lines: Vec<String> = text.split('\n').map(str::to_owned).collect();
        let borrowed: Vec<&str> = lines.iter().map(String::as_str).collect();
        let (start, end) = section(&borrowed, key)?;
        let found = (start + 1..end)
            .find(|&i| value_line(&lines[i]).is_some_and(|(n, _)| n.eq_ignore_ascii_case(value)));
        match (found, data) {
            (Some(i), Some(d)) => {
                let name = value_line(&lines[i]).map(|(n, _)| n.to_owned())?;
                lines[i] = format!("\"{name}\"={}", dword(d));
            }
            (Some(i), None) => {
                lines.remove(i);
            }
            (None, Some(d)) => {
                // Before the blank line that ends the section.
                let mut at = end;
                while at > start + 1 && lines[at - 1].trim().is_empty() {
                    at -= 1;
                }
                lines.insert(at, format!("\"{value}\"={}", dword(d)));
            }
            (None, None) => {}
        }
        Some(lines.join("\n"))
    }

    fn dword(d: u32) -> String {
        format!("dword:{d:08x}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// As Proton wrote it, trimmed.
    const USER_REG: &str = "WINE REGISTRY Version 2\n;; All keys relative to \\\\User\\\\S-1-5-21-0-0-0-1000\n\n#arch=win64\n\n[Software\\\\Eidos Montreal\\\\Shadow of the Tomb Raider\\\\Graphics] 1790250876\n#time=1dd4c1b78ad5cc8\n\"AA\"=dword:00000002\n\"DLSS\"=dword:00000000\n\"DLSS Previous\"=dword:00000003\n\"XESS\"=dword:00000000\n\"XESS Previous\"=dword:00000000\n\n[Software\\\\Eidos Montreal\\\\Shadow of the Tomb Raider\\\\Input] 1790201175\n\"XESS\"=dword:00000007\n";

    const KEY: &str = r"Software\Eidos Montreal\Shadow of the Tomb Raider\Graphics";

    fn sottr() -> InputSetting {
        InputSetting {
            input: Input::Xess,
            registry: KEY.to_owned(),
            value: "XESS".to_owned(),
            on: 3,
            exclusive: vec!["DLSS".to_owned()],
        }
    }

    fn prefix_with(text: &str) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("user.reg");
        std::fs::write(&file, text).unwrap();
        (dir, file)
    }

    fn free(_: &Path) -> bool {
        false
    }

    #[test]
    fn values_are_read_and_written_in_their_own_key_only() {
        assert_eq!(reg::get(USER_REG, KEY, "xess"), Some(0));
        assert_eq!(reg::get(USER_REG, KEY, "DLSS Previous"), Some(3));
        assert_eq!(reg::get(USER_REG, KEY, "Missing"), None);
        assert!(reg::has_key(USER_REG, KEY) && !reg::has_key(USER_REG, r"Software\Nobody"));

        let on = reg::set(USER_REG, KEY, "XESS", Some(3)).unwrap();
        assert_eq!(reg::get(&on, KEY, "XESS"), Some(3));
        assert!(on.contains("\"XESS\"=dword:00000003\n\"XESS Previous\""));
        // The same value name in another key is untouched.
        let input = r"Software\Eidos Montreal\Shadow of the Tomb Raider\Input";
        assert_eq!(reg::get(&on, input, "XESS"), Some(7));
        // Only that line changed.
        assert_eq!(on.len(), USER_REG.len());

        let added = reg::set(USER_REG, KEY, "New", Some(1)).unwrap();
        assert!(
            added.contains("\"XESS Previous\"=dword:00000000\n\"New\"=dword:00000001\n\n[Software")
        );
        assert_eq!(reg::set(&added, KEY, "New", None).unwrap(), USER_REG);
    }

    #[test]
    fn apply_switches_the_upscaler_on_and_restore_puts_it_back() {
        let dlss_on = USER_REG.replace("\"DLSS\"=dword:00000000", "\"DLSS\"=dword:00000002");
        let (dir, file) = prefix_with(&dlss_on);
        let (applied, changes) = switch_on_with(Some(dir.path()), &sottr(), &free, Duration::ZERO);
        assert_eq!(applied, Applied::TurnedOn(Input::Xess));
        let text = std::fs::read_to_string(&file).unwrap();
        assert_eq!(reg::get(&text, KEY, "XESS"), Some(3));
        assert_eq!(
            reg::get(&text, KEY, "DLSS"),
            Some(0),
            "one upscaler at a time"
        );
        assert_eq!(changes.len(), 2);
        assert_eq!((changes[0].set, changes[0].original), (3, Some(0)));
        assert_eq!((changes[1].set, changes[1].original), (0, Some(2)));

        let restored = restore_with(&changes, &free, Duration::ZERO).unwrap();
        assert_eq!(
            restored,
            [
                Restored::PutBack("XESS".into()),
                Restored::PutBack("DLSS".into())
            ]
        );
        assert_eq!(std::fs::read_to_string(&file).unwrap(), dlss_on);
    }

    #[test]
    fn a_preset_the_user_chose_is_left_alone() {
        // XeSS already on (Performance): Apply leaves it.
        let (dir, file) =
            prefix_with(&USER_REG.replace("\"XESS\"=dword:00000000", "\"XESS\"=dword:00000001"));
        let before = std::fs::read_to_string(&file).unwrap();
        let (applied, changes) = switch_on_with(Some(dir.path()), &sottr(), &free, Duration::ZERO);
        assert_eq!(
            (applied, changes.len()),
            (Applied::AlreadyOn(Input::Xess), 0)
        );
        assert_eq!(std::fs::read_to_string(&file).unwrap(), before);

        // Switched on by Apply, then changed in the game's menu: Restore
        // keeps the user's choice.
        let (dir, file) = prefix_with(USER_REG);
        let (_, changes) = switch_on_with(Some(dir.path()), &sottr(), &free, Duration::ZERO);
        let text = std::fs::read_to_string(&file).unwrap();
        std::fs::write(&file, reg::set(&text, KEY, "XESS", Some(2)).unwrap()).unwrap();
        let restored = restore_with(&changes, &free, Duration::ZERO).unwrap();
        assert_eq!(restored, [Restored::LeftAsChanged("XESS".into())]);
        let text = std::fs::read_to_string(&file).unwrap();
        assert_eq!(reg::get(&text, KEY, "XESS"), Some(2));
    }

    #[test]
    fn nothing_is_written_when_it_cannot_be_done_safely() {
        let s = sottr();
        assert_eq!(
            switch_on_with(None, &s, &free, Duration::ZERO).0,
            Applied::NotWritten(Input::Xess, Skip::NoPrefix)
        );
        // The game never saved its settings: its key is not there yet.
        let (dir, file) = prefix_with("WINE REGISTRY Version 2\n\n[Software\\\\Wine] 1\n");
        let before = std::fs::read_to_string(&file).unwrap();
        assert_eq!(
            switch_on_with(Some(dir.path()), &s, &free, Duration::ZERO).0,
            Applied::NotWritten(Input::Xess, Skip::NeverRan)
        );
        assert_eq!(std::fs::read_to_string(&file).unwrap(), before);
        // Wine's server is still up: it would write its own copy back.
        let (dir, file) = prefix_with(USER_REG);
        let busy = |_: &Path| true;
        assert_eq!(
            switch_on_with(Some(dir.path()), &s, &busy, Duration::ZERO).0,
            Applied::NotWritten(Input::Xess, Skip::PrefixBusy)
        );
        assert_eq!(std::fs::read_to_string(&file).unwrap(), USER_REG);
        let change = SettingChange {
            file: file.clone(),
            key: KEY.to_owned(),
            value: "XESS".to_owned(),
            set: 0,
            original: Some(3),
        };
        assert!(restore_with(&[change], &busy, Duration::ZERO).is_err());
        assert_eq!(std::fs::read_to_string(&file).unwrap(), USER_REG);
    }

    #[test]
    fn a_prefix_nothing_runs_in_is_free() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!prefix_in_use(dir.path()));
    }
}
