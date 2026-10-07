//! A game's own launch settings for a game Heroic starts.
//!
//! Heroic starts a game in its own process tree, which Big Game Mode cannot
//! wrap, so the game's own Gamescope, Wine FSR and vkBasalt
//! (`crate::game_launch::GameLaunch`) go where Heroic reads them: the
//! game's `GamesConfig/<app>.json`, an object keyed by the game's app name.
//! What Heroic 2.22 does with each key, as its launch code reads it:
//!
//! * **Gamescope** — the game's `gamescope` object. Heroic runs Gamescope
//!   when `enableUpscaling` or `enableLimiter` is on. `enableUpscaling`
//!   carries the sizes (`gameWidth`/`gameHeight` → `-w`/`-h`,
//!   `upscaleWidth`/`upscaleHeight` → `-W`/`-H`, digits as strings), the
//!   filter (`upscaleMethod`: `fsr`, `nis`, `integer`, `stretch`; anything
//!   else is no `-F`, Gamescope's plain scaling) and the window
//!   (`windowType`: `fullscreen` → `-f`). `enableLimiter` carries the frame
//!   limit (`fpsLimiter` → `-r`). Heroic has no sharpness: it goes into
//!   `additionalOptions`, which Heroic splits into Gamescope's arguments.
//!   Heroic's Flatpak finds `gamescope` only in Flathub's
//!   `org.freedesktop.Platform.VulkanLayer.gamescope` for its runtime; where
//!   that is missing, Heroic starts the game without Gamescope, so nothing
//!   of Gamescope's is written and the row says so.
//! * **Wine FSR** — Heroic's own switch, `enableFSR`. Heroic sets
//!   `WINE_FULLSCREEN_FSR` from it for every Wine game, after the game's
//!   variables, so a variable could not switch it: the switch is written.
//!   Its quality mode is a variable (`WINE_FULLSCREEN_FSR_MODE`).
//! * **vkBasalt** — `ENABLE_VKBASALT` in the game's variables,
//!   `enviromentOptions` (Heroic's spelling). Heroic's Flatpak needs
//!   Flathub's `org.freedesktop.Platform.VulkanLayer.vkBasalt`. Heroic
//!   gives the variables to the whole `gamescope … -- wine …` command, and
//!   Gamescope would load vkBasalt for itself and take `ENABLE_VKBASALT`
//!   from the game: with Gamescope, `DISABLE_VKBASALT=1` goes in the
//!   variables and `env -u DISABLE_VKBASALT ENABLE_VKBASALT=1` in front of
//!   the game, as a wrapper (`wrapperOptions`, which Heroic puts after
//!   Gamescope's `--`).
//! * **`MangoHud`** — Forced is Heroic's own switch (`showMangohud`, its
//!   `mangohud --dlsym` wrapper), On is `MANGOHUD=1` in the variables.
//!
//! Heroic merges a game's keys over its defaults one key at a time, so a
//! `gamescope` object or a variable list the game did not have is created
//! from Heroic's defaults (`config.json`), not from nothing: the game keeps
//! the defaults' other values. Everything else in the file stays as it is
//! (key order included), the file is backed up once before the first
//! change, and what Big Game Mode wrote is recorded in the game's settings
//! (`crate::game_settings`), so a value put back on "General configuration"
//! takes out exactly that — and a value the user has changed since in
//! Heroic is theirs and stays. Heroic keeps a game's settings in memory and
//! writes them back, so the file is written only while Heroic is closed.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::error::UserError;
use crate::game_launch::Size;
use crate::launchers::Launcher;
use crate::text::N_;

/// Flathub's Gamescope for Flatpak apps.
pub const GAMESCOPE_LAYER: &str = "org.freedesktop.Platform.VulkanLayer.gamescope";
/// Flathub's vkBasalt for Flatpak apps.
pub const VKBASALT_LAYER: &str = "org.freedesktop.Platform.VulkanLayer.vkBasalt";

/// A game's variables (Heroic's own spelling).
const ENV: &str = "enviromentOptions";
/// A game's Gamescope.
const GAMESCOPE: &str = "gamescope";
/// Heroic's Wine FSR switch.
const WINE_FSR: &str = "enableFSR";
/// Gamescope's extra arguments in Heroic.
const OPTIONS: &str = "additionalOptions";
/// Heroic's `MangoHud` switch.
const MANGOHUD: &str = "showMangohud";
/// Programs Heroic puts in front of the game (after Gamescope's `--`).
const WRAPPERS: &str = "wrapperOptions";

// ── What a game gets ────────────────────────────────────────────────────────

/// Gamescope as Heroic's `gamescope` object carries it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gamescope {
    /// `-w`/`-h`; `(0, 0)` is the game's own size.
    pub render: Size,
    /// `-W`/`-H`; `(0, 0)` is the render size.
    pub output: Size,
    /// `upscaleMethod`.
    pub method: &'static str,
    /// FSR or NIS sharpness, through `additionalOptions`.
    pub sharpness: Option<u8>,
    /// `-f`.
    pub fullscreen: bool,
    /// `-r`; 0 is none (Heroic's own limiter is left alone).
    pub frame_limit: u32,
}

