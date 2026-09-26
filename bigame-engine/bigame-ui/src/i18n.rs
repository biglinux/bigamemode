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

/// Mark a string for translation without translating it here: for labels
/// kept in constants and passed to [`i18n`] where they are shown.
#[allow(non_snake_case)]
#[must_use]
pub const fn N_(s: &'static str) -> &'static str {
    s
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

/// A sentence from bigame-core, translated: its template and every
/// translatable value in it through gettext.
#[must_use]
pub fn tr(t: &bigame_core::text::Text) -> String {
    t.render(&i18n)
}

/// An `anyhow` error as the user reads it, translated: the message a
/// [`UserError`] gives, the cause after it; the whole chain otherwise
/// ([`bigame_core::error::describe`]). For an `std::io::Error` its own text
/// is enough: the system already words it in the user's language.
///
/// [`UserError`]: bigame_core::error::UserError
#[must_use]
pub fn error_text(err: &anyhow::Error) -> String {
    tr(&bigame_core::error::describe(err))
}
