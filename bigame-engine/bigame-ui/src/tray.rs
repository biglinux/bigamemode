//! System tray integration via `StatusNotifierItem` (KDE/freedesktop).
//!
//! Provides a tray icon when the window is hidden. Left-click activates,
//! right-click menu offers Show/Quit.

use ksni::blocking::TrayMethods;
use std::sync::mpsc;
use std::sync::{Arc, RwLock};

use crate::i18n::i18n;
use gtk4::gdk_pixbuf;

fn load_icon_as_pixmap(name: &str) -> Option<ksni::Icon> {
    let resource_path = format!("/com/biglinux/BiGameMode/icons/{name}.svg");
    let pixbuf = gdk_pixbuf::Pixbuf::from_resource_at_scale(&resource_path, 22, 22, true).ok()?;

    let width = pixbuf.width();
    let height = pixbuf.height();
    let pixels = pixbuf.read_pixel_bytes();

    // SNI expects ARGB (32-bit), Pixbuf is RGBA.
    // We need to convert RGBA to ARGB.
    let mut data = Vec::with_capacity(pixels.len());
    let src = pixels.as_ref();
    for i in (0..src.len()).step_by(4) {
        let r = src[i];
        let g = src[i + 1];
        let b = src[i + 2];
        let a = src[i + 3];
        data.push(a);
        data.push(r);
        data.push(g);
        data.push(b);
    }

    Some(ksni::Icon {
        width,
        height,
        data,
    })
}

/// Status of the game mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// No profile active, idle (Blue).
    Idle,
    /// Profile active and working (Green).
    Active,
    /// Something is wrong (Yellow).
    Warning,
}

impl Status {
    #[allow(clippy::trivially_copy_pass_by_ref)]
    fn icon_name(&self) -> String {
        match self {
            Status::Idle => "input-gaming-symbolic-blue".into(),
            Status::Active => "input-gaming-symbolic-green".into(),
            Status::Warning => "input-gaming-symbolic-yellow".into(),
        }
    }
}

/// Actions the tray can request from the GTK main loop.
#[derive(Debug, Clone)]
pub enum TrayAction {
    Activate,
    Quit,
}

struct BiGameTray {
    tx: mpsc::Sender<TrayAction>,
    status: Arc<RwLock<Status>>,
}

impl ksni::Tray for BiGameTray {
    fn id(&self) -> String {
        String::from("bigame-mode")
    }

    fn icon_name(&self) -> String {
        self.status
            .read()
            .map_or_else(|_| Status::Idle.icon_name(), |s| s.icon_name())
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        let name = self.icon_name();
        if let Some(data) = load_icon_as_pixmap(&name) {
            vec![data]
        } else {
            Vec::new()
        }
    }

    fn title(&self) -> String {
        String::from("BiGame-mode")
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        let s = self.status.read().map_or(Status::Idle, |s| *s);
        let desc = match s {
            Status::Idle => i18n("Ready to play"),
            Status::Active => i18n("Gaming mode active"),
            Status::Warning => i18n("Optimization warning"),
        };

        let icon_pixmap = if let Some(data) = load_icon_as_pixmap(&s.icon_name()) {
            vec![data]
        } else {
            Vec::new()
        };

        ksni::ToolTip {
            title: String::from("BiGame-mode"),
            description: desc,
            icon_name: s.icon_name(),
            icon_pixmap,
        }
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        notify(&self.tx, TrayAction::Activate);
    }

    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        let mut items = vec![
            ksni::MenuItem::Standard(ksni::menu::StandardItem {
                label: i18n("Show Dashboard"),
                icon_name: "view-restore-symbolic".into(),
                activate: Box::new(|tray: &mut Self| {
                    notify(&tray.tx, TrayAction::Activate);
                }),
                ..Default::default()
            }),
            ksni::MenuItem::Separator,
        ];

        // No per-profile items: falcond picks a game's profile from its
        // process when it starts; there is nothing to switch to by hand, and
        // the items that were here only wrote a log line.

        items.push(ksni::MenuItem::Standard(ksni::menu::StandardItem {
            label: i18n("Quit"),
            icon_name: String::from("application-exit"),
            activate: Box::new(|tray: &mut Self| {
                notify(&tray.tx, TrayAction::Quit);
            }),
            ..Default::default()
        }));

        items
    }
}

/// Thread-safe handle to update the tray status.
#[derive(Clone)]
pub struct TrayHandle {
    status: Arc<RwLock<Status>>,
    handle: ksni::blocking::Handle<BiGameTray>,
}

impl TrayHandle {
    pub fn set_status(&self, status: Status) {
        if let Ok(mut s) = self.status.write() {
            if *s == status {
                return;
            }
            *s = status;
        }
        self.handle.update(|_| {});
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

/// Spawn system tray in a background thread. Returns a handle to update it and a receiver for actions.
pub fn spawn() -> (TrayHandle, mpsc::Receiver<TrayAction>) {
    let (tx, rx) = mpsc::channel();
    let status = Arc::new(RwLock::new(Status::Idle));
    let tray = BiGameTray {
        tx,
        status: Arc::clone(&status),
    };
    let handle = tray.spawn().expect("Failed to spawn system tray");

    (TrayHandle { status, handle }, rx)
}
