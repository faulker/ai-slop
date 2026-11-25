//! Word splitting and variable expansion for Dockerfile arguments.
//!
//! Raw instruction text is first split into words with quotes retained
//! ([`split_words`]); each word is later expanded against the stage's
//! variables with quote removal ([`expand`]).

use hullr_core::{Error, Result};

/// Split on unquoted whitespace, keeping quotes and escapes in the output.
pub fn split_words(raw: &str, esc: char) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut chars = raw.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            c if c.is_whitespace() => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            '\'' => {
                cur.push(c);
                for n in chars.by_ref() {
                    cur.push(n);
                    if n == '\'' {
                        break;
                    }
                }
            }
            '"' => {
                cur.push(c);
                while let Some(n) = chars.next() {
                    cur.push(n);
                    if n == esc {
                        if let Some(m) = chars.next() {
                            cur.push(m);
                        }
                    } else if n == '"' {
                        break;
                    }
                }
            }
            c if c == esc => {
                cur.push(c);
                if let Some(n) = chars.next() {
                    cur.push(n);
                }
            }
            c => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// Parse a JSON array of strings (`["a", "b"]`). `None` if `s` is not one.
pub fn parse_json_array(s: &str) -> Option<Vec<String>> {
    let t = s.trim();
    if !t.starts_with('[') || !t.ends_with(']') {
        return None;
    }
    serde_json::from_str::<Vec<String>>(t).ok()
}

pub type Lookup<'a> = &'a dyn Fn(&str) -> Option<String>;

/// Expand one word: remove quotes and escapes, substitute `$VAR`, `${VAR}`,
/// `${VAR:-w}`, `${VAR-w}`, `${VAR:+w}`, `${VAR+w}`, `${VAR:?w}`, `${VAR?w}`.
pub fn expand(word: &str, lookup: Lookup, esc: char) -> Result<String> {
    let chars: Vec<char> = word.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            '\'' => {
                i += 1;
                while i < chars.len() && chars[i] != '\'' {
                    out.push(chars[i]);
                    i += 1;
                }
                i += 1;
            }
            '"' => {
                i += 1;
                while i < chars.len() && chars[i] != '"' {
                    if chars[i] == esc && i + 1 < chars.len() && matches!(chars[i + 1], '"' | '$') || (chars[i] == esc && i + 1 < chars.len() && chars[i + 1] == esc) {
                        out.push(chars[i + 1]);
                        i += 2;
                    } else if chars[i] == '$' {
                        i = subst(&chars, i, lookup, esc, &mut out)?;
                    } else {
                        out.push(chars[i]);
                        i += 1;
                    }
                }
                i += 1;
            }
            c if c == esc => {
                if i + 1 < chars.len() {
                    out.push(chars[i + 1]);
                    i += 2;
                } else {
                    i += 1;
                }
            }
            '$' => i = subst(&chars, i, lookup, esc, &mut out)?,
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    Ok(out)
}

/// Substitute variables only (no quote handling), for heredoc bodies.
pub fn substitute_plain(text: &str, lookup: Lookup, esc: char) -> Result<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '$' {
            i = subst(&chars, i, lookup, esc, &mut out)?;
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }
    Ok(out)
}

