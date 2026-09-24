//! The record of what BiGame-mode changed in one game's folder.
//!
//! Everything [`super::transaction`] places in a game is listed here with the
//! hash of what was placed, and every file it replaced with the hash of the
//! original and where the original was kept. Removal works from this record
//! alone: a file is never removed because its *name* looks like a graphics
//! mod, only because the manifest says BiGame-mode put it there and its hash
//! says it is still what was put there.

use std::io::{Read, Write};
use std::path::{Component as PathComponent, Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

/// Current manifest format.
pub const SCHEMA: u32 = 1;

/// Where a transaction stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    /// Written before anything in the game folder changes. Finding a manifest
    /// in this state means an apply was interrupted and must be rolled back.
    Applying,
    /// Everything listed is in place and was verified.
    Installed,
}

/// How a file is treated on removal when it no longer matches what was
/// placed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileKind {
    /// A DLL or other binary. Changed since placed means something else owns
    /// it now: it is left where it is.
    Binary,
    /// A configuration file. Tools rewrite their own settings (`OptiScaler`
    /// saves its overlay choices to its `.ini`), so a changed config is still
    /// BiGame-mode's to remove — but the edited copy is kept first.
    Config,
}

/// An original file that was moved aside.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Backup {
    /// Where the copy is, absolute.
    pub path: PathBuf,
    /// SHA-256 of the original.
    pub sha256: String,
    /// Size of the original in bytes.
    pub size: u64,
}

/// One file BiGame-mode placed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    /// Path relative to the game's install folder.
    pub path: PathBuf,
    /// SHA-256 of what was placed.
    pub sha256: String,
    /// Binary or configuration.
    pub kind: FileKind,
    /// The file that was there before, when there was one.
    pub replaced: Option<Backup>,
}

/// Where a payload came from.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Source {
    /// Component id (`optiscaler`).
    pub component: String,
    /// Version or release tag.
    pub version: String,
    /// Download URL, when it was downloaded.
    pub url: Option<String>,
    /// SHA-256 of the downloaded archive.
    pub archive_sha256: Option<String>,
}

/// The record for one game.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    /// Format version.
    pub schema: u32,
    /// Stable key for the game (`steam-750920`, …).
    pub game_key: String,
    /// The install folder every entry is relative to.
    pub install_root: PathBuf,
    /// What was installed.
    pub source: Source,
    /// Unix time the transaction started.
    pub started_at: u64,
    /// Where the transaction stands.
    pub state: State,
    /// Files placed.
    pub entries: Vec<Entry>,
    /// Folders the transaction created, deepest last; removed again when
    /// empty.
    #[serde(default)]
    pub created_dirs: Vec<PathBuf>,
    /// Files the installed component writes itself at run time (its log),
    /// that did not exist before the install. Removal keeps a copy for
    /// diagnostics and deletes them; a file of that name that was already
    /// there is never listed, so never touched.
    #[serde(default)]
    pub generated: Vec<PathBuf>,
}

/// SHA-256 of a file, as lowercase hex.
///
/// # Errors
/// Returns the I/O error if the file cannot be read.
pub fn sha256_file(path: &Path) -> std::io::Result<String> {
    use sha2::{Digest, Sha256};
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex(&hasher.finalize()))
}

/// SHA-256 of bytes, as lowercase hex.
#[must_use]
pub fn sha256_bytes(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    hex(&Sha256::digest(bytes))
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::with_capacity(64), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

/// Check that `rel` is a plain relative path: no root, no `..`, no `.`
/// prefix tricks, not empty.
///
/// # Errors
/// Returns an error naming the path when it is not.
pub fn check_relative(rel: &Path) -> Result<()> {
    if rel.as_os_str().is_empty() {
        bail!("empty path");
    }
    for c in rel.components() {
        match c {
            PathComponent::Normal(_) => {}
            _ => bail!("path must be plain and relative: {}", rel.display()),
        }
    }
    Ok(())
}

/// Resolve `rel` under `root`, refusing any symlink on the way.
///
/// Every existing ancestor between `root` and the target is checked with
/// `symlink_metadata`, so a link planted inside the game folder cannot send a
/// write outside it. The target itself may not be a symlink either.
///
/// # Errors
/// Returns an error when the path is not plain and relative, or crosses or
/// names a symlink.
pub fn resolve_inside(root: &Path, rel: &Path) -> Result<PathBuf> {
    check_relative(rel)?;
    let mut at = root.to_path_buf();
    for c in rel.components() {
        at.push(c);
        match std::fs::symlink_metadata(&at) {
            Ok(m) if m.file_type().is_symlink() => {
                bail!("refusing to follow a symlink: {}", at.display())
            }
            Ok(_) | Err(_) => {}
        }
    }
    Ok(at)
}

impl Manifest {
    /// The manifest file for `game_key` under `state_dir`.
    #[must_use]
    pub fn path(state_dir: &Path, game_key: &str) -> PathBuf {
        state_dir.join(game_key).join("manifest.json")
    }

    /// The folder backups of `game_key` are kept in.
    #[must_use]
    pub fn backup_dir(state_dir: &Path, game_key: &str) -> PathBuf {
        state_dir.join(game_key).join("backup")
    }

    /// Load the manifest for `game_key`, if there is one.
    ///
    /// # Errors
    /// Returns an error if a manifest exists but cannot be read or parsed, or
    /// was written by a newer format.
    pub fn load(state_dir: &Path, game_key: &str) -> Result<Option<Self>> {
        let path = Self::path(state_dir, game_key);
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e).with_context(|| format!("read {}", path.display())),
        };
        let m: Self =
            serde_json::from_str(&text).with_context(|| format!("parse {}", path.display()))?;
        if m.schema > SCHEMA {
            bail!(
                "{} was written by a newer BiGame-mode (format {})",
                path.display(),
                m.schema
            );
        }
        for e in &m.entries {
            check_relative(&e.path)?;
        }
        Ok(Some(m))
    }

    /// Write the manifest atomically: to a temporary file beside it, synced,
    /// then renamed over the old one.
    ///
    /// # Errors
    /// Returns an error if the state folder cannot be created or written.
    pub fn save(&self, state_dir: &Path) -> Result<()> {
        let path = Self::path(state_dir, &self.game_key);
        let dir = path.parent().context("manifest path has no parent")?;
        std::fs::create_dir_all(dir).with_context(|| format!("create {}", dir.display()))?;
        let tmp = dir.join("manifest.json.tmp");
        {
            let mut f =
                std::fs::File::create(&tmp).with_context(|| format!("create {}", tmp.display()))?;
            f.write_all(serde_json::to_string_pretty(self)?.as_bytes())?;
            f.sync_all()?;
        }
        std::fs::rename(&tmp, &path).with_context(|| format!("replace {}", path.display()))?;
        Ok(())
    }

    /// Delete the manifest (after a complete removal).
    ///
    /// # Errors
    /// Returns an error if it exists and cannot be removed.
    pub fn delete(state_dir: &Path, game_key: &str) -> Result<()> {
        match std::fs::remove_file(Self::path(state_dir, game_key)) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.into()),
        }
    }
}

