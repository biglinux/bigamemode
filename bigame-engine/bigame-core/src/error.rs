//! Errors as the user reads them.
//!
//! A failure the user can act on — close the game, check for updates first,
//! the download does not match — is a [`UserError`]: a translatable
//! [`Text`] returned through `anyhow` like any other error. What caused it (a
//! file operation, a tool, the system) stays an ordinary `anyhow` error under
//! it. [`describe`] turns a whole chain into the sentence the UI shows.

use crate::dbus_client::{HelperFailure, helper_failure};
use crate::text::{Arg, N_, Text};

/// A failure the user can act on, in words for them.
///
/// Returned through `anyhow` like any error — `bail!(UserError::plain(…))`,
/// the template marked with [`N_`] where it is written. Its `Display` is the
/// English sentence, so logs, the command-line tools and
/// tests read as before. A cause is attached with [`UserError::caused_by`]
/// rather than with `anyhow::Context`: a context cannot be reached from
/// `chain()`, and a `UserError` inside it would stay in English.
#[derive(Debug)]
pub struct UserError {
    /// What the user reads.
    pub text: Text,
    cause: Option<anyhow::Error>,
}

impl UserError {
    /// A message with no values.
    #[must_use]
    pub fn plain(template: &'static str) -> Self {
        Text::plain(template).into()
    }

    /// A message with values for its `%s`, in order.
    #[must_use]
    pub fn with<I, A>(template: &'static str, args: I) -> Self
    where
        I: IntoIterator<Item = A>,
        A: Into<Arg>,
    {
        Text::with(template, args).into()
    }

    /// The same message, with `cause` as the error that led to it.
    #[must_use]
    pub fn caused_by(mut self, cause: impl Into<anyhow::Error>) -> Self {
        self.cause = Some(cause.into());
        self
    }
}

impl From<Text> for UserError {
    fn from(text: Text) -> Self {
        Self { text, cause: None }
    }
}

impl std::fmt::Display for UserError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.text.fmt(f)
    }
}

impl std::error::Error for UserError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.cause
            .as_ref()
            .map(|e| &**e as &(dyn std::error::Error + 'static))
    }
}

/// `err` as the user reads it.
///
/// The first [`UserError`] in the chain is the message; what lies under it
/// follows in parentheses unless the message already says it. A refusal by
/// the helper (Polkit said no, or it is not running) with no `UserError`
/// above it is said in words. Anything else is the whole chain as written:
/// the cause is what makes it useful, so it is never cut to its first line.
#[must_use]
pub fn describe(err: &anyhow::Error) -> Text {
    let mut levels = err.chain().skip_while(|e| !e.is::<UserError>());
    if let Some(lead) = levels.next().and_then(|e| e.downcast_ref::<UserError>()) {
        let mut shown = lead.text.english();
        let mut cause: Option<Text> = None;
        for level in levels {
            let text = level_text(level);
            let english = text.english();
            // A level whose words are already there adds nothing: an error
            // quoted in the message, or a wrapper repeating its source.
            if english.is_empty() || shown.contains(&english) {
                continue;
            }
            shown.push_str(&english);
            cause = Some(match cause {
                None => text,
                Some(above) => Text::with(N_("%s: %s"), [above, text]),
            });
        }
        return match cause {
            None => lead.text.clone(),
            Some(cause) => Text::with(N_("%s (%s)"), [lead.text.clone(), cause]),
        };
    }
    match err.chain().find_map(helper_failure) {
        Some(failure) => failure_text(failure),
        None => Text::raw(format!("{err:#}")),
    }
}

/// One level of a chain, on its own.
fn level_text(level: &(dyn std::error::Error + 'static)) -> Text {
    if let Some(u) = level.downcast_ref::<UserError>() {
        return u.text.clone();
    }
    helper_failure(level).map_or_else(|| Text::raw(level.to_string()), failure_text)
}

fn failure_text(failure: HelperFailure) -> Text {
    Text::plain(match failure {
        HelperFailure::Refused => N_("Authorization was refused or cancelled"),
        HelperFailure::NotRunning => N_("The BiGame-mode helper is not running"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn io(msg: &str) -> std::io::Error {
        std::io::Error::new(std::io::ErrorKind::PermissionDenied, msg.to_owned())
    }

    #[test]
    fn a_user_error_reads_in_english_and_keeps_its_cause() {
        let inner = anyhow::Error::from(io("denied")).context("write /x");
        let e = anyhow::Error::new(
            UserError::with("%s is running", ["game"])
                .caused_by(UserError::plain("inner").caused_by(inner)),
        );
        assert_eq!(e.to_string(), "game is running");
        assert_eq!(format!("{e:#}"), "game is running: inner: write /x: denied");
        let bailed = || -> anyhow::Result<()> {
            anyhow::ensure!(false, UserError::plain("closed"));
            Ok(())
        };
        assert!(bailed().unwrap_err().downcast_ref::<UserError>().is_some());
    }

    #[test]
    fn the_message_leads_and_every_level_under_it_is_translated() {
        let inner = anyhow::Error::from(io("denied")).context("write /x");
        let e = anyhow::Error::new(
            UserError::with("%s is running", ["game"])
                .caused_by(UserError::plain("inner").caused_by(inner)),
        )
        .context("outer technical step");
        let t = describe(&e);
        assert_eq!(t.english(), "game is running (inner: write /x: denied)");
        let pt = |s: &str| match s {
            "%s is running" => "%s está aberto".to_owned(),
            "inner" => "interno".to_owned(),
            other => other.to_owned(),
        };
        assert_eq!(
            t.render(&pt),
            "game está aberto (interno: write /x: denied)"
        );
    }

    #[test]
    fn a_cause_the_message_already_quotes_is_not_repeated() {
        let e = anyhow::Error::new(
            UserError::with("could not read it: %s", ["denied"]).caused_by(io("denied")),
        );
        assert_eq!(describe(&e).english(), "could not read it: denied");
        let alone = anyhow::Error::new(UserError::plain("nothing to install"));
        assert_eq!(describe(&alone).english(), "nothing to install");
    }

    #[test]
    fn a_refusal_by_the_helper_is_said_in_words() {
        let refused = anyhow::Error::from(zbus::Error::FDO(Box::new(
            zbus::fdo::Error::AccessDenied("not authorized".into()),
        )))
        .context("save profile");
        assert_eq!(
            describe(&refused).english(),
            "Authorization was refused or cancelled"
        );
        let gone = anyhow::Error::from(zbus::fdo::Error::ServiceUnknown("x".into()));
        assert_eq!(
            describe(&gone).english(),
            "The BiGame-mode helper is not running"
        );
    }

    #[test]
    fn anything_else_is_the_whole_chain() {
        let e = anyhow::Error::from(io("denied")).context("write /x");
        assert_eq!(describe(&e).english(), "write /x: denied");
    }
}
