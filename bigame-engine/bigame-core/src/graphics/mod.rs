//! AI Graphics: upscaling, frame generation and the files a game needs for
//! them — detected, planned, applied, verified and undone.
//!
//! This is not a performance daemon. CPU, scheduler and power policy belong to
//! falcond (see [`crate::turbo`]); this module owns what happens *inside the
//! game*: which upscaler and frame generator it uses, and any DLL or config
//! file BiGame-mode places in its folder to get there.

pub mod config;
pub mod gamedb;
pub mod manifest;
pub mod optiscaler;
pub mod outcomes;
pub mod pe;
pub mod plan;
pub mod report;
pub mod rules;
pub mod runtime;
pub mod scan;
pub mod support;
pub mod text;
pub mod transaction;
pub mod versions;

use std::path::{Path, PathBuf};

/// Where AI Graphics keeps its manifests and backups:
/// `$XDG_STATE_HOME/bigame-mode/graphics`.
#[must_use]
pub fn state_dir() -> PathBuf {
    std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_else(|| "/tmp".into()))
                .join(".local/state")
        })
        .join("bigame-mode/graphics")
}

/// The installed manifest for the game that runs as `process`, if
/// BiGame-mode placed anything in it.
#[must_use]
pub fn manifest_for_process(state: &Path, process: &str) -> Option<manifest::Manifest> {
    std::fs::read_dir(state).ok()?.flatten().find_map(|d| {
        let key = d.file_name().to_string_lossy().into_owned();
        manifest::Manifest::load(state, &key)
            .ok()
            .flatten()
            .filter(|m| {
                m.state == manifest::State::Installed
                    && m.process
                        .as_deref()
                        .is_some_and(|p| p.eq_ignore_ascii_case(process))
            })
    })
}

/// What a launch of `process` must turn off (§ Harmony): with `OptiScaler`
/// upscaling in the game, Gamescope upscaling and Wine FSR would be second
/// upscalers in series.
#[must_use]
pub fn launch_disables(state: &Path, process: &str) -> Vec<rules::Tech> {
    if manifest_for_process(state, process).is_some() {
        vec![rules::Tech::GamescopeUpscaling, rules::Tech::WineFsr]
    } else {
        Vec::new()
    }
}

/// Every game BiGame-mode has placed files in, as targets.
#[must_use]
pub fn installed() -> Vec<Target> {
    let state = state_dir();
    let Ok(dirs) = std::fs::read_dir(&state) else {
        return Vec::new();
    };
    let mut out: Vec<Target> = dirs
        .flatten()
        .filter_map(|d| {
            let key = d.file_name().to_string_lossy().into_owned();
            let m = manifest::Manifest::load(&state, &key).ok().flatten()?;
            let process = m.process.clone()?;
            Some(Target {
                name: m.title.clone().unwrap_or_else(|| process.clone()),
                app_id: key.strip_prefix("steam-").map(str::to_owned),
                process,
                install_root: m.install_root.clone(),
            })
        })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// A game AI Graphics works on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    /// Title.
    pub name: String,
    /// The process name it runs as — the profile's key.
    pub process: String,
    /// Steam app id.
    pub app_id: Option<String>,
    /// Install folder.
    pub install_root: PathBuf,
}

impl Target {
    /// The manifest key.
    #[must_use]
    pub fn key(&self) -> String {
        manifest::game_key(self.app_id.as_deref(), &self.process, &self.install_root)
    }
}

/// The installed game whose process is `process`, as a target — from the
/// launchers' own records (Steam libraries, Lutris, Heroic).
#[must_use]
pub fn target_for_process(process: &str) -> Option<Target> {
    crate::games::detect_all().into_iter().find_map(|g| {
        let hit = g
            .executables
            .iter()
            .any(|e| e.eq_ignore_ascii_case(process));
        match (hit, g.install_path) {
            (true, Some(root)) => Some(Target {
                name: g.name,
                process: process.to_owned(),
                app_id: g.app_id,
                install_root: root,
            }),
            _ => None,
        }
    })
}

