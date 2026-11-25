//! Compose variable interpolation and `.env` file parsing.
//!
//! Supported forms: `$VAR`, `${VAR}`, `${VAR:-d}`, `${VAR-d}`, `${VAR:+r}`,
//! `${VAR+r}`, `${VAR:?err}`, `${VAR?err}` and the `$$` escape.

use hullr_core::{Error, Result};

fn is_name_start(c: u8) -> bool {
    c.is_ascii_alphabetic() || c == b'_'
}
fn is_name_char(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}

/// Interpolate `s`, looking variables up with `lookup`. Unset variables
/// without a modifier expand to the empty string.
pub fn interpolate(s: &str, lookup: &dyn Fn(&str) -> Option<String>) -> Result<String> {
    let b = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] != b'$' {
            let ch = s[i..].chars().next().unwrap();
            out.push(ch);
            i += ch.len_utf8();
            continue;
        }
        match b.get(i + 1) {
            Some(b'$') => {
                out.push('$');
                i += 2;
            }
            Some(b'{') => {
                let mut depth = 1;
                let mut j = i + 2;
                while j < b.len() {
                    match b[j] {
                        b'{' => depth += 1,
                        b'}' => {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                        }
                        _ => {}
                    }
                    j += 1;
                }
                if j >= b.len() {
                    return Err(Error::Invalid(format!("unterminated '${{' in '{s}'")));
                }
                out.push_str(&expand_braced(&s[i + 2..j], lookup, s)?);
                i = j + 1;
            }
            Some(&c) if is_name_start(c) => {
                let mut j = i + 1;
                while j < b.len() && is_name_char(b[j]) {
                    j += 1;
                }
                out.push_str(&lookup(&s[i + 1..j]).unwrap_or_default());
                i = j;
            }
            _ => {
                out.push('$');
                i += 1;
            }
        }
    }
    Ok(out)
}

fn expand_braced(inner: &str, lookup: &dyn Fn(&str) -> Option<String>, whole: &str) -> Result<String> {
    let b = inner.as_bytes();
    if b.is_empty() || !is_name_start(b[0]) {
        return Err(Error::Invalid(format!("invalid interpolation format in '{whole}'")));
    }
    let mut n = 1;
    while n < b.len() && is_name_char(b[n]) {
        n += 1;
    }
    let name = &inner[..n];
    let rest = &inner[n..];
    let val = lookup(name);
    let set = val.is_some();
    let non_empty = val.as_deref().map(|v| !v.is_empty()).unwrap_or(false);
    let (colon, op_rest) = match rest.strip_prefix(':') {
        Some(r) => (true, r),
        None => (false, rest),
    };
    let present = if colon { non_empty } else { set };
    if rest.is_empty() {
        return Ok(val.unwrap_or_default());
    }
    let op = op_rest.chars().next();
    let arg = op.map(|c| &op_rest[c.len_utf8()..]).unwrap_or("");
    match op {
        Some('-') => {
            if present {
                Ok(val.unwrap_or_default())
            } else {
                interpolate(arg, lookup)
            }
        }
        Some('+') => {
            if present {
                interpolate(arg, lookup)
            } else {
                Ok(String::new())
            }
        }
        Some('?') => {
            if present {
                Ok(val.unwrap_or_default())
            } else {
                let msg = interpolate(arg, lookup)?;
                let msg = if msg.is_empty() { "required variable is missing".to_string() } else { msg };
                Err(Error::Invalid(format!("{name}: {msg}")))
            }
        }
        _ => Err(Error::Invalid(format!("invalid interpolation format in '{whole}'"))),
    }
}

/// Parse `.env` / `env_file` content into ordered `(key, value)` pairs.
pub fn parse_dotenv(text: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = line.strip_prefix("export ").map(str::trim_start).unwrap_or(line);
        let Some((k, v)) = line.split_once('=') else { continue };
        let key = k.trim();
        if key.is_empty() {
            continue;
        }
        out.push((key.to_string(), parse_dotenv_value(v.trim_start())));
    }
    out
}

