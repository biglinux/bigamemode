//! System tray integration via `StatusNotifierItem` (KDE/freedesktop).
//!
//! A small remote for the application: open the window, switch Turbo, pick
//! the Turbo preset, quit. The tray keeps no state of its own. What it shows
//! comes from Home's `app.turbo` and `app.turbo-preset` actions (forwarded
//! by [`TrayHandle`]), and what it asks for goes back through the same
//! actions, so Home, the tray and falcond cannot disagree.
//!
//! The icon is `bigamemode-symbolic`, given by name and never as a pixmap:
//! its SVG carries the `ColorScheme-Text` stylesheet, which Plasma fills
//! with the panel's text colour and fills again when the colour scheme
//! changes. A pixmap would carry one colour for every panel.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::mpsc;

use ksni::blocking::TrayMethods;

use bigame_core::turbo_preset::{self, Preset};

use crate::i18n::i18n;

use crate::app::NAME;

/// The icon, by name.
const ICON: &str = "bigamemode-symbolic";

/// Actions the tray can request from the GTK main loop.
#[derive(Debug, Clone)]
pub enum TrayAction {
    /// Show the window.
    Activate,
    /// Switch Turbo on or off, as Home's button does.
    SetTurbo(bool),
    /// Choose the Turbo preset, as Home's picker does.
    SetPreset(Preset),
    /// Quit the application.
    Quit,
}

/// What the tray shows.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Shown {
    /// Turbo is on.
    turbo_on: bool,
    /// Turbo can be switched now (not in the middle of switching).
    turbo_enabled: bool,
    /// The preset in force, or the one chosen for the next Turbo.
    preset: Preset,
    /// Home takes a preset now (it is not switching one).
    preset_home: bool,
    /// What is wrong, when something is.
    warning: Option<String>,
}

impl Default for Shown {
    fn default() -> Self {
        Self {
            turbo_on: false,
            // Until Home has read Turbo's state, nothing can be switched.
            turbo_enabled: false,
            preset: turbo_preset::chosen(),
            preset_home: false,
            warning: None,
        }
    }
}

impl Shown {
    /// A preset can be chosen from the tray: only while Turbo is on and
    /// settled, since a preset belongs to Turbo.
    fn preset_enabled(&self) -> bool {
        self.turbo_on && self.turbo_enabled && self.preset_home
    }

    /// The line under the name in the tooltip.
    fn description(&self) -> String {
        if let Some(w) = &self.warning {
            return w.clone();
        }
        match (self.turbo_enabled, self.turbo_on) {
            // While switching, the state is still the one it is leaving.
            (false, false) => i18n("Turning On"),
            (false, true) => i18n("Turning Off"),
            (true, true) => format!(
                "{} · {}",
                i18n("Turbo mode activated"),
                i18n(self.preset.label())
            ),
            (true, false) => i18n("Turbo mode inactive"),
        }
    }
}

struct BiGameTray {
    tx: mpsc::Sender<TrayAction>,
    shown: Shown,
    /// Where the icon is when it is not in the installed theme.
    theme_path: String,
}

impl ksni::Tray for BiGameTray {
    fn id(&self) -> String {
        String::from("bigame-mode")
    }

    fn title(&self) -> String {
        NAME.to_owned()
    }

    fn icon_name(&self) -> String {
        ICON.to_owned()
    }

    fn icon_theme_path(&self) -> String {
        self.theme_path.clone()
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip {
            title: NAME.to_owned(),
            description: self.shown.description(),
            icon_name: ICON.to_owned(),
            icon_pixmap: Vec::new(),
        }
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        notify(&self.tx, TrayAction::Activate);
    }

    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        use ksni::menu::{CheckmarkItem, RadioGroup, RadioItem, StandardItem};

