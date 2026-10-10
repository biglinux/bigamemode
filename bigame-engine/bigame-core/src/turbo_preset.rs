//! Turbo presets: one choice for what the games started while Turbo is on
//! should favour. It is made before Turbo is switched on (for when it is),
//! or while it is on ([`switch`]), from Home or the tray alike.
//!
//! A preset is a layer on top of Tuning, never a second copy of it. While
//! Turbo is on, the running session's environment carries the preset's
//! variables; switching Turbo off takes them away and puts back what Tuning
//! says. Tuning's own settings and files are never changed by a preset.
//!
//! Every lever is one the games really read, checked on the reference
//! desktop:
//!
//! - a frame cap: DXVK reads `DXVK_CONFIG` (`dxgi.maxFrameRate` for D3D10/11,
//!   `d3d9.maxFrameRate`), VKD3D-Proton reads `VKD3D_FRAME_RATE` (D3D12) —
//!   both in Proton Experimental's DLLs; Gamescope takes `-r` for games
//!   Big Game Mode starts itself. A native Linux game reads none of these;
//! - Wine FSR (`WINE_FULLSCREEN_FSR`): Wine scales a game running in
//!   exclusive fullscreen below the display's resolution;
//! - vkBasalt (`ENABLE_VKBASALT`) with Big Game Mode's own CAS-only file
//!   (`VKBASALT_CONFIG_FILE`, [`cas_config_path`]): the user's
//!   `vkBasalt.conf` may hold any effect, and "sharper image" has to mean
//!   sharpening;
//! - `FSR4_UPGRADE=1` (Valve's Proton) and `PROTON_FSR4_UPGRADE=1` (GE-Proton,
//!   which Heroic games often run; it also fetches AMD's provider): Proton
//!   hands a game's FSR 3.1 to AMD's FSR 4, on a GPU that runs FSR 4
//!   ([`crate::graphics::fsr4_upgrade`]).
//!
//! The variables go to the running `systemd --user` manager only, not to
//! `environment.d`: what that file holds is set by systemd's generator at
//! login and cannot be unset later (systemd 261), and a preset must go away
//! completely when Turbo does. Big Game Mode puts them back when it starts with
//! Turbo on ([`resync`]).
//!
//! The user may have set a preset's variables too (a `DXVK_CONFIG` of their
//! own, `PROTON_FSR4_UPGRADE=1` in their `environment.d`). Their values in
//! the session before the first preset went in are kept in the record of the
//! preset in force ([`Layer::before`]): a frame cap is merged into their
//! `DXVK_CONFIG`, a key the preset does not set keeps their value, and taking
//! the preset away puts their values back rather than unsetting them. The
//! record goes only once the session reads back without the preset.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::text::N_;

/// What the games should favour while Turbo is on.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Preset {
    /// Tuning as it is: the preset adds nothing.
    #[default]
    Standard,
    /// No frame cap, Wine FSR on, no filter: every frame the GPU can give.
    MoreFps,
    /// A 60 FPS cap: steady frame times, less heat and noise.
    Locked60,
    /// Sharper image at native resolution, FSR 4 where the GPU has it, and
    /// a 60 FPS cap so the heavier image still plays steadily.
    Enhanced,
}

/// Every preset, in the order they are offered.
pub const ALL: [Preset; 4] = [
    Preset::Standard,
    Preset::MoreFps,
    Preset::Locked60,
    Preset::Enhanced,
];

impl Preset {
    /// Its name, translatable.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Standard => N_("Standard"),
            Self::MoreFps => N_("More FPS"),
            Self::Locked60 => N_("Locked 60 FPS"),
            Self::Enhanced => N_("Enhanced graphics"),
        }
    }

    /// What it does, in a sentence, translatable.
    #[must_use]
    pub fn description(self) -> &'static str {
        match self {
            Self::Standard => N_("Games run with Tuning as it is; the preset changes nothing."),
            Self::MoreFps => N_(
                "No frame cap and no image filter. Wine FSR scales a game you run in fullscreen below the display's resolution: lowering the resolution in the game is where the frames come from.",
            ),
            Self::Locked60 => N_(
                "Proton games are capped at 60 FPS: steady frame times, a cooler and quieter GPU. Wine FSR stays available to hold 60 at a lower resolution.",
            ),
            Self::Enhanced => N_(
                "Sharper image (vkBasalt's CAS) at the display's resolution, FSR 4 for games with FSR 3.1 on a GPU that has it, and a 60 FPS cap so the heavier image plays steadily.",
            ),
        }
    }

    /// The id stored on disk.
    #[must_use]
    pub fn id(self) -> &'static str {
        match self {
            Self::Standard => "standard",
            Self::MoreFps => "more_fps",
            Self::Locked60 => "locked60",
            Self::Enhanced => "enhanced",
        }
    }

    /// The preset stored as `id`.
    #[must_use]
    pub fn from_id(id: &str) -> Option<Self> {
        ALL.into_iter().find(|p| p.id() == id)
    }
}

/// What this machine lets a preset use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Machine {
    /// vkBasalt's layer is installed.
    pub vkbasalt: bool,
    /// The GPU games render on runs FSR 4.
    pub fsr4: bool,
}

impl Machine {
    /// Read it.
    #[must_use]
    pub fn detect() -> Self {
        let hw = crate::hardware::Hardware::detect();
        Self {
            vkbasalt: crate::capabilities::vkbasalt_installed(),
            fsr4: crate::graphics::report::render_gpu(&hw)
                .is_some_and(|g| crate::graphics::report::GpuInfo::fsr4(&g)),
        }
    }
}

/// What a preset asks for. `None` leaves Tuning's value.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Levers {
    /// Frames per second to cap Proton games at; `Some(0)` removes a cap.
    pub frame_cap: Option<u32>,
    /// Wine FSR on or off.
    pub wine_fsr: Option<bool>,
    /// vkBasalt on or off.
    pub vkbasalt: Option<bool>,
    /// FSR 3.1 handed to FSR 4.
    pub fsr4_upgrade: bool,
}

