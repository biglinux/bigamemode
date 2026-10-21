//! vkBasalt in games, and in nothing else of the session.
//!
//! vkBasalt is an implicit Vulkan layer that `ENABLE_VKBASALT=1` switches on.
//! Put in the session's environment for every game, that switch reached
//! every Vulkan program of the session instead: a local AI's `llama-server`,
//! `vulkaninfo`, Steam's shader pre-compiler. vkBasalt 0.3.2.10 crashes a
//! program that makes a Vulkan device to compute rather than to show
//! (SIGSEGV in its `vkCreateDevice` and `vkDestroyDevice`, with
//! `llama-server` and `vulkaninfo` on the reference desktop), so a local AI
//! could not load a model while Tuning's vkBasalt was on.
//!
//! The session never carries that switch now. vkBasalt for every game is a
//! copy of vkBasalt's own manifest in the user's `implicit_layer.d`, which
//! the Vulkan loader enables only in a game the Steam client starts: Steam
//! sets `SteamEnv=1` there, and not in its own environment or its web
//! helper's — the way Steam's own overlay layer is switched on. The Steam
//! Runtime's container takes the user's layers in, as it does that overlay.
//!
//! Any `ENABLE_VKBASALT` a game has disables the copy (the loader disables a
//! layer whose `disable_environment` variable is set to any value, checked
//! with loader 1.4.357): a game's own `1`, from its profile's launch options,
//! loads vkBasalt once, through its system manifest, and its own `0` keeps
//! it out. Games Big Game Mode starts itself still get `ENABLE_VKBASALT`
//! for that one launch ([`crate::launcher`]).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde_json::{Value, json};

/// vkBasalt's own switch.
pub const SWITCH: &str = "ENABLE_VKBASALT";

/// What the Steam client puts in the environment of a game it starts, and
/// not in its own (read from `/proc/<pid>/environ` on the reference desktop).
const GAME_MARK: (&str, &str) = ("SteamEnv", "1");

/// How the copies' file names start: they are found by it, and it is not
/// `vkBasalt`, so a copy never passes for an installed vkBasalt
/// ([`crate::capabilities::vkbasalt_installed`]).
const PREFIX: &str = "bigame-mode-";

/// The user's implicit layers, which the loader reads before the system's.
fn dir() -> PathBuf {
    crate::paths::data_home()
        .join("vulkan")
        .join("implicit_layer.d")
}

/// Whether the games the Steam client starts get vkBasalt.
#[must_use]
pub fn is_on() -> bool {
    !ours(&dir()).is_empty()
}

/// Give vkBasalt to the games the Steam client starts (`on`), or take it
/// away. With vkBasalt not installed there is nothing to copy, and nothing
/// is put in.
///
/// # Errors
/// Returns an error when a manifest cannot be read, written or removed.
pub fn sync(on: bool) -> Result<()> {
    let mut sources = crate::capabilities::system_layer_dirs();
    // A vkBasalt installed for this user only, which
    // `crate::capabilities::vkbasalt_installed` accepts too. The copies are
    // not taken for it: their names do not start with `vkBasalt`.
    sources.push(dir());
    sync_in(&dir(), &sources, on)
}

fn sync_in(dir: &Path, system_dirs: &[PathBuf], on: bool) -> Result<()> {
    let mut wanted = Vec::new();
    if on {
        for system in system_manifests(system_dirs) {
            let text = std::fs::read_to_string(&system)
                .with_context(|| format!("read {}", system.display()))?;
            let copy = for_games(&text, &system)
                .with_context(|| format!("vkBasalt's manifest {}", system.display()))?;
            let name = system.file_name().unwrap_or_default().to_string_lossy();
            wanted.push((dir.join(format!("{PREFIX}{name}")), copy));
        }
    }
    for path in ours(dir) {
        if !wanted.iter().any(|(p, _)| *p == path) {
            std::fs::remove_file(&path).with_context(|| format!("remove {}", path.display()))?;
        }
    }
    for (path, text) in wanted {
        if std::fs::read_to_string(&path).is_ok_and(|t| t == text) {
            continue;
        }
        crate::turbo_preset::write_atomic(&path, &text)?;
    }
    Ok(())
}