        let shown = &self.shown;
        let mut items: Vec<ksni::MenuItem<Self>> = vec![
            // The menu's title: nothing to click.
            StandardItem {
                label: NAME.to_owned(),
                enabled: false,
                ..Default::default()
            }
            .into(),
        ];
        if let Some(w) = &shown.warning {
            items.push(
                StandardItem {
                    label: w.clone(),
                    icon_name: "dialog-warning-symbolic".into(),
                    enabled: false,
                    ..Default::default()
                }
                .into(),
            );
        }
        items.push(
            StandardItem {
                label: i18n("Open Big Game Mode"),
                icon_name: "view-restore-symbolic".into(),
                activate: Box::new(|tray: &mut Self| notify(&tray.tx, TrayAction::Activate)),
                ..Default::default()
            }
            .into(),
        );
        items.push(ksni::MenuItem::Separator);
        items.push(
            CheckmarkItem {
                label: i18n("Turbo mode"),
                enabled: shown.turbo_enabled,
                checked: shown.turbo_on,
                // The request, not the result: the check follows Turbo's
                // real state once Home has switched it.
                activate: Box::new(|tray: &mut Self| {
                    // A host may still send a click on a greyed item.
                    if tray.shown.turbo_enabled {
                        let wanted = !tray.shown.turbo_on;
                        notify(&tray.tx, TrayAction::SetTurbo(wanted));
                    }
                }),
                ..Default::default()
            }
            .into(),
        );
        // The presets stay in the menu with Turbo off, greyed out, so it
        // reads that they belong to Turbo; the one chosen stays marked.
        items.push(
            StandardItem {
                label: i18n("Turbo preset"),
                enabled: false,
                ..Default::default()
            }
            .into(),
        );
        items.push(
            RadioGroup {
                selected: turbo_preset::ALL
                    .iter()
                    .position(|p| *p == shown.preset)
                    .unwrap_or(0),
                select: Box::new(|tray: &mut Self, index| {
                    if !tray.shown.preset_enabled() {
                        return;
                    }
                    if let Some(preset) = turbo_preset::ALL.get(index) {
                        notify(&tray.tx, TrayAction::SetPreset(*preset));
                    }
                }),
                options: turbo_preset::ALL
                    .iter()
                    .map(|p| RadioItem {
                        label: i18n(p.label()),
                        enabled: shown.preset_enabled(),
                        ..Default::default()
                    })
                    .collect(),
            }
            .into(),
        );
        items.push(ksni::MenuItem::Separator);
        items.push(
            StandardItem {
                label: i18n("Quit"),
                icon_name: "application-exit-symbolic".into(),
                activate: Box::new(|tray: &mut Self| notify(&tray.tx, TrayAction::Quit)),
                ..Default::default()
            }
            .into(),
        );
        items
    }
}

/// Where the tray's state is kept in step, from the main thread.
///
/// Every setter compares with what the tray already shows, so the ten-second
/// status reading costs nothing when nothing changed. What changed goes to
/// the tray's own thread: ksni's blocking update waits for the tray's lock
/// and for its D-Bus signals, which the GTK main loop must not.
#[derive(Clone)]
pub struct TrayHandle {
    shown: Rc<RefCell<Shown>>,
    /// To the tray's thread; it is gone when the tray service could not
    /// start (no session bus).
    updates: mpsc::Sender<Shown>,
}

impl TrayHandle {
    fn change(&self, f: impl FnOnce(&mut Shown)) {
        let new = {
            let mut shown = self.shown.borrow_mut();
            let before = shown.clone();
            f(&mut shown);
            if *shown == before {
                return;
            }
            shown.clone()
        };
        if self.updates.send(new).is_err() {
            tracing::debug!("the tray is not running; its state is not updated");
        }
    }

    /// Say what is wrong, or that nothing is.
    pub fn set_warning(&self, warning: Option<String>) {
        self.change(|s| s.warning = warning);
    }

    /// Turbo's state, and whether it can be switched now.
    pub fn set_turbo(&self, on: bool, enabled: bool) {
        self.change(|s| {
            s.turbo_on = on;
            s.turbo_enabled = enabled;
        });
    }

