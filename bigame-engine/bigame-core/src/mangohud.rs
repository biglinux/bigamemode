//! `MangoHud` per game: off, on, or forced.
//!
//! `MangoHud` reaches a game in one of two ways, and they cover different games:
//!
//! * **On** — its implicit Vulkan layer, switched on by `MANGOHUD=1`. That is
//!   every Vulkan game, and every Proton game (DXVK and `VKD3D-Proton` are
//!   Vulkan).
//! * **Forced** — the `mangohud` wrapper, which also preloads it into `OpenGL`
//!   games, where the Vulkan layer never loads.
//!
//! A game BiGame-mode starts gets it through its launch plan (inside Gamescope,
//! as Gamescope's own `--mangoapp`). A game its own launcher starts runs in
//! that launcher's process tree, which BiGame-mode cannot reach, so the setting
//! goes where that launcher reads it:
//!
//! * **Steam** — the game's launch options, edited with Steam closed, backed
//!   up and read back (`steam::set_launch_options`). Options the user wrote
//!   stay; only what this module adds is ever removed.
//! * **Heroic** — the game's `GamesConfig/<app>.json`: Forced is Heroic's own
//!   `MangoHud` switch (`showMangohud`, its `mangohud --dlsym` wrapper), On
//!   is `MANGOHUD=1` in the game's environment variables. Heroic merges a
//!   game's file over its defaults, so only these two keys are written. It
//!   keeps a game's settings in memory and writes them back when one
//!   changes, so the file is written with Heroic closed.
//! * **Lutris** — the game's YAML: Lutris's own *FPS counter (`MangoHud`)*
//!   option, `system: mangohud: true`, for On and Forced alike.
//!
//! A launcher running as a Flatpak cannot see the system's `mangohud`: it
//! needs Flathub's `org.freedesktop.Platform.VulkanLayer.MangoHud` for its
//! runtime's branch, and Heroic refuses to start a game with its switch on
//! and no `mangohud` on its PATH. That is detected and said, with the
//! command.

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::error::UserError;
use crate::text::N_;

/// A game's `MangoHud` setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    /// Not added by BiGame-mode (`MangoHud` may still come from elsewhere).
    #[default]
    Off,
    /// The Vulkan layer (`MANGOHUD=1`): Vulkan and Proton games.
    On,
    /// The `mangohud` wrapper: `OpenGL` games too.
    Forced,
}

/// The word Steam replaces with the game's command.
const COMMAND: &str = "%command%";
/// What [`Mode::On`] adds in front of Steam's launch options.
const LAYER: &str = "MANGOHUD=1";
/// What [`Mode::Forced`] puts in front of `%command%`.
const WRAPPER: &str = "mangohud";

/// Steam launch options with `mode` applied to `current`.
///
/// Removes what an earlier call added (a leading `MANGOHUD=1`, a `mangohud`
/// right before `%command%`), then adds what `mode` needs. Everything else —
/// other variables, other wrappers, the game's own arguments — is kept in
/// place. Launch options without `%command%` are the game's arguments, and
/// Steam appends them to the command; they are kept after it.
#[must_use]
pub fn launch_options(current: &str, mode: Mode) -> String {
    let mut words: Vec<&str> = current.split_whitespace().collect();
    words.retain(|w| *w != LAYER);
    if let Some(i) = words.iter().position(|w| *w == COMMAND) {
        if i > 0 && words[i - 1] == WRAPPER {
            words.remove(i - 1);
        }
    }
    let has_command = words.contains(&COMMAND);
    if mode == Mode::Off {
        let rest = words.join(" ");
        return if rest == COMMAND { String::new() } else { rest };
    }
    if !has_command {
        // Plain arguments: they follow the command.
        words.insert(0, COMMAND);
    }
    match mode {
        Mode::On => words.insert(0, LAYER),
        Mode::Forced => {
            let i = words.iter().position(|w| *w == COMMAND).unwrap_or(0);
            words.insert(i, WRAPPER);
        }
        Mode::Off => {}
    }
    words.join(" ")
}