impl Gamescope {
    /// The Gamescope Big Game Mode's own launch would run, in Heroic's terms.
    #[must_use]
    pub fn from_config(cfg: &crate::gamescope::Config) -> Self {
        use crate::gamescope::{Filter, FrameLimit};
        let size = |w: u32, h: u32| if w == 0 || h == 0 { (0, 0) } else { (w, h) };
        Self {
            render: size(cfg.render_width, cfg.render_height),
            output: size(cfg.output_width, cfg.output_height),
            method: match cfg.filter {
                Filter::Fsr => "fsr",
                Filter::Nis => "nis",
                Filter::Integer => "integer",
                // None of Heroic's: no `-F`, Gamescope's plain scaling.
                Filter::Linear | Filter::Nearest | Filter::Pixel => "linear",
            },
            sharpness: cfg.filter.uses_sharpness().then(|| cfg.clamped_sharpness()),
            fullscreen: cfg.fullscreen,
            frame_limit: match cfg.frame_limit {
                FrameLimit::NestedRefresh(hz) => hz,
                FrameLimit::None => 0,
            },
        }
    }

    /// The object's keys and their values. Heroic runs Gamescope with
    /// `-f` only under `enableUpscaling`, so that is on whenever Gamescope
    /// runs, the frame limit alone included.
    fn fields(&self) -> Vec<(&'static str, Value)> {
        let digits = |n: u32| if n == 0 { String::new() } else { n.to_string() };
        let mut out = vec![
            ("enableUpscaling", json!(true)),
            (
                "windowType",
                json!(if self.fullscreen {
                    "fullscreen"
                } else {
                    "windowed"
                }),
            ),
            ("gameWidth", json!(digits(self.render.0))),
            ("gameHeight", json!(digits(self.render.1))),
            ("upscaleWidth", json!(digits(self.output.0))),
            ("upscaleHeight", json!(digits(self.output.1))),
            ("upscaleMethod", json!(self.method)),
        ];
        if self.frame_limit > 0 {
            out.push(("enableLimiter", json!(true)));
            out.push(("fpsLimiter", json!(self.frame_limit.to_string())));
        }
        out
    }

    /// What goes into `additionalOptions`.
    fn options(&self) -> Option<String> {
        self.sharpness.map(|s| format!("--fsr-sharpness {s}"))
    }
}

/// What a game's Heroic settings should hold from Big Game Mode.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Wanted {
    /// Its Gamescope; `None` leaves Heroic's alone.
    pub gamescope: Option<Gamescope>,
    /// Heroic's Wine FSR switch; `None` leaves it alone.
    pub wine_fsr: Option<bool>,
    /// Gamescope or `OptiScaler` upscales the game: Heroic's Wine FSR is
    /// switched off where it is on (the game's or Heroic's defaults), so
    /// two upscalers never run in series. Where it is off, nothing is
    /// written.
    pub wine_fsr_off_where_on: bool,
    /// Variables, `(name, value)`.
    pub env: Vec<(String, String)>,
    /// Heroic's own `MangoHud` switch on (`MangoHud` forced).
    pub show_mangohud: bool,
    /// A program in front of the game, `(program, arguments)`.
    pub wrapper: Option<(String, String)>,
}

impl Wanted {
    /// Whether it asks for nothing.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.gamescope.is_none()
            && self.wine_fsr.is_none()
            && !self.wine_fsr_off_where_on
            && self.env.is_empty()
            && !self.show_mangohud
            && self.wrapper.is_none()
    }

    /// vkBasalt kept out of the Gamescope Heroic runs and in the game
    /// (`crate::launcher` does the same for its own launch).
    pub fn keep_vkbasalt_in_the_game(&mut self) {
        self.env.retain(|(k, _)| k != "DISABLE_VKBASALT");
        self.env
            .push(("DISABLE_VKBASALT".to_owned(), "1".to_owned()));
        self.wrapper = Some((
            "env".to_owned(),
            "-u DISABLE_VKBASALT ENABLE_VKBASALT=1".to_owned(),
        ));
    }
}

// ── What Big Game Mode wrote ──────────────────────────────────────────────────

/// One value Big Game Mode set: what was there before (`None`: nothing) and
/// what it wrote. JSON text, except a variable's, which is its value.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Owned {
    /// The key (or the variable's name).
    pub key: String,
    /// What was there before.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before: Option<String>,
    /// What Big Game Mode wrote.
    pub written: String,
}

/// What Big Game Mode wrote into one Heroic settings file, to change or take
/// out exactly that.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Written {
    /// The file.
    pub file: String,
    /// The game's keys it set (`enableFSR`).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub keys: Vec<Owned>,
    /// Keys of the game's `gamescope` object it set.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub gamescope: Vec<Owned>,
    /// Variables it set.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub env: Vec<Owned>,
    /// Words it added to Gamescope's `additionalOptions`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub options: Option<String>,
    /// Keys the game did not have (`gamescope`, `enviromentOptions`) that it
    /// created from Heroic's defaults: what was there, and what it created.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub created: Vec<Owned>,
    /// Entries it added to `wrapperOptions`, as JSON text.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub wrappers: Vec<String>,
}

impl Written {
    /// Whether Big Game Mode wrote nothing there.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
            && self.gamescope.is_empty()
            && self.env.is_empty()
            && self.options.is_none()
            && self.created.is_empty()
            && self.wrappers.is_empty()
    }
}

/// Heroic's defaults for every game (`config.json`'s `defaultSettings`),
/// which a key the game does not have falls back to.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Defaults {
    /// Its `gamescope` object, when it has one.
    pub gamescope: Option<Value>,
    /// Its variables.
    pub env: Vec<Value>,
    /// Its Wine FSR switch.
    pub wine_fsr: bool,
    /// Its wrappers.
    pub wrappers: Vec<Value>,
}

