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
//! BiGame-mode starts means an apply was cut short; [`recover`] rolls it back.
//!
//! Removal trusts the manifest and the hashes, never file names: a file is
//! taken out only if it is still exactly what was placed. A *binary* that has
//! changed since belongs to whatever changed it and is left alone; a *config*
//! that has changed is BiGame-mode's own file with the user's edits in it, so
//! the edited copy is kept before the file is removed.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

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
}

/// A file to place.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedFile {
    /// Target, relative to the game's install folder.
    pub path: PathBuf,
    /// The file to copy there (in BiGame-mode's cache or a staging folder).
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
    /// There, but different.
    Changed,
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Copy `src` over `target` atomically: a temporary file in the target's own
/// folder (same filesystem, so the rename is atomic), synced, then renamed.
fn place(src: &Path, target: &Path) -> Result<()> {
    let name = target
        .file_name()
        .context("target has no file name")?
        .to_string_lossy();
    let tmp = target.with_file_name(format!(".{name}.bigame-new"));
    std::fs::copy(src, &tmp)
        .with_context(|| format!("copy {} → {}", src.display(), tmp.display()))?;
    std::fs::File::open(&tmp)?.sync_all()?;
    // Checked again right before the rename: the target must not have become
    // a symlink since the transaction began.
    if std::fs::symlink_metadata(target).is_ok_and(|m| m.file_type().is_symlink()) {
        let _ = std::fs::remove_file(&tmp);
        bail!(
            "{} became a symlink; refusing to replace it",
            target.display()
        );
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
                    if sha256_file(&copy)? != orig_sha {
                        bail!("backup of {} does not match the original", target.display());
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
/// Refuses when the game already has a manifest: an update is a removal
/// followed by an apply, so the original of every file is always the file
/// that was there before BiGame-mode, not a previous BiGame-mode payload.
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
    if let Some(existing) = Manifest::load(state_dir, game_key)? {
        bail!(
            "{game_key} already has {} {} installed ({:?}); remove it first",
            existing.source.component,
            existing.source.version,
            existing.state
        );
    }
    if files.is_empty() {
        bail!("nothing to install");
    }
    let started_at = now();
    let backup_root = Manifest::backup_dir(state_dir, game_key).join(started_at.to_string());

    // 1. Check, and hash what will be placed.
    let mut targets = Vec::with_capacity(files.len());
    for f in files {
        let target = resolve_inside(install_root, &f.path)?;
        if !f.source.is_file() {
            bail!("missing payload file {}", f.source.display());
        }
        targets.push((target, sha256_file(&f.source)?));
    }
    let mut seen = std::collections::HashSet::new();
    for f in files {
        if !seen.insert(f.path.to_string_lossy().to_ascii_lowercase()) {
            // Windows file names are case-insensitive: dxgi.dll and DXGI.dll
            // are one slot to the game.
            bail!("{} is listed twice", f.path.display());
        }
    }

    // 2. Back up originals, verified, before anything changes.
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
        install_root: install_root.to_path_buf(),
        source,
        started_at,
        state: State::Applying,
        entries,
        created_dirs,
        generated: fresh,
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
                bail!("{} does not match what was placed", target.display());
            }
        }
        Ok(())
    })();
    if let Err(e) = placed {
        tracing::warn!(target: "graphics", game = game_key, error = %e, "apply failed; rolling back");
        rollback(state_dir, &m).context("rollback after a failed apply")?;
        return Err(e.context("apply failed and was rolled back"));
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
/// binary is left alone; a changed config is kept as a copy first.
///
/// # Errors
/// Returns an error if a file cannot be restored or removed; the manifest is
/// then kept so the attempt can be repeated.
pub fn rollback(state_dir: &Path, m: &Manifest) -> Result<Vec<FileOutcome>> {
    let mut outcomes = Vec::new();
    let mut backups_still_needed = false;
    for e in &m.entries {
        let target = resolve_inside(&m.install_root, &e.path)?;
        // Leftover of an interrupted `place`.
        let name = target.file_name().map(|n| n.to_string_lossy().into_owned());
        if let Some(n) = name {
            let _ = std::fs::remove_file(target.with_file_name(format!(".{n}.bigame-new")));
        }
        let current = hash_if_present(&target)?;
        let ours = current.as_deref() == Some(e.sha256.as_str());
        let original = e.replaced.as_ref();
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
                    tracing::warn!(target: "graphics", file = %target.display(),
                        "changed since it was installed; left in place");
                    FileOutcome::KeptChanged(e.path.clone())
                }
                FileKind::Config => {
                    let keep = Manifest::backup_dir(state_dir, &m.game_key)
                        .join(format!("edited-{}", now()))
                        .join(&e.path);
                    std::fs::create_dir_all(keep.parent().context("no parent")?)?;
                    std::fs::copy(&target, &keep)?;
                    restore_or_remove(&target, original)?;
                    FileOutcome::EditedCopyKept(e.path.clone(), keep)
                }
            }
        };
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
        }
    }
    for d in m.created_dirs.iter().rev() {
        if let Ok(dir) = resolve_inside(&m.install_root, d) {
            // Only if empty: anything the game or the user put there stays.
            let _ = std::fs::remove_dir(dir);
        }
    }
    Manifest::delete(state_dir, &m.game_key)?;
    // The configured payload copies are only needed while installed.
    let _ = std::fs::remove_dir_all(state_dir.join(&m.game_key).join("staging"));
    if !backups_still_needed {
        let _ = std::fs::remove_dir_all(
            Manifest::backup_dir(state_dir, &m.game_key).join(m.started_at.to_string()),
        );
    }
    tracing::info!(target: "graphics", game = %m.game_key, files = outcomes.len(), "graphics rollback completed");
    Ok(outcomes)
}