/// Where a `MangoHud` setting took effect.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Applied {
    /// In the game's settings in its launcher (Heroic, Lutris).
    Launcher {
        /// The launcher's name.
        name: &'static str,
        /// The launcher is a Flatpak without `MangoHud`'s Flatpak extension
        /// for its runtime: the command that installs it. Without it the
        /// overlay cannot load (and Heroic refuses to start the game with
        /// its switch on).
        missing_extension: Option<String>,
    },
    /// Nothing changed: this launcher is running and keeps the game's
    /// settings in memory, so it would overwrite the change.
    LauncherRunning(&'static str),
    /// In the launch plan of games BiGame-mode starts; the game is not a
    /// Steam game.
    LaunchPlan,
    /// In Steam's launch options, which now read as given (read back).
    SteamLaunchOptions(String),
    /// Nothing changed: Steam is running and holds its configuration in
    /// memory, so the launch options could not be written.
    SteamRunning,
}

/// Save `mode` for the game whose process is `process`, and write it where
/// the game will see it.
///
/// # Errors
/// Returns an error if the setting cannot be saved or Steam's configuration
/// cannot be written or verified.
pub fn apply(process: &str, mode: Mode) -> Result<Applied> {
    let games: Vec<crate::games::DetectedGame> = crate::games::detect_all()
        .into_iter()
        .filter(|g| g.profile_key() == process)
        .collect();
    if let Some(launcher) = games.iter().find_map(|g| g.launcher.clone()) {
        return apply_to_launcher(process, &launcher, mode);
    }
    let apps: Vec<String> = games
        .into_iter()
        .filter(|g| g.source == crate::games::Source::Steam)
        .filter_map(|g| g.app_id)
        .collect();
    // A Steam game whose launch options cannot be written now keeps its old
    // setting too: a saved choice that is not in effect would be a lie.
    if !apps.is_empty() && crate::steam::is_running() {
        return Ok(Applied::SteamRunning);
    }
    // The launch options first, the saved choice after: if Steam's file cannot
    // be written, the choice stays as it was instead of claiming a mode the
    // game will not get.
    let mut last = String::new();
    for user in crate::steam::users(&crate::paths::home_dir()) {
        for app in &apps {
            let current = crate::steam::launch_options(&user.config, app).unwrap_or_default();
            let wanted = launch_options(&current, mode);
            if wanted != current {
                crate::steam::set_launch_options(&user.config, app, &wanted)?;
            }
            last = wanted;
        }
    }
    let mut settings = crate::game_settings::load(process).unwrap_or_default();
    settings.mangohud = mode;
    crate::game_settings::save(process, &settings)?;
    if apps.is_empty() {
        return Ok(Applied::LaunchPlan);
    }
    Ok(Applied::SteamLaunchOptions(last))
}

/// Write `mode` into the game's settings in its launcher, then save it.
fn apply_to_launcher(
    process: &str,
    launcher: &crate::games::LauncherRef,
    mode: Mode,
) -> Result<Applied> {
    use crate::games::LauncherRef;
    match launcher {
        LauncherRef::Heroic {
            app_name,
            config_dir,
        } => {
            if launcher_running("heroic") {
                return Ok(Applied::LauncherRunning("Heroic"));
            }
            let file = config_dir
                .join("GamesConfig")
                .join(format!("{app_name}.json"));
            let current = std::fs::read_to_string(&file).unwrap_or_default();
            let wanted = heroic_config(&current, app_name, mode)?;
            if wanted != current {
                write_keeping_backup(&file, &wanted)?;
            }
        }
        LauncherRef::Lutris { config_file } => {
            let current = std::fs::read_to_string(config_file)?;
            let wanted = lutris_config(&current, mode);
            if wanted != current {
                write_keeping_backup(config_file, &wanted)?;
            }
        }
    }
    let mut settings = crate::game_settings::load(process).unwrap_or_default();
    settings.mangohud = mode;
    crate::game_settings::save(process, &settings)?;
    Ok(Applied::Launcher {
        name: launcher.launcher_name(),
        missing_extension: (mode != Mode::Off)
            .then(|| launcher.flatpak_id().and_then(missing_flatpak_extension))
            .flatten(),
    })
}