impl Defaults {
    /// Read them from Heroic's configuration folder.
    #[must_use]
    pub fn read(config_dir: &Path) -> Self {
        let Some(root) = std::fs::read_to_string(config_dir.join("config.json"))
            .ok()
            .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        else {
            return Self::default();
        };
        let d = &root["defaultSettings"];
        Self {
            gamescope: d.get(GAMESCOPE).filter(|g| g.is_object()).cloned(),
            env: d
                .get(ENV)
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default(),
            wine_fsr: d.get(WINE_FSR).and_then(Value::as_bool) == Some(true),
            wrappers: d
                .get(WRAPPERS)
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default(),
        }
    }
}

/// The `gamescope` object Heroic 2.22's settings page starts from when
/// there is none.
fn heroic_default_gamescope() -> Value {
    json!({
        "enableUpscaling": false,
        "enableLimiter": false,
        "enableForceGrabCursor": false,
        "windowType": "fullscreen",
        "gameWidth": "",
        "gameHeight": "",
        "upscaleHeight": "",
        "upscaleWidth": "",
        "upscaleMethod": "fsr",
        "fpsLimiter": "",
        "fpsLimiterNoFocus": "",
        "additionalOptions": ""
    })
}

// ── The file ────────────────────────────────────────────────────────────────

fn text(v: &Value) -> String {
    v.to_string()
}

fn env_key(e: &Value) -> Option<&str> {
    e.get("key").and_then(Value::as_str)
}

fn env_value(e: &Value) -> Option<&str> {
    e.get("value").and_then(Value::as_str)
}

/// Put `o.key` in `map` back to what it was before.
fn restore(map: &mut Map<String, Value>, o: &Owned) {
    match o.before.as_deref().map(serde_json::from_str::<Value>) {
        Some(Ok(v)) => {
            map.insert(o.key.clone(), v);
        }
        // Nothing there before (or a record that does not read).
        _ => {
            map.shift_remove(&o.key);
        }
    }
}

/// Take out what `owned` set in `map`, where it still holds what was
/// written; a value changed since is the user's.
fn undo_keys(map: &mut Map<String, Value>, owned: &[Owned]) {
    for o in owned.iter().rev() {
        if map.get(&o.key).map(text).as_deref() == Some(o.written.as_str()) {
            restore(map, o);
        }
    }
}

/// Set `key` in `map` to `value`, recording it when it changes.
fn set(map: &mut Map<String, Value>, key: &str, value: Value, owned: &mut Vec<Owned>) {
    let before = map.get(key);
    if before == Some(&value) {
        return;
    }
    owned.push(Owned {
        key: key.to_owned(),
        before: before.map(text),
        written: text(&value),
    });
    map.insert(key.to_owned(), value);
}

/// `options` with `words` (added by Big Game Mode) taken out once.
fn without_words(options: &str, words: &str) -> String {
    if options == words {
        return String::new();
    }
    for (pattern, with) in [
        (format!(" {words} "), " "),
        (format!(" {words}"), ""),
        (format!("{words} "), ""),
    ] {
        let found = if with.is_empty() && pattern.starts_with(' ') {
            options
                .ends_with(&pattern)
                .then(|| options.len() - pattern.len())
        } else if with.is_empty() {
            options.starts_with(&pattern).then_some(0)
        } else {
            options.find(&pattern)
        };
        if let Some(at) = found {
            let mut out = options.to_owned();
            out.replace_range(at..at + pattern.len(), with);
            return out;
        }
    }
    options.to_owned()
}

/// Take out of `game` what Big Game Mode wrote there before (`previous`),
/// where it still holds what was written.
fn undo(game: &mut Map<String, Value>, previous: &Written) {
    if let Some(env) = game.get_mut(ENV).and_then(Value::as_array_mut) {
        for o in previous.env.iter().rev() {
            let Some(i) = env.iter().position(|e| env_key(e) == Some(o.key.as_str())) else {
                continue;
            };
            if env_value(&env[i]) != Some(o.written.as_str()) {
                continue;
            }
            match (&o.before, env[i].as_object_mut()) {
                (Some(v), Some(entry)) => {
                    entry.insert("value".into(), json!(v));
                }
                _ => {
                    env.remove(i);
                }
            }
        }
    }
    if let Some(gs) = game.get_mut(GAMESCOPE).and_then(Value::as_object_mut) {
        if let Some(words) = &previous.options {
            if let Some(Value::String(opts)) = gs.get_mut(OPTIONS) {
                *opts = without_words(opts, words);
            }
        }
        undo_keys(gs, &previous.gamescope);
    }
    if let Some(list) = game.get_mut(WRAPPERS).and_then(Value::as_array_mut) {
        for w in previous.wrappers.iter().rev() {
            if let Some(i) = list.iter().position(|e| text(e) == *w) {
                list.remove(i);
            }
        }
    }
    undo_keys(game, &previous.keys);
    // What it created goes when nothing else in it changed since.
    undo_keys(game, &previous.created);
}