/// A stable, filesystem-safe key for a game: `steam-<appid>` when it has one,
/// otherwise the executable name with a short hash of the install folder, so
/// two copies of one game do not share a manifest.
#[must_use]
pub fn game_key(app_id: Option<&str>, executable: &str, install_root: &Path) -> String {
    if let Some(id) = app_id.filter(|id| !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit())) {
        return format!("steam-{id}");
    }
    let stem: String = executable
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .take(48)
        .collect();
    let digest = sha256_bytes(install_root.as_os_str().as_encoded_bytes());
    format!("{stem}-{}", &digest[..12])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_plain_relative_paths_are_accepted() {
        for ok in ["dxgi.dll", "bin/x64/OptiScaler.ini"] {
            assert!(check_relative(Path::new(ok)).is_ok(), "{ok}");
        }
        for bad in [
            "",
            "/etc/passwd",
            "../dxgi.dll",
            "bin/../../x",
            "./dxgi.dll",
        ] {
            assert!(check_relative(Path::new(bad)).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_symlink_inside_the_game_folder_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("game");
        std::fs::create_dir_all(&root).unwrap();
        std::os::unix::fs::symlink("/tmp", root.join("bin")).unwrap();
        std::os::unix::fs::symlink("/etc/passwd", root.join("dxgi.dll")).unwrap();
        assert!(resolve_inside(&root, Path::new("bin/dxgi.dll")).is_err());
        assert!(resolve_inside(&root, Path::new("dxgi.dll")).is_err());
        assert_eq!(
            resolve_inside(&root, Path::new("new/winmm.dll")).unwrap(),
            root.join("new/winmm.dll")
        );
    }

    #[test]
    fn a_manifest_survives_a_round_trip_and_newer_formats_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let m = Manifest {
            schema: SCHEMA,
            game_key: "steam-750920".into(),
            install_root: "/games/sottr".into(),
            source: Source {
                component: "optiscaler".into(),
                version: "0.7.9".into(),
                url: Some("https://example.invalid/x.7z".into()),
                archive_sha256: Some("ab".repeat(32)),
            },
            started_at: 1,
            state: State::Installed,
            entries: vec![Entry {
                path: "dxgi.dll".into(),
                sha256: "cd".repeat(32),
                kind: FileKind::Binary,
                replaced: None,
            }],
            created_dirs: vec![],
            generated: vec![],
        };
        m.save(dir.path()).unwrap();
        assert_eq!(
            Manifest::load(dir.path(), "steam-750920").unwrap(),
            Some(m.clone())
        );
        let mut newer = m;
        newer.schema = SCHEMA + 1;
        newer.save(dir.path()).unwrap();
        assert!(Manifest::load(dir.path(), "steam-750920").is_err());
        assert_eq!(Manifest::load(dir.path(), "steam-1").unwrap(), None);
    }

    #[test]
    fn a_manifest_naming_a_path_outside_the_game_is_rejected_on_load() {
        let dir = tempfile::tempdir().unwrap();
        let p = Manifest::path(dir.path(), "k");
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(
            &p,
            r#"{"schema":1,"game_key":"k","install_root":"/g","source":{"component":"x","version":"1","url":null,"archive_sha256":null},"started_at":0,"state":"installed","entries":[{"path":"../../.bashrc","sha256":"00","kind":"binary","replaced":null}]}"#,
        )
        .unwrap();
        assert!(Manifest::load(dir.path(), "k").is_err());
    }

    #[test]
    fn game_keys_are_stable_and_safe() {
        assert_eq!(
            game_key(Some("750920"), "SOTTR.exe", Path::new("/g")),
            "steam-750920"
        );
        let k = game_key(None, "My Game!.exe", Path::new("/games/a"));
        assert!(
            k.starts_with("My_Game__exe-") && k.len() == "My_Game__exe-".len() + 12,
            "{k}"
        );
        assert_ne!(k, game_key(None, "My Game!.exe", Path::new("/games/b")));
        assert!(game_key(Some("../x"), "a.exe", Path::new("/g")).starts_with("a_exe-"));
    }

    #[test]
    fn hashes_are_sha256() {
        assert_eq!(
            sha256_bytes(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("f");
        std::fs::write(&f, b"abc").unwrap();
        assert_eq!(sha256_file(&f).unwrap(), sha256_bytes(b"abc"));
    }
}
