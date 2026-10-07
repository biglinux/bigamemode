//! Sentences bigame-core builds for the UI, kept translatable.
//!
//! bigame-core has no gettext of its own. Text it produces for people — a
//! plan's steps, a rule's reason, a report's evidence — is a template marked
//! with [`N_`] (a no-op the string extractor collects) plus the values for its
//! `%s` placeholders. A value is either raw (a number, a path, a process name,
//! an error from the system) or itself a translatable [`Text`]. The UI
//! translates with `i18n` all the way down ([`Text::render`]); logs, the
//! command-line tools and tests use [`Text::english`].
//!
//! A `Text` can be saved and read back (the Turbo report keeps one per row),
//! so a report written in one language is shown in whichever the user runs.

use std::borrow::Cow;

use serde::{Deserialize, Serialize};

/// Mark a string for translation without translating it here.
#[allow(non_snake_case)]
#[must_use]
pub const fn N_(s: &'static str) -> &'static str {
    s
}

/// A value for one `%s` of a [`Text`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Arg {
    /// Shown as it is: numbers, names, paths, errors from the system.
    Raw(String),
    /// A sentence of its own, translated with the one around it.
    Text(Text),
}

impl From<String> for Arg {
    fn from(s: String) -> Self {
        Self::Raw(s)
    }
}

impl From<&String> for Arg {
    fn from(s: &String) -> Self {
        Self::Raw(s.clone())
    }
}

impl From<&str> for Arg {
    fn from(s: &str) -> Self {
        Self::Raw(s.to_owned())
    }
}

impl From<Text> for Arg {
    fn from(t: Text) -> Self {
        Self::Text(t)
    }
}

impl PartialEq<&str> for Arg {
    fn eq(&self, other: &&str) -> bool {
        matches!(self, Self::Raw(s) if s == other)
    }
}

/// A translatable sentence: a template and the values for its `%s`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Text {
    /// The template, as marked with [`N_`] (owned only when read back).
    pub template: Cow<'static, str>,
    /// Values for the `%s` placeholders, in order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<Arg>,
}

impl Text {
    /// A sentence with no values.
    #[must_use]
    pub fn plain(template: &'static str) -> Self {
        Self {
            template: Cow::Borrowed(template),
            args: Vec::new(),
        }
    }

    /// A sentence with values for its `%s`, in order.
    #[must_use]
    pub fn with<I, A>(template: &'static str, args: I) -> Self
    where
        I: IntoIterator<Item = A>,
        A: Into<Arg>,
    {
        Self {
            template: Cow::Borrowed(template),
            args: args.into_iter().map(Into::into).collect(),
        }
    }

    /// Text that is not translatable — a message from the system or another
    /// program — carried where a `Text` is expected.
    #[must_use]
    pub fn raw(s: impl Into<String>) -> Self {
        Self::with(N_("%s"), [Arg::Raw(s.into())])
    }

    /// Fill `template`'s `%s` with `values`, in order. A translation that has
    /// to reorder them writes `%1$s`, `%2$s`… (numbered from 1, as in C's
    /// printf); a numbered placeholder does not advance the plain ones. Extra
    /// or out-of-range placeholders stay as they are; extra values are dropped.
    #[must_use]
    pub fn fill(template: &str, values: &[String]) -> String {
        let mut out = String::with_capacity(template.len());
        let mut next = values.iter();
        let mut rest = template;
        while let Some(i) = rest.find('%') {
            out.push_str(&rest[..i]);
            let after = &rest[i + 1..];
            if let Some(tail) = after.strip_prefix('s') {
                match next.next() {
                    Some(v) => out.push_str(v),
                    None => out.push_str("%s"),
                }
                rest = tail;
                continue;
            }
            let digits = after.bytes().take_while(u8::is_ascii_digit).count();
            let numbered = (digits > 0)
                .then(|| after[digits..].strip_prefix("$s"))
                .flatten()
                .and_then(|tail| {
                    let n: usize = after[..digits].parse().ok()?;
                    Some((values.get(n.checked_sub(1)?)?, tail))
                });
            if let Some((v, tail)) = numbered {
                out.push_str(v);
                rest = tail;
            } else {
                out.push('%');
                rest = after;
            }
        }
        out.push_str(rest);
        out
    }

    /// The sentence with every template, its own and its values', passed
    /// through `translate`.
    #[must_use]
    pub fn render(&self, translate: &dyn Fn(&str) -> String) -> String {
        let values: Vec<String> = self
            .args
            .iter()
            .map(|a| match a {
                Arg::Raw(s) => s.clone(),
                Arg::Text(t) => t.render(translate),
            })
            .collect();
        Self::fill(&translate(&self.template), &values)
    }

    /// The sentence in English, as written in the source.
    #[must_use]
    pub fn english(&self) -> String {
        self.render(&|s| s.to_owned())
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

    #[test]
    fn a_translation_can_reorder_numbered_placeholders() {
        let v = ["Steam".to_owned(), "Proton".to_owned()];
        assert_eq!(Text::fill("%2$s for %1$s", &v), "Proton for Steam");
        assert_eq!(Text::fill("%1$s, %1$s", &v), "Steam, Steam");
        // A numbered placeholder leaves the plain ones where they were.
        assert_eq!(Text::fill("%2$s %s %s", &v), "Proton Steam Proton");
        // Out of range, zero or malformed: kept as written.
        assert_eq!(Text::fill("%3$s %0$s %1$d", &v), "%3$s %0$s %1$d");
        // What plain `%s` filling always did, unchanged.
        assert_eq!(Text::fill("100% %s", &v), "100% Steam");
        assert_eq!(Text::fill("%%s", &v), "%Steam");
        assert_eq!(Text::fill("%", &v), "%");
        assert_eq!(Text::fill("ação %s — é %s", &v), "ação Steam — é Proton");
        let pt = |s: &str| match s {
            "%s already has %s" => "%2$s já está em %1$s".to_owned(),
            other => other.to_owned(),
        };
        let t = Text::with(N_("%s already has %s"), ["Steam", "OptiScaler"]);
        assert_eq!(t.render(&pt), "OptiScaler já está em Steam");
    }

    #[test]
    fn nested_sentences_are_translated_with_the_one_around_them() {
        let t = Text::with(
            N_("%s: %s"),
            [
                Arg::Text(Text::plain(N_("Power profile"))),
                Arg::Raw("performance".into()),
            ],
        );
        let pt = |s: &str| match s {
            "%s: %s" => "%s — %s".to_owned(),
            "Power profile" => "Perfil de energia".to_owned(),
            other => other.to_owned(),
        };
        assert_eq!(t.render(&pt), "Perfil de energia — performance");
        assert_eq!(t.english(), "Power profile: performance");
    }

    #[test]
    fn a_text_survives_being_saved_and_read_back() {
        let t = Text::with(
            N_("already %s, %s"),
            [
                Arg::Raw("performance".into()),
                Arg::Text(Text::plain(N_("verified"))),
            ],
        );
        let json = serde_json::to_string(&t).unwrap();
        let back: Text = serde_json::from_str(&json).unwrap();
        assert_eq!(back, t);
        let plain: Text = serde_json::from_str(r#"{"template":"no values"}"#).unwrap();
        assert_eq!(plain.english(), "no values");
    }
}