/// Put `wanted` into `game`, recording in `next` what changed.
fn redo(game: &mut Map<String, Value>, wanted: &Wanted, defaults: &Defaults, next: &mut Written) {
    if let Some(g) = &wanted.gamescope {
        if !game.get(GAMESCOPE).is_some_and(Value::is_object) {
            let base = defaults
                .gamescope
                .clone()
                .filter(Value::is_object)
                .unwrap_or_else(heroic_default_gamescope);
            next.created.push(Owned {
                key: GAMESCOPE.to_owned(),
                before: game.get(GAMESCOPE).map(text),
                written: text(&base),
            });
            game.insert(GAMESCOPE.to_owned(), base);
        }
        if let Some(gs) = game.get_mut(GAMESCOPE).and_then(Value::as_object_mut) {
            for (key, value) in g.fields() {
                set(gs, key, value, &mut next.gamescope);
            }
            if let Some(words) = g.options() {
                let mine = gs.get(OPTIONS).and_then(Value::as_str).unwrap_or_default();
                let joined = if mine.is_empty() {
                    words.clone()
                } else {
                    format!("{mine} {words}")
                };
                gs.insert(OPTIONS.to_owned(), json!(joined));
                next.options = Some(words);
            }
        }
    }
    if wanted.wine_fsr_off_where_on {
        let on = game
            .get(WINE_FSR)
            .map_or(defaults.wine_fsr, |v| v.as_bool() == Some(true));
        if on {
            set(game, WINE_FSR, json!(false), &mut next.keys);
        }
    } else if let Some(on) = wanted.wine_fsr {
        set(game, WINE_FSR, json!(on), &mut next.keys);
    }
    if wanted.show_mangohud {
        set(game, MANGOHUD, json!(true), &mut next.keys);
    }
    if let Some((program, args)) = &wanted.wrapper {
        if !game.get(WRAPPERS).is_some_and(Value::is_array) {
            let seed = Value::Array(defaults.wrappers.clone());
            next.created.push(Owned {
                key: WRAPPERS.to_owned(),
                before: game.get(WRAPPERS).map(text),
                written: text(&seed),
            });
            game.insert(WRAPPERS.to_owned(), seed);
        }
        if let Some(list) = game.get_mut(WRAPPERS).and_then(Value::as_array_mut) {
            let entry = json!({"exe": program, "args": args});
            if !list.contains(&entry) {
                next.wrappers.push(text(&entry));
                list.push(entry);
            }
        }
    }
    if !wanted.env.is_empty() {
        if !game.get(ENV).is_some_and(Value::is_array) {
            let seed = Value::Array(defaults.env.clone());
            next.created.push(Owned {
                key: ENV.to_owned(),
                before: game.get(ENV).map(text),
                written: text(&seed),
            });
            game.insert(ENV.to_owned(), seed);
        }
        if let Some(env) = game.get_mut(ENV).and_then(Value::as_array_mut) {
            for (key, value) in &wanted.env {
                let at = env.iter().position(|e| env_key(e) == Some(key.as_str()));
                let before = at.and_then(|i| env_value(&env[i]).map(str::to_owned));
                if before.as_deref() == Some(value.as_str()) {
                    continue;
                }
                match at.and_then(|i| env[i].as_object_mut()) {
                    Some(entry) => {
                        entry.insert("value".into(), json!(value));
                    }
                    None => env.push(json!({"key": key, "value": value})),
                }
                next.env.push(Owned {
                    key: key.clone(),
                    before,
                    written: value.clone(),
                });
            }
        }
    }
}

/// Heroic's per-game settings `current` for `app_name`, with what
/// Big Game Mode wrote before (`previous`) taken out and `wanted` put in,
/// and the record of what it wrote now.
///
/// # Errors
/// Returns an error, and nothing is to be written, when the file is not a
/// JSON object or the game's settings in it are not one.
pub fn transform(
    current: &str,
    app_name: &str,
    previous: &Written,
    wanted: &Wanted,
    defaults: &Defaults,
) -> Result<(String, Written)> {
    if previous.is_empty() && wanted.is_empty() {
        return Ok((current.to_owned(), Written::default()));
    }
    let mut root: Value = if current.trim().is_empty() {
        Value::Object(Map::new())
    } else {
        serde_json::from_str(current).map_err(|e| {
            UserError::with(
                N_("Heroic's settings for this game are not valid JSON (%s); nothing was changed"),
                [e.to_string()],
            )
        })?
    };
    let obj = root
        .as_object_mut()
        .ok_or_else(|| UserError::plain(N_("Heroic's game settings are not a JSON object")))?;
    if obj.is_empty() {
        // A new file, as Heroic writes one.
        obj.insert(app_name.to_owned(), Value::Object(Map::new()));
        obj.insert("version".into(), json!("v0"));
        obj.insert("explicit".into(), json!(true));
    }
    let game = obj
        .entry(app_name)
        .or_insert_with(|| Value::Object(Map::new()))
        .as_object_mut()
        .ok_or_else(|| {
            UserError::with(N_("Heroic's settings for %s are not an object"), [app_name])
        })?;

    // What Big Game Mode wrote before comes out first, then what is wanted
    // now goes in.
    undo(game, previous);
    let mut next = Written {
        file: previous.file.clone(),
        ..Written::default()
    };
    redo(game, wanted, defaults, &mut next);

    // Nothing changed: the file stays as it is, byte for byte.
    if serde_json::from_str::<Value>(current).ok().as_ref() == Some(&root) {
        return Ok((current.to_owned(), next));
    }
    // Heroic's own layout (`JSON.stringify(…, null, 2)`); a newline at the
    // end only where the file had one.
    let mut out = serde_json::to_string_pretty(&root)?;
    if current.ends_with('\n') {
        out.push('\n');
    }
    Ok((out, next))
}

