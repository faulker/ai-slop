//! `.dockerignore` matching and component globbing.
//!
//! Pattern syntax: `*` (any run except `/`), `?` (one char except `/`),
//! `[a-z]` classes, `\x` escapes, `**` (any run including `/`), and a leading
//! `!` to re-include. The last matching pattern wins; a path is also ignored
//! when one of its parent directories matches.

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Lit(char),
    AnyChar,
    Star,
    DoubleStar,
    /// `**/`: zero or more whole directories.
    DoubleStarSlash,
    Class { ranges: Vec<(char, char)>, negated: bool },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Pattern {
    toks: Vec<Tok>,
}

impl Pattern {
    pub fn new(pat: &str) -> Self {
        let c: Vec<char> = pat.chars().collect();
        let mut toks = Vec::new();
        let mut i = 0;
        while i < c.len() {
            match c[i] {
                '*' if c.get(i + 1) == Some(&'*') => {
                    let mut j = i + 2;
                    while c.get(j) == Some(&'*') {
                        j += 1;
                    }
                    if c.get(j) == Some(&'/') {
                        toks.push(Tok::DoubleStarSlash);
                        i = j + 1;
                    } else {
                        toks.push(Tok::DoubleStar);
                        i = j;
                    }
                }
                '*' => {
                    toks.push(Tok::Star);
                    i += 1;
                }
                '?' => {
                    toks.push(Tok::AnyChar);
                    i += 1;
                }
                '\\' if i + 1 < c.len() => {
                    toks.push(Tok::Lit(c[i + 1]));
                    i += 2;
                }
                '[' => {
                    if let Some(end) = c[i + 1..].iter().position(|x| *x == ']') {
                        let body = &c[i + 1..i + 1 + end];
                        let (negated, body) = match body.first() {
                            Some('^') | Some('!') => (true, &body[1..]),
                            _ => (false, body),
                        };
                        let mut ranges = Vec::new();
                        let mut k = 0;
                        while k < body.len() {
                            if k + 2 < body.len() && body[k + 1] == '-' {
                                ranges.push((body[k], body[k + 2]));
                                k += 3;
                            } else {
                                ranges.push((body[k], body[k]));
                                k += 1;
                            }
                        }
                        toks.push(Tok::Class { ranges, negated });
                        i += end + 2;
                    } else {
                        toks.push(Tok::Lit('['));
                        i += 1;
                    }
                }
                ch => {
                    toks.push(Tok::Lit(ch));
                    i += 1;
                }
            }
        }
        Self { toks }
    }

    pub fn matches(&self, text: &str) -> bool {
        let t: Vec<char> = text.chars().collect();
        m(&self.toks, &t)
    }
}

fn m(p: &[Tok], t: &[char]) -> bool {
    let Some(first) = p.first() else { return t.is_empty() };
    match first {
        Tok::Lit(c) => t.first() == Some(c) && m(&p[1..], &t[1..]),
        Tok::AnyChar => t.first().map(|c| *c != '/').unwrap_or(false) && m(&p[1..], &t[1..]),
        Tok::Class { ranges, negated } => match t.first() {
            Some(c) if *c != '/' => {
                let inside = ranges.iter().any(|(a, b)| *a <= *c && *c <= *b);
                inside != *negated && m(&p[1..], &t[1..])
            }
            _ => false,
        },
        Tok::Star => {
            let mut n = 0;
            loop {
                if m(&p[1..], &t[n..]) {
                    return true;
                }
                if n < t.len() && t[n] != '/' {
                    n += 1;
                } else {
                    return false;
                }
            }
        }
        Tok::DoubleStar => (0..=t.len()).any(|n| m(&p[1..], &t[n..])),
        Tok::DoubleStarSlash => {
            if m(&p[1..], t) {
                return true;
            }
            // consume whole directory components
            for (i, c) in t.iter().enumerate() {
                if *c == '/' && m(&p[1..], &t[i + 1..]) {
                    return true;
                }
            }
            false
        }
    }
}

/// Does `name` (one path component) match the glob `pat`?
pub fn glob_component(pat: &str, name: &str) -> bool {
    Pattern::new(pat).matches(name)
}

