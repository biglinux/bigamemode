//! Placing files in a game's folder as one transaction, and taking them out
//! again.
//!
//! Apply runs in this order, so that at every point a failure — or a crash —
//! leaves something that can be undone:
//!
//! 1. **check** every target: plain relative path, inside the game folder,
//!    no symlink anywhere on the way;
//! 2. **back up** every file that will be replaced, and verify each copy by
//!    hash, before anything in the game folder changes;
//! 3. **journal**: write the manifest in state [`State::Applying`], listing
//!    what is about to be placed and where each original went;
//! 4. **place** each file: copied beside its target under a temporary name,
//!    synced, then renamed over the target (a rename is atomic, so a target
//!    is always either the old file or the whole new one);
//! 5. **validate**: hash every placed file;
//! 6. **commit**: rewrite the manifest as [`State::Installed`].
//!
//! Any error in 4–5 rolls back at once. A manifest still in `Applying` when
//! Big Game Mode starts means an apply was cut short; [`recover`] rolls it back.
//!
//! Removal trusts the manifest and the hashes, never file names: a file is
//! taken out only if it is still exactly what was placed. A *binary* that has
//! changed since belongs to whatever changed it and is left alone; a *config*
//! that has changed is Big Game Mode's own file with the user's edits in it, so
//! the edited copy is kept before the file is removed.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::error::UserError;
use crate::text::N_;

use super::manifest::{
    self, Backup, Entry, FileKind, Manifest, SCHEMA, Source, State, resolve_inside, sha256_file,
};

/// The game a transaction is for.
#[derive(Debug, Clone, Copy)]
pub struct Game<'a> {
    /// Stable key ([`manifest::game_key`]).
    pub key: &'a str,
    /// Install folder every path is relative to.
    pub root: &'a Path,
    /// The process name it runs as, when known.
    pub process: Option<&'a str>,
    /// The game's title, when known.
    pub title: Option<&'a str>,
}

/// A file to place.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedFile {
    /// Target, relative to the game's install folder.
    pub path: PathBuf,
    /// The file to copy there (in Big Game Mode's cache or a staging folder).
    pub source: PathBuf,
    /// Binary or configuration.
    pub kind: FileKind,
}

/// What happened to one file on removal or rollback.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileOutcome {
    /// Removed; nothing was there before.
    Removed(PathBuf),
    /// The original was put back.
    Restored(PathBuf),
    /// Already gone; the original (if any) was put back.
    WasMissing(PathBuf),
    /// A binary that changed since it was placed: left where it is.
    KeptChanged(PathBuf),
    /// A config the user (or its tool) edited: the edited copy was kept at
    /// the second path before the file was removed or restored.
    EditedCopyKept(PathBuf, PathBuf),
}

/// What [`verify`] found for one file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileState {
    /// Present and exactly as placed.
    Intact,
    /// Not there.
    Missing,
    /// A binary that is there, but different: something else replaced it
    /// (a game update, another tool).
    Changed,
    /// A configuration file that is there, but different: its owner rewrote
    /// it. `OptiScaler` saves its ini whenever it starts (in its own
    /// `key = value` layout) and whenever a setting changes in its overlay,
    /// so this is what an installed game looks like after it has run once —
    /// not a fault, and nothing for Repair to do.
    Edited,
}

/// Hold the lock of `game_key`'s changes until the file returned is dropped,
/// waiting while another change holds it.
///
/// Every change to a game's files runs under it — apply, update, restore,
/// repair, the recovery at start: two at once (a double click, the page and
/// the Profiles menu, Restore while the recovery still runs) would each back
/// up what the other had just placed as the game's original, and the real
/// original would be gone with the first removal. A lock taken by `flock` is
/// one per opening of the file, so it also keeps two threads of one process
/// apart, and it goes with the process that holds it.
///
/// The lock files are kept apart from the games' folders, which a removal
/// deletes once empty, and are never deleted: a waiter would go on to lock
/// the deleted file while the next change locks a new one.
///
/// # Errors
/// Returns an error if the lock file cannot be created or locked.
pub fn lock(state_dir: &Path, game_key: &str) -> Result<std::fs::File> {
    use std::os::fd::AsRawFd;
    manifest::check_relative(Path::new(game_key))?;
    let dir = state_dir.join(".locks");
    std::fs::create_dir_all(&dir).with_context(|| format!("create {}", dir.display()))?;
    let path = dir.join(game_key);
    let f = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&path)
        .with_context(|| format!("lock {}", path.display()))?;
    loop {
        // SAFETY: flock on a descriptor this function owns, kept open by the
        // file returned.
        if unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX) } == 0 {
            return Ok(f);
        }
        let e = std::io::Error::last_os_error();
        if e.kind() != std::io::ErrorKind::Interrupted {
            return Err(e).with_context(|| format!("lock {}", path.display()));
        }
    }
}

/// Create a backup folder for an apply that starts at `now`, named after
/// the second it starts in — which the manifest records to find it again —
/// or the first later second whose folder is not there yet. A folder left
/// from an earlier install in the same second holds originals a removal
/// kept (files another program changed since); sharing it, this install's
/// removal would delete them with its own backups.
fn new_backup_root(state_dir: &Path, game_key: &str, now: u64) -> Result<(u64, PathBuf)> {
    let parent = Manifest::backup_dir(state_dir, game_key);
    std::fs::create_dir_all(&parent).with_context(|| format!("create {}", parent.display()))?;
    for at in now..now.saturating_add(1000) {
        let root = parent.join(at.to_string());
        match std::fs::create_dir(&root) {
            Ok(()) => return Ok((at, root)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e).with_context(|| format!("create {}", root.display())),
        }
    }
    bail!("no free backup folder in {}", parent.display())
}

/// Copy `src` over `target` atomically: a temporary file in the target's own
/// folder (same filesystem, so the rename is atomic), synced, then renamed.
fn place(src: &Path, target: &Path) -> Result<()> {
    let name = target
        .file_name()
        .context("target has no file name")?
        .to_string_lossy();
    let tmp = target.with_file_name(format!(".{name}.bigame-new"));
    // The game folder is not ours: a file (or a symlink to anything) may
    // already sit under the temporary name. Removing it does not follow a
    // link, and the new file is created exclusively, never through one.
    match std::fs::remove_file(&tmp) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e).with_context(|| format!("remove {}", tmp.display())),
    }
    {
        use std::os::unix::fs::OpenOptionsExt;
        let mut out = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&tmp)
            .with_context(|| format!("create {}", tmp.display()))?;
        std::io::copy(&mut std::fs::File::open(src)?, &mut out)
            .with_context(|| format!("copy {} → {}", src.display(), tmp.display()))?;
        out.sync_all()?;
    }
    // Checked again right before the rename: the target must not have become
    // a symlink since the transaction began.
    if std::fs::symlink_metadata(target).is_ok_and(|m| m.file_type().is_symlink()) {
        let _ = std::fs::remove_file(&tmp);
        bail!(UserError::with(
            N_("%s became a symlink; refusing to replace it"),
            [target.display().to_string()]
        ));
    }
    std::fs::rename(&tmp, target)
        .with_context(|| format!("rename {} → {}", tmp.display(), target.display()))?;
    Ok(())
}

