//! Internationalization (gettext) setup for BiGame-mode UI.

use gettextrs::{LocaleCategory, gettext};

/// Application gettext domain.
const GETTEXT_DOMAIN: &str = "bigame-mode";

/// Default locale directory for installed .mo files.
const LOCALE_DIR: &str = "/usr/share/locale";

/// Initialize gettext for the application.
///
/// `BIGAME_LOCALEDIR` points at compiled catalogues during development;
/// otherwise the installed ones in `/usr/share/locale` are used. A failure
/// leaves the interface in English rather than stopping it from starting.
pub fn init() {
    let locale_dir = std::env::var("BIGAME_LOCALEDIR").unwrap_or_else(|_| LOCALE_DIR.to_owned());
    gettextrs::setlocale(LocaleCategory::LcAll, "");
    if let Err(e) = gettextrs::bindtextdomain(GETTEXT_DOMAIN, &locale_dir)
        .and_then(|_| gettextrs::textdomain(GETTEXT_DOMAIN).map(|_| ()))
    {
        tracing::warn!(error = %e, "translations unavailable; using English");
    }
}

/// Translate a string via gettext.
#[must_use]
pub fn i18n(s: &str) -> String {
    gettext(s)
}