    /// The preset shown, and whether Home lets one be chosen now.
    pub fn set_preset(&self, preset: Preset, enabled: bool) {
        self.change(|s| {
            s.preset = preset;
            s.preset_home = enabled;
        });
    }
}

/// Send an action and wake the GTK main loop to handle it.
///
/// The main loop is woken only when there is something in the channel, so
/// nothing polls while the tray is idle.
fn notify(tx: &mpsc::Sender<TrayAction>, action: TrayAction) {
    if tx.send(action).is_ok() {
        gtk4::glib::MainContext::default().invoke(crate::app::drain_tray_actions);
    }
}

/// The icon theme directory to announce with the icon: none when
/// `bigamemode-symbolic` is in an installed theme, where every tray host
/// finds it by name; the source tree's when running from it, so a
/// development build shows the same icon.
fn icon_theme_path() -> String {
    let file = format!("hicolor/scalable/apps/{ICON}.svg");
    let data_dirs = std::env::var("XDG_DATA_DIRS")
        .ok()
        .filter(|d| !d.is_empty())
        .unwrap_or_else(|| "/usr/local/share:/usr/share".to_owned());
    let installed = data_dirs
        .split(':')
        .any(|d| std::path::Path::new(d).join("icons").join(&file).is_file());
    if installed {
        return String::new();
    }
    source_tree_icons(&file)
}

/// The source tree's icon directory, for a development build run from it.
/// A release build leaves the path out: a package would otherwise carry the
/// directory it was built in.
#[cfg(debug_assertions)]
fn source_tree_icons(file: &str) -> String {
    let tree = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../usr/share/icons");
    match std::fs::canonicalize(&tree) {
        Ok(dir) if dir.join(file).is_file() => dir.to_string_lossy().into_owned(),
        _ => String::new(),
    }
}

#[cfg(not(debug_assertions))]
fn source_tree_icons(_file: &str) -> String {
    String::new()
}

/// Spawn the system tray on its own thread. Returns a handle to keep it in
/// step and a receiver for what it asks for.
pub fn spawn() -> (TrayHandle, mpsc::Receiver<TrayAction>) {
    let (tx, rx) = mpsc::channel();
    let (updates, pending) = mpsc::channel::<Shown>();
    let shown = Shown::default();
    let tray = BiGameTray {
        tx,
        shown: shown.clone(),
        theme_path: icon_theme_path(),
    };
    let spawned = std::thread::Builder::new()
        .name("bigame-tray".into())
        .spawn(move || {
            // At login the panel may register its tray host after this runs,
            // and a desktop may have none at all: waiting for one is not an
            // error.
            let handle = match tray.assume_sni_available(true).spawn() {
                Ok(handle) => handle,
                Err(e) => {
                    tracing::warn!(error = %e, "no system tray");
                    return;
                }
            };
            // Until the application drops its handle. A burst of changes
            // becomes one update: only the last state is shown.
            while let Ok(mut next) = pending.recv() {
                while let Ok(later) = pending.try_recv() {
                    next = later;
                }
                handle.update(move |tray| tray.shown = next);
            }
        });
    if let Err(e) = spawned {
        tracing::warn!(error = %e, "could not start the tray's thread");
    }

    (
        TrayHandle {
            shown: Rc::new(RefCell::new(shown)),
            updates,
        },
        rx,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shown(on: bool, enabled: bool) -> Shown {
        Shown {
            turbo_on: on,
            turbo_enabled: enabled,
            preset: Preset::MoreFps,
            preset_home: true,
            warning: None,
        }
    }

    #[test]
    fn the_tooltip_says_what_turbo_is_doing() {
        assert_eq!(shown(false, true).description(), "Turbo mode inactive");
        assert_eq!(
            shown(true, true).description(),
            "Turbo mode activated · More FPS"
        );
        assert_eq!(shown(false, false).description(), "Turning On");
        assert_eq!(shown(true, false).description(), "Turning Off");
        let mut s = shown(true, true);
        s.warning = Some("falcond stopped unexpectedly".into());
        assert_eq!(s.description(), "falcond stopped unexpectedly");
    }
}