pub fn has_glob_meta(s: &str) -> bool {
    s.contains(['*', '?', '['])
}

#[derive(Debug, Clone, Default)]
pub struct DockerIgnore {
    rules: Vec<(Pattern, bool)>,
}

impl DockerIgnore {
    pub fn parse(text: &str) -> Self {
        let mut rules = Vec::new();
        for line in text.lines() {
            let l = line.trim();
            if l.is_empty() || l.starts_with('#') {
                continue;
            }
            let (neg, l) = match l.strip_prefix('!') {
                Some(r) => (true, r.trim()),
                None => (false, l),
            };
            let l = l.trim_start_matches("./").trim_start_matches('/').trim_end_matches('/');
            if l.is_empty() {
                continue;
            }
            rules.push((Pattern::new(l), neg));
        }
        Self { rules }
    }

    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    /// `rel` is a `/`-separated path relative to the context root.
    pub fn is_ignored(&self, rel: &str) -> bool {
        let rel = rel.trim_start_matches("./").trim_matches('/');
        if rel.is_empty() || self.rules.is_empty() {
            return false;
        }
        let mut ignored = false;
        for (pat, neg) in &self.rules {
            if pat.matches(rel) || parents(rel).any(|p| pat.matches(p)) {
                ignored = !*neg;
            }
        }
        ignored
    }
}

fn parents(rel: &str) -> impl Iterator<Item = &str> {
    rel.match_indices('/').map(move |(i, _)| &rel[..i])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ig(s: &str) -> DockerIgnore {
        DockerIgnore::parse(s)
    }

    #[test]
    fn literals_and_dirs() {
        let d = ig("node_modules\n*.log\n# comment\n\ntmp/\n");
        assert!(d.is_ignored("node_modules"));
        assert!(d.is_ignored("node_modules/a/b.js"));
        assert!(d.is_ignored("x.log"));
        assert!(!d.is_ignored("sub/x.log"));
        assert!(d.is_ignored("tmp/file"));
        assert!(!d.is_ignored("src/main.rs"));
    }

    #[test]
    fn star_question_class() {
        let d = ig("a?c\nb[0-9]\nc[^x]d");
        assert!(d.is_ignored("abc"));
        assert!(!d.is_ignored("a/c"));
        assert!(d.is_ignored("b5"));
        assert!(!d.is_ignored("bx"));
        assert!(d.is_ignored("cyd"));
        assert!(!d.is_ignored("cxd"));
    }

    #[test]
    fn double_star() {
        let d = ig("**/*.tmp\ndocs/**/draft.md\nbuild/**");
        assert!(d.is_ignored("a.tmp"));
        assert!(d.is_ignored("x/y/a.tmp"));
        assert!(d.is_ignored("docs/draft.md"));
        assert!(d.is_ignored("docs/a/b/draft.md"));
        assert!(!d.is_ignored("docs/final.md"));
        assert!(d.is_ignored("build/out/x"));
    }

    #[test]
    fn negation_last_wins() {
        let d = ig("*.md\n!README.md\n");
        assert!(d.is_ignored("CHANGES.md"));
        assert!(!d.is_ignored("README.md"));
        let d = ig("!keep\nkeep");
        assert!(d.is_ignored("keep"));
        let d = ig("docs\n!docs/api.md");
        assert!(!d.is_ignored("docs/api.md"));
        assert!(d.is_ignored("docs/other.md"));
    }

    #[test]
    fn leading_slash_and_dot() {
        let d = ig("/root-only\n./dotted");
        assert!(d.is_ignored("root-only"));
        assert!(!d.is_ignored("sub/root-only"));
        assert!(d.is_ignored("dotted"));
    }

    #[test]
    fn escapes_and_components() {
        let d = ig(r"a\*b");
        assert!(d.is_ignored("a*b"));
        assert!(!d.is_ignored("axb"));
        assert!(glob_component("*.rs", "main.rs"));
        assert!(!glob_component("*.rs", "main.py"));
        assert!(has_glob_meta("a/*"));
        assert!(!has_glob_meta("plain/path"));
    }

    #[test]
    fn empty_ignores_nothing() {
        let d = ig("");
        assert!(d.is_empty());
        assert!(!d.is_ignored("anything"));
    }
}