/// The levers `preset` pulls on `machine`: one the machine lacks is left
/// alone rather than asked for.
#[must_use]
pub fn levers(preset: Preset, machine: Machine) -> Levers {
    match preset {
        Preset::Standard => Levers::default(),
        Preset::MoreFps => Levers {
            frame_cap: Some(0),
            wine_fsr: Some(true),
            vkbasalt: Some(false),
            fsr4_upgrade: false,
        },
        Preset::Locked60 => Levers {
            frame_cap: Some(60),
            wine_fsr: Some(true),
            vkbasalt: Some(false),
            fsr4_upgrade: false,
        },
        Preset::Enhanced => Levers {
            frame_cap: Some(60),
            wine_fsr: Some(false),
            vkbasalt: machine.vkbasalt.then_some(true),
            fsr4_upgrade: machine.fsr4,
        },
    }
}

/// The variables only a preset sets among Big Game Mode's; Big Game Mode never
/// puts them in `environment.d`, though the user may have.
pub const PRESET_KEYS: &[&str] = &[
    "DXVK_CONFIG",
    "DXVK_FRAME_RATE",
    "VKD3D_FRAME_RATE",
    "FSR4_UPGRADE",
    "PROTON_FSR4_UPGRADE",
];

/// DXVK's options that cap the frame rate.
const DXVK_CAPS: [&str; 2] = ["dxgi.maxFrameRate", "d3d9.maxFrameRate"];

/// `DXVK_CONFIG` with a cap of `fps`, as a preset alone writes it.
fn dxvk_cap(fps: u32) -> String {
    format!("dxgi.maxFrameRate = {fps}; d3d9.maxFrameRate = {fps}")
}

/// The user's `DXVK_CONFIG` without its frame caps, then a cap of `fps`
/// (none for `0`). Their other options (a vendor id, a HUD) stay: a preset
/// changes the cap, not the rest of the file.
#[must_use]
pub fn merge_dxvk_config(user: Option<&str>, fps: u32) -> String {
    let mut options: Vec<String> = user
        .unwrap_or_default()
        .split(';')
        .map(str::trim)
        .filter(|o| !o.is_empty())
        .filter(|o| {
            let key = o.split_once('=').map_or(*o, |(k, _)| k).trim();
            !DXVK_CAPS.contains(&key)
        })
        .map(str::to_owned)
        .collect();
    if fps > 0 {
        options.push(dxvk_cap(fps));
    }
    options.join("; ")
}

/// The variables a preset sets for `levers` in a game Big Game Mode starts:
/// [`PRESET_KEYS`], and vkBasalt's CAS-only file when it turns vkBasalt on.
#[must_use]
pub fn preset_env(levers: Levers) -> HashMap<String, String> {
    let mut env = HashMap::new();
    if let Some(fps) = levers.frame_cap.filter(|f| *f > 0) {
        env.insert("DXVK_CONFIG".into(), dxvk_cap(fps));
        // DXVK before 2.3 reads only this name.
        env.insert("DXVK_FRAME_RATE".into(), fps.to_string());
        env.insert("VKD3D_FRAME_RATE".into(), fps.to_string());
    }
    if levers.fsr4_upgrade {
        env.insert("FSR4_UPGRADE".into(), "1".into());
        // GE-Proton reads only its own name, and then fetches the provider.
        env.insert("PROTON_FSR4_UPGRADE".into(), "1".into());
    }
    if levers.vkbasalt == Some(true) {
        env.insert(
            "VKBASALT_CONFIG_FILE".into(),
            cas_config_path().to_string_lossy().into_owned(),
        );
    }
    env
}

/// [`PRESET_KEYS`] as the session holds them under `levers`, over the
/// user's own values `before`: the cap merged into their `DXVK_CONFIG`, a
/// key the preset leaves alone at their value. "No cap" keeps their other
/// DXVK options and turns VKD3D-Proton's limiter off with `0` (one of
/// theirs from `environment.d` cannot be unset from the running session).
fn preset_keys_over(levers: Levers, before: &BTreeMap<String, String>) -> HashMap<String, String> {
    let mut env: HashMap<String, String> = before
        .iter()
        .filter(|(k, _)| PRESET_KEYS.contains(&k.as_str()))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    match levers.frame_cap {
        Some(0) => {
            if let Some(user) = before.get("DXVK_CONFIG") {
                env.insert("DXVK_CONFIG".into(), merge_dxvk_config(Some(user), 0));
            }
            for limiter in ["DXVK_FRAME_RATE", "VKD3D_FRAME_RATE"] {
                if before.contains_key(limiter) {
                    env.insert(limiter.into(), "0".into());
                }
            }
        }
        Some(fps) => {
            env.insert(
                "DXVK_CONFIG".into(),
                merge_dxvk_config(before.get("DXVK_CONFIG").map(String::as_str), fps),
            );
            env.insert("DXVK_FRAME_RATE".into(), fps.to_string());
            env.insert("VKD3D_FRAME_RATE".into(), fps.to_string());
        }
        None => {}
    }
    if levers.fsr4_upgrade {
        env.insert("FSR4_UPGRADE".into(), "1".into());
        env.insert("PROTON_FSR4_UPGRADE".into(), "1".into());
    }
    env
}

/// `env` (Tuning's session variables) with `levers` laid over it.
pub fn overlay<S: std::hash::BuildHasher>(env: &mut HashMap<String, String, S>, levers: Levers) {
    overlay_over(env, levers, &BTreeMap::new());
}

/// [`overlay`], keeping the user's own values `before` of [`PRESET_KEYS`]
/// ([`Layer::before`]).
pub fn overlay_over<S: std::hash::BuildHasher>(
    env: &mut HashMap<String, String, S>,
    levers: Levers,
    before: &BTreeMap<String, String>,
) {
    env.extend(preset_keys_over(levers, before));
    match levers.wine_fsr {
        Some(true) => {
            env.insert("WINE_FULLSCREEN_FSR".into(), "1".into());
        }
        Some(false) => {
            env.remove("WINE_FULLSCREEN_FSR");
            env.remove("WINE_FULLSCREEN_FSR_MODE");
        }
        None => {}
    }
    match levers.vkbasalt {
        Some(true) => {
            env.insert("ENABLE_VKBASALT".into(), "1".into());
            env.insert(
                "VKBASALT_CONFIG_FILE".into(),
                cas_config_path().to_string_lossy().into_owned(),
            );
        }
        Some(false) => {
            env.remove("ENABLE_VKBASALT");
            env.remove("VKBASALT_CONFIG_FILE");
        }
        None => {}
    }
}

// ── vkBasalt's sharpening ───────────────────────────────────────────────────

