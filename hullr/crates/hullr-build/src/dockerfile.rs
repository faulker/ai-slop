//! Dockerfile parser.
//!
//! [`parse`] turns Dockerfile text into a [`Dockerfile`]: global `ARG`s plus
//! a list of [`Stage`]s. Arguments are kept raw (quotes and `$VAR`
//! references intact); the builder expands them per stage using
//! [`crate::lex::expand`] because variable scope depends on build state.

use crate::lex::{parse_json_array, split_words};
use hullr_core::{Error, Result};

/// Shell form (`RUN echo hi`) or exec form (`RUN ["echo", "hi"]`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CmdForm {
    Exec(Vec<String>),
    Shell(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArgDecl {
    pub name: String,
    /// Raw default, expanded when the ARG is processed.
    pub default: Option<String>,
}

/// An inline heredoc attached to a COPY/ADD.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Heredoc {
    pub name: String,
    pub body: String,
    /// False when the delimiter was quoted (no variable expansion).
    pub expand: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RunSpec {
    pub cmd: Option<CmdForm>,
    /// Raw `--mount=` flags (ignored at build time with a warning).
    pub mounts: Vec<String>,
    /// Other flags (`--network`, `--security`), ignored with a warning.
    pub other_flags: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CopySpec {
    pub from: Option<String>,
    pub chown: Option<String>,
    pub chmod: Option<String>,
    pub sources: Vec<String>,
    pub dest: String,
    pub heredocs: Vec<Heredoc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct HealthSpec {
    pub none: bool,
    pub interval: Option<String>,
    pub timeout: Option<String>,
    pub start_period: Option<String>,
    pub retries: Option<String>,
    pub cmd: Option<CmdForm>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kind {
    Run(RunSpec),
    Cmd(CmdForm),
    Entrypoint(CmdForm),
    /// Raw `(key, value)` words; expanded at build time.
    Env(Vec<(String, String)>),
    Arg(Vec<ArgDecl>),
    Label(Vec<(String, String)>),
    Expose(Vec<String>),
    Copy(CopySpec),
    Add(CopySpec),
    Workdir(String),
    User(String),
    Volume(Vec<String>),
    Stopsignal(String),
    Healthcheck(HealthSpec),
    Shell(Vec<String>),
    /// Stored, ignored at build time.
    Onbuild(String),
    /// Accepted and ignored (`MAINTAINER`).
    Nop,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Instruction {
    /// 1-based line of the instruction keyword.
    pub line: usize,
    /// Keyword plus arguments as written (continuations joined).
    pub text: String,
    pub kind: Kind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stage {
    pub index: usize,
    /// Lower-cased `AS name`.
    pub name: Option<String>,
    /// Raw base reference (may contain `$ARG`).
    pub base: String,
    /// Raw `--platform=` value.
    pub platform: Option<String>,
    pub line: usize,
    pub instructions: Vec<Instruction>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Dockerfile {
    /// `ARG`s before the first `FROM`.
    pub global_args: Vec<ArgDecl>,
    pub stages: Vec<Stage>,
    /// The escape character in effect (`\` or `` ` ``).
    pub escape: char,
}

impl Dockerfile {
    /// Find a stage by (case-insensitive) name or numeric index.
    pub fn find_stage(&self, r: &str) -> Option<usize> {
        let lower = r.to_ascii_lowercase();
        if let Some(i) = self.stages.iter().position(|s| s.name.as_deref() == Some(lower.as_str())) {
            return Some(i);
        }
        r.parse::<usize>().ok().filter(|i| *i < self.stages.len())
    }

    /// Indices of the stages needed to build `target` (or the last stage),
    /// ascending. `expand` resolves `$ARG` in FROM / `--from` references.
    pub fn needed_stages(&self, target: Option<&str>, expand: &dyn Fn(&str) -> String) -> Result<Vec<usize>> {
        if self.stages.is_empty() {
            return Err(Error::Build("Dockerfile has no FROM instruction".into()));
        }
        let goal = match target {
            Some(t) => self
                .stages
                .iter()
                .position(|s| s.name.as_deref() == Some(t.to_ascii_lowercase().as_str()))
                .ok_or_else(|| Error::Build(format!("target stage '{t}' could not be found")))?,
            None => self.stages.len() - 1,
        };
        let mut needed = vec![false; self.stages.len()];
        let mut stack = vec![goal];
        while let Some(i) = stack.pop() {
            if std::mem::replace(&mut needed[i], true) {
                continue;
            }
            let st = &self.stages[i];
            let mut refs = vec![expand(&st.base)];
            for ins in &st.instructions {
                if let Kind::Copy(c) | Kind::Add(c) = &ins.kind {
                    if let Some(f) = &c.from {
                        refs.push(expand(f));
                    }
                }
            }
            for r in refs {
                if let Some(j) = self.find_stage(&r) {
                    if j < i {
                        stack.push(j);
                    }
                }
            }
        }
        Ok((0..needed.len()).filter(|i| needed[*i]).collect())
    }
}

fn err(line: usize, msg: impl std::fmt::Display) -> Error {
    Error::Build(format!("Dockerfile line {line}: {msg}"))
}

/// Parse Dockerfile text.
pub fn parse(text: &str) -> Result<Dockerfile> {
    let lines: Vec<&str> = text.lines().collect();
    let mut esc = '\\';
    // Parser directives: only at the very top, as comments of the form
    // `# key=value`.
    let mut start = 0;
    while start < lines.len() {
        let l = lines[start].trim();
        let Some(body) = l.strip_prefix('#') else { break };
        let Some((k, v)) = body.split_once('=') else { break };
        let k = k.trim().to_ascii_lowercase();
        if k.is_empty() || !k.chars().all(|c| c.is_ascii_alphabetic()) {
            break;
        }
        if k == "escape" {
            match v.trim() {
                "`" => esc = '`',
                "\\" => esc = '\\',
                o => return Err(err(start + 1, format!("invalid escape directive '{o}'"))),
            }
        }
        start += 1;
    }

    let mut df = Dockerfile { escape: esc, ..Default::default() };
    let mut i = start;
    while i < lines.len() {
        let first = lines[i];
        let ln = i + 1;
        let t = first.trim();
        if t.is_empty() || t.starts_with('#') {
            i += 1;
            continue;
        }
        // join continuation lines
        let mut logical = String::new();
        let mut cur = t.to_string();
        i += 1;
        loop {
            let trimmed = cur.trim_end();
            if trimmed.ends_with(esc) && !trimmed.is_empty() {
                logical.push_str(&trimmed[..trimmed.len() - esc.len_utf8()]);
                // pull the next non-comment, non-blank line
                let mut next = None;
                while i < lines.len() {
                    let n = lines[i];
                    i += 1;
                    let nt = n.trim();
                    if nt.is_empty() || nt.starts_with('#') {
                        continue;
                    }
                    next = Some(n.trim_start().to_string());
                    break;
                }
                match next {
                    Some(n) => cur = n,
                    None => break,
                }
            } else {
                logical.push_str(trimmed);
                break;
            }
        }
        let logical = logical.trim().to_string();
        let (kw, rest) = match logical.split_once(char::is_whitespace) {
            Some((k, r)) => (k.to_ascii_uppercase(), r.trim().to_string()),
            None => (logical.to_ascii_uppercase(), String::new()),
        };

        // heredocs
        let mut heredocs = Vec::new();
        let mut full_text = format!("{kw} {rest}");
        if matches!(kw.as_str(), "RUN" | "COPY" | "ADD") {
            for (name, strip, quoted) in find_heredocs(&rest) {
                let mut body = String::new();
                let mut closed = false;
                while i < lines.len() {
                    let raw = lines[i];
                    i += 1;
                    let cmp = if strip { raw.trim_start_matches('\t') } else { raw };
                    if cmp == name {
                        closed = true;
                        full_text.push('\n');
                        full_text.push_str(&body);
                        full_text.push_str(&name);
                        break;
                    }
                    let shown = if strip { raw.trim_start_matches('\t') } else { raw };
                    body.push_str(shown);
                    body.push('\n');
                }
                if !closed {
                    return Err(err(ln, format!("unterminated heredoc '{name}'")));
                }
                heredocs.push(Heredoc { name, body, expand: !quoted });
            }
        }

        let kind = parse_instruction(&kw, &rest, &full_text, heredocs, esc).map_err(|e| match e {
            Error::Build(m) if m.starts_with("Dockerfile line") => Error::Build(m),
            Error::Build(m) | Error::Invalid(m) => err(ln, m),
            o => o,
        })?;
        let ins = |kind| Instruction { line: ln, text: full_text.clone(), kind };

        match kind {
            Parsed::From { base, platform, name } => {
                let index = df.stages.len();
                if let Some(n) = &name {
                    if df.stages.iter().any(|s| s.name.as_deref() == Some(n)) {
                        return Err(err(ln, format!("duplicate stage name '{n}'")));
                    }
                }
                df.stages.push(Stage { index, name, base, platform, line: ln, instructions: vec![] });
            }
            Parsed::Ins(k) => match df.stages.last_mut() {
                Some(st) => st.instructions.push(ins(k)),
                None => match k {
                    Kind::Arg(a) => df.global_args.extend(a),
                    _ => return Err(err(ln, format!("{kw} before the first FROM"))),
                },
            },
        }
    }
    Ok(df)
}

enum Parsed {
    From { base: String, platform: Option<String>, name: Option<String> },
    Ins(Kind),
}

type Flags = Vec<(String, Option<String>)>;

/// Peel leading `--flag[=value]` words.
fn take_flags(rest: &str) -> (Flags, &str) {
    let mut flags = Vec::new();
    let mut r = rest.trim_start();
    while r.starts_with("--") {
        let end = r.find(char::is_whitespace).unwrap_or(r.len());
        let tok = &r[2..end];
        match tok.split_once('=') {
            Some((k, v)) => flags.push((k.to_ascii_lowercase(), Some(v.to_string()))),
            None => flags.push((tok.to_ascii_lowercase(), None)),
        }
        r = r[end..].trim_start();
    }
    (flags, r)
}

fn cmd_form(s: &str) -> CmdForm {
    match parse_json_array(s) {
        Some(v) => CmdForm::Exec(v),
        None => CmdForm::Shell(s.to_string()),
    }
}

/// `K=V K2="a b"` or legacy `K some value` into raw pairs.
fn parse_pairs(kw: &str, rest: &str, esc: char) -> Result<Vec<(String, String)>> {
    let words = split_words(rest, esc);
    if words.is_empty() {
        return Err(Error::Build(format!("{kw} requires at least one argument")));
    }
    if !words[0].contains('=') {
        // legacy: first word is the key, the rest of the line the value
        let (k, v) = rest
            .split_once(char::is_whitespace)
            .ok_or_else(|| Error::Build(format!("{kw} {rest}: missing value")))?;
        return Ok(vec![(k.to_string(), v.trim().to_string())]);
    }
    words
        .into_iter()
        .map(|w| {
            w.split_once('=')
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .ok_or_else(|| Error::Build(format!("{kw}: '{w}' needs KEY=value")))
        })
        .collect()
}

fn find_heredocs(s: &str) -> Vec<(String, bool, bool)> {
    let chars: Vec<char> = s.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i + 1 < chars.len() {
        if chars[i] == '<' && chars[i + 1] == '<' && (i == 0 || chars[i - 1].is_whitespace()) {
            let mut j = i + 2;
            if chars.get(j) == Some(&'<') {
                i = j + 1;
                continue;
            }
            let strip = chars.get(j) == Some(&'-');
            if strip {
                j += 1;
            }
            let quote = chars.get(j).copied().filter(|c| *c == '"' || *c == '\'');
            if quote.is_some() {
                j += 1;
            }
            let st = j;
            while j < chars.len() && (chars[j].is_ascii_alphanumeric() || chars[j] == '_') {
                j += 1;
            }
            if j > st {
                let name: String = chars[st..j].iter().collect();
                if quote.is_some() && chars.get(j) == quote.as_ref() {
                    j += 1;
                }
                out.push((name, strip, quote.is_some()));
            }
            i = j;
        } else {
            i += 1;
        }
    }
    out
}

fn parse_instruction(kw: &str, rest: &str, full: &str, heredocs: Vec<Heredoc>, esc: char) -> Result<Parsed> {
    let need = |what: &str| -> Result<()> {
        if rest.is_empty() {
            Err(Error::Build(format!("{kw} requires {what}")))
        } else {
            Ok(())
        }
    };
    let k = match kw {
        "FROM" => {
            need("an image")?;
            let (flags, r) = take_flags(rest);
            let mut platform = None;
            for (f, v) in flags {
                match f.as_str() {
                    "platform" => platform = v,
                    o => return Err(Error::Build(format!("unknown FROM flag --{o}"))),
                }
            }
            let w = split_words(r, esc);
            let (base, name) = match w.as_slice() {
                [b] => (b.clone(), None),
                [b, a, n] if a.eq_ignore_ascii_case("as") => (b.clone(), Some(n.to_ascii_lowercase())),
                _ => return Err(Error::Build(format!("invalid FROM '{rest}'"))),
            };
            return Ok(Parsed::From { base, platform, name });
        }
        "RUN" => {
            need("a command")?;
            let (flags, r) = take_flags(rest);
            let mut spec = RunSpec::default();
            for (f, v) in flags {
                let s = match v {
                    Some(v) => format!("--{f}={v}"),
                    None => format!("--{f}"),
                };
                if f == "mount" {
                    spec.mounts.push(s);
                } else {
                    spec.other_flags.push(s);
                }
            }
            let single_heredoc = heredocs.len() == 1 && {
                let w = split_words(r, esc);
                w.len() == 1 && w[0].starts_with("<<")
            };
            spec.cmd = Some(if single_heredoc {
                CmdForm::Shell(heredocs[0].body.clone())
            } else if heredocs.is_empty() {
                cmd_form(r)
            } else {
                // command with heredoc stdin: hand the whole text to the shell
                let flagless = full.strip_prefix("RUN").unwrap_or(full).trim_start();
                let (_, body) = take_flags(flagless);
                CmdForm::Shell(body.to_string())
            });
            Kind::Run(spec)
        }
        "CMD" => {
            need("arguments")?;
            Kind::Cmd(cmd_form(rest))
        }
        "ENTRYPOINT" => {
            need("arguments")?;
            Kind::Entrypoint(cmd_form(rest))
        }
        "ENV" => Kind::Env(parse_pairs(kw, rest, esc)?),
        "LABEL" => Kind::Label(parse_pairs(kw, rest, esc)?),
        "ARG" => {
            need("a name")?;
            Kind::Arg(
                split_words(rest, esc)
                    .into_iter()
                    .map(|w| match w.split_once('=') {
                        Some((n, d)) => ArgDecl { name: n.to_string(), default: Some(d.to_string()) },
                        None => ArgDecl { name: w, default: None },
                    })
                    .collect(),
            )
        }
        "EXPOSE" => {
            need("a port")?;
            Kind::Expose(split_words(rest, esc))
        }
        "COPY" | "ADD" => {
            need("sources and a destination")?;
            let (flags, r) = take_flags(rest);
            let mut spec = CopySpec { heredocs, ..Default::default() };
            for (f, v) in flags {
                match (f.as_str(), v) {
                    ("from", Some(v)) => spec.from = Some(v),
                    ("chown", Some(v)) => spec.chown = Some(v),
                    ("chmod", Some(v)) => spec.chmod = Some(v),
                    ("link" | "parents", _) => {}
                    (o, _) => return Err(Error::Build(format!("unsupported {kw} flag --{o}"))),
                }
            }
            let mut args = match parse_json_array(r) {
                Some(v) => v,
                None => split_words(r, esc),
            };
            if args.len() < 2 {
                return Err(Error::Build(format!("{kw} requires at least two arguments")));
            }
            spec.dest = args.pop().unwrap();
            spec.sources = args;
            if kw == "COPY" {
                Kind::Copy(spec)
            } else {
                Kind::Add(spec)
            }
        }
        "WORKDIR" => {
            need("a path")?;
            Kind::Workdir(rest.to_string())
        }
        "USER" => {
            need("a user")?;
            Kind::User(rest.to_string())
        }
        "VOLUME" => {
            need("a path")?;
            Kind::Volume(parse_json_array(rest).unwrap_or_else(|| split_words(rest, esc)))
        }
        "STOPSIGNAL" => {
            need("a signal")?;
            Kind::Stopsignal(rest.to_string())
        }
        "SHELL" => Kind::Shell(
            parse_json_array(rest).ok_or_else(|| Error::Build("SHELL requires a JSON array".into()))?,
        ),
        "HEALTHCHECK" => {
            need("NONE or CMD")?;
            if rest.eq_ignore_ascii_case("none") {
                Kind::Healthcheck(HealthSpec { none: true, ..Default::default() })
            } else {
                let (flags, r) = take_flags(rest);
                let mut h = HealthSpec::default();
                for (f, v) in flags {
                    match f.as_str() {
                        "interval" => h.interval = v,
                        "timeout" => h.timeout = v,
                        "start-period" => h.start_period = v,
                        "retries" => h.retries = v,
                        "start-interval" => {}
                        o => return Err(Error::Build(format!("unknown HEALTHCHECK flag --{o}"))),
                    }
                }
                let (word, cmd) = r.split_once(char::is_whitespace).unwrap_or((r, ""));
                if !word.eq_ignore_ascii_case("cmd") || cmd.trim().is_empty() {
                    return Err(Error::Build("HEALTHCHECK requires NONE or CMD <command>".into()));
                }
                h.cmd = Some(cmd_form(cmd.trim()));
                Kind::Healthcheck(h)
            }
        }
        "ONBUILD" => {
            need("an instruction")?;
            Kind::Onbuild(rest.to_string())
        }
        "MAINTAINER" => Kind::Nop,
        other => return Err(Error::Build(format!("unknown instruction '{other}'"))),
    };
    Ok(Parsed::Ins(k))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> Dockerfile {
        parse(s).unwrap()
    }
    fn only(s: &str) -> Kind {
        let d = p(&format!("FROM x\n{s}\n"));
        d.stages[0].instructions[0].kind.clone()
    }

    #[test]
    fn basic_stage_and_comments() {
        let d = p("# syntax=docker/dockerfile:1\n# a comment\nFROM alpine:3 AS Base\n\n# c\nRUN echo hi\n");
        assert_eq!(d.stages.len(), 1);
        assert_eq!(d.stages[0].name.as_deref(), Some("base"));
        assert_eq!(d.stages[0].base, "alpine:3");
        assert_eq!(d.stages[0].instructions[0].line, 6);
        assert_eq!(d.escape, '\\');
    }

    #[test]
    fn continuations_and_comments_inside() {
        let d = p("FROM x\nRUN apk add \\\n   # comment\n   curl \\\n\n   git\n");
        match &d.stages[0].instructions[0].kind {
            Kind::Run(r) => match r.cmd.as_ref().unwrap() {
                CmdForm::Shell(s) => assert_eq!(s.split_whitespace().collect::<Vec<_>>(), ["apk", "add", "curl", "git"]),
                o => panic!("{o:?}"),
            },
            o => panic!("{o:?}"),
        }
    }

    #[test]
    fn escape_directive() {
        let d = p("# escape=`\nFROM x\nRUN echo a `\n  b\nWORKDIR C:\\app\n");
        assert_eq!(d.escape, '`');
        assert!(matches!(&d.stages[0].instructions[0].kind, Kind::Run(r) if matches!(&r.cmd, Some(CmdForm::Shell(s)) if s.contains('a') && s.contains('b'))));
        assert_eq!(d.stages[0].instructions[1].kind, Kind::Workdir("C:\\app".into()));
        assert!(parse("# escape=x\nFROM a").is_err());
    }

    #[test]
    fn json_vs_shell() {
        assert_eq!(only(r#"CMD ["a", "b"]"#), Kind::Cmd(CmdForm::Exec(vec!["a".into(), "b".into()])));
        assert_eq!(only("CMD a b"), Kind::Cmd(CmdForm::Shell("a b".into())));
        assert_eq!(only("CMD [not json]"), Kind::Cmd(CmdForm::Shell("[not json]".into())));
        assert_eq!(only(r#"ENTRYPOINT ["/e"]"#), Kind::Entrypoint(CmdForm::Exec(vec!["/e".into()])));
    }

    #[test]
    fn env_forms() {
        assert_eq!(only("ENV A=1 B=\"two words\" C='x y'"), Kind::Env(vec![
            ("A".into(), "1".into()), ("B".into(), "\"two words\"".into()), ("C".into(), "'x y'".into())]));
        assert_eq!(only("ENV KEY some value here"), Kind::Env(vec![("KEY".into(), "some value here".into())]));
        assert_eq!(only("LABEL a=b"), Kind::Label(vec![("a".into(), "b".into())]));
        assert!(parse("FROM x\nENV\n").is_err());
    }

    #[test]
    fn arg_before_from() {
        let d = p("ARG V=3.19\nARG X\nFROM alpine:${V}\nARG Y=1 Z\n");
        assert_eq!(d.global_args.len(), 2);
        assert_eq!(d.global_args[0], ArgDecl { name: "V".into(), default: Some("3.19".into()) });
        assert_eq!(d.stages[0].base, "alpine:${V}");
        match &d.stages[0].instructions[0].kind {
            Kind::Arg(a) => assert_eq!(a.len(), 2),
            o => panic!("{o:?}"),
        }
        assert!(parse("RUN x\nFROM a").is_err());
    }

    #[test]
    fn from_platform_and_errors() {
        let d = p("FROM --platform=linux/arm64 alpine AS a");
        assert_eq!(d.stages[0].platform.as_deref(), Some("linux/arm64"));
        assert!(parse("FROM a b c").is_err());
        assert!(parse("FROM a AS x\nFROM b AS X").is_err());
        assert!(parse("").unwrap().stages.is_empty());
    }

    #[test]
    fn unknown_instruction_has_line() {
        let e = parse("FROM x\n\nBOGUS thing\n").unwrap_err();
        let m = e.to_string();
        assert!(m.contains("line 3") && m.contains("BOGUS"), "{m}");
    }

    #[test]
    fn copy_add_flags() {
        match only("COPY --from=build --chown=1:2 --chmod=755 a b /dst/") {
            Kind::Copy(c) => {
                assert_eq!(c.from.as_deref(), Some("build"));
                assert_eq!(c.chown.as_deref(), Some("1:2"));
                assert_eq!(c.chmod.as_deref(), Some("755"));
                assert_eq!(c.sources, ["a", "b"]);
                assert_eq!(c.dest, "/dst/");
            }
            o => panic!("{o:?}"),
        }
        match only(r#"ADD ["my file", "/d"]"#) {
            Kind::Add(c) => assert_eq!(c.sources, ["my file"]),
            o => panic!("{o:?}"),
        }
        assert!(parse("FROM x\nCOPY a\n").is_err());
        assert!(parse("FROM x\nCOPY --bogus=1 a b\n").is_err());
    }

    #[test]
    fn run_flags() {
        match only("RUN --mount=type=cache,target=/c --network=none echo hi") {
            Kind::Run(r) => {
                assert_eq!(r.mounts, ["--mount=type=cache,target=/c"]);
                assert_eq!(r.other_flags, ["--network=none"]);
                assert_eq!(r.cmd, Some(CmdForm::Shell("echo hi".into())));
            }
            o => panic!("{o:?}"),
        }
    }

    #[test]
    fn run_heredoc_only() {
        let d = p("FROM x\nRUN <<EOF\necho one\necho two\nEOF\nRUN echo after\n");
        assert_eq!(d.stages[0].instructions.len(), 2);
        match &d.stages[0].instructions[0].kind {
            Kind::Run(r) => assert_eq!(r.cmd, Some(CmdForm::Shell("echo one\necho two\n".into()))),
            o => panic!("{o:?}"),
        }
        assert_eq!(d.stages[0].instructions[1].line, 6);
    }

    #[test]
    fn run_heredoc_with_command_and_strip() {
        let d = p("FROM x\nRUN cat <<-EOT > /f\n\tindented\n\tEOT\n");
        match &d.stages[0].instructions[0].kind {
            Kind::Run(r) => match r.cmd.as_ref().unwrap() {
                CmdForm::Shell(s) => assert!(s.starts_with("cat <<-EOT > /f\n") && s.contains("indented")),
                o => panic!("{o:?}"),
            },
            o => panic!("{o:?}"),
        }
        assert!(parse("FROM x\nRUN <<EOF\nnever closed\n").is_err());
    }

    #[test]
    fn copy_heredoc() {
        let d = p("FROM x\nCOPY <<'EOF' /etc/conf\nk=$V\nEOF\n");
        match &d.stages[0].instructions[0].kind {
            Kind::Copy(c) => {
                assert_eq!(c.sources, ["<<'EOF'"]);
                assert_eq!(c.heredocs[0].name, "EOF");
                assert_eq!(c.heredocs[0].body, "k=$V\n");
                assert!(!c.heredocs[0].expand);
            }
            o => panic!("{o:?}"),
        }
    }

    #[test]
    fn healthcheck_forms() {
        assert!(matches!(only("HEALTHCHECK NONE"), Kind::Healthcheck(h) if h.none));
        match only("HEALTHCHECK --interval=5s --retries=2 CMD curl -f http://x") {
            Kind::Healthcheck(h) => {
                assert_eq!(h.interval.as_deref(), Some("5s"));
                assert_eq!(h.retries.as_deref(), Some("2"));
                assert_eq!(h.cmd, Some(CmdForm::Shell("curl -f http://x".into())));
            }
            o => panic!("{o:?}"),
        }
        assert!(parse("FROM x\nHEALTHCHECK curl\n").is_err());
    }

    #[test]
    fn misc_instructions() {
        assert_eq!(only("EXPOSE 80 443/udp"), Kind::Expose(vec!["80".into(), "443/udp".into()]));
        assert_eq!(only(r#"VOLUME ["/a", "/b"]"#), Kind::Volume(vec!["/a".into(), "/b".into()]));
        assert_eq!(only("VOLUME /a /b"), Kind::Volume(vec!["/a".into(), "/b".into()]));
        assert_eq!(only(r#"SHELL ["/bin/bash", "-c"]"#), Kind::Shell(vec!["/bin/bash".into(), "-c".into()]));
        assert!(parse("FROM x\nSHELL bash\n").is_err());
        assert_eq!(only("STOPSIGNAL SIGTERM"), Kind::Stopsignal("SIGTERM".into()));
        assert_eq!(only("ONBUILD RUN x"), Kind::Onbuild("RUN x".into()));
        assert_eq!(only("MAINTAINER me"), Kind::Nop);
        assert_eq!(only("user app"), Kind::User("app".into()));
    }

    #[test]
    fn multi_stage_graph_and_target() {
        let d = p("FROM a AS one\nFROM b AS two\nFROM one AS three\nCOPY --from=two /x /y\nFROM c AS unused\nFROM three AS final\n");
        let id = |s: &str| s.to_string();
        assert_eq!(d.needed_stages(None, &id).unwrap(), [0, 1, 2, 4]);
        assert_eq!(d.needed_stages(Some("TWO"), &id).unwrap(), [1]);
        assert_eq!(d.needed_stages(Some("three"), &id).unwrap(), [0, 1, 2]);
        assert!(d.needed_stages(Some("nope"), &id).is_err());
        assert_eq!(d.find_stage("1"), Some(1));
        assert_eq!(d.find_stage("One"), Some(0));
        assert_eq!(d.find_stage("zzz"), None);
    }

    #[test]
    fn copy_from_index_dependency() {
        let d = p("FROM a\nFROM b\nCOPY --from=0 /x /y\n");
        assert_eq!(d.needed_stages(None, &|s| s.to_string()).unwrap(), [0, 1]);
    }
}
