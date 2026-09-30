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
//! - vkBasalt (`ENABLE_VKBASALT`): the sharpening filter (CAS) in its
//!   configuration;
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

use std::collections::HashMap;
use std::path::PathBuf;

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

/// The variables only a preset sets for `levers`; they are never in
/// `environment.d`.
#[must_use]
pub fn preset_env(levers: Levers) -> HashMap<String, String> {
    let mut env = HashMap::new();
    if let Some(fps) = levers.frame_cap.filter(|f| *f > 0) {
        env.insert(
            "DXVK_CONFIG".into(),
            format!("dxgi.maxFrameRate = {fps}; d3d9.maxFrameRate = {fps}"),
        );
        env.insert("VKD3D_FRAME_RATE".into(), fps.to_string());
    }
    if levers.fsr4_upgrade {
        env.insert("FSR4_UPGRADE".into(), "1".into());
        // GE-Proton reads only its own name, and then fetches the provider.
        env.insert("PROTON_FSR4_UPGRADE".into(), "1".into());
    }
    env
}

/// `env` (Tuning's session variables) with `levers` laid over it.
pub fn overlay<S: std::hash::BuildHasher>(env: &mut HashMap<String, String, S>, levers: Levers) {
    env.extend(preset_env(levers));
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
        }
        Some(false) => {
            env.remove("ENABLE_VKBASALT");
            env.remove("VKBASALT_CONFIG_FILE");
        }
        None => {}
    }
}

// ── Frame generation under a cap ────────────────────────────────────────────

/// The games that generate frames: lsfg-vk entries with a multiplier above
/// 1 (while lsfg-vk's general switch is on), and games whose AI Graphics
/// has `OptiScaler`'s frame generation. A frame cap counts the frames shown,
/// generated ones included, so under a 60 FPS cap such a game renders about
/// 30 (Shadow of the Tomb Raider with `OptiScaler` generation: 62.5 → 30.0 in
/// its own benchmark, on the reference desktop).
#[must_use]
pub fn frame_generation_games() -> Vec<String> {
    let mut games = Vec::new();
    if crate::optimization::lsfg_general_on(&crate::video_config::load()) {
        let text = std::fs::read_to_string(crate::fg::config_path()).unwrap_or_default();
        games.extend(lsfg_generating(&text));
    }
    if let Ok(dir) = std::fs::read_dir(crate::game_settings::dir()) {
        for entry in dir.flatten() {
            let path = entry.path();
            let Some(name) = path
                .file_name()
                .and_then(|n| n.to_str())
                .and_then(|n| n.strip_suffix(".toml"))
            else {
                continue;
            };
            if crate::game_settings::load(name)
                .is_ok_and(|s| s.ai_graphics.optiscaler_frame_generation())
            {
                games.push(name.to_owned());
            }
        }
    }
    games.sort();
    games.dedup();
    games
}

/// The executables lsfg-vk's configuration generates frames for. `proton`
/// is not a game's process (Proton's launcher script), so it is left out.
fn lsfg_generating(conf: &str) -> Vec<String> {
    let Ok(table) = conf.parse::<toml::Table>() else {
        return Vec::new();
    };
    table
        .get("game")
        .and_then(toml::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|g| {
            let exe = g.get("exe")?.as_str()?;
            let multiplier = g.get("multiplier")?.as_integer()?;
            (multiplier > 1 && exe != "proton").then(|| exe.to_owned())
        })
        .collect()
}

// ── What was chosen, and what is in force ───────────────────────────────────

#[derive(Debug, Default, Serialize, Deserialize)]
struct Stored {
    #[serde(default)]
    preset: Preset,
}

fn chosen_path() -> PathBuf {
    crate::paths::config_home()
        .join("bigame-mode")
        .join("turbo-preset.toml")
}

fn active_path() -> PathBuf {
    std::env::var_os("XDG_STATE_HOME")
        .map_or_else(
            || crate::paths::home_dir().join(".local/state"),
            PathBuf::from,
        )
        .join("bigame-mode")
        .join("turbo-preset-active.toml")
}

fn read(path: &std::path::Path) -> Option<Preset> {
    let text = std::fs::read_to_string(path).ok()?;
    toml::from_str::<Stored>(&text).ok().map(|s| s.preset)
}

fn write(path: &std::path::Path, preset: Preset) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("create {}", dir.display()))?;
    }
    let text = toml::to_string(&Stored { preset }).context("serialize the preset")?;
    std::fs::write(path, text).with_context(|| format!("write {}", path.display()))
}

/// The preset chosen for the next time Turbo is switched on.
#[must_use]
pub fn chosen() -> Preset {
    read(&chosen_path()).unwrap_or_default()
}

/// Choose the preset for the next time Turbo is switched on.
///
/// # Errors
/// Returns an error when the choice cannot be written.
pub fn set_chosen(preset: Preset) -> Result<()> {
    write(&chosen_path(), preset)
}

/// The preset in force now: the one Turbo was switched on with, until it is
/// switched off.
#[must_use]
pub fn active() -> Preset {
    read(&active_path()).unwrap_or_default()
}

/// The levers in force now.
#[must_use]
pub fn active_levers() -> Levers {
    match active() {
        Preset::Standard => Levers::default(),
        preset => levers(preset, Machine::detect()),
    }
}

/// Put `preset` in force: remembered, and the session's environment
/// brought to Tuning's plus the preset's. Returns what the session holds
/// afterwards, read back.
///
/// # Errors
/// Returns an error when the state cannot be written or the session's
/// environment cannot be set.
pub fn activate(preset: Preset) -> Result<Vec<String>> {
    write(&active_path(), preset)?;
    crate::video_config::sync_session_env(&crate::video_config::load())
}

/// Take the preset away: the session goes back to Tuning's variables.
///
/// # Errors
/// Returns an error when the session's environment cannot be set.
pub fn deactivate() -> Result<Vec<String>> {
    let path = active_path();
    if path.exists() {
        std::fs::remove_file(&path).with_context(|| format!("remove {}", path.display()))?;
    }
    crate::video_config::sync_session_env(&crate::video_config::load())
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

/// Bring the session in line when Big Game Mode starts: the preset's
/// variables live only in the running session, so after a login they are
/// set again while Turbo is on, and a preset left behind by a Turbo switched
/// off elsewhere is dropped.
///
/// # Errors
/// Returns an error when the session's environment cannot be set.
pub fn resync(turbo_on: bool) -> Result<()> {
    if active() == Preset::Standard {
        return Ok(());
    }
    if turbo_on {
        crate::video_config::sync_session_env(&crate::video_config::load())?;
    } else {
        deactivate()?;
    }
    Ok(())
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
    }

    #[test]
    fn a_preset_is_stored_by_its_id() {
        for p in ALL {
            assert_eq!(Preset::from_id(p.id()), Some(p));
            let text = toml::to_string(&Stored { preset: p }).unwrap();
            assert_eq!(toml::from_str::<Stored>(&text).unwrap().preset, p);
        }
        assert_eq!(Preset::from_id("nope"), None);
    }
}