fn parse_dotenv_value(v: &str) -> String {
    if let Some(rest) = v.strip_prefix('"') {
        let mut out = String::new();
        let mut chars = rest.chars();
        while let Some(c) = chars.next() {
            match c {
                '"' => break,
                '\\' => match chars.next() {
                    Some('n') => out.push('\n'),
                    Some('t') => out.push('\t'),
                    Some('r') => out.push('\r'),
                    Some(o) => out.push(o),
                    None => out.push('\\'),
                },
                c => out.push(c),
            }
        }
        out
    } else if let Some(rest) = v.strip_prefix('\'') {
        rest.split('\'').next().unwrap_or("").to_string()
    } else {
        let end = v.find(" #").unwrap_or(v.len());
        v[..end].trim_end().to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(s: &str) -> Result<String> {
        let lookup = |k: &str| match k {
            "A" => Some("alpha".to_string()),
            "EMPTY" => Some(String::new()),
            _ => None,
        };
        interpolate(s, &lookup)
    }

    #[test]
    fn basic_forms() {
        assert_eq!(run("$A").unwrap(), "alpha");
        assert_eq!(run("${A}").unwrap(), "alpha");
        assert_eq!(run("x-$A-y").unwrap(), "x-alpha-y");
        assert_eq!(run("$UNSET|").unwrap(), "|");
        assert_eq!(run("price $$5").unwrap(), "price $5");
        assert_eq!(run("$$A").unwrap(), "$A");
        assert_eq!(run("cost $ 5").unwrap(), "cost $ 5");
        assert_eq!(run("héllo $A").unwrap(), "héllo alpha");
    }

    #[test]
    fn defaults() {
        assert_eq!(run("${UNSET:-d}").unwrap(), "d");
        assert_eq!(run("${EMPTY:-d}").unwrap(), "d");
        assert_eq!(run("${EMPTY-d}").unwrap(), "");
        assert_eq!(run("${UNSET-d}").unwrap(), "d");
        assert_eq!(run("${A:-d}").unwrap(), "alpha");
        assert_eq!(run("${UNSET:-${A}x}").unwrap(), "alphax");
        assert_eq!(run("${UNSET:-a:b}").unwrap(), "a:b");
    }

    #[test]
    fn alternate_and_required() {
        assert_eq!(run("${A:+yes}").unwrap(), "yes");
        assert_eq!(run("${EMPTY:+yes}").unwrap(), "");
        assert_eq!(run("${EMPTY+yes}").unwrap(), "yes");
        assert_eq!(run("${UNSET+yes}").unwrap(), "");
        assert!(run("${UNSET:?must set}").unwrap_err().to_string().contains("must set"));
        assert!(run("${EMPTY:?boom}").is_err());
        assert_eq!(run("${EMPTY?boom}").unwrap(), "");
        assert!(run("${UNSET?boom}").is_err());
        assert_eq!(run("${A:?boom}").unwrap(), "alpha");
    }

    #[test]
    fn invalid() {
        assert!(run("${A").is_err());
        assert!(run("${}").is_err());
        assert!(run("${1A}").is_err());
        assert!(run("${A%x}").is_err());
    }

    #[test]
    fn dotenv() {
        let t = "# c\nA=1\nexport B = two\nC=\"x y\\n\"\nD='lit $x'\nE=val # comment\nF=a#b\n\nBARE\n";
        let v = parse_dotenv(t);
        let get = |k: &str| v.iter().find(|(a, _)| a == k).map(|(_, b)| b.clone());
        assert_eq!(get("A").as_deref(), Some("1"));
        assert_eq!(get("B").as_deref(), Some("two"));
        assert_eq!(get("C").as_deref(), Some("x y\n"));
        assert_eq!(get("D").as_deref(), Some("lit $x"));
        assert_eq!(get("E").as_deref(), Some("val"));
        assert_eq!(get("F").as_deref(), Some("a#b"));
        assert_eq!(get("BARE"), None);
    }
}