/// Whether a process named `name` runs (`/proc/<pid>/comm`).
fn launcher_running(name: &str) -> bool {
    std::fs::read_dir("/proc").is_ok_and(|d| {
        d.flatten()
            .any(|p| std::fs::read_to_string(p.path().join("comm")).is_ok_and(|c| c.trim() == name))
    })
}

/// Replace `file` with `text`, atomically. The launcher's own version is kept
/// once, the first time BiGame-mode changes the file, under
/// `$XDG_STATE_HOME/bigame-mode/launcher-backups/` — never beside it, where
/// the launcher might read a stray file.
fn write_keeping_backup(file: &std::path::Path, text: &str) -> Result<()> {
    use anyhow::Context;
    if file.exists() {
        let dir = crate::paths::state_home().join("bigame-mode/launcher-backups");
        std::fs::create_dir_all(&dir)?;
        let name = file
            .to_string_lossy()
            .trim_start_matches('/')
            .replace('/', "__");
        let bak = dir.join(name);
        if !bak.exists() {
            std::fs::copy(file, &bak).with_context(|| format!("back up {}", file.display()))?;
        }
    } else if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = file.with_extension("bigame-new");
    std::fs::write(&tmp, text).with_context(|| format!("write {}", tmp.display()))?;
    std::fs::rename(&tmp, file).with_context(|| format!("replace {}", file.display()))
}

/// Heroic's per-game settings with `mode` applied: `showMangohud` for
/// Forced, a `MANGOHUD=1` environment variable for On, neither for Off.
/// Everything else in the file — the game's other settings, other games,
/// Heroic's `version` and `explicit` — is kept.
///
/// # Errors
/// Returns an error if the file is not a JSON object.
pub fn heroic_config(current: &str, app_name: &str, mode: Mode) -> Result<String> {
    use serde_json::{Map, Value, json};
    let mut root: Value = if current.trim().is_empty() {
        Value::Object(Map::new())
    } else {
        serde_json::from_str(current)?
    };
    let obj = root
        .as_object_mut()
        .ok_or_else(|| UserError::plain(N_("Heroic's game settings are not a JSON object")))?;
    obj.entry("version").or_insert_with(|| json!("v0"));
    obj.entry("explicit").or_insert_with(|| json!(true));
    let game = obj
        .entry(app_name)
        .or_insert_with(|| Value::Object(Map::new()))
        .as_object_mut()
        .ok_or_else(|| {
            UserError::with(N_("Heroic's settings for %s are not an object"), [app_name])
        })?;
    game.insert("showMangohud".into(), json!(mode == Mode::Forced));
    let mut env: Vec<Value> = game
        .get("enviromentOptions")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    env.retain(|e| e.get("key").and_then(Value::as_str) != Some("MANGOHUD"));
    if mode == Mode::On {
        env.push(json!({"key": "MANGOHUD", "value": "1"}));
    }
    // Heroic's own spelling.
    game.insert("enviromentOptions".into(), Value::Array(env));
    let mut text = serde_json::to_string_pretty(&root)?;
    text.push('\n');
    Ok(text)
}

/// A Lutris game YAML with `mode` applied to its `system:` section's
/// `mangohud` option. Only that line (and, when there was none, the section
/// header) changes; comments, order and every other key stay byte for byte.
#[must_use]
pub fn lutris_config(current: &str, mode: Mode) -> String {
    let lines: Vec<&str> = current.lines().collect();
    let header = lines
        .iter()
        .position(|l| *l == "system:" || l.starts_with("system: "));
    let on = mode != Mode::Off;
    let mut out: Vec<String> = lines.iter().map(|l| (*l).to_owned()).collect();
    match header {
        None if !on => return current.to_owned(),
        None => {
            out.push("system:".into());
            out.push("  mangohud: true".into());
        }
        Some(h) => {
            // `system: {}` or another flow value: make it a block.
            if out[h] != "system:" {
                out[h] = "system:".into();
            }
            let end = (h + 1..out.len())
                .find(|&i| !out[i].is_empty() && !out[i].starts_with(' '))
                .unwrap_or(out.len());
            let found = (h + 1..end).find(|&i| {
                let t = out[i].trim_start();
                out[i].starts_with("  ") && !out[i].starts_with("   ") && t.starts_with("mangohud:")
            });
            match (found, on) {
                (Some(i), true) => out[i] = "  mangohud: true".into(),
                (Some(i), false) => {
                    out.remove(i);
                    // A section left with nothing in it goes too.
                    if (h + 1..end - 1).all(|j| out[j].trim().is_empty()) {
                        out.remove(h);
                    }
                }
                (None, true) => out.insert(h + 1, "  mangohud: true".into()),
                (None, false) => {}
            }
        }
    }
    let mut text = out.join("\n");
    if current.ends_with('\n') || current.is_empty() {
        text.push('\n');
    }
    text
}

