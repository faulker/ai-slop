//! Pure formatting and parsing helpers used by the state and the views.

use hullr_core::{ContainerSpec, Mount, PortMapping, Protocol, RestartPolicy};
use std::path::Path;

/// Remove ANSI escape sequences and other control characters so log text can
/// never corrupt the screen. Tabs become four spaces.
pub fn sanitize_line(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\u{1b}' => match chars.peek() {
                // CSI: ESC [ params final-byte (0x40..=0x7e)
                Some('[') => {
                    chars.next();
                    for n in chars.by_ref() {
                        if ('\u{40}'..='\u{7e}').contains(&n) {
                            break;
                        }
                    }
                }
                // OSC: ESC ] ... BEL or ESC \
                Some(']') => {
                    chars.next();
                    while let Some(n) = chars.next() {
                        if n == '\u{7}' {
                            break;
                        }
                        if n == '\u{1b}' {
                            chars.next();
                            break;
                        }
                    }
                }
                Some(_) => {
                    chars.next();
                }
                None => {}
            },
            '\t' => out.push_str("    "),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out
}

/// Shorten `s` to at most `max` characters, ending with an ellipsis.
pub fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    if max == 0 {
        return String::new();
    }
    let mut out: String = s.chars().take(max - 1).collect();
    out.push('\u{2026}');
    out
}

/// Compact port list: `8080->80, 53->53/udp`.
pub fn ports_summary(ports: &[PortMapping]) -> String {
    ports
        .iter()
        .map(|p| {
            let proto = if p.protocol == Protocol::Udp { "/udp" } else { "" };
            format!("{}->{}{}", p.host_port, p.container_port, proto)
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Drop the noisy `docker.io/library/` prefix of normalized references.
pub fn short_ref(r: &str) -> &str {
    r.strip_prefix("docker.io/library/")
        .or_else(|| r.strip_prefix("docker.io/"))
        .unwrap_or(r)
}

/// `YYYY-MM-DD HH:MM` in UTC.
pub fn fmt_datetime(unix: i64) -> String {
    let days = unix.div_euclid(86400);
    let secs = unix.rem_euclid(86400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02} {:02}:{:02}", secs / 3600, (secs % 3600) / 60)
}

/// `5 minutes ago`.
pub fn ago(now: i64, ts: i64) -> String {
    format!("{} ago", hullr_core::container::human_duration(now - ts))
}

/// A text progress bar such as `[#####-----]`.
pub fn bar(current: u64, total: u64, width: usize) -> String {
    let filled = if total == 0 {
        0
    } else {
        ((current.min(total) as f64 / total as f64) * width as f64).round() as usize
    };
    format!("[{}{}]", "#".repeat(filled), "-".repeat(width.saturating_sub(filled)))
}

/// Split a command line into words, honouring single and double quotes.
pub fn split_words(line: &str) -> Result<Vec<String>, String> {
    let mut words = Vec::new();
    let mut cur = String::new();
    let mut in_word = false;
    let mut quote: Option<char> = None;
    for c in line.chars() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), c) => cur.push(c),
            (None, '"') | (None, '\'') => {
                quote = Some(c);
                in_word = true;
            }
            (None, c) if c.is_whitespace() => {
                if in_word {
                    words.push(std::mem::take(&mut cur));
                    in_word = false;
                }
            }
            (None, c) => {
                cur.push(c);
                in_word = true;
            }
        }
    }
    if quote.is_some() {
        return Err("unclosed quote".into());
    }
    if in_word {
        words.push(cur);
    }
    Ok(words)
}