/// Big Game Mode's own vkBasalt file for "Enhanced graphics": CAS and nothing
/// else. Never the user's `~/.config/vkBasalt/vkBasalt.conf`, which can hold
/// any look (or the Nara Linux style, whose shaders may be missing).
#[must_use]
pub fn cas_config_path() -> PathBuf {
    crate::paths::config_home()
        .join("bigame-mode")
        .join("vkbasalt-cas.conf")
}

/// Write [`cas_config_path`]'s file when `levers` turn vkBasalt on. vkBasalt
/// falls back to the user's own file when the one named is missing, so it
/// is written again whenever the session is synced.
///
/// # Errors
/// Returns an error when the file cannot be written.
pub fn prepare(levers: Levers) -> Result<()> {
    if levers.vkbasalt != Some(true) {
        return Ok(());
    }
    write_cas_config(&cas_config_path())
}

fn write_cas_config(path: &Path) -> Result<()> {
    let text = crate::vkbasalt::style_config(crate::vkbasalt::Style::Cas, Path::new(""))
        .context("vkBasalt's CAS style")?;
    if std::fs::read_to_string(path).is_ok_and(|t| t == text) {
        return Ok(());
    }
    write_atomic(path, &text)
}

/// Replace `path` with `text` in one rename: a reader sees the old file or
/// the new one, never half of one.
fn write_atomic(path: &Path, text: &str) -> Result<()> {
    use std::io::Write as _;
    let dir = path.parent().context("path has no parent directory")?;
    std::fs::create_dir_all(dir).with_context(|| format!("create {}", dir.display()))?;
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let tmp = dir.join(format!(".{name}.{}", std::process::id()));
    let written = (|| {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(text.as_bytes())?;
        f.sync_all()?;
        std::fs::rename(&tmp, path)
    })();
    if let Err(e) = written {
        if let Err(cleanup) = std::fs::remove_file(&tmp)
            && cleanup.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!(file = %tmp.display(), error = %cleanup, "could not remove a temporary file");
        }
        return Err(e).with_context(|| format!("write {}", path.display()));
    }
    Ok(())
}

// ── Frame generation under a cap ────────────────────────────────────────────

/// The games that generate frames, by which frames a DXVK or VKD3D-Proton
/// frame cap holds in them ([`crate::optimization::CappedFrames`]).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FrameGenerators {
    /// The cap counts the frames shown: `OptiScaler`'s generation runs in
    /// the game, above the cap, so under 60 FPS the game renders about 30
    /// (Shadow of the Tomb Raider: 62.5 → 30.0 in its own benchmark, on the
    /// reference desktop).
    pub shown: Vec<String>,
    /// The cap holds the frames rendered, with lsfg-vk's multiplier: lsfg-vk
    /// is a Vulkan layer under the limiter and multiplies what is shown (not
    /// measured here).
    pub rendered: Vec<(String, u32)>,
}

impl FrameGenerators {
    /// Whether no game generates frames.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.shown.is_empty() && self.rendered.is_empty()
    }
}

/// The games that generate frames on this machine ([`FrameGenerators`]):
/// every game lsfg-vk generates for while its general switch is on, and
/// every game whose AI Graphics has `OptiScaler`'s frame generation, chosen
/// or switched on later from `OptiScaler`'s overlay — each told apart as a
/// launch tells it ([`crate::optimization::capped_frames`]).
#[must_use]
pub fn frame_generation_games() -> FrameGenerators {
    let lsfg_conf = crate::optimization::lsfg_general_on(&crate::video_config::load())
        .then(|| std::fs::read_to_string(crate::fg::config_path()).unwrap_or_default());
    let (optiscaler, lsfg) = frame_generators_in(
        &crate::graphics::state_dir(),
        &crate::game_settings::dir(),
        lsfg_conf.as_deref(),
    );
    classify(
        optiscaler.into_iter().chain(lsfg),
        crate::optimization::capped_frames,
    )
}

/// `games` sorted by what a cap holds in each (`capped`); one that does
/// not generate after all is left out.
fn classify(
    games: impl IntoIterator<Item = String>,
    capped: impl Fn(&str) -> Option<crate::optimization::CappedFrames>,
) -> FrameGenerators {
    use crate::optimization::CappedFrames;
    let mut found = FrameGenerators::default();
    for game in games {
        match capped(&game) {
            Some(CappedFrames::Shown) => found.shown.push(game),
            Some(CappedFrames::Rendered { multiplier }) => found.rendered.push((game, multiplier)),
            None => {}
        }
    }
    found.shown.sort();
    found.shown.dedup();
    found.rendered.sort();
    found.rendered.dedup();
    found
}

/// The games that may generate frames, from AI Graphics' manifests in
/// `state`, the per-game settings in `settings`, and lsfg-vk's file (`None`
/// while its general switch is off): those with `OptiScaler`'s generation,
/// then lsfg-vk's others.
fn frame_generators_in(
    state: &Path,
    settings: &Path,
    lsfg_conf: Option<&str>,
) -> (Vec<String>, Vec<String>) {
    let mut optiscaler: Vec<String> = Vec::new();
    if let Ok(dir) = std::fs::read_dir(settings) {
        for entry in dir.flatten() {
            let path = entry.path();
            let Some(name) = path
                .file_name()
                .and_then(|n| n.to_str())
                .and_then(|n| n.strip_suffix(".toml"))
            else {
                continue;
            };
            if crate::game_settings::load_from(settings, name)
                .is_ok_and(|s| s.ai_graphics.optiscaler_frame_generation())
            {
                optiscaler.push(name.to_owned());
            }
        }
    }
    // Switched on from OptiScaler's overlay: the ini in the game says so,
    // which is what a launch reads too (it turns lsfg-vk off for them).
    for process in crate::graphics::installed_processes(state) {
        let generates = crate::graphics::launch_disables(state, settings, &process)
            .contains(&crate::graphics::rules::Tech::LsfgVk);
        if generates && !optiscaler.iter().any(|g| g.eq_ignore_ascii_case(&process)) {
            optiscaler.push(process);
        }
    }
    let mut lsfg = lsfg_conf.map(lsfg_generating).unwrap_or_default();
    // A game with OptiScaler's generation runs with lsfg-vk off.
    lsfg.retain(|g| !optiscaler.iter().any(|o| o.eq_ignore_ascii_case(g)));
    for list in [&mut optiscaler, &mut lsfg] {
        list.sort();
        list.dedup();
    }
    (optiscaler, lsfg)
}

