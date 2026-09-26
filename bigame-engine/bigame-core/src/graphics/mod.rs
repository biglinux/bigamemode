//! AI Graphics: upscaling, frame generation and the files a game needs for
//! them — detected, planned, applied, verified and undone.
//!
//! This is not a performance daemon. CPU, scheduler and power policy belong to
//! falcond (see [`crate::turbo`]); this module owns what happens *inside the
//! game*: which upscaler and frame generator it uses, and any DLL or config
//! file BiGame-mode places in its folder to get there.

pub mod backend;
pub mod config;
pub mod diagnose;
pub mod external;
pub mod fsr4_upgrade;
pub mod gamedb;
pub mod ingame;
pub mod manifest;
pub mod optiscaler;
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
    crate::paths::state_home().join("bigame-mode/graphics")
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

/// The process names, in lower case, of every game BiGame-mode has files
/// installed in: every manifest read once, for a whole library.
#[must_use]
pub fn installed_processes(state: &Path) -> std::collections::HashSet<String> {
    let Ok(dir) = std::fs::read_dir(state) else {
        return std::collections::HashSet::new();
    };
    dir.flatten()
        .filter_map(|d| {
            let key = d.file_name().to_string_lossy().into_owned();
            manifest::Manifest::load(state, &key).ok().flatten()
        })
        .filter(|m| m.state == manifest::State::Installed)
        .filter_map(|m| m.process.map(|p| p.to_lowercase()))
        .collect()
}

/// What a launch of `process` must turn off: with `OptiScaler` upscaling in the
/// game, Gamescope upscaling and Wine FSR would be second upscalers in series,
/// and with its frame generation on, lsfg-vk a second frame generator.
///
/// `state` holds the manifests, `settings` the per-game settings
/// ([`crate::game_settings::dir`]). The settings are read under the name the
/// manifest records, which is the name AI Graphics saved them under, whatever
/// the case of `process`.
#[must_use]
pub fn launch_disables(state: &Path, settings: &Path, process: &str) -> Vec<rules::Tech> {
    let Some(m) = manifest_for_process(state, process) else {
        return Vec::new();
    };
    let mut off = vec![rules::Tech::GamescopeUpscaling, rules::Tech::WineFsr];
    // Frame generation is on when it was chosen, or when it was switched on
    // later from OptiScaler's overlay, which writes the ini in the game.
    let name = m.process.as_deref().unwrap_or(process);
    let chosen = crate::game_settings::load_from(settings, name)
        .is_ok_and(|s| s.ai_graphics.optiscaler_frame_generation());
    if chosen || optiscaler_frame_gen_on(&m) {
        off.push(rules::Tech::LsfgVk);
    }
    off
}

/// Whether the `OptiScaler.ini` in the game has frame generation on — the
/// file in the game, not what was planned: `OptiScaler`'s overlay writes its
/// changes there.
fn optiscaler_frame_gen_on(m: &manifest::Manifest) -> bool {
    m.entries
        .iter()
        .find(|e| {
            e.path
                .file_name()
                .is_some_and(|n| n.eq_ignore_ascii_case("OptiScaler.ini"))
        })
        .and_then(|e| manifest::resolve_inside(&m.install_root, &e.path).ok())
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|t| optiscaler::get_ini(&t, "FrameGen", "Enabled"))
        .is_some_and(|v| v.eq_ignore_ascii_case("true"))
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
    /// What `OptiScaler` is doing now.
    pub status: runtime::Status,
    /// What the game's own graphics path is doing now.
    pub native: runtime::NativeRuntime,
    /// Where neural rendering stands.
    pub neural: external::Status,
    /// What is installed differs from what the plan would install now: a
    /// choice changed since Apply ([`pending_changes`]).
    pub pending_changes: bool,
    /// `OptiScaler`'s frame generation is on in the game's own ini — what
    /// is installed, including a change made in its overlay. `None` when
    /// BiGame-mode installed nothing.
    pub installed_frame_generation: Option<bool>,
}

/// The running game, when it is `target`.
fn running_as(target: &Target) -> Option<crate::running::GameIdentity> {
    crate::running::detect().filter(|g| g.process_name.eq_ignore_ascii_case(&target.process))
}

