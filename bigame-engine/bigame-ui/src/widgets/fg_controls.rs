//! lsfg-vk's general setting: where its `Lossless.dll` is.
//!
//! The DLL is lsfg-vk's `[global] dll`, shared by every game; a game's own
//! frame generation (on, multiplier, flow scale…) is in its profile
//! (`widgets::optimization::FrameGenFields`), so Tuning holds only what is
//! general.

use adw::prelude::*;
use gtk4::gio;
use libadwaita as adw;

use crate::i18n::i18n;

/// The `Lossless.dll` path row, with a file chooser and the licence note.
/// `on_changed(ready)` runs after each write with whether the file exists.
#[must_use]
#[allow(clippy::too_many_lines)]
pub fn dll_row(on_changed: impl Fn(bool) + 'static) -> adw::EntryRow {
    let row = adw::EntryRow::builder()
        .title(i18n("Path to Lossless.dll"))
        .text(bigame_core::fg::read_global_dll().unwrap_or_default())
        .build();

    let file_btn = gtk4::Button::builder()
        .icon_name("folder-open-symbolic")
        .tooltip_text(i18n("Select Lossless.dll"))
        .valign(gtk4::Align::Center)
        .css_classes(["flat"])
        .build();
    row.add_suffix(&file_btn);

    // Found where Steam installs it: one click fills the path.
    if let Some(found) = bigame_core::fg::find_steam_dll() {
        let find_btn = gtk4::Button::builder()
            .icon_name("edit-find-symbolic")
            .tooltip_text(i18n("Use the Lossless.dll from Steam's Lossless Scaling"))
            .valign(gtk4::Align::Center)
            .css_classes(["flat"])
            .build();
        let row2 = row.clone();
        find_btn.connect_clicked(move |_| row2.set_text(&found.to_string_lossy()));
        row.add_suffix(&find_btn);
    }

    let info_btn = crate::widgets::info::dialog_button(&i18n("About Lossless.dll"), || {
        use crate::widgets::info::Entry;
        (
            i18n("Lossless Scaling is required"),
            i18n(
                "lsfg-vk is free, but the frame generation it runs is Lossless Scaling's, a paid Windows program. lsfg-vk loads its Lossless.dll, which BiGame-mode cannot ship or download: you need your own copy, bought on Steam.",
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
                        "lsfg-vk loads and generates nothing, so BiGame-mode switches it off at launch.",
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

    row.connect_changed(move |r| {
        let text = r.text().to_string();
        let dll = (!text.is_empty()).then_some(text);
        if let Err(e) = bigame_core::fg::write_global_dll(dll) {
            crate::widgets::toast::error(
                r,
                &i18n("Could not save the Lossless.dll path"),
                &crate::i18n::error_text(&e),
            );
        }
        on_changed(bigame_core::fg::is_lossless_dll_ready());
    });
    row
}