/// The executables lsfg-vk's configuration generates frames for. `proton`
/// is not a game's process (Proton's launcher script), so it is left out.
/// lsfg-vk's file in either version's layout (1.x `[[game]]`, 2.x
/// `[[profile]]`), so a package update does not hide a generating game.
fn lsfg_generating(conf: &str) -> Vec<String> {
    conf.parse::<toml::Table>()
        .map_or_else(|_| Vec::new(), |t| crate::fg::generating_in(&t))
}

// ── What was chosen, and what is in force ───────────────────────────────────

#[derive(Debug, Default, Serialize, Deserialize)]
struct Stored {
    #[serde(default)]
    preset: Preset,
    /// In the record of the preset in force: the user's own values of
    /// [`PRESET_KEYS`] in the session before the first preset went in.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    before: BTreeMap<String, String>,
}

fn chosen_path() -> PathBuf {
    crate::paths::config_home()
        .join("bigame-mode")
        .join("turbo-preset.toml")
}

fn active_path() -> PathBuf {
    crate::paths::state_home()
        .join("bigame-mode")
        .join("turbo-preset-active.toml")
}

fn read_stored(path: &Path) -> Option<Stored> {
    let text = std::fs::read_to_string(path).ok()?;
    toml::from_str::<Stored>(&text).ok()
}

fn write(path: &Path, stored: &Stored) -> Result<()> {
    let text = toml::to_string(stored).context("serialize the preset")?;
    write_atomic(path, &text)
}

/// The preset chosen for the next time Turbo is switched on.
#[must_use]
pub fn chosen() -> Preset {
    read_stored(&chosen_path()).unwrap_or_default().preset
}

/// Choose the preset for the next time Turbo is switched on.
///
/// # Errors
/// Returns an error when the choice cannot be written.
pub fn set_chosen(preset: Preset) -> Result<()> {
    write(
        &chosen_path(),
        &Stored {
            preset,
            before: BTreeMap::new(),
        },
    )
}

/// The preset in force now: the one Turbo was switched on with, until it is
/// switched off and the session reads back without it.
#[must_use]
pub fn active() -> Preset {
    read_stored(&active_path()).unwrap_or_default().preset
}

/// The levers in force now.
#[must_use]
pub fn active_levers() -> Levers {
    match active() {
        Preset::Standard => Levers::default(),
        preset => levers(preset, Machine::detect()),
    }
}

/// What the session's environment carries over Tuning's for a preset.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Layer {
    /// The preset's levers.
    pub levers: Levers,
    /// The user's own values of [`PRESET_KEYS`] before the first preset.
    pub before: BTreeMap<String, String>,
    /// Whether a preset's record is there, so [`PRESET_KEYS`] are
    /// Big Game Mode's to set and unset. Without one they are the user's alone
    /// and a session sync leaves them as they are.
    pub owns_preset_keys: bool,
}

/// The layer of the preset in force.
#[must_use]
pub fn layer() -> Layer {
    read_stored(&active_path()).map_or_else(Layer::default, |s| Layer {
        levers: match s.preset {
            Preset::Standard => Levers::default(),
            preset => levers(preset, Machine::detect()),
        },
        before: s.before,
        owns_preset_keys: true,
    })
}

/// The user's own values of [`PRESET_KEYS`] in a session environment. A
/// frame cap exactly as a preset writes it (`DXVK_CONFIG` and
/// `VKD3D_FRAME_RATE` together, and `DXVK_FRAME_RATE` at the same rate) is
/// what an earlier build left behind when taking its preset away failed, not
/// the user's: it is not kept as theirs.
fn user_values(session: &HashMap<String, String>) -> BTreeMap<String, String> {
    let leftover_rate = session.get("VKD3D_FRAME_RATE").filter(|fps| {
        fps.parse::<u32>()
            .is_ok_and(|n| session.get("DXVK_CONFIG") == Some(&dxvk_cap(n)))
    });
    let leftover = |key: &str| match (key, leftover_rate) {
        ("DXVK_CONFIG" | "VKD3D_FRAME_RATE", Some(_)) => true,
        ("DXVK_FRAME_RATE", Some(rate)) => session.get(key) == Some(rate),
        _ => false,
    };
    PRESET_KEYS
        .iter()
        .filter(|k| !leftover(k))
        .filter_map(|k| session.get(*k).map(|v| ((*k).to_owned(), v.clone())))
        .collect()
}

/// Put `preset` in force: recorded (with the user's own values of its keys,
/// the first time), and the session's environment brought to Tuning's plus
/// the preset's. Returns what the session holds afterwards, read back.
///
/// # Errors
/// Returns an error when the session cannot be read, the record cannot be
/// written or the session's environment cannot be set. The record stays
/// then, so taking the preset away can still put the user's values back.
pub fn activate(preset: Preset) -> Result<Vec<String>> {
    let path = active_path();
    // Switching between presets keeps what was there before the first one.
    let before = match read_stored(&path) {
        Some(record) => record.before,
        None => user_values(&crate::video_config::session_environment()?),
    };
    write(&path, &Stored { preset, before })?;
    let set = crate::video_config::sync_session_env(&crate::video_config::load())?;
    follow_in_launchers();
    Ok(set)
}

/// Whether a preset's record is there, readable or not. [`active`] reads an
/// unreadable one as Standard, yet its variables may still be in the
/// session and [`deactivate`] must still run to take them away.
#[must_use]
pub fn has_record() -> bool {
    active_path().exists()
}

/// The mark that the launchers still owe the preset's change: an open
/// launcher would write its own copy back over it, so it was not written.
fn launchers_owed_path() -> PathBuf {
    crate::paths::state_home()
        .join("bigame-mode")
        .join("turbo-preset-launchers-owed")
}

/// Steam's launch options and Heroic's settings carry a game's vkBasalt
/// switch, which follows the preset in force: they are brought in line with
/// it. While a launcher is open nothing is written to it (it would write its
/// own copy back), so a Turbo off would leave the preset's vkBasalt in its
/// games for good; the mark [`launchers_owed_path`] keeps that owed, and
/// [`resync`] and the next Turbo switch try again until it is written.
fn follow_in_launchers() {
    follow_in_launchers_at(
        &launchers_owed_path(),
        crate::optimization::refresh_steam_gamescope,
        crate::optimization::refresh_heroic,
    );
}