/// What else is configured that the plan has to reconcile.
fn launch_context(target: &Target, cfg: &config::AiGraphicsConfig) -> plan::Context {
    let video = crate::video_config::load();
    let cache = optiscaler::cache_dir();
    plan::Context {
        fsr4_upgrade: fsr4_upgrade::is_enabled(target.app_id.as_deref()),
        gamescope_upscaling: video.upscaling.gamescope_enabled && video.upscaling.base_width > 0,
        wine_fsr: video.upscaling.wine_fsr_enabled,
        lsfg: crate::fg::is_active_for_game(&target.process),
        mangohud: false,
        // From what is known, no network: a plan is a dry run.
        optiscaler_version: versions::resolve(&cache, &cfg.version, &versions::load(&cache))
            .ok()
            .map(|r| r.version),
    }
}

/// The Proton prefix of a Steam game: `compatdata/<appid>/pfx` in the
/// library that holds its install folder (a stale prefix in another library
/// is not the one the game writes to).
#[must_use]
pub fn proton_prefix(target: &Target) -> Option<PathBuf> {
    let id = target.app_id.as_deref()?;
    let steamapps = target
        .install_root
        .ancestors()
        .find(|a| a.file_name().is_some_and(|n| n == "steamapps"))?;
    let prefix = steamapps.join("compatdata").join(id).join("pfx");
    prefix.is_dir().then_some(prefix)
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
    // The running game's prefix first; a prefix the launcher's records name
    // that is not a real prefix (stale, or the wrong library) falls through.
    let proton = running
        .as_ref()
        .and_then(|g| g.compatdata_path.as_deref())
        .and_then(report::proton_info)
        .or_else(|| proton_prefix(target).and_then(|p| report::proton_info(&p)));
    let report = report::build(
        &target.name,
        target.app_id.as_deref(),
        &scanned,
        running.as_ref(),
        &hw,
        installed.clone(),
        proton,
    )
    .with_listing(
        gamedb::GameDb::load()
            .lookup(target.app_id.as_deref(), &target.process)
            .cloned(),
    );
    let gpu = report.gpu().map(|g| g.name.clone());
    let plan = plan::plan(&report, cfg, &launch_context(target, cfg));
    let status = status_of(installed.as_ref(), running.as_ref(), &scanned);
    let maps = |pid: u32| std::fs::read_to_string(format!("/proc/{pid}/maps")).ok();
    let live_maps = running.as_ref().and_then(|g| maps(g.pid));
    let mut native = runtime::native_runtime(live_maps.as_deref());
    native.fsr4_upgrade_env = running
        .as_ref()
        .and_then(|g| fsr4_upgrade::in_environment(g.pid));
    let exe_dir = scanned
        .executable_dir()
        .unwrap_or_else(|| scanned.root.clone());
    let live = running
        .as_ref()
        .and_then(|g| runtime::process_age(g.pid).map(|age| (g.pid, exe_dir.as_path(), age)));
    let neural = external::status(&report, live, &maps, &external::fresh_log);
    tracing::info!(target: "graphics", game = %target.process, backend = plan.backend.id(),
        standing = ?plan.standing, summary = %plan.summary,
        frame_generation = ?plan.frame_generation, "graphics plan generated");
    tracing::info!(target: "graphics", game = %target.process, gpu = gpu.as_deref().unwrap_or("?"),
        api = ?report.api.api, translation = report.api.translation.unwrap_or("-"),
        native_fsr4 = report.native_fsr4_path(), neural = ?std::mem::discriminant(&neural),
        "graphics backend selection");
    let pending_changes = pending_changes(target, &plan);
    let installed_frame_generation = installed.as_ref().map(optiscaler_frame_gen_on);
    Analysis {
        report,
        plan,
        status,
        native,
        neural,
        pending_changes,
        installed_frame_generation,
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

/// Whether the running game's own FSR path can reach FSR 4 on this machine:
/// the game ships AMD's `FidelityFX` API and renders on an RDNA 4 card. It
/// reads the game library, the install folder and the hardware, none of which
/// change while the game runs, so a caller asks once per game and then
/// follows [`native_fsr4_loaded`].
#[must_use]
pub fn native_fsr4_applies(game: &crate::running::GameIdentity) -> bool {
    let Some(root) = game.install_path.as_ref() else {
        return false;
    };
    // The executable's folder: where a Steam game keeps its runtimes. Only a
    // library game's folder is scanned.
    let exe_dir = crate::games::detect_all()
        .into_iter()
        .find(|g| g.install_path.as_deref() == Some(root.as_path()))
        .and_then(|_| {
            let s = scan::scan(root, Some(&game.process_name));
            s.executable_dir()
        })
        .unwrap_or_else(|| root.clone());
    if !exe_dir.join("amd_fidelityfx_dx12.dll").is_file() {
        return false;
    }
    let hw = crate::hardware::Hardware::detect();
    game.render_card
        .as_deref()
        .and_then(|c| hw.gpus.iter().find(|g| g.card == c))
        .or_else(|| hw.render_gpu())
        .is_some_and(|g| {
            g.vendor == crate::hardware::GpuVendor::Amd
                && report::rdna_generation(&report::device_name(&g.pci_id).unwrap_or_default())
                    == Some(4)
        })
}

/// For a game where [`native_fsr4_applies`]: `Some(true)` when Proton's FSR 4
/// provider is mapped in it (its FSR path runs FSR 4), `Some(false)` when it
/// runs without the provider, `None` when its maps cannot be read.
#[must_use]
pub fn native_fsr4_loaded(pid: u32) -> Option<bool> {
    let maps = std::fs::read_to_string(format!("/proc/{pid}/maps")).ok()?;
    runtime::native_runtime(Some(&maps)).fsr4_provider_loaded
}

/// Whether `target` is running now.
#[must_use]
pub fn is_running(target: &Target) -> bool {
    running_as(target).is_some()
}

/// The release `policy` means, fetching the release list first if the
/// policy names a release not known yet.
fn release_for(
    cache: &Path,
    policy: &config::VersionPolicy,
) -> anyhow::Result<optiscaler::Release> {
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

/// What [`install`] did.
#[derive(Debug, Clone)]
pub struct Installed {
    /// The record of what was placed.
    pub manifest: manifest::Manifest,
    /// What was done with the game's own switch for the upscaler
    /// `OptiScaler` takes over, when the game list says where it is.
    pub game_setting: Option<ingame::Applied>,
}

/// Carry out `plan` for `target`: download (or reuse) the `OptiScaler`
/// release `version` names, build the payload and apply it as a
/// transaction. Then, when the game list says where the game keeps the
/// switch for the upscaler `OptiScaler` takes over, switch it on if it is
/// off — without it `OptiScaler` has nothing to replace — and record the
/// change for Restore.
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
) -> anyhow::Result<Installed> {
    let o = plan
        .optiscaler
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("this plan installs nothing"))?;
    ensure_closed(target)?;
    let exe_dir = exe_dir(target)?;
    let cache = optiscaler::cache_dir();
    let cached = optiscaler::fetch(&cache, &release_for(&cache, version)?)?;
    let mut m = apply_release(target, o, &cached, &exe_dir)?;
    tracing::info!(target: "graphics", game = %target.process, version = %m.source.version,
        "graphics enhancement installed; active from the next start");
    let game_setting = match input_setting(target, o.input) {
        Some(s) => {
            let (applied, changes) = ingame::switch_on(proton_prefix(target).as_deref(), &s);
            if !changes.is_empty() {
                m.settings = changes;
                if let Err(e) = m.save(&state_dir()) {
                    // Unrecorded, Restore could not put it back: undo it now.
                    let _ = ingame::restore(&m.settings);
                    return Err(
                        e.context("the game's setting could not be recorded; it was put back")
                    );
                }
            }
            Some(applied)
        }
        None => None,
    };
    Ok(Installed {
        manifest: m,
        game_setting,
    })
}

/// Whether what is installed in `target` differs from what `plan` would
/// install: another file set (frame generation adds one), or another
/// setting BiGame-mode writes in `OptiScaler.ini` (the output, the input,
/// frame generation). The ini compared is BiGame-mode's own staged copy,
/// not the one in the game, which `OptiScaler` rewrites on every start and
/// whose overlay changes are the user's.
///
/// `false` when nothing is installed or the plan installs nothing (the page
/// then offers Restore).
#[must_use]
pub fn pending_changes(target: &Target, plan: &plan::Plan) -> bool {
    let Some(o) = plan.optiscaler.as_ref() else {
        return false;
    };
    let state = state_dir();
    let key = target.key();
    let Ok(Some(m)) = manifest::Manifest::load(&state, &key) else {
        return false;
    };
    let staged = std::fs::read_to_string(state.join(&key).join("staging").join("OptiScaler.ini"))
        .unwrap_or_default();
    differs(&m, &staged, o)
}

/// [`pending_changes`], from the manifest, the staged ini and the options.
fn differs(m: &manifest::Manifest, staged_ini: &str, o: &optiscaler::Options) -> bool {
    let name = |p: &Path| {
        p.file_name()
            .map(|n| n.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default()
    };
    let mut installed: Vec<String> = m.entries.iter().map(|e| name(&e.path)).collect();
    let mut wanted: Vec<String> = [o.proxy.as_str(), "OptiScaler.ini"]
        .into_iter()
        .chain(optiscaler::release_files(o))
        .map(|f| name(Path::new(f)))
        .collect();
    installed.sort();
    installed.dedup();
    wanted.sort();
    wanted.dedup();
    if installed != wanted {
        return true;
    }
    optiscaler::ini_settings(o)
        .into_iter()
        .any(|(section, key, value)| {
            optiscaler::get_ini(staged_ini, section, key)
                .is_none_or(|v| !v.eq_ignore_ascii_case(&value))
        })
}

/// Replace what is installed in `target` with what `plan` installs now —
/// the page's *Apply changes*, after a different choice. Everything placed
/// is removed first (originals and the game's own settings put back), then
/// the plan is installed as a new transaction.
///
/// # Errors
/// Returns an error if the game is running, the removal fails (nothing was
/// installed anew), or the install fails — the game then has its own files,
/// as after Restore, and the error says so.
pub fn reinstall(
    target: &Target,
    plan: &plan::Plan,
    version: &config::VersionPolicy,
) -> anyhow::Result<Installed> {
    anyhow::ensure!(plan.optiscaler.is_some(), "this plan installs nothing");
    ensure_closed(target)?;
    // Everything that can fail without touching the game first: the
    // release, downloaded and checked.
    let cache = optiscaler::cache_dir();
    optiscaler::fetch(&cache, &release_for(&cache, version)?)?;
    remove(target)?;
    install(target, plan, version).map_err(|e| {
        e.context(
            "the new choice could not be installed; the game has its own files, as after Restore",
        )
    })
}

/// Where `target` keeps the switch for its `input` upscaler, from the game
/// list.
fn input_setting(target: &Target, input: optiscaler::Input) -> Option<ingame::InputSetting> {
    gamedb::GameDb::load()
        .lookup(target.app_id.as_deref(), &target.process)
        .and_then(|e| e.input_setting.clone())
        .filter(|s| s.input == input)
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
            // The game's own settings were not touched by the update.
            m.settings.clone_from(&old.settings);
            m.save(&state)?;
            tracing::info!(target: "graphics", game = %target.process, version = %to.version,
                "OptiScaler updated; the previous version is kept to go back to");
            Ok(m)
        }
        Err(e) => {
            tracing::warn!(target: "graphics", game = %target.process, error = %e,
                "OptiScaler update failed; putting the previous version back");
            match apply_release(target, o, &old_cached, &exe_dir) {
                Ok(mut back) => {
                    back.settings.clone_from(&old.settings);
                    let _ = back.save(&state);
                    Err(e.context(format!(
                    "the update failed; OptiScaler {} was put back",
                    old.source.version
                    )))
                }
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
    (m.state == manifest::State::Installed && m.source.component == optiscaler::COMPONENT).then(
        || {
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
        },
    )
}

/// Remove everything BiGame-mode placed in `target`, restoring originals —
/// files, and the game's own settings Apply switched on.
///
/// # Errors
/// Returns an error if the game is running, its Wine prefix stays in use,
/// or a file cannot be restored.
pub fn remove(target: &Target) -> anyhow::Result<Vec<transaction::FileOutcome>> {
    ensure_closed(target)?;
    let state = state_dir();
    let settings = manifest::Manifest::load(&state, &target.key())?
        .map(|m| m.settings)
        .unwrap_or_default();
    if !settings.is_empty() {
        for r in ingame::restore(&settings)? {
            tracing::info!(target: "graphics", game = %target.process, outcome = ?r,
                "the game's setting restored");
        }
    }
    transaction::remove(&state, &target.key())
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
        let settings = dir.path().join("settings");
        assert!(launch_disables(&state, &settings, "SOTTR.exe").is_empty());
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
            launch_disables(&state, &settings, "sottr.exe"),
            [rules::Tech::GamescopeUpscaling, rules::Tech::WineFsr]
        );
        assert!(launch_disables(&state, &settings, "other.exe").is_empty());

        // OptiScaler's frame generation chosen: lsfg-vk goes too, found under
        // the name the game was saved as whatever the launch spells it.
        let mut chosen = crate::game_settings::GameSettings::default();
        chosen.ai_graphics.mode = config::Mode::Advanced;
        chosen.ai_graphics.frame_generation = config::FrameGeneration::OptiScaler;
        chosen.ai_graphics.experimental = true;
        crate::game_settings::save_to(&settings, "SOTTR.exe", &chosen).unwrap();
        assert!(launch_disables(&state, &settings, "sottr.exe").contains(&rules::Tech::LsfgVk));
    }

    #[test]
    fn a_different_choice_is_a_pending_change_and_the_same_one_is_not() {
        use crate::graphics::optiscaler::{Api, FrameGen, Input, Options, Output};
        let o = Options {
            proxy: "dxgi.dll".into(),
            api: Api::Dx12,
            input: Input::Xess,
            output: Output::Fsr,
            frame_gen: FrameGen::Off,
            nvidia: false,
            dlss: false,
            watermark: false,
        };
        let entry = |p: &str| manifest::Entry {
            path: p.into(),
            sha256: String::new(),
            kind: manifest::FileKind::Binary,
            replaced: None,
        };
        let mut m = manifest::Manifest {
            schema: manifest::SCHEMA,
            game_key: "steam-1".into(),
            process: None,
            title: None,
            install_root: "/g".into(),
            source: manifest::Source::default(),
            started_at: 0,
            state: manifest::State::Installed,
            entries: [
                "dxgi.dll",
                "OptiScaler.ini",
                "amd_fidelityfx_dx12.dll",
                "amd_fidelityfx_upscaler_dx12.dll",
            ]
            .into_iter()
            .map(entry)
            .collect(),
            created_dirs: vec![],
            generated: vec![],
            previous: None,
            managed: true,
            settings: vec![],
        };
        let staged = optiscaler::ini_settings(&o)
            .into_iter()
            .fold(String::new(), |t, (s, k, v)| {
                optiscaler::set_ini(&t, s, k, &v)
            });
        assert!(!differs(&m, &staged, &o), "what is installed");

        // Frame generation chosen: one more file, other settings.
        let fg = Options {
            frame_gen: FrameGen::OptiFgFsr,
            ..o.clone()
        };
        assert!(differs(&m, &staged, &fg));
        m.entries
            .push(entry("amd_fidelityfx_framegeneration_dx12.dll"));
        assert!(
            differs(&m, &staged, &fg),
            "same files, the ini still says off"
        );

        // XeSS as the output instead of FSR: other files.
        m.entries.pop();
        let xess = Options {
            output: Output::Xess,
            ..o
        };
        assert!(differs(&m, &staged, &xess));
    }

    #[test]
    fn optiscaler_frame_generation_on_in_the_game_turns_lsfg_off_for_the_launch() {
        let dir = tempfile::tempdir().unwrap();
        let (state, game) = (dir.path().join("state"), dir.path().join("game"));
        std::fs::create_dir_all(&game).unwrap();
        let ini = dir.path().join("OptiScaler.ini");
        std::fs::write(&ini, "[FrameGen]\nEnabled=false\n").unwrap();
        apply(
            &state,
            &Game {
                key: "steam-1",
                root: &game,
                process: Some("Game.exe"),
                title: None,
            },
            manifest::Source::default(),
            &[PlannedFile {
                path: "OptiScaler.ini".into(),
                source: ini,
                kind: FileKind::Config,
            }],
            &[],
        )
        .unwrap();
        // Not chosen in the settings (there are none)…
        let settings = dir.path().join("settings");
        assert!(!launch_disables(&state, &settings, "Game.exe").contains(&rules::Tech::LsfgVk));
        // …but switched on later from OptiScaler's overlay, which writes the ini.
        std::fs::write(game.join("OptiScaler.ini"), "[FrameGen]\nEnabled=true\n").unwrap();
        assert!(launch_disables(&state, &settings, "Game.exe").contains(&rules::Tech::LsfgVk));
    }
}
