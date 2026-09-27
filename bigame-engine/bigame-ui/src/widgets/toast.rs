//! Toast notification helpers.

use adw::prelude::*;
use libadwaita as adw;

/// Find the nearest `AdwToastOverlay` ancestor of a widget.
pub fn find_overlay(widget: &impl IsA<gtk4::Widget>) -> Option<adw::ToastOverlay> {
    let mut current = widget.ancestor(adw::ToastOverlay::static_type());
    while let Some(w) = current {
        if let Ok(overlay) = w.clone().downcast::<adw::ToastOverlay>() {
            return Some(overlay);
        }
        current = w.ancestor(adw::ToastOverlay::static_type());
    }
    None
}

/// Show a toast message by searching for the nearest `ToastOverlay`.
pub fn show(widget: &impl IsA<gtk4::Widget>, message: &str) {
    if let Some(overlay) = find_overlay(widget) {
        overlay.add_toast(adw::Toast::new(message));
    }
}

/// Say that something failed: a short toast with the message, and a
/// *Details* button that shows the technical text. The full error also goes
/// to the log.
pub fn error(widget: &impl IsA<gtk4::Widget>, message: &str, details: &str) {
    tracing::warn!(message, details, "shown to the user");
    let Some(overlay) = find_overlay(widget) else {
        return;
    };
    let toast = adw::Toast::builder().title(message).timeout(8).build();
    if !details.is_empty() {
        toast.set_button_label(Some(&crate::i18n::i18n("Details")));
        let (heading, body) = (message.to_owned(), details.to_owned());
        let anchor = overlay.clone();
        toast.connect_button_clicked(move |_| {
            let dialog = adw::AlertDialog::builder()
                .heading(&heading)
                .body(&body)
                .build();
            dialog.add_response("close", &crate::i18n::i18n("Close"));
            dialog.present(Some(&anchor));
        });
    }
    overlay.add_toast(toast);
}
