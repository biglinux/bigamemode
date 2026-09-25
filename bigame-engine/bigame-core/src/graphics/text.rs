//! Sentences bigame-core builds for the UI, kept translatable.
//!
//! bigame-core has no gettext of its own. Text it produces for people — a
//! plan's steps, a rule's reason, a report's evidence — is a template marked
//! with [`N_`] (a no-op the string extractor collects) plus the values for its
//! `%s` placeholders. The UI translates the template with `i18n` and fills the
//! values in; logs, reports and tests use [`Text::english`].

use serde::Serialize;

/// Mark a string for translation without translating it here.
#[allow(non_snake_case)]
#[must_use]
pub const fn N_(s: &'static str) -> &'static str {
    s
}

/// A translatable sentence: a template and the values for its `%s`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Text {
    /// The template, as marked with [`N_`].
    pub template: &'static str,
    /// Values for the `%s` placeholders, in order.
    pub args: Vec<String>,
}

impl Text {
    /// A sentence with no values.
    #[must_use]
    pub fn plain(template: &'static str) -> Self {
        Self {
            template,
            args: Vec::new(),
        }
    }

    /// A sentence with values for its `%s`, in order.
    #[must_use]
    pub fn with<I, S>(template: &'static str, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            template,
            args: args.into_iter().map(Into::into).collect(),
        }
    }

    /// Fill `template`'s `%s` with `args`, in order. Extra placeholders stay
    /// as they are; extra values are dropped.
    #[must_use]
    pub fn fill(template: &str, args: &[String]) -> String {
        let mut out = String::with_capacity(template.len());
        let mut values = args.iter();
        let mut rest = template;
        while let Some(i) = rest.find("%s") {
            out.push_str(&rest[..i]);
            match values.next() {
                Some(v) => out.push_str(v),
                None => out.push_str("%s"),
            }
            rest = &rest[i + 2..];
        }
        out.push_str(rest);
        out
    }

    /// The sentence in English, as written in the source.
    #[must_use]
    pub fn english(&self) -> String {
        Self::fill(self.template, &self.args)
    }
}

/// A sentence with no values; the literal must still be marked with [`N_`]
/// where it is written, or the catalogue will not have it.
impl From<&'static str> for Text {
    fn from(template: &'static str) -> Self {
        Self::plain(template)
    }
}

impl std::fmt::Display for Text {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.english())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placeholders_are_filled_in_order() {
        let t = Text::with(N_("choose %s in the game, %s runs"), ["XeSS", "FSR"]);
        assert_eq!(t.english(), "choose XeSS in the game, FSR runs");
        assert_eq!(Text::fill("a %s b %s", &["1".into()]), "a 1 b %s");
        assert_eq!(Text::plain(N_("no values")).to_string(), "no values");
    }
}
