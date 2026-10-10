//! AI Graphics: upscaling, frame generation and the files a game needs for
//! them — detected, planned, applied, verified and undone.
//!
//! This is not a performance daemon. CPU, scheduler and power policy belong to
//! falcond (see [`crate::turbo`]); this module owns what happens *inside the
//! game*: which upscaler and frame generator it uses, and any DLL or config
//! file Big Game Mode places in its folder to get there.

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
pub use crate::text;
pub mod transaction;
pub mod versions;

use std::path::{Path, PathBuf};

use anyhow::Context as _;

use crate::error::UserError;
use crate::text::N_;

/// Where AI Graphics keeps its manifests and backups:
/// `$XDG_STATE_HOME/bigame-mode/graphics`.
#[must_use]
pub fn state_dir() -> PathBuf {
    crate::paths::state_home().join("bigame-mode/graphics")
}

/// What the state holds for the game that runs as `process`.
enum ForProcess {
    /// Files installed. A manifest that only keeps the game's own settings
    /// placed nothing in the game, and is not this.
    Files(Box<manifest::Manifest>),
    /// A manifest that names that process and does not load (damaged, or
    /// from a newer Big Game Mode).
    Unreadable,
}

fn record_for_process(state: &Path, process: &str) -> Option<ForProcess> {
    let ours = |p: Option<&str>| p.is_some_and(|p| p.eq_ignore_ascii_case(process));
    std::fs::read_dir(state).ok()?.flatten().find_map(|d| {
        let key = d.file_name().to_string_lossy().into_owned();
        match manifest::Manifest::load(state, &key) {
            Ok(Some(m)) => (m.state == manifest::State::Installed
                && !m.entries.is_empty()
                && ours(m.process.as_deref()))
            .then(|| ForProcess::Files(Box::new(m))),
            Ok(None) => None,
            Err(_) => manifest::Manifest::peek(state, &key)
                .filter(|(p, _)| ours(p.as_deref()))
                .map(|_| ForProcess::Unreadable),
        }
    })
}

/// The process names, in lower case, of every game Big Game Mode has files
/// installed in: every manifest read once, for a whole library. A manifest
/// that does not load counts, by the process its text names: it is still a
/// game Restore has to be offered for.
#[must_use]
pub fn installed_processes(state: &Path) -> std::collections::HashSet<String> {
    let Ok(dir) = std::fs::read_dir(state) else {
        return std::collections::HashSet::new();
    };
    dir.flatten()
        .filter_map(|d| {
            let key = d.file_name().to_string_lossy().into_owned();
            match manifest::Manifest::load(state, &key) {
                Ok(m) => m
                    .filter(|m| m.state == manifest::State::Installed)
                    .and_then(|m| m.process),
                Err(_) => manifest::Manifest::peek(state, &key).and_then(|(p, _)| p),
            }
        })
        .map(|p| p.to_lowercase())
        .collect()
}