/// Parse the one-line "run" prompt, a small subset of `docker run`:
/// `[-p 8080:80] [-e K=V] [-v host:ctr] [--name n] [--rm] [--restart p] IMAGE [CMD...]`.
pub fn parse_run_line(line: &str, base: &Path) -> Result<ContainerSpec, String> {
    let words = split_words(line)?;
    let mut spec = ContainerSpec::default();
    let mut it = words.into_iter().peekable();
    let mut image: Option<String> = None;
    let mut cmd: Vec<String> = Vec::new();
    while let Some(w) = it.next() {
        if image.is_some() {
            cmd.push(w);
            continue;
        }
        // Support `--flag=value`.
        let (flag, inline) = match w.split_once('=') {
            Some((f, v)) if f.starts_with("--") => (f.to_string(), Some(v.to_string())),
            _ => (w.clone(), None),
        };
        let mut value = |name: &str| -> Result<String, String> {
            inline
                .clone()
                .or_else(|| it.next())
                .ok_or_else(|| format!("{name} needs a value"))
        };
        match flag.as_str() {
            "-p" | "--publish" => {
                let v = value("-p")?;
                spec.ports.push(PortMapping::parse(&v).map_err(|e| e.to_string())?);
            }
            "-e" | "--env" => spec.env.push(value("-e")?),
            "-v" | "--volume" => {
                let v = value("-v")?;
                spec.mounts.push(Mount::parse(&v, base).map_err(|e| e.to_string())?);
            }
            "--name" => spec.name = Some(value("--name")?),
            "--restart" => {
                let v = value("--restart")?;
                spec.restart = RestartPolicy::parse(&v).map_err(|e| e.to_string())?;
            }
            "--rm" => spec.auto_remove = true,
            "-d" | "--detach" => {}
            f if f.starts_with('-') => return Err(format!("unknown option {f}")),
            _ => image = Some(w),
        }
    }
    spec.image = image.ok_or("type an image name, e.g. nginx:latest -p 8080:80")?;
    if !cmd.is_empty() {
        spec.cmd = Some(cmd);
    }
    Ok(spec)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_ansi_and_controls() {
        assert_eq!(sanitize_line("\u{1b}[31mred\u{1b}[0m ok\r"), "red ok");
        assert_eq!(sanitize_line("a\tb"), "a    b");
        assert_eq!(sanitize_line("\u{1b}]0;title\u{7}x"), "x");
    }

    #[test]
    fn truncates() {
        assert_eq!(truncate("hello", 10), "hello");
        assert_eq!(truncate("hello world", 6), "hello\u{2026}");
        assert_eq!(truncate("x", 0), "");
    }

    #[test]
    fn datetime() {
        assert_eq!(fmt_datetime(0), "1970-01-01 00:00");
        assert_eq!(fmt_datetime(1_700_000_000), "2023-11-14 22:13");
    }

    #[test]
    fn ports() {
        let p = vec![
            PortMapping::parse("8080:80").unwrap(),
            PortMapping::parse("53:53/udp").unwrap(),
        ];
        assert_eq!(ports_summary(&p), "8080->80, 53->53/udp");
        assert_eq!(ports_summary(&[]), "");
    }

    #[test]
    fn bars() {
        assert_eq!(bar(5, 10, 10), "[#####-----]");
        assert_eq!(bar(0, 0, 4), "[----]");
        assert_eq!(bar(20, 10, 4), "[####]");
    }

    #[test]
    fn short_refs() {
        assert_eq!(short_ref("docker.io/library/nginx:latest"), "nginx:latest");
        assert_eq!(short_ref("ghcr.io/a/b:1"), "ghcr.io/a/b:1");
    }

    #[test]
    fn words() {
        assert_eq!(split_words("a 'b c' \"d e\" f").unwrap(), vec!["a", "b c", "d e", "f"]);
        assert!(split_words("a 'b").is_err());
    }

    #[test]
    fn run_line() {
        let s = parse_run_line(
            "-p 8080:80 -e A=1 --name web --restart=always nginx:latest nginx -g 'daemon off;'",
            Path::new("/p"),
        )
        .unwrap();
        assert_eq!(s.image, "nginx:latest");
        assert_eq!(s.name.as_deref(), Some("web"));
        assert_eq!(s.ports[0].host_port, 8080);
        assert_eq!(s.env, vec!["A=1"]);
        assert_eq!(s.restart, RestartPolicy::Always);
        assert_eq!(s.cmd.unwrap(), vec!["nginx", "-g", "daemon off;"]);
        assert!(parse_run_line("-p 80", Path::new("/")).is_err());
        assert!(parse_run_line("--bogus x", Path::new("/")).is_err());
        assert!(parse_run_line("-p x:y nginx", Path::new("/")).is_err());
    }
}