/// [`follow_in_launchers`] with the mark at `owed` and the launchers
/// refreshed by `steam` and `heroic`. Returns whether every game follows.
/// A game whose settings cannot be written stays owed too: what failed may
/// be fixed by the next try, and the mark goes only once nothing is left.
fn follow_in_launchers_at(
    owed: &Path,
    steam: impl FnOnce() -> Vec<(String, Result<crate::steam_gamescope::Applied>)>,
    heroic: impl FnOnce() -> Vec<(String, Result<crate::heroic_launch::Applied>)>,
) -> bool {
    let mut waiting: Vec<String> = Vec::new();
    let mut failed = false;
    for (game, result) in steam() {
        match result {
            Ok(crate::steam_gamescope::Applied::SteamRunning) => waiting.push(game),
            Ok(_) => {}
            Err(e) => {
                failed = true;
                tracing::warn!(%game, error = %format!("{e:#}"), "Steam's launch options do not follow the Turbo preset");
            }
        }
    }
    for (game, result) in heroic() {
        match result {
            Ok(crate::heroic_launch::Applied::HeroicRunning { .. }) => waiting.push(game),
            Ok(_) => {}
            Err(e) => {
                failed = true;
                tracing::warn!(%game, error = %format!("{e:#}"), "Heroic's settings do not follow the Turbo preset");
            }
        }
    }
    if !waiting.is_empty() {
        tracing::warn!(games = %waiting.join(", "), "a launcher is open: these games follow the Turbo preset once it is closed");
    }
    let done = waiting.is_empty() && !failed;
    let marked = if done {
        remove_if_any(owed)
    } else {
        write_atomic(owed, "")
    };
    if let Err(e) = marked {
        tracing::warn!(error = %format!("{e:#}"), "could not record whether the launchers follow the Turbo preset");
    }
    done
}

/// Take the preset away: the session goes back to Tuning's variables and
/// the user's own values of the preset's keys. The record is removed only
/// once the session reads back that way, so a failure leaves something to
/// retry from ([`resync`], the next Turbo off).
///
/// # Errors
/// Returns an error when the session's environment cannot be set or the
/// record cannot be removed.
pub fn deactivate() -> Result<Vec<String>> {
    deactivate_in(
        &active_path(),
        &launchers_owed_path(),
        |layer| crate::video_config::sync_session_env_with(&crate::video_config::load(), layer),
        follow_in_launchers,
    )
}

/// [`deactivate`] with the record at `record` and the launchers' mark at
/// `owed`; `sync` brings the session to a layer, `follow` brings the
/// launchers along.
fn deactivate_in(
    record: &Path,
    owed: &Path,
    sync: impl FnOnce(&Layer) -> Result<Vec<String>>,
    follow: impl FnOnce(),
) -> Result<Vec<String>> {
    let layer = if let Some(stored) = read_stored(record) {
        Layer {
            levers: Levers::default(),
            before: stored.before,
            owns_preset_keys: true,
        }
    } else {
        let unreadable = record.exists();
        if unreadable {
            // Unreadable: the user's values are lost with it, but a
            // preset's variables must still go.
            tracing::warn!(file = %record.display(), "the Turbo preset's record cannot be read; clearing its variables");
        }
        Layer {
            owns_preset_keys: unreadable,
            ..Layer::default()
        }
    };
    let had_record = layer.owns_preset_keys;
    let set = sync(&layer)?;
    remove_if_any(record)?;
    if had_record || owed.exists() {
        follow();
    }
    Ok(set)
}

fn remove_if_any(path: &Path) -> Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e).with_context(|| format!("remove {}", path.display())),
    }
}

/// Change the preset while Turbo is on: chosen, put in force, and the
/// session's environment brought to it. Games already running keep what
/// they started with; games started from now on get `preset`. A launcher
/// that is open keeps its own environment until it is opened again
/// ([`crate::launchers::behind_the_session`]).
///
/// # Errors
/// Returns an error when the choice or the state cannot be written, or the
/// session's environment cannot be set.
pub fn switch(preset: Preset) -> Result<Vec<String>> {
    set_chosen(preset)?;
    if preset == Preset::Standard {
        deactivate()
    } else {
        activate(preset)
    }
}

/// Bring the session in line when Big Game Mode starts, and whenever Turbo is
/// found switched off from outside: the preset's variables live only in the
/// running session, so after a login they are set again while Turbo is on,
/// and a preset left behind by a Turbo switched off elsewhere (or a Turbo
/// off whose undo failed) is taken away. Launchers that were open when the
/// preset last changed are brought along now ([`follow_owed`]).
///
/// # Errors
/// Returns an error when the session's environment cannot be set.
pub fn resync(turbo_on: bool) -> Result<()> {
    if has_record() {
        if !turbo_on {
            // Brings the launchers along itself.
            deactivate()?;
            return Ok(());
        }
        crate::video_config::sync_session_env(&crate::video_config::load())?;
    }
    follow_owed();
    Ok(())
}

/// Bring along the launchers that were open when the preset last changed,
/// if any: one may be closed by now. Steam's launch options and Heroic's
/// settings carry a game's vkBasalt switch, which follows the preset in
/// force, and an open launcher was left out then rather than written over.
pub fn follow_owed() {
    if launchers_owed_path().exists() {
        follow_in_launchers();
    }
}

/// Whether the session's environment `session` holds `layer`: every
/// variable the preset sets at its value, and every switch it turns off off.
#[must_use]
pub fn session_holds<S: std::hash::BuildHasher>(
    layer: &Layer,
    session: &HashMap<String, String, S>,
) -> bool {
    let levers = layer.levers;
    let switch_is = |key: &str, on: bool| (session.get(key).map(String::as_str) == Some("1")) == on;
    let wanted = preset_keys_over(levers, &layer.before);
    wanted.iter().all(|(k, v)| session.get(k) == Some(v))
        && levers
            .wine_fsr
            .is_none_or(|on| switch_is("WINE_FULLSCREEN_FSR", on))
        && levers.vkbasalt.is_none_or(|on| {
            switch_is("ENABLE_VKBASALT", on)
                && (!on
                    || session.get("VKBASALT_CONFIG_FILE").map(String::as_str)
                        == Some(cas_config_path().to_string_lossy().as_ref()))
        })
}