// ── The games ───────────────────────────────────────────────────────────────

/// A game Heroic starts: where its settings are, and which Heroic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    /// Its app name in Heroic.
    pub app_name: String,
    /// Heroic's configuration folder.
    pub config_dir: PathBuf,
    /// The Heroic that reads it.
    pub launcher: Launcher,
}

impl Target {
    /// Its settings file.
    #[must_use]
    pub fn file(&self) -> PathBuf {
        self.config_dir
            .join("GamesConfig")
            .join(format!("{}.json", self.app_name))
    }

    /// Whether it is Heroic's Flatpak.
    #[must_use]
    pub fn flatpak(&self) -> bool {
        self.launcher.flatpak_id().is_some()
    }
}

/// The Heroic games whose process is `process`.
#[must_use]
pub fn targets(process: &str) -> Vec<Target> {
    let mut out: Vec<Target> = Vec::new();
    for g in crate::games::detect_all() {
        if g.profile_key() != process {
            continue;
        }
        let Some(
            reference @ crate::games::LauncherRef::Heroic {
                app_name,
                config_dir,
                ..
            },
        ) = &g.launcher
        else {
            continue;
        };
        let t = Target {
            app_name: app_name.clone(),
            config_dir: config_dir.clone(),
            launcher: Launcher::Heroic {
                flatpak: reference.flatpak_id().is_some(),
            },
        };
        if !out.iter().any(|o| o.file() == t.file()) {
            out.push(t);
        }
    }
    out
}

/// The command that installs Gamescope for Heroic's Flatpak, when it is
/// missing: without it Heroic starts the game without Gamescope.
#[must_use]
pub fn flatpak_gamescope_missing() -> Option<String> {
    crate::mangohud::missing_flatpak_layer(crate::launchers::HEROIC_FLATPAK, GAMESCOPE_LAYER)
}

/// The command that installs vkBasalt for Heroic's Flatpak, when it is
/// missing: without it `ENABLE_VKBASALT` does nothing there.
#[must_use]
pub fn flatpak_vkbasalt_missing() -> Option<String> {
    crate::mangohud::missing_flatpak_layer(crate::launchers::HEROIC_FLATPAK, VKBASALT_LAYER)
}

/// What writing a game's Heroic settings did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Applied {
    /// Not a game Heroic starts.
    NotHeroic,
    /// Nothing to change.
    Unchanged,
    /// Nothing written: this Heroic is open, and would write its own copy
    /// of the game's settings back over the change.
    HeroicRunning {
        /// The Heroic that is open.
        launcher: Launcher,
        /// It runs a game (or a task of its own) now, so it must not be
        /// closed for this.
        game_running: bool,
    },
    /// Written.
    Written,
}

/// One settings file's change, worked out before anything is written.
struct Plan<'a> {
    target: &'a Target,
    file: PathBuf,
    current: String,
    next: String,
    written: Written,
}