fn hash_if_present(path: &Path) -> Result<Option<String>> {
    match std::fs::symlink_metadata(path) {
        Ok(m) if m.file_type().is_symlink() => bail!("{} is a symlink", path.display()),
        Ok(m) if !m.is_file() => bail!("{} is not a regular file", path.display()),
        Ok(_) => Ok(Some(sha256_file(path)?)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// What is at one of a manifest's paths when it is taken out.
enum Found {
    /// Nothing.
    Missing,
    /// A plain file, with its hash.
    File(String),
    /// A symlink, at the path or on the way to it, or something that is not
    /// a plain file: a mod manager deploys its files as links, or the user
    /// moved a folder elsewhere. Not Big Game Mode's to follow or remove.
    Foreign,
}

/// What is at `rel` under `root`, and the path it was looked for at.
///
/// # Errors
/// Returns an error for a path that is not plain and relative (it could
/// name something outside the game), or a file that cannot be read.
fn found_at(root: &Path, rel: &Path) -> Result<(PathBuf, Found)> {
    manifest::check_relative(rel)?;
    let Ok(target) = resolve_inside(root, rel) else {
        return Ok((root.join(rel), Found::Foreign));
    };
    let found = match std::fs::symlink_metadata(&target) {
        Ok(m) if m.file_type().is_symlink() || !m.is_file() => Found::Foreign,
        Ok(_) => Found::File(sha256_file(&target)?),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Found::Missing,
        // A folder on the way is a file now: the game's, whatever it is.
        Err(e) if e.raw_os_error() == Some(libc::ENOTDIR) => Found::Foreign,
        Err(e) => return Err(e).with_context(|| format!("read {}", target.display())),
    };
    Ok((target, found))
}

/// Sync a folder, so the renames and deletions in it are on disk.
fn sync_dir(dir: &Path) -> Result<()> {
    std::fs::File::open(dir)
        .and_then(|d| d.sync_all())
        .with_context(|| format!("sync {}", dir.display()))
}

/// The name a file of `target`'s name already has on disk, when it is
/// there in another letter case. Windows file names ignore case, so to the
/// game `AMD_FidelityFX_DX12.dll` is the slot `amd_fidelityfx_dx12.dll`
/// names: it is that file that is backed up and replaced, never a second one
/// placed beside it (which of the two Wine would load is not defined).
///
/// # Errors
/// Returns an error when the folder cannot be listed, or holds the name in
/// more than one spelling.
fn spelling_on_disk(target: &Path) -> Result<Option<std::ffi::OsString>> {
    let (Some(dir), Some(name)) = (target.parent(), target.file_name()) else {
        return Ok(None);
    };
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e).with_context(|| format!("list {}", dir.display())),
    };
    let name = name.to_string_lossy();
    let mut found: Vec<std::ffi::OsString> = entries
        .flatten()
        .map(|e| e.file_name())
        .filter(|n| n.to_string_lossy().eq_ignore_ascii_case(&name))
        .collect();
    if found.len() > 1 {
        bail!(UserError::with(
            N_(
                "%s is in the game's folder more than once, in different letter case; remove one of them first"
            ),
            [target.display().to_string()]
        ));
    }
    Ok(found.pop())
}

/// Back up every file that `files` will replace, verified by hash.
///
/// Every target is examined first, so a path that cannot be used fails here
/// with nothing written anywhere — not even a backup. A failure while copying
/// removes the partial backup.
fn back_up_originals(
    files: &[PlannedFile],
    targets: &[(PathBuf, String)],
    backup_root: &Path,
) -> Result<Vec<Entry>> {
    let originals = targets
        .iter()
        .map(|(t, _)| hash_if_present(t))
        .collect::<Result<Vec<_>>>()?;
    let copied = (|| -> Result<Vec<Entry>> {
        let mut entries = Vec::with_capacity(files.len());
        for ((f, (target, new_sha)), original) in files.iter().zip(targets).zip(originals) {
            let replaced = match original {
                None => None,
                Some(orig_sha) => {
                    let copy = backup_root.join(&f.path);
                    std::fs::create_dir_all(copy.parent().context("backup path has no parent")?)?;
                    std::fs::copy(target, &copy)
                        .with_context(|| format!("back up {}", target.display()))?;
                    // On disk before the journal and the replacement: after a
                    // power cut the rename can persist while an unsynced
                    // backup does not, and the original would be gone. The
                    // folders it is in are new too (`backup/<time>/…`), and
                    // each one's entry is in its parent.
                    std::fs::File::open(&copy)?.sync_all()?;
                    let game_dir = backup_root
                        .parent()
                        .and_then(Path::parent)
                        .context("backup folder has no parent")?;
                    for dir in copy.ancestors().skip(1) {
                        sync_dir(dir)?;
                        if dir == game_dir {
                            break;
                        }
                    }
                    if sha256_file(&copy)? != orig_sha {
                        bail!(UserError::with(
                            N_("the backup of %s does not match the original"),
                            [target.display().to_string()]
                        ));
                    }
                    Some(Backup {
                        path: copy,
                        sha256: orig_sha,
                        size: std::fs::metadata(target)?.len(),
                    })
                }
            };
            entries.push(Entry {
                path: f.path.clone(),
                sha256: new_sha.clone(),
                kind: f.kind,
                replaced,
            });
        }
        Ok(entries)
    })();
    if copied.is_err() {
        let _ = std::fs::remove_dir_all(backup_root);
    }
    copied
}

/// Folders under `install_root` that placing `files` will create, shallowest
/// first.
fn dirs_to_create(install_root: &Path, files: &[PlannedFile]) -> Vec<PathBuf> {
    let mut created = Vec::new();
    for f in files {
        let mut rel = PathBuf::new();
        for c in f.path.parent().into_iter().flat_map(Path::components) {
            rel.push(c);
            if !install_root.join(&rel).exists() && !created.contains(&rel) {
                created.push(rel.clone());
            }
        }
    }
    created
}

/// Place `files` in `install_root` as one transaction.
///
/// Refuses when the game already has files installed: an update is a
/// removal followed by an apply, so the original of every file is always the
/// file that was there before Big Game Mode, not a previous Big Game Mode
/// payload. A manifest that only keeps the game's own settings (a removal
/// that could not put them back yet) is taken over: the new journal carries
/// them, so they are never without a record.
///
/// # Errors
/// Returns an error — with the game folder as it was — when a check fails, a
/// backup cannot be made and verified, or placing or validating fails.
pub fn apply(
    state_dir: &Path,
    game: &Game<'_>,
    source: Source,
    files: &[PlannedFile],
    generated: &[PathBuf],
) -> Result<Manifest> {
    let (game_key, install_root) = (game.key, game.root);
    let settings = match Manifest::load(state_dir, game_key)? {
        Some(left) if left.state == State::Installed && left.entries.is_empty() => left.settings,
        Some(existing) => bail!(UserError::with(
            N_("%s already has %s %s installed (%s); remove it first"),
            [
                game_key.to_owned(),
                existing.source.component,
                existing.source.version,
                format!("{:?}", existing.state),
            ]
        )),
        None => Vec::new(),
    };
    if files.is_empty() {
        bail!(UserError::plain(N_("nothing to install")));
    }
    // 1. Check, and hash what will be placed. A file already there under
    // another letter case is the one replaced.
    let mut files = files.to_vec();
    let mut targets = Vec::with_capacity(files.len());
    for f in &mut files {
        let mut target = resolve_inside(install_root, &f.path)?;
        if let Some(name) = spelling_on_disk(&target)?
            && Some(name.as_os_str()) != target.file_name()
        {
            f.path.set_file_name(&name);
            target = resolve_inside(install_root, &f.path)?;
        }
        if !f.source.is_file() {
            bail!("missing payload file {}", f.source.display());
        }
        targets.push((target, sha256_file(&f.source)?));
    }
    let files = files.as_slice();
    let mut seen = std::collections::HashSet::new();
    for f in files {
        if !seen.insert(f.path.to_string_lossy().to_ascii_lowercase()) {
            // Windows file names are case-insensitive: dxgi.dll and DXGI.dll
            // are one slot to the game.
            bail!("{} is listed twice", f.path.display());
        }
    }

    // 2. Back up originals, verified, before anything changes.
    let (started_at, backup_root) = new_backup_root(state_dir, game_key, crate::unix_now())?;
    let entries = back_up_originals(files, &targets, &backup_root)?;
    let created_dirs = dirs_to_create(install_root, files);

    // Run-time files of the component that are not there yet; one that is
    // already there belongs to someone else and is not listed.
    let mut fresh = Vec::new();
    for g in generated {
        let t = resolve_inside(install_root, g)?;
        if std::fs::symlink_metadata(&t).is_err() {
            fresh.push(g.clone());
        }
    }

    // 3. Journal.
    let mut m = Manifest {
        schema: SCHEMA,
        game_key: game_key.to_owned(),
        process: game.process.map(str::to_owned),
        title: game.title.map(str::to_owned),
        install_root: install_root.to_path_buf(),
        source,
        started_at,
        state: State::Applying,
        entries,
        created_dirs,
        generated: fresh,
        previous: None,
        managed: true,
        settings,
    };
    m.save(state_dir)?;
    tracing::info!(target: "graphics", game = game_key, files = files.len(), "backup created; applying");

    // 4–5. Place and validate; any failure rolls back.
    let placed = (|| -> Result<()> {
        for d in &m.created_dirs {
            let dir = resolve_inside(install_root, d)?;
            std::fs::create_dir_all(&dir)?;
        }
        for (f, (target, _)) in files.iter().zip(&targets) {
            place(&f.source, target)?;
        }
        for (e, (target, _)) in m.entries.iter().zip(&targets) {
            if sha256_file(target)? != e.sha256 {
                bail!(UserError::with(
                    N_("%s does not match what was placed"),
                    [target.display().to_string()]
                ));
            }
        }
        Ok(())
    })();
    if let Err(e) = placed {
        tracing::warn!(target: "graphics", game = game_key, error = %e, "apply failed; rolling back");
        rollback(state_dir, &m).context("rollback after a failed apply")?;
        return Err(UserError::plain(N_("apply failed and was rolled back"))
            .caused_by(e)
            .into());
    }

    // 6. Commit.
    m.state = State::Installed;
    m.save(state_dir)?;
    tracing::info!(target: "graphics", game = game_key, component = %m.source.component,
        version = %m.source.version, "graphics files installed and verified");
    Ok(m)
}

/// Undo `m` file by file, whatever state it is in, then delete it.
///
/// Used for failed and interrupted applies, and by [`remove`]. For each
/// entry: a file that is exactly what was placed is taken out and the
/// original put back; a missing file gets its original back; a changed
/// binary — or a link, or anything that is not a plain file — is left
/// alone; a changed config is kept as a copy first. What is left alone
/// keeps its original's backup, listed in [`kept`].
///
/// The game's own settings the manifest records are not undone here
/// ([`super::ingame::restore`] is the caller's): while there are any, the
/// manifest is not deleted but kept with them alone, so they always have a
/// record.
///
/// # Errors
/// Returns an error if the game's folder is not there (it was moved, or its
/// drive is not mounted: every file would read as missing, and nothing
/// would be undone), or a file cannot be restored or removed; the manifest
/// is then kept so the attempt can be repeated.
// One entry after another, then what they leave; the order is the safety.
#[allow(clippy::too_many_lines)]
pub fn rollback(state_dir: &Path, m: &Manifest) -> Result<Vec<FileOutcome>> {
    if !m.install_root.is_dir() {
        bail!(UserError::with(
            N_("the game's folder %s is not there; nothing was changed"),
            [m.install_root.display().to_string()]
        ));
    }
    let mut outcomes = Vec::new();
    let mut backups_still_needed = false;
    let mut kept_now = Vec::new();
    let mut touched = std::collections::BTreeSet::new();
    // One folder for every edited config this rollback keeps, named apart
    // from any other rollback's: two in the same second would otherwise
    // write over each other's copies.
    let edited_root = Manifest::backup_dir(state_dir, &m.game_key).join(manifest::unique_name(
        &format!("edited-{}-", crate::unix_now()),
    ));
    for e in &m.entries {
        let (target, found) = found_at(&m.install_root, &e.path)?;
        let original = e.replaced.as_ref();
        let current = match found {
            Found::Foreign => {
                backups_still_needed |= original.is_some();
                if let Some(b) = original {
                    kept_now.push((e.path.clone(), b.path.clone()));
                }
                tracing::warn!(target: "graphics", file = %target.display(),
                    "a link or not a plain file now; left in place");
                outcomes.push(FileOutcome::KeptChanged(e.path.clone()));
                continue;
            }
            Found::Missing => None,
            Found::File(h) => Some(h),
        };
        // Leftover of an interrupted `place`.
        if let Some(n) = target.file_name().map(|n| n.to_string_lossy().into_owned()) {
            let _ = std::fs::remove_file(target.with_file_name(format!(".{n}.bigame-new")));
        }
        let ours = current.as_deref() == Some(e.sha256.as_str());
        let still_original =
            original.is_some_and(|b| current.as_deref() == Some(b.sha256.as_str()));
        let outcome = if still_original {
            // Never replaced (an apply interrupted before this file).
            FileOutcome::Restored(e.path.clone())
        } else if ours || current.is_none() {
            restore_or_remove(&target, original)?;
            if current.is_none() {
                FileOutcome::WasMissing(e.path.clone())
            } else if original.is_some() {
                FileOutcome::Restored(e.path.clone())
            } else {
                FileOutcome::Removed(e.path.clone())
            }
        } else {
            match e.kind {
                FileKind::Binary => {
                    backups_still_needed |= original.is_some();
                    if let Some(b) = original {
                        kept_now.push((e.path.clone(), b.path.clone()));
                    }
                    tracing::warn!(target: "graphics", file = %target.display(),
                        "changed since it was installed; left in place");
                    FileOutcome::KeptChanged(e.path.clone())
                }
                FileKind::Config => {
                    let keep = edited_root.join(&e.path);
                    std::fs::create_dir_all(keep.parent().context("no parent")?)?;
                    std::fs::copy(&target, &keep)?;
                    restore_or_remove(&target, original)?;
                    kept_now.push((e.path.clone(), keep.clone()));
                    FileOutcome::EditedCopyKept(e.path.clone(), keep)
                }
            }
        };
        if let Some(dir) = target.parent() {
            touched.insert(dir.to_path_buf());
        }
        outcomes.push(outcome);
    }
    for g in &m.generated {
        let Ok(target) = resolve_inside(&m.install_root, g) else {
            continue;
        };
        if std::fs::symlink_metadata(&target).is_ok_and(|md| md.is_file()) {
            // Kept for diagnostics: the last log of a removed install is what
            // a support report needs.
            let keep = state_dir.join(&m.game_key).join("last-run").join(g);
            if let Some(parent) = keep.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let _ = std::fs::copy(&target, &keep);
            std::fs::remove_file(&target)
                .with_context(|| format!("remove {}", target.display()))?;
            if let Some(dir) = target.parent() {
                touched.insert(dir.to_path_buf());
            }
        }
    }
    for d in m.created_dirs.iter().rev() {
        if let Ok(dir) = resolve_inside(&m.install_root, d) {
            // Only if empty: anything the game or the user put there stays.
            let _ = std::fs::remove_dir(dir);
        }
    }
    // The game's folder can be on another filesystem than the state: the
    // originals put back are on disk before the record of them and their
    // backups go.
    for dir in touched.iter().filter(|d| d.is_dir()) {
        sync_dir(dir)?;
    }
    if !kept_now.is_empty()
        && let Err(e) = remember_kept(state_dir, m, &kept_now)
    {
        tracing::warn!(target: "graphics", game = %m.game_key, error = %format!("{e:#}"),
                "the list of kept copies could not be written");
    }
    if m.settings.is_empty() {
        Manifest::delete(state_dir, &m.game_key)?;
    } else {
        // Files gone, the game's own settings still to put back: the record
        // stays, with nothing else in it.
        Manifest {
            state: State::Installed,
            entries: Vec::new(),
            created_dirs: Vec::new(),
            generated: Vec::new(),
            ..m.clone()
        }
        .save(state_dir)?;
    }
    // The configured payload copies are only needed while installed.
    let _ = std::fs::remove_dir_all(state_dir.join(&m.game_key).join("staging"));
    if !backups_still_needed {
        let _ = std::fs::remove_dir_all(
            Manifest::backup_dir(state_dir, &m.game_key).join(m.started_at.to_string()),
        );
    }
    // The game's folder in the state, once nothing is left in it: kept
    // copies of edited configs and backups still needed keep it.
    let _ = std::fs::remove_dir(Manifest::backup_dir(state_dir, &m.game_key));
    let _ = std::fs::remove_dir(state_dir.join(&m.game_key));
    tracing::info!(target: "graphics", game = %m.game_key, files = outcomes.len(), "graphics rollback completed");
    Ok(outcomes)
}

/// A copy Big Game Mode keeps after a removal: the original of a file someone
/// else changed since (nothing in the game refers to it any more), or a
/// configuration with the user's edits.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Kept {
    /// The game's key.
    pub game_key: String,
    /// The game's title, when known.
    #[serde(default)]
    pub title: Option<String>,
    /// The file in the game, relative to its folder.
    pub file: PathBuf,
    /// The copy, absolute.
    pub copy: PathBuf,
    /// Unix time of the removal.
    pub at: u64,
}