/// The command that installs `MangoHud`'s Flatpak extension for the runtime
/// of Flatpak app `app_id`, when it is missing; `None` when it is there, or
/// the app is not a Flatpak this machine has.
#[must_use]
pub fn missing_flatpak_extension(app_id: &str) -> Option<String> {
    let home = crate::paths::home_dir();
    let installs = [
        std::path::PathBuf::from("/var/lib/flatpak"),
        home.join(".local/share/flatpak"),
    ];
    let branch = installs.iter().find_map(|base| {
        let meta = std::fs::read_to_string(
            base.join("app")
                .join(app_id)
                .join("current/active/metadata"),
        )
        .ok()?;
        flatpak_runtime_branch(&meta)
    })?;
    let ext = "org.freedesktop.Platform.VulkanLayer.MangoHud";
    let present = installs.iter().any(|base| {
        base.join("runtime")
            .join(ext)
            .join("x86_64")
            .join(&branch)
            .is_dir()
    });
    (!present).then(|| format!("flatpak install flathub {ext}//{branch}"))
}

/// The branch of the runtime a Flatpak app's `metadata` names:
/// `runtime=org.freedesktop.Platform/x86_64/25.08` → `25.08`.
fn flatpak_runtime_branch(metadata: &str) -> Option<String> {
    metadata
        .lines()
        .find_map(|l| l.strip_prefix("runtime="))
        .and_then(|r| r.rsplit('/').next())
        .filter(|b| !b.is_empty())
        .map(str::to_owned)
}