/// Write what `wanted` gives each Heroic game whose process is `process`
/// into its settings, taking out what Big Game Mode wrote there before.
///
/// # Errors
/// Returns an error, and writes nothing, when a settings file cannot be
/// read or is not JSON; also when one cannot be written or the game's
/// settings cannot be saved.
pub fn apply(process: &str, wanted: impl Fn(&Target) -> Wanted) -> Result<Applied> {
    let targets = targets(process);
    if targets.is_empty() {
        return Ok(Applied::NotHeroic);
    }
    let mut settings = crate::game_settings::load(process)?;
    let mut plans = Vec::new();
    for target in &targets {
        let file = target.file();
        let key = file.to_string_lossy().into_owned();
        let previous = settings
            .heroic
            .iter()
            .find(|w| w.file == key)
            .cloned()
            .unwrap_or_default();
        let current = match std::fs::read_to_string(&file) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(e).with_context(|| format!("read {}", file.display())),
        };
        let (next, mut written) = transform(
            &current,
            &target.app_name,
            &previous,
            &wanted(target),
            &Defaults::read(&target.config_dir),
        )?;
        written.file = key;
        plans.push(Plan {
            target,
            file,
            current,
            next,
            written,
        });
    }
    let changed = plans.iter().any(|p| p.next != p.current);
    for p in plans.iter().filter(|p| p.next != p.current) {
        let launcher = p.target.launcher;
        if crate::launchers::is_open(launcher) {
            return Ok(Applied::HeroicRunning {
                launcher,
                game_running: crate::launchers::ensure_launcher_idle(launcher).is_err(),
            });
        }
    }
    for p in plans.iter().filter(|p| p.next != p.current) {
        crate::mangohud::write_keeping_backup(&p.file, &p.next)?;
    }
    let files: Vec<String> = plans.iter().map(|p| p.written.file.clone()).collect();
    let mut records: Vec<Written> = settings
        .heroic
        .iter()
        .filter(|w| !files.contains(&w.file))
        .cloned()
        .collect();
    records.extend(
        plans
            .into_iter()
            .map(|p| p.written)
            .filter(|w| !w.is_empty()),
    );
    if records != settings.heroic {
        settings.heroic = records;
        crate::game_settings::save(process, &settings)?;
    }
    Ok(if changed {
        Applied::Written
    } else {
        Applied::Unchanged
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The Outer Worlds' file as Heroic 2.22 writes it after a setting
    /// changed on the game's page: the user's own variable, and keys
    /// Big Game Mode never touches.
    const FILE: &str = r#"{
  "cb3bf": {
    "wineVersion": {
      "name": "Proton - GE-Proton-latest",
      "type": "proton"
    },
    "enviromentOptions": [
      {
        "key": "DXVK_HUD",
        "value": "fps"
      }
    ],
    "showMangohud": false,
    "maxSharpness": 3
  },
  "version": "v0",
  "explicit": true
}"#;

    fn gamescope() -> Gamescope {
        Gamescope {
            render: (1280, 720),
            output: (2560, 1440),
            method: "fsr",
            sharpness: Some(5),
            fullscreen: true,
            frame_limit: 60,
        }
    }

    fn wanted() -> Wanted {
        Wanted {
            gamescope: Some(gamescope()),
            wine_fsr: Some(true),
            env: vec![
                ("WINE_FULLSCREEN_FSR_MODE".into(), "quality".into()),
                ("ENABLE_VKBASALT".into(), "1".into()),
            ],
            ..Wanted::default()
        }
    }

    #[test]
    fn a_second_upscaler_switches_heroics_wine_fsr_off_only_where_it_is_on() {
        let upscaled = Wanted {
            gamescope: Some(gamescope()),
            wine_fsr_off_where_on: true,
            ..Wanted::default()
        };
        // Off already (absent, and off in Heroic's defaults): not written.
        let (out, written) = transform(
            FILE,
            "cb3bf",
            &Written::default(),
            &upscaled,
            &Defaults::default(),
        )
        .unwrap();
        assert!(json(&out)["cb3bf"].get("enableFSR").is_none(), "{out}");
        assert!(written.keys.is_empty());
        // On in Heroic's defaults, or in the game's own: switched off, and
        // back when the game no longer upscales.
        let on = Defaults {
            wine_fsr: true,
            ..Defaults::default()
        };
        let (out, written) = transform(FILE, "cb3bf", &Written::default(), &upscaled, &on).unwrap();
        assert_eq!(json(&out)["cb3bf"]["enableFSR"], false);
        let (back, _) = transform(&out, "cb3bf", &written, &Wanted::default(), &on).unwrap();
        assert_eq!(back, FILE);
        let mine = FILE.replace(
            "\"maxSharpness\": 3",
            "\"maxSharpness\": 3,\n    \"enableFSR\": true",
        );
        let (out, _) = transform(
            &mine,
            "cb3bf",
            &Written::default(),
            &upscaled,
            &Defaults::default(),
        )
        .unwrap();
        assert_eq!(json(&out)["cb3bf"]["enableFSR"], false);
    }

    fn json(text: &str) -> Value {
        serde_json::from_str(text).unwrap()
    }

    #[test]
    fn a_game_gets_its_values_and_keeps_everything_else() {
        let (out, written) = transform(
            FILE,
            "cb3bf",
            &Written::default(),
            &wanted(),
            &Defaults::default(),
        )
        .unwrap();
        let v = json(&out);
        let game = &v["cb3bf"];
        assert_eq!(game["wineVersion"]["type"], "proton", "kept");
        assert_eq!(game["maxSharpness"], 3, "kept");
        assert_eq!(game["enableFSR"], true);
        let env = game["enviromentOptions"].as_array().unwrap();
        assert_eq!(env[0], json(r#"{"key": "DXVK_HUD", "value": "fps"}"#));
        assert!(
            env.iter()
                .any(|e| e["key"] == "ENABLE_VKBASALT" && e["value"] == "1")
        );
        let gs = &game["gamescope"];
        assert_eq!(gs["enableUpscaling"], true);
        assert_eq!(
            (gs["gameWidth"].as_str(), gs["gameHeight"].as_str()),
            (Some("1280"), Some("720"))
        );
        assert_eq!(gs["upscaleWidth"], "2560");
        assert_eq!(gs["upscaleMethod"], "fsr");
        assert_eq!(
            (gs["enableLimiter"].as_bool(), gs["fpsLimiter"].as_str()),
            (Some(true), Some("60"))
        );
        assert_eq!(gs["additionalOptions"], "--fsr-sharpness 5");
        assert_eq!(gs["enableForceGrabCursor"], false, "Heroic's default");
        // The order Heroic wrote stays: the game first, then its own keys.
        let keys: Vec<&String> = v.as_object().unwrap().keys().collect();
        assert_eq!(keys, ["cb3bf", "version", "explicit"]);
        let own: Vec<&String> = game.as_object().unwrap().keys().collect();
        assert_eq!(
            &own[..4],
            [
                "wineVersion",
                "enviromentOptions",
                "showMangohud",
                "maxSharpness"
            ]
        );
        assert!(!written.is_empty());
        assert!(
            !out.ends_with('\n'),
            "Heroic's file had no newline at the end"
        );
    }

    #[test]
    fn general_configuration_takes_out_exactly_what_was_written() {
        let (on, written) = transform(
            FILE,
            "cb3bf",
            &Written::default(),
            &wanted(),
            &Defaults::default(),
        )
        .unwrap();
        let (off, left) = transform(
            &on,
            "cb3bf",
            &written,
            &Wanted::default(),
            &Defaults::default(),
        )
        .unwrap();
        assert_eq!(off, FILE, "byte for byte");
        assert!(left.is_empty());
        // Applying the same twice changes nothing.
        let (again, same) =
            transform(&on, "cb3bf", &written, &wanted(), &Defaults::default()).unwrap();
        assert_eq!(again, on);
        assert_eq!(same, written);
    }

    #[test]
    fn a_changed_value_replaces_the_old_one_and_only_that() {
        let (on, written) = transform(
            FILE,
            "cb3bf",
            &Written::default(),
            &wanted(),
            &Defaults::default(),
        )
        .unwrap();
        let mut other = wanted();
        other.gamescope = Some(Gamescope {
            render: (1920, 1080),
            sharpness: Some(2),
            frame_limit: 0,
            ..gamescope()
        });
        other.env = vec![("ENABLE_VKBASALT".into(), "0".into())];
        let (changed, written) =
            transform(&on, "cb3bf", &written, &other, &Defaults::default()).unwrap();
        let v = json(&changed);
        let gs = &v["cb3bf"]["gamescope"];
        assert_eq!(gs["gameWidth"], "1920");
        assert_eq!(
            gs["additionalOptions"], "--fsr-sharpness 2",
            "not added twice"
        );
        assert_eq!(gs["enableLimiter"], false, "Heroic's default again");
        let env = v["cb3bf"]["enviromentOptions"].as_array().unwrap();
        assert_eq!(env.len(), 2, "{env:?}");
        assert!(
            env.iter()
                .any(|e| e["key"] == "ENABLE_VKBASALT" && e["value"] == "0")
        );
        // And back.
        let (off, _) = transform(
            &changed,
            "cb3bf",
            &written,
            &Wanted::default(),
            &Defaults::default(),
        )
        .unwrap();
        assert_eq!(off, FILE);
    }

    #[test]
    fn what_the_user_changed_in_heroic_since_is_theirs() {
        let (on, written) = transform(
            FILE,
            "cb3bf",
            &Written::default(),
            &wanted(),
            &Defaults::default(),
        )
        .unwrap();
        // In Heroic: a width of their own, a sharper grab, their own options.
        let mut v = json(&on);
        v["cb3bf"]["gamescope"]["gameWidth"] = json!("1600");
        v["cb3bf"]["gamescope"]["enableForceGrabCursor"] = json!(true);
        v["cb3bf"]["gamescope"]["additionalOptions"] = json!("--hdr-enabled --fsr-sharpness 5");
        v["cb3bf"]["enableFSR"] = json!(false);
        let edited = serde_json::to_string_pretty(&v).unwrap();
        let (off, _) = transform(
            &edited,
            "cb3bf",
            &written,
            &Wanted::default(),
            &Defaults::default(),
        )
        .unwrap();
        let v = json(&off);
        let gs = &v["cb3bf"]["gamescope"];
        assert_eq!(gs["gameWidth"], "1600", "theirs");
        assert_eq!(gs["gameHeight"], "", "ours, back to Heroic's default");
        assert_eq!(gs["additionalOptions"], "--hdr-enabled");
        assert_eq!(gs["enableForceGrabCursor"], true);
        assert_eq!(v["cb3bf"]["enableFSR"], false, "theirs");
        let env = v["cb3bf"]["enviromentOptions"].as_array().unwrap();
        assert_eq!(env.len(), 1, "ours out, theirs in");
    }

    #[test]
    fn a_variable_the_user_had_gets_its_value_back() {
        let file = r#"{"g": {"enviromentOptions": [{"key": "ENABLE_VKBASALT", "value": "1"}]}}"#;
        let off = Wanted {
            env: vec![("ENABLE_VKBASALT".into(), "0".into())],
            ..Wanted::default()
        };
        let (out, written) =
            transform(file, "g", &Written::default(), &off, &Defaults::default()).unwrap();
        assert_eq!(json(&out)["g"]["enviromentOptions"][0]["value"], "0");
        let (back, _) = transform(
            &out,
            "g",
            &written,
            &Wanted::default(),
            &Defaults::default(),
        )
        .unwrap();
        assert_eq!(json(&back)["g"]["enviromentOptions"][0]["value"], "1");
        // The same value as the user's is not Big Game Mode's to take out.
        let on = Wanted {
            env: vec![("ENABLE_VKBASALT".into(), "1".into())],
            ..Wanted::default()
        };
        let (same, written) =
            transform(file, "g", &Written::default(), &on, &Defaults::default()).unwrap();
        assert_eq!(same, file);
        assert!(written.is_empty());
    }

    #[test]
    fn a_game_without_its_own_keys_starts_from_heroics_defaults() {
        let defaults = Defaults {
            gamescope: Some(json!({"enableUpscaling": false, "additionalOptions": "--mine"})),
            env: vec![json!({"key": "PROTON_LOG", "value": "1"})],
            wine_fsr: false,
            wrappers: Vec::new(),
        };
        for current in [
            "",
            "{}",
            r#"{"app": {}, "version": "v0", "explicit": true}"#,
        ] {
            let (out, written) =
                transform(current, "app", &Written::default(), &wanted(), &defaults).unwrap();
            let v = json(&out);
            assert_eq!(
                (v["version"].as_str(), v["explicit"].as_bool()),
                (Some("v0"), Some(true))
            );
            let env = v["app"]["enviromentOptions"].as_array().unwrap();
            assert_eq!(env[0]["key"], "PROTON_LOG", "the default's variables stay");
            assert_eq!(
                v["app"]["gamescope"]["additionalOptions"],
                "--mine --fsr-sharpness 5"
            );
            // Taken out, the game falls back to the defaults again.
            let (off, _) = transform(&out, "app", &written, &Wanted::default(), &defaults).unwrap();
            let v = json(&off);
            assert_eq!(v["app"], json!({}), "{off}");
        }
    }

    #[test]
    fn bad_json_is_refused_not_overwritten() {
        for bad in ["{\"g\": ", "[1]", r#"{"g": 3}"#] {
            assert!(
                transform(
                    bad,
                    "g",
                    &Written::default(),
                    &wanted(),
                    &Defaults::default()
                )
                .is_err(),
                "{bad}"
            );
        }
        // With nothing to do, the file is not even read as JSON.
        let (same, _) = transform(
            "not json",
            "g",
            &Written::default(),
            &Wanted::default(),
            &Defaults::default(),
        )
        .unwrap();
        assert_eq!(same, "not json");
    }

    #[test]
    fn the_filter_maps_to_heroics_methods() {
        use crate::gamescope::{Config, Filter, FrameLimit};
        let cfg = Config {
            render_width: 1280,
            render_height: 720,
            filter: Filter::Nis,
            sharpness: 30,
            frame_limit: FrameLimit::NestedRefresh(72),
            ..Config::default()
        };
        let g = Gamescope::from_config(&cfg);
        assert_eq!(
            (g.method, g.sharpness, g.frame_limit),
            ("nis", Some(20), 72)
        );
        assert_eq!(g.output, (0, 0));
        let plain = Gamescope::from_config(&Config {
            filter: Filter::Integer,
            ..Config::default()
        });
        assert_eq!((plain.method, plain.sharpness), ("integer", None));
        assert_eq!(Gamescope::from_config(&Config::default()).method, "linear");
    }

    #[test]
    fn options_lose_only_the_words_bigame_mode_added() {
        let w = "--fsr-sharpness 5";
        assert_eq!(without_words(w, w), "");
        assert_eq!(without_words(&format!("--hdr {w}"), w), "--hdr");
        assert_eq!(without_words(&format!("{w} --hdr"), w), "--hdr");
        assert_eq!(without_words(&format!("-a {w} -b"), w), "-a -b");
        assert_eq!(without_words("--fsr-sharpness 50", w), "--fsr-sharpness 50");
    }

    #[test]
    fn the_file_is_written_with_a_backup_and_read_back() {
        let dir = crate::tests::tempdir("heroic_launch");
        let file = dir.join("GamesConfig/cb3bf.json");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, FILE).unwrap();
        let (next, _) = transform(
            FILE,
            "cb3bf",
            &Written::default(),
            &wanted(),
            &Defaults::default(),
        )
        .unwrap();
        let backups = dir.join("launcher-backups");
        crate::mangohud::write_keeping_backup_in(&file, &next, &backups).unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), next);
        // A second change keeps the first backup: the user's own file.
        crate::mangohud::write_keeping_backup_in(&file, FILE, &backups).unwrap();
        let kept: Vec<_> = std::fs::read_dir(&backups).unwrap().flatten().collect();
        assert_eq!(kept.len(), 1);
        assert_eq!(std::fs::read_to_string(kept[0].path()).unwrap(), FILE);
    }

    #[test]
    fn the_record_reads_back_from_the_games_settings() {
        let (_, written) = transform(
            FILE,
            "cb3bf",
            &Written::default(),
            &wanted(),
            &Defaults::default(),
        )
        .unwrap();
        let settings = crate::game_settings::GameSettings {
            heroic: vec![Written {
                file: "/h/GamesConfig/cb3bf.json".into(),
                ..written
            }],
            ..crate::game_settings::GameSettings::default()
        };
        let text = toml::to_string_pretty(&settings).unwrap();
        let back: crate::game_settings::GameSettings = toml::from_str(&text).unwrap();
        assert_eq!(back, settings, "{text}");
    }

    #[test]
    fn vkbasalt_goes_into_the_game_heroic_runs_in_gamescope_and_comes_out_again() {
        let file = r#"{
  "g": {
    "wineVersion": {"name": "Proton - GE-Proton-latest", "type": "proton"}
  },
  "version": "v0",
  "explicit": true
}"#;
        let defaults = Defaults {
            wrappers: vec![json!({"exe": "gamemoderun", "args": ""})],
            ..Defaults::default()
        };
        let mut wanted = Wanted {
            gamescope: Some(Gamescope::from_config(&crate::gamescope::Config::default())),
            env: vec![("ENABLE_VKBASALT".into(), "1".into())],
            ..Wanted::default()
        };
        wanted.keep_vkbasalt_in_the_game();
        let (out, written) = transform(file, "g", &Written::default(), &wanted, &defaults).unwrap();
        let v = json(&out);
        let env = v["g"]["enviromentOptions"].as_array().unwrap();
        assert!(
            env.iter()
                .any(|e| e["key"] == "DISABLE_VKBASALT" && e["value"] == "1")
        );
        // The defaults' wrapper stays, ours goes after it, after `--`.
        assert_eq!(
            v["g"]["wrapperOptions"],
            json!([
                {"exe": "gamemoderun", "args": ""},
                {"exe": "env", "args": "-u DISABLE_VKBASALT ENABLE_VKBASALT=1"}
            ])
        );
        // Taken out, the file is as it was.
        let (back, _) = transform(&out, "g", &written, &Wanted::default(), &defaults).unwrap();
        assert_eq!(json(&back), json(file));
        // A record written by an older version reads.
        let old: Written = serde_json::from_str(r#"{"file": "x", "keys": []}"#).unwrap();
        assert!(old.wrappers.is_empty());
    }
}