/// What a launch of `process` must turn off: with `OptiScaler` upscaling in the
/// game, Gamescope upscaling and Wine FSR would be second upscalers in series,
/// and with its frame generation on, lsfg-vk a second frame generator.
///
/// `state` holds the manifests, `settings` the per-game settings
/// ([`crate::game_settings::dir`]). The settings are read under the name the
/// manifest records, which is the name AI Graphics saved them under, whatever
/// the case of `process`. A manifest that does not load still names a game
/// with `OptiScaler` in it: its upscalers are turned off all the same.
#[must_use]
pub fn launch_disables(state: &Path, settings: &Path, process: &str) -> Vec<rules::Tech> {
    let installed = match record_for_process(state, process) {
        None => return Vec::new(),
        Some(ForProcess::Files(m)) => Some(m),
        Some(ForProcess::Unreadable) => None,
    };
    let mut off = vec![rules::Tech::GamescopeUpscaling, rules::Tech::WineFsr];
    // Frame generation is on when it was chosen, or when it was switched on
    // later from OptiScaler's overlay, which writes the ini in the game.
    let name = installed
        .as_ref()
        .and_then(|m| m.process.as_deref())
        .unwrap_or(process);
    let chosen = crate::game_settings::load_from(settings, name)
        .is_ok_and(|s| s.ai_graphics.optiscaler_frame_generation());
    if chosen || installed.as_deref().is_some_and(optiscaler_frame_gen_on) {
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

/// Every game Big Game Mode has placed files in, as targets — a game whose
/// manifest does not load included, by what its text says, so Restore is
/// still offered for it.
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
            let (title, process, root) = if let Ok(m) = manifest::Manifest::load(&state, &key) {
                let m = m?;
                (m.title, m.process?, m.install_root)
            } else {
                let (process, root) = manifest::Manifest::peek(&state, &key)?;
                (None, process?, root)
            };
            Some(Target {
                name: title.unwrap_or_else(|| process.clone()),
                app_id: key.strip_prefix("steam-").map(str::to_owned),
                process,
                install_root: root,
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

/// What the state holds for one game, as checked against the game's folder.
#[derive(Debug)]
enum Record {
    /// Nothing installed.
    Nothing,
    /// Installed in the game's folder. `true` when the record followed the
    /// game to its new folder ([`followed`]) and is not saved so yet.
    Installed(Box<manifest::Manifest>, bool),
    /// Installed into another folder, which is not the game's.
    Elsewhere(PathBuf),
    /// The record does not load.
    Unreadable(anyhow::Error),
}

/// The record for `key`, checked against the game's folder `root`.
fn record_in(state: &Path, key: &str, root: &Path) -> Record {
    match manifest::Manifest::load(state, key) {
        Ok(None) => Record::Nothing,
        Err(e) => Record::Unreadable(e),
        Ok(Some(m)) if m.is_for(root) => Record::Installed(Box::new(m), false),
        // Only the game's own settings are left: they are in its prefix,
        // not in its folder.
        Ok(Some(m)) if m.settings_only() => Record::Installed(
            Box::new(manifest::Manifest {
                install_root: root.to_path_buf(),
                ..m
            }),
            true,
        ),
        Ok(Some(m)) => match followed(&m, root) {
            Some(moved) => Record::Installed(Box::new(moved), true),
            None => Record::Elsewhere(m.install_root),
        },
    }
}

/// `m` for the game's folder `root`, when the game was moved there (Steam's
/// "Move install folder"): the folder the record names is gone, and the
/// game's is there. Its paths are relative, so they are looked for in the
/// game's folder, where removal still goes by the hashes. While the folder
/// the record names is there (a copy of the game, or a record that is not
/// this game's), it is not taken: only the game's folder as its launcher
/// records it is trusted, and which copy holds Big Game Mode's files is not
/// guessed.
fn followed(m: &manifest::Manifest, root: &Path) -> Option<manifest::Manifest> {
    if m.install_root.exists() || !root.is_dir() {
        return None;
    }
    Some(manifest::Manifest {
        install_root: root.to_path_buf(),
        ..m.clone()
    })
}

/// The status a record that cannot be used reads as.
fn unusable_status(r: &Record) -> Option<runtime::Status> {
    match r {
        Record::Elsewhere(dir) => Some(runtime::Status::Moved {
            installed_in: dir.clone(),
        }),
        Record::Unreadable(e) => Some(runtime::Status::Unreadable {
            error: crate::error::describe(e),
        }),
        Record::Nothing | Record::Installed(..) => None,
    }
}

/// The manifest of what is installed in `target`, for a change to it: a
/// record that followed the game to a new folder is saved so; one for
/// another folder, or one that does not load, is refused.
fn installed_manifest(state: &Path, target: &Target) -> anyhow::Result<manifest::Manifest> {
    match record_in(state, &target.key(), &target.install_root) {
        Record::Nothing => Err(UserError::with(
            N_("Big Game Mode has installed nothing in %s"),
            [&target.name],
        )
        .into()),
        Record::Unreadable(e) => Err(e),
        Record::Elsewhere(dir) => Err(UserError::with(
            N_("Big Game Mode installed into %s, which is not this game's folder (%s): the game was moved without its files, or the record is another game's. Nothing was changed"),
            [dir.display().to_string(), target.install_root.display().to_string()],
        )
        .into()),
        Record::Installed(m, false) => Ok(*m),
        Record::Installed(m, true) => {
            m.save(state)?;
            tracing::info!(target: "graphics", game = %target.process,
                root = %m.install_root.display(), "the record follows the game to its new folder");
            Ok(*m)
        }
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
    /// Big Game Mode installed nothing.
    pub installed_frame_generation: Option<bool>,
    /// The Steam launch option `FSR4_UPGRADE=1` is set for this game.
    pub fsr4_upgrade_set: bool,
    /// Apply switched the game's upscaler on in its settings, and it has
    /// been turned off again since (in its menu): `OptiScaler` then has
    /// nothing to take over.
    pub game_setting_off: bool,
}

/// Where the user's choice stands. Selected, configured, loaded and active
/// are different things, and the page says which one it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChoiceState {
    /// The choice needs no file and no launch option: nothing to apply.
    NothingToApply,
    /// Chosen and not applied yet: Apply does it.
    Selected,
    /// The choice needs nothing Big Game Mode installed here: Restore puts
    /// the game's own files back.
    NeedsRestore,
    /// Applied; it takes effect when the game starts.
    Configured,
    /// The game runs and loaded it, and it has not started working yet.
    Loaded,
    /// Working in the running game.
    Active,
    /// Applied, and the game shows it failed or did not load it, or files
    /// changed since: Diagnose says why.
    Failed,
    /// Not offered for this game (anti-cheat, or no way to do it).
    Blocked,
}

/// The facts [`choice_state`] reads, so the rule can be tested alone.
// Independent facts about one analysis, read in one place.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ChoiceFacts {
    blocked: bool,
    pending_changes: bool,
    installed: Option<RuntimeStage>,
    /// The plan installs files.
    needs_files: bool,
    /// The plan's only action is the FSR 4 launch option.
    launch_option_only: bool,
    launch_option_set: bool,
    /// The running game has the FSR 4 provider mapped (`None`: not running).
    provider_loaded: Option<bool>,
    /// The running game has `FSR4_UPGRADE=1` in its environment (`None`:
    /// not running, or its environment cannot be read).
    upgrade_env: Option<bool>,
}

/// The runtime status of what Big Game Mode installed, in [`ChoiceState`]'s
/// terms.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RuntimeStage {
    Waiting,
    Loaded,
    Active,
    Failed,
}

fn choice_state_of(f: ChoiceFacts) -> ChoiceState {
    if f.blocked {
        return ChoiceState::Blocked;
    }
    if f.pending_changes {
        return ChoiceState::Selected;
    }
    if let Some(stage) = f.installed {
        if !f.needs_files {
            return ChoiceState::NeedsRestore;
        }
        return match stage {
            RuntimeStage::Waiting => ChoiceState::Configured,
            RuntimeStage::Loaded => ChoiceState::Loaded,
            RuntimeStage::Active => ChoiceState::Active,
            RuntimeStage::Failed => ChoiceState::Failed,
        };
    }
    if f.launch_option_only {
        // Active needs both halves the evidence table names: the variable
        // in the game's environment and the provider mapped. The provider
        // alone is not the upgrade (it has been seen running the FSR 3
        // model).
        return match (f.launch_option_set, f.provider_loaded, f.upgrade_env) {
            (false, _, _) => ChoiceState::Selected,
            (true, None, _) => ChoiceState::Configured,
            (true, Some(true), Some(true)) => ChoiceState::Active,
            (true, Some(true), None) => ChoiceState::Loaded,
            (true, Some(true), Some(false)) | (true, Some(false), _) => ChoiceState::Failed,
        };
    }
    if f.needs_files {
        ChoiceState::Selected
    } else {
        ChoiceState::NothingToApply
    }
}

/// Where the choice in `a` stands.
#[must_use]
pub fn choice_state(a: &Analysis) -> ChoiceState {
    choice_state_of(choice_facts(a))
}

/// Where what is installed stands, leaving a pending change aside: for a
/// page that knows which part of the choice changed.
#[must_use]
pub fn installed_state(a: &Analysis) -> ChoiceState {
    choice_state_of(ChoiceFacts {
        pending_changes: false,
        needs_files: true,
        ..choice_facts(a)
    })
}

fn choice_facts(a: &Analysis) -> ChoiceFacts {
    use runtime::Status as S;
    let installed = a.report.installed.as_ref().map(|_| match &a.status {
        S::NotInstalled | S::Configured => RuntimeStage::Waiting,
        S::Starting | S::Loaded { .. } => RuntimeStage::Loaded,
        S::Active { .. } => RuntimeStage::Active,
        S::FilesChanged { .. }
        | S::NotDetected
        | S::Failed { .. }
        | S::SettingsLeft
        | S::Moved { .. }
        | S::Unreadable { .. } => RuntimeStage::Failed,
    });
    ChoiceFacts {
        // A record that cannot be used stops every change until it is
        // settled: an Apply over it would take Big Game Mode's own files for
        // the game's originals.
        blocked: a.plan.standing == plan::Standing::Blocked
            || matches!(a.status, S::Moved { .. } | S::Unreadable { .. }),
        pending_changes: a.pending_changes,
        installed,
        needs_files: a.plan.optiscaler.is_some(),
        launch_option_only: a.plan.optiscaler.is_none() && a.plan.native_action.is_some(),
        launch_option_set: a.fsr4_upgrade_set,
        provider_loaded: a.native.fsr4_provider_loaded,
        upgrade_env: a.native.fsr4_upgrade_env,
    }
}

/// The running game, when it is `target` — by the name its launcher
/// records, or by the game an Unreal Engine bootstrap of that name starts
/// (The Outer Worlds: Spacer's Choice Edition runs as
/// `IndianaEpicGameStore-Win64-Shipping.exe`, not as the `.exe` Heroic
/// starts). Without the second, a running game would read as closed, and
/// its files would not be protected from a change while in use.
fn running_as(target: &Target) -> Option<crate::running::GameIdentity> {
    let names = scan::runs_as(&target.install_root, &target.process);
    crate::running::detect()
        .filter(|g| names.iter().any(|n| g.process_name.eq_ignore_ascii_case(n)))
}

/// What else is configured that the plan has to reconcile.
fn launch_context(target: &Target, cfg: &config::AiGraphicsConfig) -> plan::Context {
    let video = crate::video_config::load();
    let cache = optiscaler::cache_dir();
    plan::Context {
        fsr4_upgrade: fsr4_upgrade::is_enabled(&target.process, target.app_id.as_deref()),
        gamescope_upscaling: video.upscaling.gamescope_enabled && video.upscaling.base_width > 0,
        wine_fsr: video.upscaling.wine_fsr_enabled,
        lsfg: crate::fg::is_active_for_game(&target.process),
        mangohud: false,
        heroic: target.app_id.is_none()
            && !crate::heroic_launch::targets(&target.process).is_empty(),
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
    let record = record_in(&state, &target.key(), &target.install_root);
    let unusable = unusable_status(&record);
    let installed = match record {
        Record::Installed(m, _) => Some(*m),
        Record::Nothing | Record::Elsewhere(_) | Record::Unreadable(_) => None,
    };
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
    let context = launch_context(target, cfg);
    let plan = plan::plan(&report, cfg, &context);
    let status =
        unusable.unwrap_or_else(|| status_of(installed.as_ref(), running.as_ref(), &scanned));
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
    let pending_changes = installed
        .as_ref()
        .is_some_and(|m| pending_in(&state, &target.key(), m, &plan));
    let installed_frame_generation = installed.as_ref().map(optiscaler_frame_gen_on);
    let game_setting_off = installed
        .as_ref()
        .is_some_and(|m| !ingame::switched_off_again(&m.settings).is_empty());
    Analysis {
        report,
        plan,
        status,
        native,
        neural,
        pending_changes,
        installed_frame_generation,
        fsr4_upgrade_set: context.fsr4_upgrade,
        game_setting_off,
    }
}

/// Switch the game's upscaler back on in its settings, as Apply did, after
/// it was turned off in the game's menu (`Analysis::game_setting_off`).
/// Not while the game runs: Wine writes its copy of the registry back when
/// the prefix exits.
///
/// # Errors
/// Returns an error when the game runs, nothing is installed, or the
/// registry cannot be written.
pub fn switch_game_setting_on_again(target: &Target) -> anyhow::Result<usize> {
    let state = state_dir();
    locked(&state, target, || {
        ensure_closed(target)?;
        let m = installed_manifest(&state, target)?;
        ingame::switch_on_again(&m.settings)
    })
}

/// Run `change` to `target`'s files or record under the game's lock
/// ([`transaction::lock`]), so a second change to the game — a double click,
/// the page and the Profiles menu at once — waits for the first and then
/// finds what it left, instead of backing up its files as the originals.
fn locked<R>(
    state: &Path,
    target: &Target,
    change: impl FnOnce() -> anyhow::Result<R>,
) -> anyhow::Result<R> {
    let _lock = transaction::lock(state, &target.key())?;
    change()
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
    let record = record_in(&state, &target.key(), &target.install_root);
    if let Some(s) = unusable_status(&record) {
        return s;
    }
    let installed = match record {
        Record::Installed(m, _) => Some(*m),
        Record::Nothing | Record::Elsewhere(_) | Record::Unreadable(_) => {
            return runtime::Status::NotInstalled;
        }
    };
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
/// scan. `None` when Big Game Mode has installed nothing in it.
#[must_use]
pub fn status_running(game: &crate::running::GameIdentity) -> Option<runtime::Status> {
    let root = game.install_path.as_ref()?;
    let key = manifest::game_key(game.steam_app_id.as_deref(), &game.process_name, root);
    let record = record_in(&state_dir(), &key, root);
    if let Some(s) = unusable_status(&record) {
        return Some(s);
    }
    let Record::Installed(installed, _) = record else {
        return None;
    };
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
    // Only a library game's folder is scanned (an Unreal game keeps its FSR
    // in a plugin folder, not beside the executable); anything else is
    // looked at where it runs from.
    let library_game = crate::games::detect_all()
        .into_iter()
        .any(|g| g.install_path.as_deref() == Some(root.as_path()));
    let ffx_api = if library_game {
        scan::scan(root, Some(&game.process_name)).has(scan::ComponentKind::FfxApi)
    } else {
        root.join("amd_fidelityfx_dx12.dll").is_file()
    };
    if !ffx_api {
        return false;
    }
    let hw = crate::hardware::Hardware::detect();
    game.render_card
        .as_deref()
        .and_then(|c| hw.gpus.iter().find(|g| g.card == c))
        .or_else(|| hw.render_gpu())
        .is_some_and(|g| {
            g.vendor == crate::hardware::GpuVendor::Amd
                && report::rdna_of(&g.card, &report::device_name(&g.pci_id).unwrap_or_default())
                    == Some(4)
        })
}

/// For a game where [`native_fsr4_applies`]: `Some(true)` when it runs with
/// `FSR4_UPGRADE=1` and Proton's FSR 4 provider mapped — the provider is
/// active; which model it runs only the game shows — `Some(false)` when
/// either is missing, `None` when its maps or environment cannot be read.
#[must_use]
pub fn native_fsr4_loaded(pid: u32) -> Option<bool> {
    let maps = std::fs::read_to_string(format!("/proc/{pid}/maps")).ok()?;
    let provider = runtime::native_runtime(Some(&maps)).fsr4_provider_loaded?;
    let env = fsr4_upgrade::in_environment(pid)?;
    Some(provider && env)
}

/// Whether `target` is running now: identified as the running game, or
/// any process of this user with a file of its folder mapped.
#[must_use]
pub fn is_running(target: &Target) -> bool {
    running_as(target).is_some()
        || mapping_process(Path::new("/proc"), &target.install_root).is_some()
}

/// A process listed under `proc` (`/proc`) that has a file under `root`
/// mapped. The running game Big Game Mode identifies is only the busiest one,
/// and only by the names its launcher records; a game also counts when it
/// runs under another name, or beside a busier one. Another user's maps
/// cannot be read, and are not this user's game.
///
/// Only Windows programs are read: AI Graphics installs into nothing else,
/// and under Wine every one has its `.exe` in its command line. Reading the
/// maps of every process would take a second with a browser open.
fn mapping_process(proc: &Path, root: &Path) -> Option<u32> {
    let root = std::fs::canonicalize(root).ok()?;
    let me = std::process::id();
    std::fs::read_dir(proc).ok()?.flatten().find_map(|p| {
        let pid: u32 = p.file_name().to_str()?.parse().ok()?;
        if pid == me {
            return None;
        }
        let cmdline = std::fs::read(p.path().join("cmdline")).ok()?;
        memchr::memmem::find(&cmdline.to_ascii_lowercase(), b".exe")?;
        let maps = std::fs::read_to_string(p.path().join("maps")).ok()?;
        runtime::mapped_paths(&maps)
            .iter()
            .any(|m| m.starts_with(&root))
            .then_some(pid)
    })
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
        !is_running(target),
        UserError::with(
            N_("%s is running; close it before changing its files"),
            [&target.name]
        )
    );
    Ok(())
}

/// Refuse to place anything in a game whose files show anti-cheat now, that
/// the game list blocks, or whose folder could not be read whole — checked
/// at the moment of the change, not taken from a plan made earlier: a game
/// update can bring anti-cheat with it.
fn injection_veto(target: &Target, db: &gamedb::GameDb) -> Option<UserError> {
    let scanned = scan::scan(&target.install_root, Some(&target.process));
    if let Some(ac) = scanned.anti_cheat.first() {
        return Some(UserError::with(
            N_(
                "%s protects this game (%s): external graphics injection is disabled to avoid compatibility problems or account penalties",
            ),
            [ac.name.clone(), ac.evidence.display().to_string()],
        ));
    }
    if let Some(reason) = db
        .lookup(target.app_id.as_deref(), &target.process)
        .and_then(|e| e.block.clone())
    {
        return Some(UserError::with(
            N_("the game list blocks graphics injection for this game: %s"),
            [reason],
        ));
    }
    scanned
        .truncated
        .then(|| UserError::plain(plan::TOO_LARGE_TO_CHECK))
}

/// The folder of the game's executable, relative to its install folder.
fn exe_dir(target: &Target) -> anyhow::Result<PathBuf> {
    let scanned = scan::scan(&target.install_root, Some(&target.process));
    let exe = scanned
        .executable
        .ok_or_else(|| UserError::plain(N_("the game's executable was not found")))?;
    Ok(exe.parent().map(Path::to_path_buf).unwrap_or_default())
}

/// Place `cached` in `target` as one transaction, with `carry` — what the
/// user changed in the `OptiScaler.ini` being replaced — kept in the new
/// ini.
///
/// The ini is configured in a folder of its own and becomes the staged copy
/// (what [`pending_changes`] and Repair read) only once the transaction is
/// committed: a refused or failed apply leaves the staged copy of what is
/// installed as it was.
fn apply_release(
    state: &Path,
    target: &Target,
    o: &optiscaler::Options,
    cached: &optiscaler::Cached,
    exe_dir: &Path,
    carry: &[optiscaler::IniValue],
) -> anyhow::Result<manifest::Manifest> {
    let key = target.key();
    let staging = state.join(&key).join("staging");
    let next = state.join(&key).join(manifest::unique_name("staging-"));
    remove_dir_if_there(&next)?;
    let applied = optiscaler::payload(cached, o, exe_dir, &next, carry).and_then(|files| {
        transaction::apply(
            state,
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
    });
    match applied {
        Ok(m) => {
            remove_dir_if_there(&staging)?;
            std::fs::rename(&next, &staging)
                .with_context(|| format!("rename {} → {}", next.display(), staging.display()))?;
            Ok(m)
        }
        Err(e) => {
            if let Err(cleanup) = remove_dir_if_there(&next) {
                tracing::warn!(target: "graphics", error = %format!("{cleanup:#}"),
                    "an unused staging folder was left");
            }
            Err(e)
        }
    }
}

fn remove_dir_if_there(dir: &Path) -> anyhow::Result<()> {
    match std::fs::remove_dir_all(dir) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e).with_context(|| format!("remove {}", dir.display())),
    }
}

/// What [`install`] did.
#[derive(Debug, Clone)]
pub struct Installed {
    /// The record of what was placed.
    pub manifest: manifest::Manifest,
    /// What was done with the game's own switch for the upscaler
    /// `OptiScaler` takes over, when the game list says where it is.
    pub game_setting: Option<ingame::Applied>,
    /// Settings the user had changed in the `OptiScaler.ini` that was
    /// replaced, kept in the new one (Apply changes).
    pub kept_settings: usize,
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
/// Returns an error if the plan installs nothing, the game is running or
/// shows anti-cheat now, the download or the transaction fails (the game
/// folder is then as it was).
pub fn install(
    target: &Target,
    plan: &plan::Plan,
    version: &config::VersionPolicy,
) -> anyhow::Result<Installed> {
    let (state, cache) = (state_dir(), optiscaler::cache_dir());
    let done = locked(&state, target, || {
        install_in(
            &state,
            &cache,
            &gamedb::GameDb::load(),
            target,
            plan,
            version,
            &[],
        )
    });
    tidy_cache(&state, &cache);
    done
}

fn install_in(
    state: &Path,
    cache: &Path,
    db: &gamedb::GameDb,
    target: &Target,
    plan: &plan::Plan,
    version: &config::VersionPolicy,
    carry: &[optiscaler::IniValue],
) -> anyhow::Result<Installed> {
    let o = plan
        .optiscaler
        .as_ref()
        .ok_or_else(|| UserError::plain(N_("this plan installs nothing")))?;
    ensure_closed(target)?;
    if let Some(veto) = injection_veto(target, db) {
        return Err(veto.into());
    }
    let exe_dir = exe_dir(target)?;
    let cached = optiscaler::fetch(cache, &release_for(cache, version)?)?;
    let mut m = apply_release(state, target, o, &cached, &exe_dir, carry)?;
    tracing::info!(target: "graphics", game = %target.process, version = %m.source.version,
        "graphics enhancement installed; active from the next start");
    let game_setting = match input_setting(db, target, o.input) {
        Some(s) => {
            let (applied, changes) = ingame::switch_on(proton_prefix(target).as_deref(), &s);
            if !changes.is_empty() {
                // Added to what an earlier removal could not put back yet.
                m.settings.extend(changes.iter().cloned());
                if let Err(e) = m.save(state) {
                    // Unrecorded, Restore could not put it back: undo it now.
                    return Err(match ingame::restore(&changes) {
                        Ok(_) => UserError::plain(N_(
                            "the game's setting could not be recorded; it was put back",
                        ))
                        .caused_by(e),
                        Err(undo) => UserError::plain(N_(
                            "the game's setting could not be recorded, nor put back: it stays switched on in the game's settings",
                        ))
                        .caused_by(e.context(format!("{undo:#}"))),
                    }
                    .into());
                }
            }
            Some(applied)
        }
        None => None,
    };
    Ok(Installed {
        manifest: m,
        game_setting,
        kept_settings: carry.len(),
    })
}

/// Whether what is installed in `target` differs from what `plan` would
/// install: another file set (frame generation adds one), or another
/// setting Big Game Mode writes in `OptiScaler.ini` (the output, the input,
/// frame generation). The ini compared is Big Game Mode's own staged copy,
/// not the one in the game, which `OptiScaler` rewrites on every start and
/// whose overlay changes are the user's.
///
/// `false` when nothing is installed or the plan installs nothing (the page
/// then offers Restore).
#[must_use]
pub fn pending_changes(target: &Target, plan: &plan::Plan) -> bool {
    let state = state_dir();
    match record_in(&state, &target.key(), &target.install_root) {
        Record::Installed(m, _) => pending_in(&state, &target.key(), &m, plan),
        Record::Nothing | Record::Elsewhere(_) | Record::Unreadable(_) => false,
    }
}

fn pending_in(state: &Path, key: &str, m: &manifest::Manifest, plan: &plan::Plan) -> bool {
    let Some(o) = plan.optiscaler.as_ref() else {
        return false;
    };
    // Only the game's own settings are left: Restore, not Apply, is next.
    if m.settings_only() {
        return false;
    }
    let staged = std::fs::read_to_string(state.join(key).join("staging").join("OptiScaler.ini"))
        .unwrap_or_default();
    differs(m, &staged, o)
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

/// What the user changed in `OptiScaler.ini` since it was installed: the
/// copy a removal kept of the edited ini ([`transaction::FileOutcome::EditedCopyKept`]),
/// against the ini Big Game Mode staged for it.
fn user_ini_edits(
    staged: Option<&str>,
    removed: &[transaction::FileOutcome],
) -> Vec<optiscaler::IniValue> {
    let Some(staged) = staged else {
        return Vec::new();
    };
    let copy = removed.iter().find_map(|o| match o {
        transaction::FileOutcome::EditedCopyKept(p, copy)
            if p.file_name()
                .is_some_and(|n| n.eq_ignore_ascii_case("OptiScaler.ini")) =>
        {
            Some(copy)
        }
        _ => None,
    });
    let Some(copy) = copy else {
        return Vec::new();
    };
    match std::fs::read_to_string(copy) {
        Ok(edited) => optiscaler::ini_edits(staged, &edited),
        Err(e) => {
            // The copy stays where it is; the new ini starts from the release.
            tracing::warn!(target: "graphics", copy = %copy.display(), error = %e,
                "the edited OptiScaler.ini could not be read to carry its settings over");
            Vec::new()
        }
    }
}

/// Big Game Mode's staged copy of the installed ini, read before a removal
/// deletes it.
fn staged_ini(state: &Path, key: &str) -> Option<String> {
    std::fs::read_to_string(state.join(key).join("staging").join("OptiScaler.ini")).ok()
}

/// Replace what is installed in `target` with what `plan` installs now —
/// the page's *Apply changes*, after a different choice. Everything placed
/// is removed first (originals and the game's own settings put back), then
/// the plan is installed as a new transaction, with the settings the user
/// changed in `OptiScaler`'s overlay kept.
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
    let (state, cache) = (state_dir(), optiscaler::cache_dir());
    // Once at the end: between its restore and its install no game names
    // the release it is about to install again.
    let done = locked(&state, target, || {
        reinstall_in(
            &state,
            &cache,
            &gamedb::GameDb::load(),
            target,
            plan,
            version,
        )
    });
    tidy_cache(&state, &cache);
    done
}

fn reinstall_in(
    state: &Path,
    cache: &Path,
    db: &gamedb::GameDb,
    target: &Target,
    plan: &plan::Plan,
    version: &config::VersionPolicy,
) -> anyhow::Result<Installed> {
    anyhow::ensure!(
        plan.optiscaler.is_some(),
        UserError::plain(N_("this plan installs nothing"))
    );
    ensure_closed(target)?;
    if let Some(veto) = injection_veto(target, db) {
        return Err(veto.into());
    }
    // Everything that can fail without touching the game first: the
    // release, downloaded and checked.
    optiscaler::fetch(cache, &release_for(cache, version)?)?;
    let staged = staged_ini(state, &target.key());
    let removed = remove_in(state, target)?;
    let carry = user_ini_edits(staged.as_deref(), &removed);
    install_in(state, cache, db, target, plan, version, &carry).map_err(|e| {
        UserError::plain(N_(
            "the new choice could not be installed; the game has its own files, as after Restore",
        ))
        .caused_by(e)
        .into()
    })
}

/// Where `target` keeps the switch for its `input` upscaler, from the game
/// list.
fn input_setting(
    db: &gamedb::GameDb,
    target: &Target,
    input: optiscaler::Input,
) -> Option<ingame::InputSetting> {
    db.lookup(target.app_id.as_deref(), &target.process)
        .and_then(|e| e.input_setting.clone())
        .filter(|s| s.input == input)
}

/// Replace the installed `OptiScaler` with `to`, keeping the one that was
/// there as the version to go back to.
///
/// Everything that can fail without touching the game happens first: the
/// installed release is found in the cache (so it can be put back), and `to`
/// is downloaded and checked. Then the installed files are removed —
/// originals restored — and `to` is applied as a new transaction, with the
/// settings the user changed in `OptiScaler`'s overlay kept. If that fails,
/// the previous version is applied again, so the game is left with the
/// version that worked, not with nothing. The game's own settings Apply
/// changed stay recorded throughout ([`transaction::rollback`] keeps them).
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
    let (state, cache) = (state_dir(), optiscaler::cache_dir());
    let done = locked(&state, target, || {
        update_in(&state, &cache, &gamedb::GameDb::load(), target, plan, to)
    });
    tidy_cache(&state, &cache);
    done
}

fn update_in(
    state: &Path,
    cache: &Path,
    db: &gamedb::GameDb,
    target: &Target,
    plan: &plan::Plan,
    to: &optiscaler::Release,
) -> anyhow::Result<manifest::Manifest> {
    let o = plan
        .optiscaler
        .as_ref()
        .ok_or_else(|| UserError::plain(N_("this plan installs nothing")))?;
    ensure_closed(target)?;
    let key = target.key();
    let old = installed_manifest(state, target)?;
    anyhow::ensure!(
        old.state == manifest::State::Installed && !old.entries.is_empty(),
        UserError::with(N_("the last change to %s did not finish"), [&target.name])
    );
    anyhow::ensure!(
        old.source.archive_sha256.as_deref() != Some(to.sha256.as_str()),
        UserError::with(N_("OptiScaler %s is already installed"), [&to.version])
    );
    if let Some(veto) = injection_veto(target, db) {
        return Err(veto.into());
    }
    // Both releases in hand before the game folder changes.
    let old_cached = optiscaler::fetch(cache, &versions::for_installed(cache, &old.source)?)?;
    let new_cached = optiscaler::fetch(cache, to)?;
    let exe_dir = exe_dir(target)?;

    tracing::info!(target: "graphics", game = %target.process, from = %old.source.version,
        to = %to.version, "OptiScaler update started");
    let staged = staged_ini(state, &key);
    let removed = transaction::remove(state, &key)?;
    let carry = user_ini_edits(staged.as_deref(), &removed);
    match apply_release(state, target, o, &new_cached, &exe_dir, &carry) {
        Ok(mut m) => {
            m.previous = Some(old.source.clone());
            m.save(state)?;
            tracing::info!(target: "graphics", game = %target.process, version = %to.version,
                "OptiScaler updated; the previous version is kept to go back to");
            Ok(m)
        }
        Err(e) => {
            tracing::warn!(target: "graphics", game = %target.process, error = %e,
                "OptiScaler update failed; putting the previous version back");
            match apply_release(state, target, o, &old_cached, &exe_dir, &carry) {
                Ok(mut back) => {
                    back.previous.clone_from(&old.previous);
                    if let Err(e) = back.save(state) {
                        // Only the version to go back to is lost with it.
                        tracing::warn!(target: "graphics", game = %target.process,
                            error = %format!("{e:#}"), "the earlier version to go back to was not recorded");
                    }
                    Err(UserError::with(
                        N_("the update failed; OptiScaler %s was put back"),
                        [&old.source.version],
                    )
                    .caused_by(e)
                    .into())
                }
                Err(e2) => Err(UserError::with(
                    N_("the update failed, and putting OptiScaler %s back failed too (%s); the game has its own files"),
                    [
                        text::Arg::Raw(old.source.version.clone()),
                        text::Arg::Text(crate::error::describe(&e2)),
                    ],
                )
                .caused_by(e)
                .into()),
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
    let m = installed_manifest(&state_dir(), target)?;
    let previous = m
        .previous
        .ok_or_else(|| UserError::plain(N_("there is no earlier version to go back to")))?;
    let release = versions::for_installed(&optiscaler::cache_dir(), &previous)?;
    update(target, plan, &release)
}

/// The update offer for `target`: the version installed, a newer one if
/// one is offered under `cfg`, and the version before the last update.
/// Refreshes the release list when it is a day old — call it off the UI
/// thread. `None` when Big Game Mode has installed nothing.
#[must_use]
pub fn update_offer(target: &Target, cfg: &config::AiGraphicsConfig) -> Option<versions::Offer> {
    let Record::Installed(m, _) = record_in(&state_dir(), &target.key(), &target.install_root)
    else {
        return None;
    };
    let usable = m.state == manifest::State::Installed
        && !m.entries.is_empty()
        && m.source.component == optiscaler::COMPONENT;
    usable.then(|| {
        let known = versions::load_fresh(&optiscaler::cache_dir());
        // Never offered: a version the game list records as broken here.
        let bad = gamedb::GameDb::load()
            .lookup(target.app_id.as_deref(), &target.process)
            .map(|e| e.bad_optiscaler.clone())
            .unwrap_or_default();
        versions::Offer {
            available: versions::offer(
                &m.source.version,
                &cfg.version,
                cfg.skipped_update.as_deref(),
                &bad,
                &known,
            ),
            installed: m.source.version.clone(),
            previous: m.previous.map(|p| p.version),
        }
    })
}

/// What [`restore`] did.
#[derive(Debug, Clone, Default)]
pub struct Restoration {
    /// What happened to each file.
    pub files: Vec<transaction::FileOutcome>,
    /// What happened to each of the game's own settings Apply changed.
    pub settings: Vec<ingame::Restored>,
    /// Why the game's own settings could not be put back. The files were;
    /// the record keeps the settings, and Restore tries them again.
    pub settings_error: Option<text::Text>,
}

/// Remove everything Big Game Mode placed in `target`, restoring originals —
/// files, and the game's own settings Apply switched on — then the cached
/// releases no game uses any more. The files are put back even when the
/// settings cannot be (the prefix stays in use): that is said apart, and the
/// settings stay recorded for the next Restore. A prefix that is gone has no
/// settings to put back.
///
/// # Errors
/// Returns an error if the game is running, nothing is installed, the record
/// is not this game's folder's, or a file cannot be restored.
pub fn restore(target: &Target) -> anyhow::Result<Restoration> {
    let state = state_dir();
    let done = locked(&state, target, || restore_in(&state, target));
    tidy_cache(&state, &optiscaler::cache_dir());
    done
}

/// The `OptiScaler` versions some game needs from the cache: the one installed
/// and the one Go back returns to, from every manifest, finished or not.
fn versions_in_use(state: &Path) -> Vec<String> {
    let Ok(dir) = std::fs::read_dir(state) else {
        return Vec::new();
    };
    dir.flatten()
        .filter_map(|d| manifest::Manifest::load(state, &d.file_name().to_string_lossy()).ok()?)
        .flat_map(|m| [Some(m.source), m.previous])
        .flatten()
        .filter(|s| s.component == optiscaler::COMPONENT)
        .map(|s| s.version)
        .collect()
}

/// Drop from the download cache what no game needs (see
/// [`optiscaler::prune_cache`]). Run after every change to what is
/// installed; a release fetched in the last day stays for the next Apply.
fn tidy_cache(state: &Path, cache: &Path) {
    let freed = optiscaler::prune_cache(
        cache,
        &versions_in_use(state),
        std::time::Duration::from_secs(24 * 3600),
    );
    if freed > 0 {
        tracing::info!(target: "graphics", freed_bytes = freed,
            "OptiScaler releases no game uses removed from the cache");
    }
}

fn restore_in(state: &Path, target: &Target) -> anyhow::Result<Restoration> {
    ensure_closed(target)?;
    let mut m = installed_manifest(state, target)?;
    let mut done = Restoration::default();
    if !m.settings.is_empty() {
        match ingame::restore(&m.settings) {
            Ok(r) => {
                for outcome in &r {
                    tracing::info!(target: "graphics", game = %target.process, ?outcome,
                        "the game's setting restored");
                }
                done.settings = r;
                m.settings.clear();
            }
            Err(e) => {
                tracing::warn!(target: "graphics", game = %target.process,
                    error = %format!("{e:#}"), "the game's setting could not be put back; kept for the next Restore");
                done.settings_error = Some(crate::error::describe(&e));
            }
        }
    }
    done.files = transaction::rollback(state, &m)?;
    Ok(done)
}

/// [`restore`], as one result: the files' outcomes, or an error that also
/// says when the files were put back and only the game's own setting was
/// not.
///
/// # Errors
/// As [`restore`], and when the game's own settings could not be put back.
pub fn remove(target: &Target) -> anyhow::Result<Vec<transaction::FileOutcome>> {
    let state = state_dir();
    let done = locked(&state, target, || remove_in(&state, target));
    tidy_cache(&state, &optiscaler::cache_dir());
    done
}

fn remove_in(state: &Path, target: &Target) -> anyhow::Result<Vec<transaction::FileOutcome>> {
    let done = restore_in(state, target)?;
    if let Some(why) = done.settings_error {
        return Err(UserError::with(
            N_("the game's files were put back, but its own setting could not be (%s); Restore tries it again"),
            [why],
        )
        .into());
    }
    Ok(done.files)
}

/// Put back files that are missing (a game update, a file check by Steam),
/// from the cache and the configured copy kept while installed — not in a
/// game that shows anti-cheat now, or that the game list blocks: there,
/// Restore is what is left to do.
///
/// # Errors
/// Returns an error if nothing is installed, the game is running or may not
/// be injected into now, or a file cannot be restored.
pub fn repair(target: &Target) -> anyhow::Result<Vec<PathBuf>> {
    let state = state_dir();
    locked(&state, target, || {
        repair_in(
            &state,
            &optiscaler::cache_dir(),
            &gamedb::GameDb::load(),
            target,
        )
    })
}

fn repair_in(
    state: &Path,
    cache: &Path,
    db: &gamedb::GameDb,
    target: &Target,
) -> anyhow::Result<Vec<PathBuf>> {
    ensure_closed(target)?;
    let key = target.key();
    let m = installed_manifest(state, target)?;
    if let Some(veto) = injection_veto(target, db) {
        return Err(UserError::plain(N_(
            "Repair puts nothing back in this game now; Restore Game Graphics removes what Big Game Mode placed",
        ))
        .caused_by(veto)
        .into());
    }
    // The files of the version that is installed, not of whatever version a
    // profile would get today: their hashes are what the manifest checks.
    let cached = optiscaler::fetch(cache, &versions::for_installed(cache, &m.source)?)?;
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
                // The release spells its files its own way; the game's copy
                // may be in another case.
                cached
                    .files
                    .keys()
                    .find(|f| f.eq_ignore_ascii_case(&name))
                    .map_or_else(|| cached.dir.join(&name), |f| cached.dir.join(f))
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

    #[test]
    fn selected_configured_loaded_and_active_are_told_apart() {
        let base = ChoiceFacts {
            blocked: false,
            pending_changes: false,
            installed: None,
            needs_files: true,
            launch_option_only: false,
            launch_option_set: false,
            provider_loaded: None,
            upgrade_env: None,
        };
        assert_eq!(choice_state_of(base), ChoiceState::Selected);
        let installed = |stage| ChoiceFacts {
            installed: Some(stage),
            ..base
        };
        assert_eq!(
            choice_state_of(installed(RuntimeStage::Waiting)),
            ChoiceState::Configured
        );
        assert_eq!(
            choice_state_of(installed(RuntimeStage::Loaded)),
            ChoiceState::Loaded
        );
        assert_eq!(
            choice_state_of(installed(RuntimeStage::Active)),
            ChoiceState::Active
        );
        assert_eq!(
            choice_state_of(installed(RuntimeStage::Failed)),
            ChoiceState::Failed
        );
        // A changed choice over an install is pending, whatever runs.
        let pending = ChoiceFacts {
            pending_changes: true,
            ..installed(RuntimeStage::Active)
        };
        assert_eq!(choice_state_of(pending), ChoiceState::Selected);
        // Installed, and the choice needs none of it: Restore, not Apply.
        let not_needed = ChoiceFacts {
            needs_files: false,
            ..installed(RuntimeStage::Active)
        };
        assert_eq!(choice_state_of(not_needed), ChoiceState::NeedsRestore);
        let nothing = ChoiceFacts {
            needs_files: false,
            ..base
        };
        assert_eq!(choice_state_of(nothing), ChoiceState::NothingToApply);
        let blocked = ChoiceFacts {
            blocked: true,
            ..pending
        };
        assert_eq!(choice_state_of(blocked), ChoiceState::Blocked);
    }

    #[test]
    fn the_fsr4_launch_option_is_active_only_when_the_game_shows_it() {
        let option = ChoiceFacts {
            blocked: false,
            pending_changes: false,
            installed: None,
            needs_files: false,
            launch_option_only: true,
            launch_option_set: false,
            provider_loaded: None,
            upgrade_env: None,
        };
        assert_eq!(choice_state_of(option), ChoiceState::Selected);
        let set = ChoiceFacts {
            launch_option_set: true,
            ..option
        };
        assert_eq!(choice_state_of(set), ChoiceState::Configured);
        let running = |loaded, env| ChoiceFacts {
            provider_loaded: Some(loaded),
            upgrade_env: env,
            ..set
        };
        // Both halves of the evidence, or not active.
        assert_eq!(
            choice_state_of(running(true, Some(true))),
            ChoiceState::Active
        );
        assert_eq!(choice_state_of(running(true, None)), ChoiceState::Loaded);
        assert_eq!(
            choice_state_of(running(true, Some(false))),
            ChoiceState::Failed
        );
        assert_eq!(
            choice_state_of(running(false, Some(true))),
            ChoiceState::Failed
        );
    }
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
            game_xess: false,
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

    /// A release in `cache`, unpacked and recorded as `fetch` leaves it:
    /// what install, update and repair read, with no network.
    fn cached_release(cache: &Path, version: &str) -> optiscaler::Release {
        let release = optiscaler::Release {
            tag: format!("v{version}"),
            version: version.into(),
            asset: format!("Optiscaler_{version}.7z"),
            sha256: manifest::sha256_bytes(version.as_bytes()),
            size: 1,
            published: String::new(),
        };
        let dir = cache.join(version).join("files");
        std::fs::create_dir_all(&dir).unwrap();
        let mut files = std::collections::BTreeMap::new();
        for (name, bytes) in [
            ("OptiScaler.dll", format!("MZ optiscaler {version}")),
            (
                "OptiScaler.ini",
                "[Upscalers]\nDx12Upscaler=auto\n\n[Menu]\nScale=auto\n".to_owned(),
            ),
            ("amd_fidelityfx_dx12.dll", format!("MZ ffx {version}")),
            (
                "amd_fidelityfx_upscaler_dx12.dll",
                format!("MZ upscaler {version}"),
            ),
        ] {
            std::fs::write(dir.join(name), &bytes).unwrap();
            files.insert(name.to_owned(), manifest::sha256_bytes(bytes.as_bytes()));
        }
        let c = optiscaler::Cached {
            release: release.clone(),
            dir,
            url: release.url(),
            downloaded_at: 0,
            license: "GPL-3.0".into(),
            files,
        };
        std::fs::write(
            cache.join(version).join("release.json"),
            serde_json::to_string(&c).unwrap(),
        )
        .unwrap();
        release
    }

    /// A Steam game in `dir`, with its executable.
    fn steam_game(dir: &Path, folder: &str) -> Target {
        let root = dir.join(folder);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("Game.exe"), b"MZ game").unwrap();
        Target {
            name: "Game".into(),
            process: "Game.exe".into(),
            app_id: Some("4242".into()),
            install_root: root,
        }
    }

    fn optiscaler_plan() -> plan::Plan {
        use crate::graphics::optiscaler::{Api, FrameGen, Input, Options, Output};
        plan::Plan {
            standing: plan::Standing::Recommended,
            backend: backend::Backend::OptiScaler,
            frame_generation: plan::FrameGenPlan::Off,
            native_action: None,
            summary: text::Text::raw(""),
            steps: vec![],
            optiscaler: Some(Options {
                proxy: "dxgi.dll".into(),
                api: Api::Dx12,
                input: Input::Xess,
                output: Output::Fsr,
                frame_gen: FrameGen::Off,
                nvidia: false,
                dlss: false,
                watermark: false,
                game_xess: false,
            }),
            files: vec![],
            disable: vec![],
            problems: vec![],
        }
    }

    struct Setup {
        _dir: tempfile::TempDir,
        state: PathBuf,
        cache: PathBuf,
        db: gamedb::GameDb,
        game: Target,
        v1: optiscaler::Release,
    }

    /// A game with release 1.0.0 installed in it.
    fn installed_game() -> Setup {
        let dir = tempfile::tempdir().unwrap();
        let (state, cache) = (dir.path().join("state"), dir.path().join("cache"));
        let v1 = cached_release(&cache, "1.0.0");
        let game = steam_game(dir.path(), "library/Game");
        let db = gamedb::GameDb::from_texts(None);
        install_in(
            &state,
            &cache,
            &db,
            &game,
            &optiscaler_plan(),
            &config::VersionPolicy::Pinned("1.0.0".into()),
            &[],
        )
        .unwrap();
        assert!(game.install_root.join("dxgi.dll").is_file());
        Setup {
            _dir: dir,
            state,
            cache,
            db,
            game,
            v1,
        }
    }

    #[test]
    fn two_applies_at_once_never_take_ours_for_the_games_original() {
        let dir = tempfile::tempdir().unwrap();
        let (state, cache) = (dir.path().join("state"), dir.path().join("cache"));
        cached_release(&cache, "1.0.0");
        let game = steam_game(dir.path(), "library/Game");
        std::fs::write(game.install_root.join("dxgi.dll"), b"the game's own").unwrap();
        let db = gamedb::GameDb::from_texts(None);
        let plan = optiscaler_plan();
        let version = config::VersionPolicy::Pinned("1.0.0".into());
        // A double click: two Apply runs of one game, side by side.
        let done: Vec<bool> = std::thread::scope(|s| {
            let runs: Vec<_> = (0..2)
                .map(|_| {
                    s.spawn(|| {
                        locked(&state, &game, || {
                            install_in(&state, &cache, &db, &game, &plan, &version, &[])
                        })
                        .is_ok()
                    })
                })
                .collect();
            runs.into_iter().map(|r| r.join().unwrap()).collect()
        });
        // The second waits, finds the first's install and is refused.
        assert_eq!(done.iter().filter(|ok| **ok).count(), 1, "{done:?}");
        restore_in(&state, &game).unwrap();
        assert_eq!(
            std::fs::read(game.install_root.join("dxgi.dll")).unwrap(),
            b"the game's own"
        );
        assert!(
            manifest::Manifest::load(&state, &game.key())
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn a_game_moved_to_another_library_is_restored_where_it_is_now() {
        let s = installed_game();
        let moved = Target {
            install_root: s.game.install_root.with_file_name("Moved"),
            ..s.game.clone()
        };
        std::fs::rename(&s.game.install_root, &moved.install_root).unwrap();
        let done = restore_in(&s.state, &moved).unwrap();
        assert!(
            done.files
                .iter()
                .all(|o| matches!(o, transaction::FileOutcome::Removed(_)))
        );
        assert!(!moved.install_root.join("dxgi.dll").exists());
        assert!(!s.game.install_root.exists(), "nothing made where it was");
        assert!(
            manifest::Manifest::load(&s.state, &moved.key())
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn a_record_for_another_folder_that_is_still_there_changes_nothing() {
        let s = installed_game();
        // A copy of the game elsewhere, or a record naming a folder that is
        // not this game's: neither is touched.
        let other = Target {
            install_root: s.game.install_root.with_file_name("Copy"),
            ..s.game.clone()
        };
        std::fs::create_dir_all(&other.install_root).unwrap();
        std::fs::write(other.install_root.join("dxgi.dll"), b"the copy's own").unwrap();
        let err = restore_in(&s.state, &other).unwrap_err();
        assert!(
            format!("{err:#}").contains("not this game's folder"),
            "{err:#}"
        );
        assert!(repair_in(&s.state, &s.cache, &s.db, &other).is_err());
        assert!(s.game.install_root.join("dxgi.dll").is_file());
        assert_eq!(
            std::fs::read(other.install_root.join("dxgi.dll")).unwrap(),
            b"the copy's own"
        );
        let r = record_in(&s.state, &other.key(), &other.install_root);
        assert!(matches!(
            unusable_status(&r),
            Some(runtime::Status::Moved { installed_in }) if installed_in == s.game.install_root
        ));
    }

    #[test]
    fn a_prefix_that_is_gone_does_not_stop_restore() {
        let s = installed_game();
        let key = s.game.key();
        let mut m = manifest::Manifest::load(&s.state, &key).unwrap().unwrap();
        m.settings = vec![ingame::SettingChange {
            file: s.state.with_file_name("compatdata/4242/pfx/user.reg"),
            key: r"Software\Game".into(),
            value: "XESS".into(),
            set: 3,
            original: Some(0),
        }];
        m.save(&s.state).unwrap();
        let done = restore_in(&s.state, &s.game).unwrap();
        assert_eq!(done.settings, [ingame::Restored::Gone("XESS".into())]);
        assert!(done.settings_error.is_none());
        assert!(!s.game.install_root.join("dxgi.dll").exists());
        assert!(manifest::Manifest::load(&s.state, &key).unwrap().is_none());
    }

    #[test]
    fn files_are_put_back_even_when_the_games_setting_cannot_be_yet() {
        let s = installed_game();
        let key = s.game.key();
        let prefix = s.state.with_file_name("pfx");
        let reg = prefix.join("user.reg");
        // Unreadable as a registry: a folder where the file should be.
        std::fs::create_dir_all(&reg).unwrap();
        let mut m = manifest::Manifest::load(&s.state, &key).unwrap().unwrap();
        m.settings = vec![ingame::SettingChange {
            file: reg.clone(),
            key: r"Software\Game".into(),
            value: "XESS".into(),
            set: 3,
            original: Some(0),
        }];
        m.save(&s.state).unwrap();
        let done = restore_in(&s.state, &s.game).unwrap();
        assert!(done.settings_error.is_some());
        assert!(
            !s.game.install_root.join("dxgi.dll").exists(),
            "files put back"
        );
        let left = manifest::Manifest::load(&s.state, &key).unwrap().unwrap();
        assert!(left.settings_only(), "the setting keeps its record");
        assert_eq!(
            runtime::status(Some(&left), None, &|_| None, &|_, _| None),
            runtime::Status::SettingsLeft
        );
        // The one-result form says so instead of claiming success.
        let err = remove_in(&s.state, &s.game).unwrap_err();
        assert!(
            format!("{err:#}").contains("files were put back"),
            "{err:#}"
        );
        // Once the registry can be written, Restore finishes.
        std::fs::remove_dir(&reg).unwrap();
        std::fs::write(
            &reg,
            "WINE REGISTRY Version 2\n\n[Software\\\\Game] 1\n\"XESS\"=dword:00000003\n",
        )
        .unwrap();
        let done = restore_in(&s.state, &s.game).unwrap();
        assert_eq!(done.settings, [ingame::Restored::PutBack("XESS".into())]);
        assert!(
            std::fs::read_to_string(&reg)
                .unwrap()
                .contains("\"XESS\"=dword:00000000")
        );
        assert!(manifest::Manifest::load(&s.state, &key).unwrap().is_none());
    }

    #[test]
    fn repair_puts_nothing_back_in_a_game_that_has_anti_cheat_now() {
        let s = installed_game();
        // A game update deletes the proxy and brings Easy Anti-Cheat.
        std::fs::remove_file(s.game.install_root.join("dxgi.dll")).unwrap();
        std::fs::create_dir(s.game.install_root.join("EasyAntiCheat")).unwrap();
        let err = repair_in(&s.state, &s.cache, &s.db, &s.game).unwrap_err();
        assert!(format!("{err:#}").contains("Easy Anti-Cheat"), "{err:#}");
        assert!(!s.game.install_root.join("dxgi.dll").exists());
        // Listed as blocked: the same.
        std::fs::remove_dir(s.game.install_root.join("EasyAntiCheat")).unwrap();
        let blocked = gamedb::GameDb::from_texts(Some(
            "[[game]]\nsteam_app_id = \"4242\"\nblock = \"crashes\"\n",
        ));
        assert!(repair_in(&s.state, &s.cache, &blocked, &s.game).is_err());
        assert!(!s.game.install_root.join("dxgi.dll").exists());
        // Without either, Repair does its job.
        assert_eq!(
            repair_in(&s.state, &s.cache, &s.db, &s.game).unwrap(),
            [PathBuf::from("dxgi.dll")]
        );
    }

    #[test]
    fn an_update_keeps_what_the_user_changed_in_optiscalers_overlay() {
        let s = installed_game();
        let v2 = cached_release(&s.cache, "2.0.0");
        // OptiScaler saves the ini in its own layout, with the user's change.
        let ini = s.game.install_root.join("OptiScaler.ini");
        let text = std::fs::read_to_string(&ini).unwrap();
        std::fs::write(
            &ini,
            text.replace("Scale=auto", "Scale = 1.5")
                .replace("Dx12Upscaler=fsr31", "Dx12Upscaler = xess"),
        )
        .unwrap();
        let plan = optiscaler_plan();
        let m = update_in(&s.state, &s.cache, &s.db, &s.game, &plan, &v2).unwrap();
        assert_eq!(m.source.version, "2.0.0");
        assert_eq!(
            m.previous.as_ref().map(|p| p.version.as_str()),
            Some("1.0.0")
        );
        let now = std::fs::read_to_string(&ini).unwrap();
        assert_eq!(
            optiscaler::get_ini(&now, "Menu", "Scale").as_deref(),
            Some("1.5")
        );
        assert_eq!(
            optiscaler::get_ini(&now, "Upscalers", "Dx12Upscaler").as_deref(),
            Some("fsr31"),
            "the choice's own setting is the choice's"
        );
        assert_eq!(
            std::fs::read(s.game.install_root.join("dxgi.dll")).unwrap(),
            b"MZ optiscaler 2.0.0"
        );
        // The staged copy is what was installed: nothing is pending.
        assert!(!pending_in(&s.state, &s.game.key(), &m, &plan));
    }

    #[test]
    fn a_refused_apply_leaves_the_staged_copy_of_what_is_installed() {
        let s = installed_game();
        let key = s.game.key();
        let staged = s.state.join(&key).join("staging").join("OptiScaler.ini");
        let before = std::fs::read_to_string(&staged).unwrap();
        let cached = optiscaler::fetch(&s.cache, &s.v1).unwrap();
        let mut other = optiscaler_plan().optiscaler.unwrap();
        other.output = optiscaler::Output::Xess;
        // Something is installed: the transaction refuses.
        assert!(apply_release(&s.state, &s.game, &other, &cached, Path::new(""), &[]).is_err());
        assert_eq!(std::fs::read_to_string(&staged).unwrap(), before);
        let left: Vec<_> = std::fs::read_dir(s.state.join(&key))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with("staging-"))
            .collect();
        assert!(left.is_empty(), "{left:?}");
    }

    #[test]
    fn a_game_running_under_any_name_is_found_by_what_it_maps() {
        let dir = tempfile::tempdir().unwrap();
        let game = dir.path().join("Game Folder");
        std::fs::create_dir(&game).unwrap();
        let proc = dir.path().join("proc");
        let dll = format!(
            "7f00-7f10 r-xp 0 08:01 1 {}/dxgi.dll\n",
            std::fs::canonicalize(&game).unwrap().display()
        );
        for (pid, cmdline, maps) in [
            (
                "17",
                "firefox\0",
                "7f00-7f10 r-xp 0 08:01 1 /usr/lib/libc.so.6\n",
            ),
            // A native program with the folder open is not a game under Wine.
            ("18", "/usr/bin/file-roller\0", dll.as_str()),
            (
                "4321",
                "Z:\\games\\Game Folder\\Renamed.EXE\0",
                dll.as_str(),
            ),
        ] {
            std::fs::create_dir_all(proc.join(pid)).unwrap();
            std::fs::write(proc.join(pid).join("cmdline"), cmdline).unwrap();
            std::fs::write(proc.join(pid).join("maps"), maps).unwrap();
        }
        std::fs::create_dir_all(proc.join("self")).unwrap();
        assert_eq!(mapping_process(&proc, &game), Some(4321));
        assert_eq!(mapping_process(&proc, &dir.path().join("other")), None);
    }

    #[test]
    fn a_record_that_does_not_load_still_protects_the_launch_and_blocks_apply() {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("state");
        let p = manifest::Manifest::path(&state, "steam-7");
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        // Written by a newer Big Game Mode.
        std::fs::write(
            &p,
            r#"{"schema":99,"game_key":"steam-7","process":"Game.exe","install_root":"/g","source":{"component":"optiscaler","version":"1","url":null,"archive_sha256":null},"started_at":0,"state":"installed","entries":[]}"#,
        )
        .unwrap();
        let settings = dir.path().join("settings");
        assert_eq!(
            launch_disables(&state, &settings, "game.exe"),
            [rules::Tech::GamescopeUpscaling, rules::Tech::WineFsr]
        );
        assert!(installed_processes(&state).contains("game.exe"));
        let r = record_in(&state, "steam-7", Path::new("/g"));
        assert!(matches!(
            unusable_status(&r),
            Some(runtime::Status::Unreadable { .. })
        ));
    }
}