fn is_name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// Handle a `$` at `chars[i]`; returns the index after the construct.
fn subst(chars: &[char], i: usize, lookup: Lookup, esc: char, out: &mut String) -> Result<usize> {
    let next = chars.get(i + 1).copied();
    match next {
        Some('{') => {
            let mut depth = 1;
            let mut j = i + 2;
            while j < chars.len() {
                match chars[j] {
                    '{' => depth += 1,
                    '}' => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    _ => {}
                }
                j += 1;
            }
            if j >= chars.len() {
                return Err(Error::Build(format!(
                    "unterminated variable reference in '{}'",
                    chars.iter().collect::<String>()
                )));
            }
            let inner: Vec<char> = chars[i + 2..j].to_vec();
            let mut n = 0;
            while n < inner.len() && is_name_char(inner[n]) {
                n += 1;
            }
            let name: String = inner[..n].iter().collect();
            if name.is_empty() {
                return Err(Error::Build("empty variable name in '${}'".into()));
            }
            let rest: String = inner[n..].iter().collect();
            let val = lookup(&name);
            let set = val.is_some();
            let non_empty = val.as_deref().map(|v| !v.is_empty()).unwrap_or(false);
            let (colon, op_rest) = match rest.strip_prefix(':') {
                Some(r) => (true, r),
                None => (false, rest.as_str()),
            };
            let present = if colon { non_empty } else { set };
            let mut oc = op_rest.chars();
            let res = match (rest.is_empty(), oc.next()) {
                (true, _) => val.unwrap_or_default(),
                (false, Some('-')) => {
                    if present { val.unwrap_or_default() } else { expand(oc.as_str(), lookup, esc)? }
                }
                (false, Some('+')) => {
                    if present { expand(oc.as_str(), lookup, esc)? } else { String::new() }
                }
                (false, Some('?')) => {
                    if present {
                        val.unwrap_or_default()
                    } else {
                        let m = expand(oc.as_str(), lookup, esc)?;
                        let m = if m.is_empty() { "not set".to_string() } else { m };
                        return Err(Error::Build(format!("{name}: {m}")));
                    }
                }
                _ => return Err(Error::Build(format!("unsupported variable modifier in '${{{}}}'", inner.iter().collect::<String>()))),
            };
            out.push_str(&res);
            Ok(j + 1)
        }
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {
            let mut j = i + 1;
            while j < chars.len() && is_name_char(chars[j]) {
                j += 1;
            }
            let name: String = chars[i + 1..j].iter().collect();
            out.push_str(&lookup(&name).unwrap_or_default());
            Ok(j)
        }
        _ => {
            out.push('$');
            Ok(i + 1)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lk(k: &str) -> Option<String> {
        match k {
            "A" => Some("alpha".into()),
            "E" => Some(String::new()),
            "SP" => Some("a b".into()),
            _ => None,
        }
    }
    fn ex(s: &str) -> String {
        expand(s, &lk, '\\').unwrap()
    }

    #[test]
    fn words_keep_quotes() {
        assert_eq!(split_words(r#"a="x y" b 'c d' e\ f"#, '\\'), [r#"a="x y""#, "b", "'c d'", r"e\ f"]);
        assert!(split_words("   ", '\\').is_empty());
    }

    #[test]
    fn json_arrays() {
        assert_eq!(parse_json_array(r#"["a", "b c"]"#).unwrap(), ["a", "b c"]);
        assert!(parse_json_array("[a, b]").is_none());
        assert!(parse_json_array("echo hi").is_none());
        assert_eq!(parse_json_array("[]").unwrap(), Vec::<String>::new());
    }

    #[test]
    fn expansion() {
        assert_eq!(ex("$A"), "alpha");
        assert_eq!(ex("${A}x"), "alphax");
        assert_eq!(ex("$U|"), "|");
        assert_eq!(ex("${U:-d}"), "d");
        assert_eq!(ex("${E:-d}"), "d");
        assert_eq!(ex("${E-d}"), "");
        assert_eq!(ex("${A:-d}"), "alpha");
        assert_eq!(ex("${A:+yes}"), "yes");
        assert_eq!(ex("${U:+yes}"), "");
        assert_eq!(ex("${U:-${A}z}"), "alphaz");
        assert_eq!(ex("'$A'"), "$A");
        assert_eq!(ex("\"$A $SP\""), "alpha a b");
        assert_eq!(ex(r"\$A"), "$A");
        assert_eq!(ex(r#""\$A \"q\"""#), "$A \"q\"");
        assert_eq!(ex("a\\ b"), "a b");
        assert_eq!(ex("cost$"), "cost$");
        assert!(expand("${U:?need}", &lk, '\\').is_err());
        assert!(expand("${A", &lk, '\\').is_err());
    }

    #[test]
    fn backtick_escape() {
        assert_eq!(expand("`$A", &lk, '`').unwrap(), "$A");
        assert_eq!(expand("C:\\x", &lk, '`').unwrap(), "C:\\x");
    }

    #[test]
    fn plain_substitution_keeps_quotes() {
        assert_eq!(substitute_plain("echo '$A' \"x\"", &lk, '\\').unwrap(), "echo 'alpha' \"x\"");
    }
}
