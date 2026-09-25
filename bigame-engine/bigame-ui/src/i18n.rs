//! Internationalization (gettext) setup for BiGame-mode UI.

use gettextrs::{LocaleCategory, gettext, ngettext};

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

/// Translate a count-dependent message: `singular` when `n` takes the
/// language's singular form, `plural` otherwise (both may hold `%n`).
#[must_use]
pub fn ni18n(singular: &str, plural: &str, n: usize) -> String {
    ngettext(singular, plural, u32::try_from(n).unwrap_or(u32::MAX)).replace("%n", &n.to_string())
}

/// A sentence from bigame-core, translated: the template through gettext,
/// then its values filled in.
#[must_use]
pub fn tr(t: &bigame_core::graphics::text::Text) -> String {
    bigame_core::graphics::text::Text::fill(&i18n(t.template), &t.args)
}