/// The game's `MangoHud` setting, as saved.
#[must_use]
pub fn mode_for(process: &str) -> Mode {
    crate::game_settings::load(process).map_or(Mode::Off, |s| s.mangohud)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_launch_options_get_the_layer_or_the_wrapper() {
        assert_eq!(launch_options("", Mode::On), "MANGOHUD=1 %command%");
        assert_eq!(launch_options("", Mode::Forced), "mangohud %command%");
        assert_eq!(launch_options("", Mode::Off), "");
    }

    #[test]
    fn the_users_own_options_are_kept_in_place() {
        let mine = "PROTON_LOG=1 gamemoderun %command% -dx12";
        assert_eq!(
            launch_options(mine, Mode::On),
            "MANGOHUD=1 PROTON_LOG=1 gamemoderun %command% -dx12"
        );
        assert_eq!(
            launch_options(mine, Mode::Forced),
            "PROTON_LOG=1 gamemoderun mangohud %command% -dx12"
        );
        // Plain arguments follow the command.
        assert_eq!(
            launch_options("-novid", Mode::On),
            "MANGOHUD=1 %command% -novid"
        );
    }

    #[test]
    fn switching_mode_replaces_what_this_module_added_and_off_removes_it() {
        let on = launch_options("gamemoderun %command%", Mode::On);
        let forced = launch_options(&on, Mode::Forced);
        assert_eq!(forced, "gamemoderun mangohud %command%");
        assert_eq!(launch_options(&forced, Mode::Off), "gamemoderun %command%");
        assert_eq!(launch_options(&launch_options("", Mode::On), Mode::Off), "");
        // Applying twice changes nothing.
        assert_eq!(launch_options(&on, Mode::On), on);
    }

    #[test]
    fn heroic_gets_its_own_switch_or_the_variable_and_keeps_everything_else() {
        // What Heroic writes after a setting was changed on the game's page.
        let file = r#"{
  "cb3bf": {
    "wineVersion": {"name": "Proton - GE-Proton-latest", "type": "proton"},
    "enviromentOptions": [{"key": "DXVK_HUD", "value": "fps"}],
    "showMangohud": false
  },
  "version": "v0",
  "explicit": true
}"#;
        let forced: serde_json::Value =
            serde_json::from_str(&heroic_config(file, "cb3bf", Mode::Forced).unwrap()).unwrap();
        assert_eq!(forced["cb3bf"]["showMangohud"], true);
        assert_eq!(forced["cb3bf"]["wineVersion"]["type"], "proton", "kept");
        assert_eq!(forced["cb3bf"]["enviromentOptions"][0]["key"], "DXVK_HUD");

        let on_text = heroic_config(file, "cb3bf", Mode::On).unwrap();
        let on: serde_json::Value = serde_json::from_str(&on_text).unwrap();
        assert_eq!(on["cb3bf"]["showMangohud"], false);
        let env = on["cb3bf"]["enviromentOptions"].as_array().unwrap();
        assert!(
            env.iter()
                .any(|e| e["key"] == "MANGOHUD" && e["value"] == "1")
        );
        assert!(env.iter().any(|e| e["key"] == "DXVK_HUD"));

        // Off takes back only what was added; twice is once.
        let off: serde_json::Value =
            serde_json::from_str(&heroic_config(&on_text, "cb3bf", Mode::Off).unwrap()).unwrap();
        let env = off["cb3bf"]["enviromentOptions"].as_array().unwrap();
        assert_eq!(env.len(), 1);
        assert_eq!(heroic_config(&on_text, "cb3bf", Mode::On).unwrap(), on_text);
    }

    #[test]
    fn a_heroic_game_without_a_settings_file_gets_only_the_two_keys() {
        // Heroic merges the game's keys over its defaults, so nothing else
        // (its Proton, its prefix) is frozen into the file.
        for current in ["", "{}"] {
            let v: serde_json::Value =
                serde_json::from_str(&heroic_config(current, "app", Mode::Forced).unwrap())
                    .unwrap();
            assert_eq!(v["app"].as_object().unwrap().len(), 2, "{v}");
            assert_eq!(
                (v["version"].as_str(), v["explicit"].as_bool()),
                (Some("v0"), Some(true))
            );
        }
        assert!(heroic_config("[1]", "app", Mode::On).is_err());
    }

    #[test]
    fn lutris_gets_its_own_option_and_nothing_else_moves() {
        // As Lutris writes a game with no system options.
        let plain = "game:\n  exe: /games/Game/run.sh\nname: Game\nrunner: linux\n";
        let on = lutris_config(plain, Mode::Forced);
        assert_eq!(on, format!("{plain}system:\n  mangohud: true\n"));
        assert_eq!(
            lutris_config(&on, Mode::On),
            on,
            "On is Lutris's own option too"
        );
        assert_eq!(
            lutris_config(&on, Mode::Off),
            plain,
            "the section it added goes"
        );

        // An existing section keeps its other options.
        let with = "game:\n  exe: /g/x.exe\nsystem:\n  env:\n    LANG: C\n  mangohud: false\nwine:\n  version: ge\n";
        let on = lutris_config(with, Mode::On);
        assert!(
            on.contains("system:\n  env:\n    LANG: C\n  mangohud: true\nwine:"),
            "{on}"
        );
        let off = lutris_config(&on, Mode::Off);
        assert_eq!(
            off,
            "game:\n  exe: /g/x.exe\nsystem:\n  env:\n    LANG: C\nwine:\n  version: ge\n"
        );

        // A flow mapping becomes a block.
        assert_eq!(
            lutris_config("system: {}\n", Mode::On),
            "system:\n  mangohud: true\n"
        );
        // Off with nothing to remove changes nothing.
        assert_eq!(lutris_config(plain, Mode::Off), plain);
    }

    #[test]
    fn the_runtime_branch_is_read_from_the_flatpak_metadata() {
        let meta = "[Application]\nname=com.heroicgameslauncher.hgl\nruntime=org.freedesktop.Platform/x86_64/25.08\nsdk=org.freedesktop.Sdk/x86_64/25.08\n";
        assert_eq!(flatpak_runtime_branch(meta).as_deref(), Some("25.08"));
        assert_eq!(flatpak_runtime_branch("[Application]\n"), None);
    }
}
