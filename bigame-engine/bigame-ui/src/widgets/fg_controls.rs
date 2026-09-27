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

    let info_btn = gtk4::Button::builder()
        .icon_name("dialog-information-symbolic")
        .tooltip_text(i18n(
            "Lossless Scaling is proprietary.
Click to visit losslessscaling.com",
        ))
        .valign(gtk4::Align::Center)
        .css_classes(["flat", "circular"])
        .build();
    info_btn.connect_clicked(|btn| {
        let dialog = adw::AlertDialog::builder()
            .heading(i18n("Lossless Scaling Required"))
            .body(i18n(
                "This feature uses LSFG-VK which requires the proprietary Lossless.dll to function.

You must legally acquire Lossless Scaling on Steam or other platforms to obtain this file.",
            ))
            .build();
        dialog.add_response("cancel", &i18n("Close"));
        dialog.add_response("web", &i18n("Visit Website"));
        dialog.set_response_appearance("web", adw::ResponseAppearance::Suggested);
        let win = btn.root().and_downcast::<gtk4::Window>();
        dialog.connect_response(None, move |_, response| {
            if response == "web" {
                let launcher = gtk4::UriLauncher::new("https://losslessscaling.com/");
                launcher.launch(win.as_ref(), gio::Cancellable::NONE, |_| {});
            }
        });
        dialog.present(Some(btn));
    });
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