/// Everything the AI Graphics page shows for a game.
#[derive(Debug, Clone)]
pub struct Analysis {
    /// What was found.
    pub report: report::Report,
    /// What would be done.
    pub plan: plan::Plan,
    /// What is happening now.
    pub status: runtime::Status,
}

/// The running game, when it is `target`.
fn running_as(target: &Target) -> Option<crate::running::GameIdentity> {
    crate::running::detect().filter(|g| g.process_name.eq_ignore_ascii_case(&target.process))
}

/// What else is configured that the plan has to reconcile.
fn launch_context(
    target: &Target,
    cfg: &config::AiGraphicsConfig,
    gpu: Option<&str>,
) -> plan::Context {
    let video = crate::video_config::load();
    let cache = optiscaler::cache_dir();
    plan::Context {
        gamescope_upscaling: video.upscaling.gamescope_enabled && video.upscaling.base_width > 0,
        wine_fsr: video.upscaling.wine_fsr_enabled,
        lsfg: crate::fg::is_active_for_game(&target.process),
        mangohud: false,
        // From what is known, no network: a plan is a dry run.
        optiscaler_version: versions::resolve(&cache, &cfg.version, &versions::load(&cache))
            .ok()
            .map(|r| r.version),
        measured: gpu
            .map(|g| {
                outcomes::for_game(&outcomes::load(&outcomes::path()), &target.key(), g)
                    .into_iter()
                    .cloned()
                    .collect()
            })
            .unwrap_or_default(),
    }
}

/// Scan, report, plan and status for `target` — reads only.
///
/// Scanning is bounded ([`scan`]) and takes milliseconds, but it reads the
/// game folder: call it off the UI thread.
#[must_use]
pub fn analyze(target: &Target, cfg: &config::AiGraphicsConfig) -> Analysis {
    tracing::info!(target: "graphics", game = %target.process, "graphics detection started");
    let running = running_as(target);
    let scanned = scan::scan(&target.install_root, Some(&target.process));
    let state = state_dir();
    let installed = manifest::Manifest::load(&state, &target.key())
        .ok()
        .flatten();
    let hw = crate::hardware::Hardware::detect();
    let report = report::build(
        &target.name,
        target.app_id.as_deref(),
        &scanned,
        running.as_ref(),
        &hw,
        installed.clone(),
    )
    .with_listing(
        gamedb::GameDb::load()
            .lookup(target.app_id.as_deref(), &target.process)
            .cloned(),
    );
    let gpu = report.gpu().map(|g| g.name.clone());
    let plan = plan::plan(&report, cfg, &launch_context(target, cfg, gpu.as_deref()));
    let status = status_of(installed.as_ref(), running.as_ref(), &scanned);
    tracing::info!(target: "graphics", game = %target.process, standing = ?plan.standing,
        summary = %plan.summary, "graphics plan generated");
    Analysis {
        report,
        plan,
        status,
    }
}

fn status_of(
    installed: Option<&manifest::Manifest>,
    running: Option<&crate::running::GameIdentity>,
    scanned: &scan::GameScan,
) -> runtime::Status {
    let exe_dir = scanned
        .executable_dir()
        .unwrap_or_else(|| scanned.root.clone());
    let live = running.and_then(|g| runtime::process_age(g.pid).map(|age| (g.pid, age)));
    runtime::status(
        installed,
        live.map(|(pid, age)| (pid, exe_dir.as_path(), age)),
        &|pid| std::fs::read_to_string(format!("/proc/{pid}/maps")).ok(),
        &runtime::fresh_log,
    )
}

/// The status of AI Graphics for `target` now — cheap enough for a
/// once-every-few-seconds refresh while a game runs.
#[must_use]
pub fn status(target: &Target) -> runtime::Status {
    let state = state_dir();
    let installed = manifest::Manifest::load(&state, &target.key())
        .ok()
        .flatten();
    if installed.is_none() {
        return runtime::Status::NotInstalled;
    }
    let running = running_as(target);
    let exe_dir = installed
        .as_ref()
        .and_then(|m| {
            m.entries
                .iter()
                .find(|e| {
                    e.path
                        .file_name()
                        .is_some_and(|n| n.eq_ignore_ascii_case("OptiScaler.ini"))
                })
                .map(|e| {
                    m.install_root
                        .join(e.path.parent().unwrap_or_else(|| Path::new("")))
                })
        })
        .unwrap_or_else(|| target.install_root.clone());
    let live = running
        .as_ref()
        .and_then(|g| runtime::process_age(g.pid).map(|age| (g.pid, age)));
    runtime::status(
        installed.as_ref(),
        live.map(|(pid, age)| (pid, exe_dir.as_path(), age)),
        &|pid| std::fs::read_to_string(format!("/proc/{pid}/maps")).ok(),
        &runtime::fresh_log,
    )
}