fn restore_or_remove(target: &Path, original: Option<&Backup>) -> Result<()> {
    match original {
        Some(b) => {
            if manifest::sha256_file(&b.path)? != b.sha256 {
                bail!("backup {} is damaged; not restoring it", b.path.display());
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
    let m = Manifest::load(state_dir, game_key)?
        .with_context(|| format!("BiGame-mode has installed nothing in {game_key}"))?;
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
        if let Ok(Some(m)) = Manifest::load(state_dir, &key) {
            if m.state == State::Applying {
                tracing::warn!(target: "graphics", game = %key, "interrupted apply found; rolling back");
                done.push((key, rollback(state_dir, &m)));
            }
        }
    }
    Ok(done)
}

/// Check every file of `m` against what was placed.
#[must_use]
pub fn verify(m: &Manifest) -> Vec<(PathBuf, FileState)> {
    m.entries
        .iter()
        .map(|e| {
            let state = resolve_inside(&m.install_root, &e.path)
                .ok()
                .and_then(|t| hash_if_present(&t).ok())
                .map_or(FileState::Missing, |h| match h {
                    None => FileState::Missing,
                    Some(h) if h == e.sha256 => FileState::Intact,
                    Some(_) => FileState::Changed,
                });
            (e.path.clone(), state)
        })
        .collect()
}

/// Put back files of `m` that are missing, from `payload` (the same files the
/// install used, from BiGame-mode's cache). Changed files are not touched:
/// a changed binary has another owner now, and a changed config holds the
/// user's settings.
///
/// # Errors
/// Returns an error if a missing file cannot be placed or its payload does
/// not match the manifest.
pub fn repair_missing(m: &Manifest, payload: &[PlannedFile]) -> Result<Vec<PathBuf>> {
    let mut repaired = Vec::new();
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
            bail!(
                "the cached copy of {} is not what was installed",
                path.display()
            );
        }
        let target = resolve_inside(&m.install_root, &path)?;
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        place(&src.source, &target)?;
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
        }
    }

    fn src() -> Source {
        Source {
            component: "optiscaler".into(),
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
}