/// The preset whose variables the running session really holds: the one in
/// force by its record, read back from the user manager's environment. After
/// a login without Big Game Mode running, the record says a preset the session
/// no longer carries.
///
/// # Errors
/// Returns an error when the session's environment cannot be read.
pub fn in_session() -> Result<Option<Preset>> {
    // Asked every few seconds by Home: the machine (hardware, vkBasalt) is
    // read once, not each time.
    static MACHINE: std::sync::OnceLock<Machine> = std::sync::OnceLock::new();
    let Some(record) = read_stored(&active_path()) else {
        return Ok(None);
    };
    if record.preset == Preset::Standard {
        return Ok(None);
    }
    let layer = Layer {
        levers: levers(record.preset, *MACHINE.get_or_init(Machine::detect)),
        before: record.before,
        owns_preset_keys: true,
    };
    let session = crate::video_config::session_environment()?;
    Ok(session_holds(&layer, &session).then_some(record.preset))
}

#[cfg(test)]
mod tests {
    use super::*;

    const DESKTOP: Machine = Machine {
        vkbasalt: true,
        fsr4: true,
    };

    fn tuning() -> HashMap<String, String> {
        HashMap::from([
            ("WINE_FULLSCREEN_FSR".to_owned(), "1".to_owned()),
            (
                "WINE_FULLSCREEN_FSR_MODE".to_owned(),
                "performance".to_owned(),
            ),
        ])
    }

    #[test]
    fn standard_changes_nothing() {
        let mut env = tuning();
        overlay(&mut env, levers(Preset::Standard, DESKTOP));
        assert_eq!(env, tuning());
    }

    #[test]
    fn a_frame_cap_reaches_dxvk_and_vkd3d_proton() {
        let mut env = HashMap::new();
        overlay(&mut env, levers(Preset::Locked60, DESKTOP));
        assert_eq!(env["VKD3D_FRAME_RATE"], "60");
        // DXVK before 2.3 reads only DXVK_FRAME_RATE.
        assert_eq!(env["DXVK_FRAME_RATE"], "60");
        let dxvk = &env["DXVK_CONFIG"];
        assert!(dxvk.contains("dxgi.maxFrameRate = 60"), "{dxvk}");
        assert!(dxvk.contains("d3d9.maxFrameRate = 60"), "{dxvk}");
        // More FPS removes a cap rather than setting one.
        let mut env = HashMap::new();
        overlay(&mut env, levers(Preset::MoreFps, DESKTOP));
        assert!(!env.contains_key("VKD3D_FRAME_RATE") && !env.contains_key("DXVK_CONFIG"));
    }

    #[test]
    fn enhanced_turns_wine_fsr_off_and_uses_what_the_machine_has() {
        let mut env = tuning();
        overlay(&mut env, levers(Preset::Enhanced, DESKTOP));
        assert!(
            !env.contains_key("WINE_FULLSCREEN_FSR"),
            "native resolution"
        );
        assert_eq!(env["ENABLE_VKBASALT"], "1");
        assert_eq!(env["FSR4_UPGRADE"], "1");
        assert_eq!(env["PROTON_FSR4_UPGRADE"], "1", "GE-Proton's name");
        // Without vkBasalt or an FSR 4 GPU those are not asked for.
        let bare = Machine::default();
        let mut env = tuning();
        overlay(&mut env, levers(Preset::Enhanced, bare));
        assert!(!env.contains_key("ENABLE_VKBASALT"));
        assert!(!env.contains_key("FSR4_UPGRADE"));
    }

    #[test]
    fn more_fps_takes_the_filter_away_and_turns_wine_fsr_on() {
        let mut env = HashMap::from([("ENABLE_VKBASALT".to_owned(), "1".to_owned())]);
        overlay(&mut env, levers(Preset::MoreFps, DESKTOP));
        assert!(!env.contains_key("ENABLE_VKBASALT"));
        assert_eq!(env["WINE_FULLSCREEN_FSR"], "1");
    }

    #[test]
    fn lsfg_entries_that_generate_are_found() {
        let conf = r#"
version = 1
[[game]]
exe = "Bodycam-Win64-Shipping.exe"
multiplier = 2
[[game]]
exe = "Off.exe"
multiplier = 1
[[game]]
exe = "proton"
multiplier = 2
"#;
        assert_eq!(lsfg_generating(conf), ["Bodycam-Win64-Shipping.exe"]);
        assert!(lsfg_generating("not toml [").is_empty());
        let v2 = r#"
version = 2
[global]
[[profile]]
name = "Big Game Mode: no game"
active_in = []
[[profile]]
name = "Bodycam"
active_in = ["Bodycam-Win64-Shipping.exe"]
multiplier = 3
[[profile]]
name = "Off"
active_in = "Off.exe"
multiplier = 1
"#;
        assert_eq!(lsfg_generating(v2), ["Bodycam-Win64-Shipping.exe"]);
    }

    #[test]
    fn a_preset_is_stored_by_its_id() {
        for p in ALL {
            assert_eq!(Preset::from_id(p.id()), Some(p));
            let text = toml::to_string(&Stored {
                preset: p,
                before: BTreeMap::new(),
            })
            .unwrap();
            assert_eq!(toml::from_str::<Stored>(&text).unwrap().preset, p);
        }
        assert_eq!(Preset::from_id("nope"), None);
        // A record written before the user's values were kept still loads.
        let old: Stored = toml::from_str("preset = \"locked60\"\n").unwrap();
        assert_eq!(old.preset, Preset::Locked60);
        assert!(old.before.is_empty());
    }

    /// A private folder per test, removed afterwards.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "bigame_preset_{tag}_{}_{:?}",
                std::process::id(),
                std::thread::current().id()
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn enhanced_points_vkbasalt_at_its_own_cas_file() {
        let cas = cas_config_path().to_string_lossy().into_owned();
        let mut env = tuning();
        overlay(&mut env, levers(Preset::Enhanced, DESKTOP));
        assert_eq!(env["VKBASALT_CONFIG_FILE"], cas);
        assert!(
            !cas.ends_with("vkBasalt/vkBasalt.conf"),
            "never the user's own file: {cas}"
        );
        // Games Big Game Mode starts get the same file.
        assert_eq!(
            preset_env(levers(Preset::Enhanced, DESKTOP))["VKBASALT_CONFIG_FILE"],
            cas
        );
        // Only Enhanced turns vkBasalt on, so only it names a file.
        assert!(
            !preset_env(levers(Preset::Locked60, DESKTOP)).contains_key("VKBASALT_CONFIG_FILE")
        );
    }