/// The status for a game that is running, from its identity — no process
/// scan. `None` when BiGame-mode has installed nothing in it.
#[must_use]
pub fn status_running(game: &crate::running::GameIdentity) -> Option<runtime::Status> {
    let root = game.install_path.as_ref()?;
    let key = manifest::game_key(game.steam_app_id.as_deref(), &game.process_name, root);
    let installed = manifest::Manifest::load(&state_dir(), &key)
        .ok()
        .flatten()?;
    let exe_dir = installed
        .entries
        .iter()
        .find(|e| {
            e.path
                .file_name()
                .is_some_and(|n| n.eq_ignore_ascii_case("OptiScaler.ini"))
        })
        .map_or_else(
            || root.clone(),
            |e| {
                installed
                    .install_root
                    .join(e.path.parent().unwrap_or_else(|| Path::new("")))
            },
        );
    let age = runtime::process_age(game.pid)?;
    Some(runtime::status(
        Some(&installed),
        Some((game.pid, exe_dir.as_path(), age)),
        &|pid| std::fs::read_to_string(format!("/proc/{pid}/maps")).ok(),
        &runtime::fresh_log,
    ))
}

/// Whether `target` is running now.
#[must_use]
pub fn is_running(target: &Target) -> bool {
    running_as(target).is_some()
}

/// The release `policy` means, fetching the release list first if the
/// policy names a release not known yet.
fn release_for(cache: &Path, policy: &config::VersionPolicy) -> anyhow::Result<optiscaler::Release> {
    let known = versions::load(cache);
    versions::resolve(cache, policy, &known).or_else(|first| {
        if matches!(policy, config::VersionPolicy::Recommended) {
            return Err(first);
        }
        let known = versions::refresh(cache).map_err(|e| first.context(e))?;
        versions::resolve(cache, policy, &known)
    })
}

fn ensure_closed(target: &Target) -> anyhow::Result<()> {
    anyhow::ensure!(
        running_as(target).is_none(),
        "{} is running; close it before changing its files",
        target.name
    );
    Ok(())
}

/// The folder of the game's executable, relative to its install folder.
fn exe_dir(target: &Target) -> anyhow::Result<PathBuf> {
    let scanned = scan::scan(&target.install_root, Some(&target.process));
    let exe = scanned
        .executable
        .ok_or_else(|| anyhow::anyhow!("the game's executable was not found"))?;
    Ok(exe.parent().map(Path::to_path_buf).unwrap_or_default())
}

/// Place `cached` in `target` as one transaction.
fn apply_release(
    target: &Target,
    o: &optiscaler::Options,
    cached: &optiscaler::Cached,
    exe_dir: &Path,
) -> anyhow::Result<manifest::Manifest> {
    let state = state_dir();
    let key = target.key();
    let files = optiscaler::payload(cached, o, exe_dir, &state.join(&key).join("staging"))?;
    transaction::apply(
        &state,
        &transaction::Game {
            key: &key,
            root: &target.install_root,
            process: Some(&target.process),
            title: Some(&target.name),
        },
        cached.source(),
        &files,
        &[exe_dir.join("OptiScaler.log")],
    )
}

