//! Minimal shell-words splitter used for string-form `command:` and
//! `entrypoint:` values. Handles single quotes, double quotes and backslashes
//! but performs no expansion.

use hullr_core::{Error, Result};

/// Split `s` into arguments the way a POSIX shell would tokenize it.
pub fn split(s: &str) -> Result<Vec<String>> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_word = false;
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            c if c.is_whitespace() => {
                if in_word {
                    out.push(std::mem::take(&mut cur));
                    in_word = false;
                }
            }
            '\'' => {
                in_word = true;
                loop {
                    match chars.next() {
                        Some('\'') => break,
                        Some(ch) => cur.push(ch),
                        None => return Err(unterminated(s)),
                    }
                }
            }
            '"' => {
                in_word = true;
                loop {
                    match chars.next() {
                        Some('"') => break,
                        Some('\\') => match chars.next() {
                            Some(n @ ('"' | '\\' | '$' | '`')) => cur.push(n),
                            Some('\n') => {}
                            Some(n) => {
                                cur.push('\\');
                                cur.push(n);
                            }
                            None => return Err(unterminated(s)),
                        },
                        Some(ch) => cur.push(ch),
                        None => return Err(unterminated(s)),
                    }
                }
            }
            '\\' => {
                in_word = true;
                match chars.next() {
                    Some('\n') => {}
                    Some(n) => cur.push(n),
                    None => cur.push('\\'),
                }
            }
            c => {
                in_word = true;
                cur.push(c);
            }
        }
    }
    if in_word {
        out.push(cur);
    }
    Ok(out)
}

fn unterminated(s: &str) -> Error {
    Error::Invalid(format!("unterminated quote in command '{s}'"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_and_quotes() {
        assert_eq!(split("a b   c").unwrap(), ["a", "b", "c"]);
        assert_eq!(split("echo 'a  b' \"c d\"").unwrap(), ["echo", "a  b", "c d"]);
        assert_eq!(split("a\\ b c").unwrap(), ["a b", "c"]);
        assert_eq!(split("x \"\" y").unwrap(), ["x", "", "y"]);
        assert_eq!(split("\"a\\\"b\" 'c\\d'").unwrap(), ["a\"b", "c\\d"]);
        assert_eq!(split("a'b'\"c\"").unwrap(), ["abc"]);
        assert_eq!(split("\"a\\nb\"").unwrap(), ["a\\nb"]);
        assert!(split("   ").unwrap().is_empty());
    }

    #[test]
    fn unterminated_is_error() {
        assert!(split("echo 'oops").is_err());
        assert!(split("echo \"oops").is_err());
    }
}
