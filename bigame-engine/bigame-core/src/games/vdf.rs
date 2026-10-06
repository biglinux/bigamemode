//! Valve's text `KeyValues` format (`libraryfolders.vdf`, `appmanifest_*.acf`),
//! as far as reading values goes.
//!
//! Quoted and bare tokens, braces, `//` comments, `[$WIN32]`-style
//! conditions, and the escapes Steam writes inside quotes (`\\`, `\"`, `\n`,
//! `\t`): a library on `D:\Games` is stored as `"D:\\Games"`, and a quote in
//! a folder or title name as `\"`. Keys compare case-insensitively, as in
//! Steam's own parser (`"Name"` and `"name"` are one key).

/// One token of a text VDF document.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Token {
    Open,
    Close,
    Text(String),
}

fn tokens(text: &str) -> Vec<Token> {
    let mut out = Vec::new();
    let mut chars = text.chars().peekable();
    while let Some(&c) = chars.peek() {
        match c {
            c if c.is_whitespace() => {
                chars.next();
            }
            '{' => {
                chars.next();
                out.push(Token::Open);
            }
            '}' => {
                chars.next();
                out.push(Token::Close);
            }
            '/' => {
                chars.next();
                if chars.peek() == Some(&'/') {
                    for c in chars.by_ref() {
                        if c == '\n' {
                            break;
                        }
                    }
                } else {
                    out.push(Token::Text(bare(&mut chars, "/")));
                }
            }
            // A platform condition after a value (`[$WIN32]`) says when the
            // pair applies; it is not a token of its own.
            '[' => {
                for c in chars.by_ref() {
                    if c == ']' {
                        break;
                    }
                }
            }
            '"' => {
                chars.next();
                let mut value = String::new();
                while let Some(c) = chars.next() {
                    match c {
                        '"' => break,
                        '\\' => match chars.next() {
                            Some('n') => value.push('\n'),
                            Some('t') => value.push('\t'),
                            Some(c @ ('\\' | '"')) => value.push(c),
                            // Not an escape Steam writes: kept as it is,
                            // as a path written by hand would be.
                            Some(other) => {
                                value.push('\\');
                                value.push(other);
                            }
                            None => value.push('\\'),
                        },
                        c => value.push(c),
                    }
                }
                out.push(Token::Text(value));
            }
            _ => out.push(Token::Text(bare(&mut chars, ""))),
        }
    }
    out
}

/// An unquoted token, up to whitespace, a brace or a quote.
fn bare(chars: &mut std::iter::Peekable<std::str::Chars<'_>>, start: &str) -> String {
    let mut value = start.to_owned();
    while let Some(&c) = chars.peek() {
        if c.is_whitespace() || matches!(c, '{' | '}' | '"') {
            break;
        }
        value.push(c);
        chars.next();
    }
    value
}

/// Every `"key" "value"` pair of a document, at any depth, in order.
fn pairs(text: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut tokens = tokens(text).into_iter().peekable();
    while let Some(token) = tokens.next() {
        let Token::Text(key) = token else {
            continue;
        };
        if let Some(Token::Text(_)) = tokens.peek() {
            let Some(Token::Text(value)) = tokens.next() else {
                unreachable!("peeked a text token");
            };
            out.push((key, value));
        }
        // A key followed by `{` opens a section; its pairs follow in order.
    }
    out
}

/// The first value of `key`, at any depth.
pub(super) fn first_value(text: &str, key: &str) -> Option<String> {
    pairs(text)
        .into_iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(key))
        .map(|(_, v)| v)
}

/// Every value of `key`, at any depth, in order.
pub(super) fn values(text: &str, key: &str) -> Vec<String> {
    pairs(text)
        .into_iter()
        .filter(|(k, _)| k.eq_ignore_ascii_case(key))
        .map(|(_, v)| v)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_inside_quotes_are_decoded() {
        let vdf = "\"libraryfolders\"\n{\n\t\"0\"\n\t{\n\t\t\"path\"\t\t\"/mnt/My \\\"Games\\\"\"\n\t}\n\t\"1\"\n\t{\n\t\t\"path\"\t\t\"D:\\\\SteamLibrary\"\n\t}\n}\n";
        assert_eq!(
            values(vdf, "path"),
            ["/mnt/My \"Games\"", "D:\\SteamLibrary"]
        );
    }

    #[test]
    fn keys_are_case_insensitive_and_sections_do_not_pair() {
        let acf = "\"AppState\"\n{\n\t\"Name\"\t\"Gauntlet™ \"\n\t\"UserConfig\"\n\t{\n\t\t\"BetaKey\"\t\"public\"\n\t}\n}\n";
        assert_eq!(first_value(acf, "name").as_deref(), Some("Gauntlet™ "));
        assert_eq!(first_value(acf, "betakey").as_deref(), Some("public"));
        // A section name is not a value.
        assert_eq!(first_value(acf, "UserConfig"), None);
        assert_eq!(first_value(acf, "AppState"), None);
    }

    #[test]
    fn comments_conditions_and_bare_tokens_are_understood() {
        let vdf = "// written by Steam\n\"a\" { \"k\" \"v\" [$WIN32]\n bare value //tail\n \"t\" \"tab\\there\" }";
        assert_eq!(first_value(vdf, "k").as_deref(), Some("v"));
        assert_eq!(first_value(vdf, "bare").as_deref(), Some("value"));
        assert_eq!(first_value(vdf, "t").as_deref(), Some("tab\there"));
    }
}