/// Carry out `plan` for `target`: download (or reuse) the `OptiScaler`
/// release `version` names, build the payload and apply it as a
/// transaction.
///
/// Refuses while the game is running: its DLLs are loaded, and the change
/// would only take effect at the next start anyway.
///
/// # Errors
/// Returns an error if the plan installs nothing, the game is running, the
/// download or the transaction fails (the game folder is then as it was).
pub fn install(
    target: &Target,
    plan: &plan::Plan,
    version: &config::VersionPolicy,
) -> anyhow::Result<manifest::Manifest> {
    let o = plan
        .optiscaler
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("this plan installs nothing"))?;
    ensure_closed(target)?;
    let exe_dir = exe_dir(target)?;
    let cache = optiscaler::cache_dir();
    let cached = optiscaler::fetch(&cache, &release_for(&cache, version)?)?;
    let m = apply_release(target, o, &cached, &exe_dir)?;
    tracing::info!(target: "graphics", game = %target.process, version = %m.source.version,
        "graphics enhancement installed; active from the next start");
    Ok(m)
}

/// Replace the installed `OptiScaler` with `to`, keeping the one that was
/// there as the version to go back to.
///
/// Everything that can fail without touching the game happens first: the
/// installed release is found in the cache (so it can be put back), and `to`
/// is downloaded and checked. Then the installed files are removed —
/// originals restored — and `to` is applied as a new transaction. If that
/// fails, the previous version is applied again, so the game is left with
/// the version that worked, not with nothing.
///
/// # Errors
/// Returns an error if nothing is installed, the game is running, `to` is
/// what is installed, a download fails (nothing changed), or the update
/// failed — the error then says whether the previous version was put back.
pub fn update(
    target: &Target,
    plan: &plan::Plan,
    to: &optiscaler::Release,
) -> anyhow::Result<manifest::Manifest> {
    let o = plan
        .optiscaler
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("this plan installs nothing"))?;
    ensure_closed(target)?;
    let state = state_dir();
    let key = target.key();
    let cache = optiscaler::cache_dir();
    let old = manifest::Manifest::load(&state, &key)?
        .ok_or_else(|| anyhow::anyhow!("BiGame-mode has installed nothing in {}", target.name))?;
    anyhow::ensure!(
        old.state == manifest::State::Installed,
        "the last change to {} did not finish",
        target.name
    );
    anyhow::ensure!(
        old.source.archive_sha256.as_deref() != Some(to.sha256.as_str()),
        "OptiScaler {} is already installed",
        to.version
    );
    // Both releases in hand before the game folder changes.
    let old_cached = optiscaler::fetch(&cache, &versions::for_installed(&cache, &old.source)?)?;
    let new_cached = optiscaler::fetch(&cache, to)?;
    let exe_dir = exe_dir(target)?;

    tracing::info!(target: "graphics", game = %target.process, from = %old.source.version,
        to = %to.version, "OptiScaler update started");
    transaction::remove(&state, &key)?;
    match apply_release(target, o, &new_cached, &exe_dir) {
        Ok(mut m) => {
            m.previous = Some(old.source.clone());
            m.save(&state)?;
            tracing::info!(target: "graphics", game = %target.process, version = %to.version,
                "OptiScaler updated; the previous version is kept to go back to");
            Ok(m)
        }
        Err(e) => {
            tracing::warn!(target: "graphics", game = %target.process, error = %e,
                "OptiScaler update failed; putting the previous version back");
            match apply_release(target, o, &old_cached, &exe_dir) {
                Ok(_) => Err(e.context(format!(
                    "the update failed; OptiScaler {} was put back",
                    old.source.version
                ))),
                Err(e2) => Err(e.context(format!(
                    "the update failed, and putting OptiScaler {} back failed too ({e2:#}); the game has its own files",
                    old.source.version
                ))),
            }
        }
    }
}

/// Go back to the version installed before the last update.
///
/// # Errors
/// Returns an error if there was no update to go back from, or as
/// [`update`].
pub fn go_back(target: &Target, plan: &plan::Plan) -> anyhow::Result<manifest::Manifest> {
    let state = state_dir();
    let m = manifest::Manifest::load(&state, &target.key())?
        .ok_or_else(|| anyhow::anyhow!("BiGame-mode has installed nothing in {}", target.name))?;
    let previous = m
        .previous
        .ok_or_else(|| anyhow::anyhow!("there is no earlier version to go back to"))?;
    let release = versions::for_installed(&optiscaler::cache_dir(), &previous)?;
    update(target, plan, &release)
}