/// The copies in `dir`.
fn ours(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            is_json(p)
                && p.file_name()
                    .is_some_and(|n| n.to_string_lossy().starts_with(PREFIX))
        })
        .collect()
}

/// Whether `path` names a JSON file, as the loader reads them.
fn is_json(path: &Path) -> bool {
    path.extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("json"))
}

/// vkBasalt's manifests in the system's folders, one per file name (a
/// 32-bit build has its own), the first folder's when two have it.
fn system_manifests(dirs: &[PathBuf]) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = Vec::new();
    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };
        let mut here: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                is_json(p)
                    && p.file_name().is_some_and(|n| {
                        n.to_string_lossy()
                            .to_ascii_lowercase()
                            .starts_with("vkbasalt")
                    })
            })
            .collect();
        here.sort();
        for path in here {
            if !found.iter().any(|f| f.file_name() == path.file_name()) {
                found.push(path);
            }
        }
    }
    found
}

/// vkBasalt's manifest `text`, read from `from`, as the copy for games: its
/// own library and entry points, under a name of its own, enabled by
/// [`GAME_MARK`] and disabled by any [`SWITCH`].
fn for_games(text: &str, from: &Path) -> Result<String> {
    let mut manifest: Value = serde_json::from_str(text).context("not JSON")?;
    let layer = manifest
        .get_mut("layer")
        .and_then(Value::as_object_mut)
        .context("no \"layer\" in it")?;
    let name = layer
        .get("name")
        .and_then(Value::as_str)
        .context("the layer has no name")?
        .to_owned();
    // A relative path with a folder in it is relative to the manifest, which
    // the copy is not beside; a bare file name is the library search's.
    if let Some(library) = layer.get("library_path").and_then(Value::as_str)
        && library.contains('/')
        && Path::new(library).is_relative()
        && let Some(folder) = from.parent()
    {
        let absolute = folder.join(library).to_string_lossy().into_owned();
        layer.insert("library_path".into(), absolute.into());
    }
    layer.insert("name".into(), format!("{name}_games").into());
    layer.insert(
        "description".into(),
        "vkBasalt in the games the Steam client starts (Big Game Mode)".into(),
    );
    layer.insert(
        "enable_environment".into(),
        json!({ GAME_MARK.0: GAME_MARK.1 }),
    );
    layer.insert("disable_environment".into(), json!({ SWITCH: "1" }));
    Ok(serde_json::to_string_pretty(&manifest)? + "\n")
}