fn kept_path(state_dir: &Path) -> PathBuf {
    state_dir.join("kept.json")
}

/// Every copy kept after a removal, oldest first, whose copy is still there.
#[must_use]
pub fn kept(state_dir: &Path) -> Vec<Kept> {
    std::fs::read_to_string(kept_path(state_dir))
        .ok()
        .and_then(|t| serde_json::from_str::<Vec<Kept>>(&t).ok())
        .unwrap_or_default()
        .into_iter()
        .filter(|k| k.copy.exists())
        .collect()
}

fn remember_kept(state_dir: &Path, m: &Manifest, now: &[(PathBuf, PathBuf)]) -> Result<()> {
    // One list for every game, and a game's lock only keeps changes to that
    // game apart: two games restored at once would each write the list
    // without the other's copies.
    let _list = lock(state_dir, ".kept")?;
    let mut all = kept(state_dir);
    let at = crate::unix_now();
    for (file, copy) in now {
        all.push(Kept {
            game_key: m.game_key.clone(),
            title: m.title.clone(),
            file: file.clone(),
            copy: copy.clone(),
            at,
        });
    }
    let path = kept_path(state_dir);
    let tmp = state_dir.join(manifest::unique_name("kept.json.tmp-"));
    std::fs::write(&tmp, serde_json::to_string_pretty(&all)?)?;
    std::fs::rename(&tmp, &path)?;
    Ok(())
}