/// The update offer for `target`: the version installed, a newer one if
/// one is offered under `cfg`, and the version before the last update.
/// Refreshes the release list when it is a day old — call it off the UI
/// thread. `None` when BiGame-mode has installed nothing.
#[must_use]
pub fn update_offer(target: &Target, cfg: &config::AiGraphicsConfig) -> Option<versions::Offer> {
    let m = manifest::Manifest::load(&state_dir(), &target.key()).ok()??;
    (m.state == manifest::State::Installed && m.source.component == optiscaler::COMPONENT).then(|| {
        let known = versions::load_fresh(&optiscaler::cache_dir());
        versions::Offer {
            available: versions::offer(
                &m.source.version,
                &cfg.version,
                cfg.skipped_update.as_deref(),
                &known,
            ),
            installed: m.source.version.clone(),
            previous: m.previous.map(|p| p.version),
        }
    })
}

/// Remove everything BiGame-mode placed in `target`, restoring originals.
///
/// # Errors
/// Returns an error if the game is running or a file cannot be restored.
pub fn remove(target: &Target) -> anyhow::Result<Vec<transaction::FileOutcome>> {
    ensure_closed(target)?;
    transaction::remove(&state_dir(), &target.key())
}

/// Put back files that are missing (a game update, a file check by Steam),
/// from the cache and the configured copy kept while installed.
///
/// # Errors
/// Returns an error if nothing is installed, the game is running, or a file
/// cannot be restored.
pub fn repair(target: &Target) -> anyhow::Result<Vec<PathBuf>> {
    ensure_closed(target)?;
    let state = state_dir();
    let key = target.key();
    let m = manifest::Manifest::load(&state, &key)?
        .ok_or_else(|| anyhow::anyhow!("BiGame-mode has installed nothing in {}", target.name))?;
    // The files of the version that is installed, not of whatever version a
    // profile would get today: their hashes are what the manifest checks.
    let cache = optiscaler::cache_dir();
    let cached = optiscaler::fetch(&cache, &versions::for_installed(&cache, &m.source)?)?;
    let staging = state.join(&key).join("staging");
    // Every entry's source: the configured ini from staging, the proxy from
    // OptiScaler.dll, the rest by name from the release. The hashes in the
    // manifest decide whether each is the right file.
    let payload: Vec<transaction::PlannedFile> = m
        .entries
        .iter()
        .map(|e| {
            let name = e
                .path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let source = if name.eq_ignore_ascii_case("OptiScaler.ini") {
                staging.join("OptiScaler.ini")
            } else if scan::PROXY_SLOTS.contains(&name.to_ascii_lowercase().as_str()) {
                cached.dir.join("OptiScaler.dll")
            } else {
                cached.dir.join(&name)
            };
            transaction::PlannedFile {
                path: e.path.clone(),
                source,
                kind: e.kind,
            }
        })
        .collect();
    transaction::repair_missing(&m, &payload)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graphics::manifest::FileKind;
    use crate::graphics::transaction::{Game, PlannedFile, apply};

    #[test]
    fn a_game_with_optiscaler_installed_gets_no_second_upscaler() {
        let dir = tempfile::tempdir().unwrap();
        let (state, game) = (dir.path().join("state"), dir.path().join("game"));
        std::fs::create_dir_all(&game).unwrap();
        assert!(launch_disables(&state, "SOTTR.exe").is_empty());
        let src = dir.path().join("dxgi");
        std::fs::write(&src, b"x").unwrap();
        apply(
            &state,
            &Game {
                key: "steam-750920",
                root: &game,
                process: Some("SOTTR.exe"),
                title: None,
            },
            manifest::Source::default(),
            &[PlannedFile {
                path: "dxgi.dll".into(),
                source: src,
                kind: FileKind::Binary,
            }],
            &[],
        )
        .unwrap();
        assert_eq!(
            launch_disables(&state, "sottr.exe"),
            [rules::Tech::GamescopeUpscaling, rules::Tech::WineFsr]
        );
        assert!(launch_disables(&state, "other.exe").is_empty());
    }
}
