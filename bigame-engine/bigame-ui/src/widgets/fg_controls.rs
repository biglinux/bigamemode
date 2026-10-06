//! lsfg-vk's general setting: where its `Lossless.dll` is.
//!
//! The DLL is lsfg-vk's `[global] dll`, shared by every game; a game's own
//! frame generation (on, multiplier, flow scale…) is in its profile
//! (`widgets::optimization::FrameGenFields`), so Tuning holds only what is
//! general.

use std::cell::Cell;
use std::rc::Rc;

use adw::prelude::*;
use gtk4::{gio, glib};
use libadwaita as adw;

use crate::i18n::i18n;

/// How long typing must rest before the path is written.
const SETTLE_MS: u64 = 600;

/// The `Lossless.dll` path row, with a file chooser and the licence note.
/// `on_changed(ready)` runs after each write with whether the file exists.
///
/// The path is lsfg-vk's own file, shared by every game, so it is written
/// as soon as typing rests (or on Enter), even in a profile's editor — never
/// a half-typed path, which lsfg-vk would read at once in a running game.
#[must_use]
#[allow(clippy::too_many_lines)]
pub fn dll_row(on_changed: impl Fn(bool) + 'static) -> adw::EntryRow {
    let row = adw::EntryRow::builder()
        .title(i18n("Path to Lossless.dll"))
        .text(bigame_core::fg::read_global_dll().unwrap_or_default())
        .tooltip_text(i18n(
            "Shared by every game, and saved as soon as you stop typing",
        ))
        .build();

    let file_btn = gtk4::Button::builder()
        .icon_name("folder-open-symbolic")
        .tooltip_text(i18n("Select Lossless.dll"))
        .valign(gtk4::Align::Center)
        .css_classes(["flat"])
        .build();
    row.add_suffix(&file_btn);

    // Found where Steam installs it: one click fills the path. Looking
    // stats every Steam library (one may be a disk that has to spin up), so
    // the button appears once the search, off the main thread, finds it.
    let find_btn = gtk4::Button::builder()
        .icon_name("edit-find-symbolic")
        .tooltip_text(i18n("Use the Lossless.dll from Steam's Lossless Scaling"))
        .valign(gtk4::Align::Center)
        .css_classes(["flat"])
        .visible(false)
        .build();
    row.add_suffix(&find_btn);
    {
        let row = row.clone();
        glib::spawn_future_local(async move {
            let Some(found) = gio::spawn_blocking(bigame_core::fg::find_steam_dll)
                .await
                .ok()
                .flatten()
            else {
                return;
            };
            find_btn.connect_clicked(move |_| row.set_text(&found.to_string_lossy()));
            find_btn.set_visible(true);
        });
    }

    let info_btn = crate::widgets::info::dialog_button(&i18n("About Lossless.dll"), || {
        use crate::widgets::info::Entry;
        (
            i18n("Lossless Scaling is required"),
            i18n(
                "lsfg-vk is free, but the frame generation it runs is Lossless Scaling's, a paid Windows program. lsfg-vk loads its Lossless.dll, which Big Game Mode cannot ship or download: you need your own copy, bought on Steam.",
            ),
            vec![
                Entry {
                    title: i18n("Where to buy it"),
                    body: format!(
                        "Steam: https://store.steampowered.com/app/{}/Lossless_Scaling/\n{}",
                        bigame_core::fg::LOSSLESS_SCALING_APP,
                        i18n("Developer and publisher: THS · E-mail: losslessscaling@gmail.com")
                    ),
                },
                Entry {
                    title: i18n("Where Lossless.dll is"),
                    body: i18n(
                        "Install Lossless Scaling from your Steam library (on Linux, Steam installs it through Proton; it never needs to run). The file is then in the game's folder: …/steamapps/common/Lossless Scaling/Lossless.dll. When it is there, the search button beside the path fills it in; otherwise choose it with the folder button.",
                    ),
                },
                Entry {
                    title: i18n("Without it"),
                    body: i18n(
                        "lsfg-vk loads and generates nothing, so Big Game Mode switches it off at launch.",
                    ),
                },
            ],
        )
    });
    let store_btn = gtk4::Button::builder()
        .icon_name("web-browser-symbolic")
        .tooltip_text(i18n("Open Lossless Scaling in the Steam store"))
        .valign(gtk4::Align::Center)
        .css_classes(["flat", "circular"])
        .build();
    store_btn.connect_clicked(|btn| {
        let win = btn.root().and_downcast::<gtk4::Window>();
        let launcher = gtk4::UriLauncher::new(&format!(
            "https://store.steampowered.com/app/{}/Lossless_Scaling/",
            bigame_core::fg::LOSSLESS_SCALING_APP
        ));
        launcher.launch(win.as_ref(), gio::Cancellable::NONE, |_| {});
    });
    row.add_suffix(&store_btn);
    row.add_suffix(&info_btn);

    {
        let row = row.clone();
        file_btn.connect_clicked(move |btn| {
            let dialog = gtk4::FileDialog::builder()
                .title(i18n("Select Lossless.dll"))
                .modal(true)
                .build();
            let dll_filter = gtk4::FileFilter::new();
            dll_filter.set_name(Some(&format!("{} (*.dll)", i18n("DLL files"))));
            dll_filter.add_pattern("*.dll");
            let filters = gio::ListStore::new::<gtk4::FileFilter>();
            filters.append(&dll_filter);
            dialog.set_filters(Some(&filters));
            let row = row.clone();
            let win = btn.root().and_downcast::<gtk4::Window>();
            dialog.open(win.as_ref(), gio::Cancellable::NONE, move |res| {
                if let Some(path) = res.ok().and_then(|f| f.path()) {
                    // The row's own change handler writes it.
                    row.set_text(&path.to_string_lossy());
                }
            });
        });
    }

    let on_changed = Rc::new(on_changed);
    let generation = Rc::new(Cell::new(0_u32));
    let write = Rc::new(move |r: &adw::EntryRow| {
        let text = r.text().to_string();
        let dll = (!text.is_empty()).then_some(text);
        let (r, on_changed) = (r.clone(), Rc::clone(&on_changed));
        // Saved, then asked of lsfg-vk whether it can use the DLL (a short
        // run of its own tool), off the main thread.
        glib::spawn_future_local(async move {
            let done = gio::spawn_blocking(move || {
                let saved = bigame_core::fg::write_global_dll(dll.clone());
                let check = dll.and_then(|d| bigame_core::fg::check_dll(std::path::Path::new(&d)));
                (saved, check, bigame_core::fg::is_lossless_dll_ready())
            })
            .await;
            let Ok((saved, check, ready)) = done else {
                return;
            };
            if let Err(e) = saved {
                crate::widgets::toast::error(
                    &r,
                    &i18n("Could not save the Lossless.dll path"),
                    &crate::i18n::error_text(&e),
                );
            } else if let Some(bigame_core::fg::DllCheck::Unusable(reason)) = check {
                crate::widgets::toast::error(
                    &r,
                    &i18n("Frame generation (lsfg-vk)"),
                    &i18n("lsfg-vk cannot generate frames with this Lossless.dll (%s): update Lossless Scaling and choose its Lossless.dll again")
                        .replace("%s", &reason),
                );
            }
            on_changed(ready);
        });
    });
    {
        let (generation, write) = (Rc::clone(&generation), Rc::clone(&write));
        row.connect_changed(move |r| {
            let now = generation.get().wrapping_add(1);
            generation.set(now);
            let (generation, write, r) = (Rc::clone(&generation), Rc::clone(&write), r.clone());
            glib::timeout_add_local_once(std::time::Duration::from_millis(SETTLE_MS), move || {
                if generation.get() == now {
                    write(&r);
                }
            });
        });
    }
    row.connect_entry_activated(move |r| {
        // Written now; the change still waiting is not written twice.
        generation.set(generation.get().wrapping_add(1));
        write(r);
    });
    row
}