    #[test]
    fn the_cas_file_holds_sharpening_and_nothing_else() {
        let dir = Scratch::new("cas");
        let path = dir.0.join("bigame-mode").join("vkbasalt-cas.conf");
        write_cas_config(&path).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        let effects: Vec<&str> = text
            .lines()
            .filter_map(|l| l.strip_prefix("effects"))
            .collect();
        assert_eq!(effects.len(), 1, "{text}");
        assert_eq!(effects[0].trim_start_matches([' ', '=']).trim(), "cas");
        assert!(!text.contains("FakeHDR") && !text.contains("FilmGrain"));
        // Written again unchanged, in one rename: no temporary file is left.
        write_cas_config(&path).unwrap();
        let names: Vec<String> = std::fs::read_dir(path.parent().unwrap())
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["vkbasalt-cas.conf"]);
    }

    #[test]
    fn an_open_launcher_leaves_the_preset_owed_until_it_follows() {
        use crate::heroic_launch::Applied as Heroic;
        use crate::steam_gamescope::Applied as Steam;
        let dir = Scratch::new("owed");
        let owed = dir
            .0
            .join("bigame-mode")
            .join("turbo-preset-launchers-owed");
        // Steam open: nothing written there, so the change stays owed.
        let followed = follow_in_launchers_at(
            &owed,
            || vec![("SOTTR.exe".to_owned(), Ok(Steam::SteamRunning))],
            Vec::new,
        );
        assert!(!followed);
        assert!(owed.exists());
        // Heroic open: owed as well, even with Steam's games written.
        let followed = follow_in_launchers_at(
            &owed,
            || vec![("SOTTR.exe".to_owned(), Ok(Steam::Written(String::new())))],
            || {
                vec![(
                    "Hades.exe".to_owned(),
                    Ok(Heroic::HeroicRunning {
                        launcher: crate::launchers::Launcher::Heroic { flatpak: false },
                        game_running: false,
                    }),
                )]
            },
        );
        assert!(!followed);
        assert!(owed.exists());
        // A game that could not be written is tried again too.
        let followed = follow_in_launchers_at(
            &owed,
            || vec![("SOTTR.exe".to_owned(), Err(anyhow::anyhow!("read-only")))],
            Vec::new,
        );
        assert!(!followed);
        assert!(owed.exists());
        // Everything in line: the mark goes.
        let followed = follow_in_launchers_at(
            &owed,
            || vec![("SOTTR.exe".to_owned(), Ok(Steam::Unchanged))],
            || vec![("Hades.exe".to_owned(), Ok(Heroic::Written))],
        );
        assert!(followed);
        assert!(!owed.exists());
    }

    /// The files [`deactivate_in`] works on, in a scratch folder.
    struct Files {
        record: PathBuf,
        owed: PathBuf,
    }

    impl Files {
        fn in_(dir: &Scratch) -> Self {
            Self {
                record: dir.0.join("turbo-preset-active.toml"),
                owed: dir.0.join("turbo-preset-launchers-owed"),
            }
        }
    }

    #[test]
    fn taking_a_preset_away_brings_the_launchers_along() {
        let dir = Scratch::new("deactivate");
        let f = Files::in_(&dir);
        write(
            &f.record,
            &Stored {
                preset: Preset::Enhanced,
                before: BTreeMap::from([("PROTON_FSR4_UPGRADE".to_owned(), "1".to_owned())]),
            },
        )
        .unwrap();
        let mut seen = None;
        let mut followed = false;
        deactivate_in(
            &f.record,
            &f.owed,
            |layer| {
                seen = Some(layer.clone());
                Ok(Vec::new())
            },
            || followed = true,
        )
        .unwrap();
        let layer = seen.unwrap();
        assert_eq!(layer.levers, Levers::default());
        assert_eq!(layer.before["PROTON_FSR4_UPGRADE"], "1");
        assert!(layer.owns_preset_keys);
        assert!(!f.record.exists());
        // After the record: the launchers then read no preset in force.
        assert!(followed);
        // No record and nothing owed: the launchers are left alone.
        let mut followed = false;
        deactivate_in(&f.record, &f.owed, |_| Ok(Vec::new()), || followed = true).unwrap();
        assert!(!followed);
        // Still owed from an earlier Turbo off: tried again.
        std::fs::write(&f.owed, "").unwrap();
        deactivate_in(&f.record, &f.owed, |_| Ok(Vec::new()), || followed = true).unwrap();
        assert!(followed);
    }

    #[test]
    fn an_unreadable_record_is_still_taken_away() {
        let dir = Scratch::new("deactivate_unreadable");
        let f = Files::in_(&dir);
        std::fs::write(&f.record, "preset = [not toml").unwrap();
        assert!(read_stored(&f.record).is_none());
        let mut owned = false;
        let mut followed = false;
        deactivate_in(
            &f.record,
            &f.owed,
            |layer| {
                owned = layer.owns_preset_keys;
                Ok(Vec::new())
            },
            || followed = true,
        )
        .unwrap();
        // Its variables are Big Game Mode's to unset, and the launchers follow.
        assert!(owned);
        assert!(followed);
        assert!(!f.record.exists());
    }

    #[test]
    fn a_session_that_does_not_change_keeps_the_record() {
        let dir = Scratch::new("deactivate_fails");
        let f = Files::in_(&dir);
        write(&f.record, &Stored::default()).unwrap();
        let mut followed = false;
        let result = deactivate_in(
            &f.record,
            &f.owed,
            |_| anyhow::bail!("the session environment did not change"),
            || followed = true,
        );
        assert!(result.is_err());
        assert!(f.record.exists(), "kept to retry from");
        assert!(!followed);
    }

    #[test]
    fn a_cap_is_merged_into_the_users_dxvk_config() {
        assert_eq!(
            merge_dxvk_config(
                Some("dxgi.customVendorId = 10de; dxgi.maxFrameRate = 144"),
                60
            ),
            "dxgi.customVendorId = 10de; dxgi.maxFrameRate = 60; d3d9.maxFrameRate = 60"
        );
        assert_eq!(merge_dxvk_config(None, 60), dxvk_cap(60));
        // No cap: the user's other options stay, their caps go.
        assert_eq!(
            merge_dxvk_config(Some("d3d9.maxFrameRate=30;dxgi.hideNvidiaGpu = False"), 0),
            "dxgi.hideNvidiaGpu = False"
        );
    }

    #[test]
    fn a_preset_keeps_the_users_own_values() {
        let before = BTreeMap::from([
            (
                "DXVK_CONFIG".to_owned(),
                "dxgi.customVendorId = 10de".to_owned(),
            ),
            ("VKD3D_FRAME_RATE".to_owned(), "144".to_owned()),
            ("PROTON_FSR4_UPGRADE".to_owned(), "1".to_owned()),
        ]);
        let mut env = HashMap::new();
        overlay_over(&mut env, levers(Preset::Locked60, DESKTOP), &before);
        assert_eq!(
            env["DXVK_CONFIG"],
            "dxgi.customVendorId = 10de; dxgi.maxFrameRate = 60; d3d9.maxFrameRate = 60"
        );
        assert_eq!(env["VKD3D_FRAME_RATE"], "60");
        // A key Locked 60 does not set keeps the user's value.
        assert_eq!(env["PROTON_FSR4_UPGRADE"], "1");
        assert!(!env.contains_key("FSR4_UPGRADE"));
        // More FPS: no cap, the rest of their DXVK options kept, and their
        // VKD3D-Proton limit switched off rather than unset.
        let mut env = HashMap::new();
        overlay_over(&mut env, levers(Preset::MoreFps, DESKTOP), &before);
        assert_eq!(env["DXVK_CONFIG"], "dxgi.customVendorId = 10de");
        assert_eq!(env["VKD3D_FRAME_RATE"], "0");
    }

    #[test]
    fn a_cap_an_earlier_build_left_behind_is_not_taken_as_the_users() {
        let leftover = HashMap::from([
            ("DXVK_CONFIG".to_owned(), dxvk_cap(60)),
            ("DXVK_FRAME_RATE".to_owned(), "60".to_owned()),
            ("VKD3D_FRAME_RATE".to_owned(), "60".to_owned()),
            ("PROTON_FSR4_UPGRADE".to_owned(), "1".to_owned()),
            ("HOME".to_owned(), "/home/u".to_owned()),
        ]);
        let mine = user_values(&leftover);
        assert_eq!(
            mine,
            BTreeMap::from([("PROTON_FSR4_UPGRADE".to_owned(), "1".to_owned())])
        );
        // A cap of the user's own, in their own words, is theirs.
        let own = HashMap::from([
            (
                "DXVK_CONFIG".to_owned(),
                "dxgi.maxFrameRate = 60".to_owned(),
            ),
            ("VKD3D_FRAME_RATE".to_owned(), "60".to_owned()),
        ]);
        assert_eq!(user_values(&own).len(), 2);
    }

    #[test]
    fn what_the_session_holds_is_checked_variable_by_variable() {
        let layer = Layer {
            levers: levers(Preset::Locked60, DESKTOP),
            before: BTreeMap::new(),
            owns_preset_keys: true,
        };
        let mut session = HashMap::from([
            ("DXVK_CONFIG".to_owned(), dxvk_cap(60)),
            ("DXVK_FRAME_RATE".to_owned(), "60".to_owned()),
            ("VKD3D_FRAME_RATE".to_owned(), "60".to_owned()),
            ("WINE_FULLSCREEN_FSR".to_owned(), "1".to_owned()),
            ("ENABLE_VKBASALT".to_owned(), "0".to_owned()),
        ]);
        assert!(session_holds(&layer, &session));
        // After a login the manager has none of it: the record is not enough.
        session.remove("VKD3D_FRAME_RATE");
        assert!(!session_holds(&layer, &session));
        session.insert("VKD3D_FRAME_RATE".into(), "60".into());
        session.insert("ENABLE_VKBASALT".into(), "1".into());
        assert!(
            !session_holds(&layer, &session),
            "vkBasalt is off in Locked 60"
        );
    }

    #[test]
    fn frame_generation_is_told_apart_by_technology() {
        let dir = Scratch::new("fg");
        let settings = dir.0.join("games");
        let mut opti = crate::game_settings::GameSettings::default();
        opti.ai_graphics.mode = crate::graphics::config::Mode::Advanced;
        opti.ai_graphics.frame_generation = crate::graphics::config::FrameGeneration::OptiScaler;
        opti.ai_graphics.experimental = true;
        crate::game_settings::save_to(&settings, "SOTTR.exe", &opti).unwrap();
        crate::game_settings::save_to(
            &settings,
            "Plain.exe",
            &crate::game_settings::GameSettings::default(),
        )
        .unwrap();
        let lsfg = r#"
version = 1
[[game]]
exe = "Bodycam-Win64-Shipping.exe"
multiplier = 2
[[game]]
exe = "sottr.exe"
multiplier = 2
"#;
        let none = dir.0.join("no-manifests");
        let (optiscaler, lsfg_games) = frame_generators_in(&none, &settings, Some(lsfg));
        assert_eq!(optiscaler, ["SOTTR.exe"]);
        // lsfg-vk is off in a game that runs OptiScaler's generation.
        assert_eq!(lsfg_games, ["Bodycam-Win64-Shipping.exe"]);
        // lsfg-vk's general switch off: none of its entries generate.
        let (_, lsfg_games) = frame_generators_in(&none, &settings, None);
        assert!(lsfg_games.is_empty());
    }

    #[test]
    fn each_game_is_worded_by_what_the_cap_holds_in_it() {
        use crate::optimization::CappedFrames;
        let capped = |game: &str| match game {
            "SOTTR.exe" => Some(CappedFrames::Shown),
            "Bodycam-Win64-Shipping.exe" => Some(CappedFrames::Rendered { multiplier: 3 }),
            _ => None,
        };
        let found = classify(
            ["Bodycam-Win64-Shipping.exe", "SOTTR.exe", "Gone.exe"].map(str::to_owned),
            capped,
        );
        assert_eq!(found.shown, ["SOTTR.exe"]);
        assert_eq!(
            found.rendered,
            [("Bodycam-Win64-Shipping.exe".to_owned(), 3)]
        );
        assert!(classify(["Gone.exe".to_owned()], capped).is_empty());
    }
}