/// `env` as the games started from it see vkBasalt's switch: one it does
/// not set comes from the copy, when it is there (`on`).
pub fn in_games<S: std::hash::BuildHasher>(env: &mut HashMap<String, String, S>, on: bool) {
    if on && !env.contains_key(SWITCH) {
        env.insert(SWITCH.to_owned(), "1".to_owned());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// vkBasalt 0.3.2.10's manifest, as Arch Linux installs it.
    const SYSTEM: &str = r#"{
  "file_format_version" : "1.0.0",
  "layer" : {
    "name": "VK_LAYER_VKBASALT_post_processing",
    "type": "GLOBAL",
    "library_path": "libvkbasalt.so",
    "api_version": "1.3.223",
    "implementation_version": "1",
    "description": "a post processing layer",
    "functions": {
      "vkGetInstanceProcAddr": "vkBasalt_GetInstanceProcAddr",
      "vkGetDeviceProcAddr":   "vkBasalt_GetDeviceProcAddr"
    },
    "enable_environment": {
      "ENABLE_VKBASALT": "1"
    },
    "disable_environment": {
      "DISABLE_VKBASALT": "1"
    }
  }
}"#;

    fn temp(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "bigame_vkbasalt_layer_{tag}_{}_{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn the_copy_is_switched_on_by_steams_games_only() {
        let copy: Value = serde_json::from_str(
            &for_games(SYSTEM, Path::new("/usr/share/x/vkBasalt.json")).unwrap(),
        )
        .unwrap();
        let layer = &copy["layer"];
        assert_eq!(layer["name"], "VK_LAYER_VKBASALT_post_processing_games");
        assert_eq!(layer["enable_environment"], json!({ "SteamEnv": "1" }));
        assert_eq!(
            layer["disable_environment"],
            json!({ "ENABLE_VKBASALT": "1" })
        );
        // vkBasalt's own library and entry points.
        assert_eq!(layer["library_path"], "libvkbasalt.so");
        assert_eq!(
            layer["functions"]["vkGetDeviceProcAddr"],
            "vkBasalt_GetDeviceProcAddr"
        );
        // A library named relative to its manifest stays the same file.
        let relative = SYSTEM.replace("libvkbasalt.so", "../../../lib/libvkbasalt.so");
        let copy: Value = serde_json::from_str(
            &for_games(
                &relative,
                Path::new("/usr/share/vulkan/implicit_layer.d/vkBasalt.json"),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            copy["layer"]["library_path"],
            "/usr/share/vulkan/implicit_layer.d/../../../lib/libvkbasalt.so"
        );
        assert!(for_games("{}", Path::new("/x.json")).is_err());
    }

    #[test]
    fn the_copies_follow_the_switch_and_the_installed_vkbasalt() {
        let root = temp("sync");
        let (user, system) = (root.join("user"), root.join("system"));
        std::fs::create_dir_all(&system).unwrap();
        std::fs::write(system.join("vkBasalt.json"), SYSTEM).unwrap();
        std::fs::write(system.join("MangoHud.x86_64.json"), "{}").unwrap();
        let systems = [system.clone()];

        sync_in(&user, &systems, true).unwrap();
        let copies = ours(&user);
        assert_eq!(copies, [user.join("bigame-mode-vkBasalt.json")]);
        // Written once: a second sync leaves the same file.
        let written = std::fs::read_to_string(&copies[0]).unwrap();
        sync_in(&user, &systems, true).unwrap();
        assert_eq!(std::fs::read_to_string(&copies[0]).unwrap(), written);
        // The user's other layers stay.
        std::fs::write(user.join("steamoverlay_x86_64.json"), "{}").unwrap();

        sync_in(&user, &systems, false).unwrap();
        assert!(ours(&user).is_empty());
        assert!(user.join("steamoverlay_x86_64.json").exists());

        // vkBasalt uninstalled: nothing to copy, and a copy left goes.
        sync_in(&user, &systems, true).unwrap();
        std::fs::remove_file(system.join("vkBasalt.json")).unwrap();
        sync_in(&user, &systems, true).unwrap();
        assert!(ours(&user).is_empty());

        // A vkBasalt installed for this user only, beside the copy: copied,
        // and the copy is never taken for a vkBasalt of its own.
        std::fs::write(user.join("vkBasalt.json"), SYSTEM).unwrap();
        let sources = [system.clone(), user.clone()];
        sync_in(&user, &sources, true).unwrap();
        assert_eq!(ours(&user), [user.join("bigame-mode-vkBasalt.json")]);
        sync_in(&user, &sources, true).unwrap();
        assert_eq!(ours(&user).len(), 1);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_games_own_switch_wins_over_the_copy() {
        let mut env = HashMap::from([("HOME".to_owned(), "/h".to_owned())]);
        in_games(&mut env, false);
        assert!(!env.contains_key(SWITCH));
        in_games(&mut env, true);
        assert_eq!(env[SWITCH], "1");
        let mut off = HashMap::from([(SWITCH.to_owned(), "0".to_owned())]);
        in_games(&mut off, true);
        assert_eq!(off[SWITCH], "0");
    }
}