fn restore_or_remove(target: &Path, original: Option<&Backup>) -> Result<()> {
    match original {
        Some(b) => {
            if manifest::sha256_file(&b.path)? != b.sha256 {
                bail!(UserError::with(
                    N_("backup %s is damaged; not restoring it"),
                    [b.path.display().to_string()]
                ));
            }
            place(&b.path, target)
        }
        None => match std::fs::remove_file(target) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.into()),
        },
    }
}

/// Take out everything the game's manifest lists, restoring originals.
///
/// # Errors
/// Returns an error if there is no manifest, or a file cannot be restored.
pub fn remove(state_dir: &Path, game_key: &str) -> Result<Vec<FileOutcome>> {
    let m = Manifest::load(state_dir, game_key)?.ok_or_else(|| {
        UserError::with(N_("Big Game Mode has installed nothing in %s"), [game_key])
    })?;
    rollback(state_dir, &m)
}

/// Roll back every manifest left in [`State::Applying`] — applies cut short by
/// a crash or power loss.
///
/// # Errors
/// Returns an error if the state folder cannot be listed.
pub fn recover(state_dir: &Path) -> Result<Vec<(String, Result<Vec<FileOutcome>>)>> {
    let mut done = Vec::new();
    let Ok(dirs) = std::fs::read_dir(state_dir) else {
        return Ok(done);
    };
    for d in dirs.flatten() {
        let key = d.file_name().to_string_lossy().into_owned();
        let applying =
            |m: &Option<Manifest>| m.as_ref().is_some_and(|m| m.state == State::Applying);
        if !Manifest::load(state_dir, &key).is_ok_and(|m| applying(&m)) {
            continue;
        }
        // An apply of this process may be running (the page is open while
        // this runs at start): read again once it is done, under the lock.
        let Ok(_lock) = lock(state_dir, &key) else {
            continue;
        };
        if let Ok(Some(m)) = Manifest::load(state_dir, &key)
            && m.state == State::Applying
        {
            tracing::warn!(target: "graphics", game = %key, "interrupted apply found; rolling back");
            done.push((key, rollback(state_dir, &m)));
        }
    }
    Ok(done)
}

/// What identifies one version of a file for [`hash_for_status`]: device,
/// inode, size, modification time and change time. The change time cannot be
/// set from user space, so a file rewritten in place with its old
/// modification time restored (`cp -p`, `touch -r`) still counts as changed.
type FileKey = (u64, u64, u64, std::time::SystemTime, i64, i64);

/// Hashes [`verify`] computed, by path, with the file's key when it was
/// hashed.
type HashCache = std::collections::HashMap<PathBuf, (FileKey, String)>;

static VERIFY_HASHES: std::sync::Mutex<Option<HashCache>> = std::sync::Mutex::new(None);

/// [`hash_if_present`] for status reads, remembered while the file's key
/// stays the same.
///
/// The status of an installed game is read every few seconds (Home, Details)
/// and each read checked every placed file; the `OptiScaler` DLL and AMD's
/// FSR runtime alone are 55 MB, hashed again on every read. A file replaced
/// or rewritten gets a new key, so it is hashed again. The lock is not held
/// while hashing, so one slow file does not hold up every other reader. The
/// transaction's own checks never use this: they hash every time.
fn hash_for_status(path: &Path) -> Result<Option<String>> {
    use std::os::unix::fs::MetadataExt;
    let meta = match std::fs::symlink_metadata(path) {
        Ok(m) if m.file_type().is_symlink() => bail!("{} is a symlink", path.display()),
        Ok(m) if !m.is_file() => bail!("{} is not a regular file", path.display()),
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    let key: FileKey = (
        meta.dev(),
        meta.ino(),
        meta.len(),
        meta.modified()?,
        meta.ctime(),
        meta.ctime_nsec(),
    );
    let hit = with_hash_cache(|cache| {
        cache
            .get(path)
            .filter(|(k, _)| *k == key)
            .map(|(_, hash)| hash.clone())
    });
    if hit.is_some() {
        return Ok(hit);
    }
    let hash = sha256_file(path)?;
    with_hash_cache(|cache| cache.insert(path.to_path_buf(), (key, hash.clone())));
    Ok(Some(hash))
}

fn with_hash_cache<R>(f: impl FnOnce(&mut HashCache) -> R) -> R {
    let mut cache = VERIFY_HASHES
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    f(cache.get_or_insert_with(HashCache::new))
}

/// Check every file of `m` against what was placed.
#[must_use]
pub fn verify(m: &Manifest) -> Vec<(PathBuf, FileState)> {
    m.entries
        .iter()
        .map(|e| {
            let state = resolve_inside(&m.install_root, &e.path)
                .ok()
                .and_then(|t| hash_for_status(&t).ok())
                .map_or(FileState::Missing, |h| match h {
                    None => FileState::Missing,
                    Some(h) if h == e.sha256 => FileState::Intact,
                    Some(_) if e.kind == FileKind::Config => FileState::Edited,
                    Some(_) => FileState::Changed,
                });
            (e.path.clone(), state)
        })
        .collect()
}

/// Put back files of `m` that are missing, from `payload` (the same files the
/// install used, from Big Game Mode's cache). Changed files are not touched:
/// a changed binary has another owner now, and a changed config holds the
/// user's settings.
///
/// # Errors
/// Returns an error if a missing file cannot be placed or its payload does
/// not match the manifest.
pub fn repair_missing(m: &Manifest, payload: &[PlannedFile]) -> Result<Vec<PathBuf>> {
    if !m.install_root.is_dir() {
        bail!(UserError::with(
            N_("the game's folder %s is not there; nothing was changed"),
            [m.install_root.display().to_string()]
        ));
    }
    // Everything checked before anything is placed.
    let mut todo = Vec::new();
    for (path, state) in verify(m) {
        if state != FileState::Missing {
            continue;
        }
        let entry = m
            .entries
            .iter()
            .find(|e| e.path == path)
            .context("entry vanished")?;
        let src = payload
            .iter()
            .find(|p| p.path == path)
            .with_context(|| format!("no payload for {}", path.display()))?;
        if sha256_file(&src.source)? != entry.sha256 {
            bail!(UserError::with(
                N_("the cached copy of %s is not what was installed"),
                [path.display().to_string()]
            ));
        }
        // Only a folder the install itself created is made again; any other
        // is the game's, and its absence means the game is not what the
        // manifest describes.
        let rel_dir = path.parent().unwrap_or_else(|| Path::new(""));
        let ours = |d: &PathBuf| rel_dir.starts_with(d);
        let target = resolve_inside(&m.install_root, &path)?;
        let parent = target.parent().context("target has no folder")?;
        if !parent.is_dir() && !m.created_dirs.iter().any(ours) {
            bail!(UserError::with(
                N_("the game's folder %s is not there; nothing was changed"),
                [parent.display().to_string()]
            ));
        }
        todo.push((path, target, src.source.clone()));
    }
    let mut repaired = Vec::new();
    for (path, target, source) in todo {
        for d in m
            .created_dirs
            .iter()
            .filter(|d| path.parent().is_some_and(|p| p.starts_with(d)))
        {
            let dir = resolve_inside(&m.install_root, d)?;
            if !dir.is_dir() {
                std::fs::create_dir(&dir).with_context(|| format!("create {}", dir.display()))?;
            }
        }
        place(&source, &target)?;
        repaired.push(path);
    }
    Ok(repaired)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture {
        _dir: tempfile::TempDir,
        state: PathBuf,
        game: PathBuf,
        payload: PathBuf,
    }

    fn fixture() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("state");
        let game = dir.path().join("game");
        let payload = dir.path().join("payload");
        for d in [&state, &game, &payload] {
            std::fs::create_dir_all(d).unwrap();
        }
        Fixture {
            _dir: dir,
            state,
            game,
            payload,
        }
    }

    fn g(fx: &Fixture) -> Game<'_> {
        Game {
            key: "g",
            root: &fx.game,
            process: Some("Game.exe"),
            title: Some("Game"),
        }
    }

    fn src() -> Source {
        Source {
            component: "optiscaler".into(),
            backend: "optiscaler".into(),
            version: "1".into(),
            url: None,
            archive_sha256: None,
        }
    }

    fn planned(fx: &Fixture, rel: &str, bytes: &[u8], kind: FileKind) -> PlannedFile {
        let source = fx.payload.join(rel.replace('/', "_"));
        std::fs::write(&source, bytes).unwrap();
        PlannedFile {
            path: rel.into(),
            source,
            kind,
        }
    }

    fn read(p: &Path) -> Vec<u8> {
        std::fs::read(p).unwrap()
    }

    #[test]
    fn apply_backs_up_replaces_and_remove_puts_everything_back() {
        let fx = fixture();
        std::fs::write(fx.game.join("dxgi.dll"), b"original dxgi").unwrap();
        let files = [
            planned(&fx, "dxgi.dll", b"optiscaler dxgi", FileKind::Binary),
            planned(&fx, "OptiScaler.ini", b"[Upscalers]", FileKind::Config),
            planned(
                &fx,
                "D3D12_Optiscaler/D3D12Core.dll",
                b"core",
                FileKind::Binary,
            ),
        ];
        let m = apply(&fx.state, &g(&fx), src(), &files, &[]).unwrap();
        assert_eq!(m.state, State::Installed);
        assert_eq!(read(&fx.game.join("dxgi.dll")), b"optiscaler dxgi");
        assert!(fx.game.join("D3D12_Optiscaler/D3D12Core.dll").is_file());
        assert_eq!(
            verify(&m)
                .iter()
                .filter(|(_, s)| *s == FileState::Intact)
                .count(),
            3
        );
        let backup = m.entries[0].replaced.as_ref().unwrap();
        assert_eq!(read(&backup.path), b"original dxgi");

        let out = remove(&fx.state, "g").unwrap();
        assert_eq!(read(&fx.game.join("dxgi.dll")), b"original dxgi");
        assert!(!fx.game.join("OptiScaler.ini").exists());
        assert!(
            !fx.game.join("D3D12_Optiscaler").exists(),
            "created folder removed"
        );
        assert!(out.contains(&FileOutcome::Restored("dxgi.dll".into())));
        assert!(Manifest::load(&fx.state, "g").unwrap().is_none());
        assert!(
            !fx.state.join("g").exists(),
            "nothing of the game is left in the state"
        );
    }

    #[test]
    fn a_binary_changed_by_someone_else_is_left_alone_on_removal() {
        let fx = fixture();
        std::fs::write(fx.game.join("dxgi.dll"), b"original").unwrap();
        let files = [planned(&fx, "dxgi.dll", b"ours", FileKind::Binary)];
        apply(&fx.state, &g(&fx), src(), &files, &[]).unwrap();
        std::fs::write(fx.game.join("dxgi.dll"), b"reshade installed later").unwrap();
        let out = remove(&fx.state, "g").unwrap();
        assert_eq!(out, [FileOutcome::KeptChanged("dxgi.dll".into())]);
        assert_eq!(read(&fx.game.join("dxgi.dll")), b"reshade installed later");
    }

    #[test]
    fn an_edited_config_is_kept_as_a_copy_before_it_is_removed() {
        let fx = fixture();
        let files = [planned(&fx, "OptiScaler.ini", b"a=1", FileKind::Config)];
        apply(&fx.state, &g(&fx), src(), &files, &[]).unwrap();
        std::fs::write(fx.game.join("OptiScaler.ini"), b"a=2 (user)").unwrap();
        let out = remove(&fx.state, "g").unwrap();
        let FileOutcome::EditedCopyKept(_, copy) = &out[0] else {
            panic!("{out:?}")
        };
        assert_eq!(read(copy), b"a=2 (user)");
        assert!(!fx.game.join("OptiScaler.ini").exists());
        // The kept copy keeps its folder.
        assert!(copy.is_file());
    }

    #[test]
    fn a_file_deleted_by_a_game_update_gets_its_original_back() {
        let fx = fixture();
        std::fs::write(fx.game.join("winmm.dll"), b"original").unwrap();
        apply(
            &fx.state,
            &g(&fx),
            src(),
            &[planned(&fx, "winmm.dll", b"ours", FileKind::Binary)],
            &[],
        )
        .unwrap();
        std::fs::remove_file(fx.game.join("winmm.dll")).unwrap();
        assert_eq!(
            remove(&fx.state, "g").unwrap(),
            [FileOutcome::WasMissing("winmm.dll".into())]
        );
        assert_eq!(read(&fx.game.join("winmm.dll")), b"original");
    }

    #[test]
    fn a_failure_while_placing_rolls_back_what_was_already_placed() {
        use std::os::unix::fs::PermissionsExt;
        let fx = fixture();
        std::fs::write(fx.game.join("dxgi.dll"), b"original").unwrap();
        // The checks pass (the target does not exist yet), but nothing can be
        // created in a read-only folder: the failure comes after dxgi.dll has
        // already been placed.
        let ro = fx.game.join("ro");
        std::fs::create_dir(&ro).unwrap();
        std::fs::set_permissions(&ro, std::fs::Permissions::from_mode(0o555)).unwrap();
        let files = [
            planned(&fx, "dxgi.dll", b"ours", FileKind::Binary),
            planned(&fx, "ro/nvngx.dll", b"ours", FileKind::Binary),
        ];
        let err = apply(&fx.state, &g(&fx), src(), &files, &[]).unwrap_err();
        std::fs::set_permissions(&ro, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(format!("{err:#}").contains("rolled back"), "{err:#}");
        assert_eq!(read(&fx.game.join("dxgi.dll")), b"original");
        assert!(!ro.join("nvngx.dll").exists());
        assert!(Manifest::load(&fx.state, "g").unwrap().is_none());
    }

    #[test]
    fn a_target_that_cannot_exist_fails_before_anything_changes() {
        let fx = fixture();
        std::fs::write(fx.game.join("dxgi.dll"), b"original").unwrap();
        std::fs::write(fx.game.join("blocker"), b"a file, not a folder").unwrap();
        let files = [
            planned(&fx, "dxgi.dll", b"ours", FileKind::Binary),
            planned(&fx, "blocker/nvngx.dll", b"ours", FileKind::Binary),
        ];
        assert!(apply(&fx.state, &g(&fx), src(), &files, &[]).is_err());
        assert_eq!(read(&fx.game.join("dxgi.dll")), b"original");
        assert!(Manifest::load(&fx.state, "g").unwrap().is_none());
        assert!(
            !Manifest::backup_dir(&fx.state, "g").exists(),
            "no backup either"
        );
    }

    #[test]
    fn an_apply_cut_short_is_rolled_back_by_recover() {
        let fx = fixture();
        std::fs::write(fx.game.join("dxgi.dll"), b"original").unwrap();
        let files = [planned(&fx, "dxgi.dll", b"ours", FileKind::Binary)];
        let mut m = apply(&fx.state, &g(&fx), src(), &files, &[]).unwrap();
        // Pretend the process died after placing but before committing.
        m.state = State::Applying;
        m.save(&fx.state).unwrap();
        std::fs::write(fx.game.join(".dxgi.dll.bigame-new"), b"half").unwrap();
        let done = recover(&fx.state).unwrap();
        assert_eq!(done.len(), 1);
        assert!(done[0].1.is_ok());
        assert_eq!(read(&fx.game.join("dxgi.dll")), b"original");
        assert!(!fx.game.join(".dxgi.dll.bigame-new").exists());
    }

    #[test]
    fn nothing_escapes_the_game_folder() {
        let fx = fixture();
        std::os::unix::fs::symlink(&fx.payload, fx.game.join("bin")).unwrap();
        for rel in ["../outside.dll", "/etc/x.dll", "bin/dxgi.dll"] {
            let f = PlannedFile {
                path: rel.into(),
                source: planned(&fx, "x.dll", b"x", FileKind::Binary).source,
                kind: FileKind::Binary,
            };
            assert!(
                apply(&fx.state, &g(&fx), src(), &[f], &[]).is_err(),
                "{rel}"
            );
        }
        assert!(!fx.payload.join("dxgi.dll").exists());
        assert!(Manifest::load(&fx.state, "g").unwrap().is_none());
    }

    #[test]
    fn a_symlink_under_the_temporary_name_is_not_written_through() {
        let fx = fixture();
        let victim = fx.payload.join("victim.txt");
        std::fs::write(&victim, b"keep me").unwrap();
        std::os::unix::fs::symlink(&victim, fx.game.join(".dxgi.dll.bigame-new")).unwrap();
        let f = planned(&fx, "dxgi.dll", b"new dll", FileKind::Binary);
        apply(&fx.state, &g(&fx), src(), &[f], &[]).unwrap();
        assert_eq!(std::fs::read(&victim).unwrap(), b"keep me");
        let placed = fx.game.join("dxgi.dll");
        assert!(
            !std::fs::symlink_metadata(&placed)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(std::fs::read(&placed).unwrap(), b"new dll");
    }

    #[test]
    fn a_second_apply_and_duplicate_slots_are_refused() {
        let fx = fixture();
        let a = planned(&fx, "dxgi.dll", b"a", FileKind::Binary);
        let b = PlannedFile {
            path: "DXGI.dll".into(),
            ..a.clone()
        };
        assert!(apply(&fx.state, &g(&fx), src(), &[a.clone(), b], &[]).is_err());
        apply(&fx.state, &g(&fx), src(), std::slice::from_ref(&a), &[]).unwrap();
        assert!(apply(&fx.state, &g(&fx), src(), &[a], &[]).is_err());
    }

    #[test]
    fn a_log_the_component_writes_is_removed_with_it_but_a_pre_existing_one_is_not() {
        let fx = fixture();
        let files = [planned(&fx, "dxgi.dll", b"ours", FileKind::Binary)];
        let logs = [PathBuf::from("OptiScaler.log")];
        apply(&fx.state, &g(&fx), src(), &files, &logs).unwrap();
        std::fs::write(fx.game.join("OptiScaler.log"), b"run log").unwrap();
        remove(&fx.state, "g").unwrap();
        assert!(!fx.game.join("OptiScaler.log").exists());
        assert_eq!(
            read(&fx.state.join("g/last-run/OptiScaler.log")),
            b"run log"
        );

        // A log that was there before the install is not ours.
        std::fs::write(fx.game.join("OptiScaler.log"), b"someone else's").unwrap();
        let m = apply(&fx.state, &g(&fx), src(), &files, &logs).unwrap();
        assert!(m.generated.is_empty());
        remove(&fx.state, "g").unwrap();
        assert_eq!(read(&fx.game.join("OptiScaler.log")), b"someone else's");
    }

    #[test]
    fn a_file_rewritten_after_a_status_read_is_hashed_again() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("dxgi.dll");
        std::fs::write(&f, b"one").unwrap();
        let first = hash_for_status(&f).unwrap().unwrap();
        assert_eq!(hash_for_status(&f).unwrap().unwrap(), first, "remembered");
        // Replaced the way a tool or a game update replaces a file: a new
        // file renamed over it (new inode), same size.
        let tmp = dir.path().join("new");
        std::fs::write(&tmp, b"two").unwrap();
        std::fs::rename(&tmp, &f).unwrap();
        assert_ne!(hash_for_status(&f).unwrap().unwrap(), first);
        std::fs::remove_file(&f).unwrap();
        assert!(hash_for_status(&f).unwrap().is_none());
    }

    #[test]
    fn repair_puts_back_only_what_is_missing() {
        let fx = fixture();
        let files = [
            planned(&fx, "dxgi.dll", b"ours", FileKind::Binary),
            planned(&fx, "OptiScaler.ini", b"cfg", FileKind::Config),
        ];
        let m = apply(&fx.state, &g(&fx), src(), &files, &[]).unwrap();
        std::fs::remove_file(fx.game.join("dxgi.dll")).unwrap();
        std::fs::write(fx.game.join("OptiScaler.ini"), b"user edit").unwrap();
        assert_eq!(
            repair_missing(&m, &files).unwrap(),
            [PathBuf::from("dxgi.dll")]
        );
        assert_eq!(read(&fx.game.join("dxgi.dll")), b"ours");
        assert_eq!(read(&fx.game.join("OptiScaler.ini")), b"user edit");
    }

    #[test]
    fn a_damaged_backup_is_never_restored() {
        let fx = fixture();
        std::fs::write(fx.game.join("dxgi.dll"), b"original").unwrap();
        let m = apply(
            &fx.state,
            &g(&fx),
            src(),
            &[planned(&fx, "dxgi.dll", b"ours", FileKind::Binary)],
            &[],
        )
        .unwrap();
        std::fs::write(&m.entries[0].replaced.as_ref().unwrap().path, b"corrupt").unwrap();
        assert!(remove(&fx.state, "g").is_err());
        assert_eq!(read(&fx.game.join("dxgi.dll")), b"ours", "left as it was");
        assert!(
            Manifest::load(&fx.state, "g").unwrap().is_some(),
            "kept for a retry"
        );
    }

    #[test]
    fn a_game_folder_that_is_not_there_is_never_read_as_all_files_missing() {
        let fx = fixture();
        std::fs::write(fx.game.join("dxgi.dll"), b"original").unwrap();
        let files = [planned(&fx, "dxgi.dll", b"ours", FileKind::Binary)];
        apply(&fx.state, &g(&fx), src(), &files, &[]).unwrap();
        // The library's drive is not mounted, or the game moved away.
        let away = fx.game.with_file_name("away");
        std::fs::rename(&fx.game, &away).unwrap();
        let err = remove(&fx.state, "g").unwrap_err();
        assert!(format!("{err:#}").contains("is not there"), "{err:#}");
        assert!(!fx.game.exists(), "nothing made where the game was");
        assert!(Manifest::load(&fx.state, "g").unwrap().is_some(), "kept");
        assert_eq!(read(&away.join("dxgi.dll")), b"ours");
    }

    #[test]
    fn repair_makes_again_only_folders_the_install_created() {
        let fx = fixture();
        std::fs::create_dir(fx.game.join("bin")).unwrap();
        let files = [
            planned(&fx, "bin/dxgi.dll", b"ours", FileKind::Binary),
            planned(&fx, "made/x.dll", b"x", FileKind::Binary),
        ];
        let m = apply(&fx.state, &g(&fx), src(), &files, &[]).unwrap();
        assert_eq!(m.created_dirs, [PathBuf::from("made")]);
        // A folder of the game's gone: nothing is placed, nor made.
        std::fs::remove_dir_all(fx.game.join("bin")).unwrap();
        std::fs::remove_dir_all(fx.game.join("made")).unwrap();
        assert!(repair_missing(&m, &files).is_err());
        assert!(!fx.game.join("bin").exists() && !fx.game.join("made").exists());
        // Its own folder is made again.
        std::fs::create_dir(fx.game.join("bin")).unwrap();
        repair_missing(&m, &files).unwrap();
        assert_eq!(read(&fx.game.join("made/x.dll")), b"x");
        assert_eq!(read(&fx.game.join("bin/dxgi.dll")), b"ours");
    }

    #[test]
    fn a_file_already_there_in_another_letter_case_is_the_one_replaced() {
        let fx = fixture();
        std::fs::write(fx.game.join("AMD_FidelityFX_DX12.dll"), b"the game's").unwrap();
        let files = [planned(
            &fx,
            "amd_fidelityfx_dx12.dll",
            b"ours",
            FileKind::Binary,
        )];
        let m = apply(&fx.state, &g(&fx), src(), &files, &[]).unwrap();
        let names: Vec<String> = std::fs::read_dir(&fx.game)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            names,
            ["AMD_FidelityFX_DX12.dll"],
            "no second file beside it"
        );
        assert_eq!(m.entries[0].path, PathBuf::from("AMD_FidelityFX_DX12.dll"));
        assert!(m.entries[0].replaced.is_some(), "backed up");
        remove(&fx.state, "g").unwrap();
        assert_eq!(
            read(&fx.game.join("AMD_FidelityFX_DX12.dll")),
            b"the game's"
        );

        // Two spellings already: which one the game loads is not guessed.
        std::fs::write(fx.game.join("amd_fidelityfx_dx12.dll"), b"another").unwrap();
        assert!(apply(&fx.state, &g(&fx), src(), &files, &[]).is_err());
        assert!(Manifest::load(&fx.state, "g").unwrap().is_none());
    }

    #[test]
    fn a_link_where_a_file_was_placed_is_left_alone_and_the_rest_is_undone() {
        let fx = fixture();
        std::fs::write(fx.game.join("dxgi.dll"), b"original").unwrap();
        std::fs::create_dir(fx.game.join("bin")).unwrap();
        let files = [
            planned(&fx, "bin/OptiScaler.ini", b"cfg", FileKind::Config),
            planned(&fx, "dxgi.dll", b"ours", FileKind::Binary),
        ];
        apply(&fx.state, &g(&fx), src(), &files, &[]).unwrap();
        // A mod manager links the folder elsewhere.
        let elsewhere = fx.payload.join("bin-elsewhere");
        std::fs::rename(fx.game.join("bin"), &elsewhere).unwrap();
        std::os::unix::fs::symlink(&elsewhere, fx.game.join("bin")).unwrap();
        let out = remove(&fx.state, "g").unwrap();
        assert!(out.contains(&FileOutcome::KeptChanged("bin/OptiScaler.ini".into())));
        assert!(out.contains(&FileOutcome::Restored("dxgi.dll".into())));
        assert_eq!(read(&fx.game.join("dxgi.dll")), b"original");
        assert_eq!(
            read(&elsewhere.join("OptiScaler.ini")),
            b"cfg",
            "not followed"
        );
        assert!(Manifest::load(&fx.state, "g").unwrap().is_none());
    }

    #[test]
    fn a_backup_folder_from_the_same_second_is_never_shared() {
        let fx = fixture();
        // Originals an earlier removal kept, in folders named after the
        // seconds this apply may start in.
        let now = crate::unix_now();
        let backups = Manifest::backup_dir(&fx.state, "g");
        for at in now..now + 10 {
            std::fs::create_dir_all(backups.join(at.to_string())).unwrap();
            std::fs::write(backups.join(at.to_string()).join("dxgi.dll"), b"kept").unwrap();
        }
        std::fs::write(fx.game.join("dxgi.dll"), b"original").unwrap();
        let files = [planned(&fx, "dxgi.dll", b"ours", FileKind::Binary)];
        let m = apply(&fx.state, &g(&fx), src(), &files, &[]).unwrap();
        assert!(m.started_at >= now + 10, "{}", m.started_at);
        let backup = m.entries[0].replaced.as_ref().unwrap();
        assert!(
            backup
                .path
                .starts_with(backups.join(m.started_at.to_string()))
        );
        remove(&fx.state, "g").unwrap();
        assert_eq!(read(&fx.game.join("dxgi.dll")), b"original");
        for at in now..now + 10 {
            assert_eq!(
                read(&backups.join(at.to_string()).join("dxgi.dll")),
                b"kept"
            );
        }
        assert!(!backups.join(m.started_at.to_string()).exists());
    }

    #[test]
    fn edited_configs_two_removals_keep_are_never_written_over() {
        let fx = fixture();
        let files = [planned(&fx, "OptiScaler.ini", b"a=1", FileKind::Config)];
        let mut copies = Vec::new();
        for edit in [b"a=2", b"a=3"] {
            apply(&fx.state, &g(&fx), src(), &files, &[]).unwrap();
            std::fs::write(fx.game.join("OptiScaler.ini"), edit).unwrap();
            let out = remove(&fx.state, "g").unwrap();
            let [FileOutcome::EditedCopyKept(_, copy)] = out.as_slice() else {
                panic!("{out:?}")
            };
            copies.push(copy.clone());
        }
        assert_ne!(copies[0], copies[1]);
        assert_eq!(read(&copies[0]), b"a=2");
        assert_eq!(read(&copies[1]), b"a=3");
    }

    #[test]
    fn a_games_lock_keeps_a_second_change_waiting() {
        let fx = fixture();
        let first = lock(&fx.state, "g").unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        let state = fx.state.clone();
        let waiter = std::thread::spawn(move || {
            let _second = lock(&state, "g").unwrap();
            tx.send(()).unwrap();
        });
        // Another game is not held up.
        drop(lock(&fx.state, "other").unwrap());
        assert!(
            rx.recv_timeout(std::time::Duration::from_millis(200))
                .is_err(),
            "a second lock of one game was given while the first was held"
        );
        drop(first);
        rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
        waiter.join().unwrap();
        // Not a folder a removal or the recovery takes for a game's.
        assert!(Manifest::load(&fx.state, ".locks").unwrap().is_none());
        assert!(lock(&fx.state, "../g").is_err());
    }

    #[test]
    fn what_is_kept_after_a_removal_is_listed() {
        let fx = fixture();
        std::fs::write(fx.game.join("dxgi.dll"), b"original").unwrap();
        let files = [
            planned(&fx, "dxgi.dll", b"ours", FileKind::Binary),
            planned(&fx, "OptiScaler.ini", b"a=1", FileKind::Config),
        ];
        apply(&fx.state, &g(&fx), src(), &files, &[]).unwrap();
        std::fs::write(fx.game.join("dxgi.dll"), b"reshade").unwrap();
        std::fs::write(fx.game.join("OptiScaler.ini"), b"a=2").unwrap();
        remove(&fx.state, "g").unwrap();
        let kept = kept(&fx.state);
        assert_eq!(kept.len(), 2, "{kept:?}");
        let original = kept
            .iter()
            .find(|k| k.file == Path::new("dxgi.dll"))
            .unwrap();
        assert_eq!(read(&original.copy), b"original");
        assert_eq!(original.title.as_deref(), Some("Game"));
        assert!(
            kept.iter()
                .any(|k| k.file == Path::new("OptiScaler.ini") && read(&k.copy) == b"a=2")
        );
    }

    #[test]
    fn the_games_own_settings_stay_recorded_until_they_are_put_back() {
        let fx = fixture();
        let files = [planned(&fx, "dxgi.dll", b"ours", FileKind::Binary)];
        let mut m = apply(&fx.state, &g(&fx), src(), &files, &[]).unwrap();
        let change = super::super::ingame::SettingChange {
            file: fx.payload.join("pfx/user.reg"),
            key: r"Software\Game".into(),
            value: "XESS".into(),
            set: 3,
            original: Some(0),
        };
        m.settings = vec![change.clone()];
        m.save(&fx.state).unwrap();
        // An update's removal: the files go, the setting's record stays.
        remove(&fx.state, "g").unwrap();
        assert!(!fx.game.join("dxgi.dll").exists());
        let left = Manifest::load(&fx.state, "g").unwrap().unwrap();
        assert!(left.settings_only());
        assert_eq!(left.settings, std::slice::from_ref(&change));
        // The next apply takes the record over, settings and all.
        let m = apply(&fx.state, &g(&fx), src(), &files, &[]).unwrap();
        assert_eq!(m.settings, [change]);
        assert_eq!(read(&fx.game.join("dxgi.dll")), b"ours");
    }
}
