//! Terminal output helpers: colors, tables, progress.

use hullr_core::api::{Progress, ProgressFn};
use std::collections::HashMap;
use std::io::{IsTerminal, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

static COLOR: AtomicBool = AtomicBool::new(true);

pub fn init_color(disabled: bool) {
    COLOR.store(!disabled && std::io::stderr().is_terminal(), Ordering::Relaxed);
}

fn paint(code: &str, s: &str) -> String {
    if COLOR.load(Ordering::Relaxed) {
        format!("\x1b[{code}m{s}\x1b[0m")
    } else {
        s.to_string()
    }
}

pub fn dim(s: &str) -> String {
    paint("2", s)
}
pub fn bold(s: &str) -> String {
    paint("1", s)
}
pub fn green(s: &str) -> String {
    paint("32", s)
}
pub fn yellow(s: &str) -> String {
    paint("33", s)
}

/// Colors cycled for compose service prefixes.
pub fn service_color(i: usize, s: &str) -> String {
    const C: [&str; 6] = ["36", "33", "32", "35", "34", "91"];
    paint(C[i % C.len()], s)
}

pub fn error(msg: &str) {
    eprintln!("{} {msg}", paint("1;31", "Error:"));
}

pub fn warn(msg: &str) {
    eprintln!("{} {msg}", paint("1;33", "Warning:"));
}

/// Left-aligned columns separated by three spaces, Docker style.
pub fn table(headers: &[&str], rows: &[Vec<String>]) {
    let mut w: Vec<usize> = headers.iter().map(|h| h.len()).collect();
    for r in rows {
        for (i, c) in r.iter().enumerate() {
            if i < w.len() {
                w[i] = w[i].max(c.chars().count());
            }
        }
    }
    let line = |cells: Vec<String>| {
        let n = cells.len();
        let mut s = String::new();
        for (i, c) in cells.into_iter().enumerate() {
            s.push_str(&c);
            if i + 1 < n {
                s.push_str(&" ".repeat(w[i] - c.chars().count() + 3));
            }
        }
        s
    };
    println!("{}", bold(&line(headers.iter().map(|s| s.to_string()).collect())));
    for r in rows {
        println!("{}", line(r.clone()));
    }
}

pub fn ago(ts: i64) -> String {
    format!("{} ago", hullr_core::human_duration(hullr_core::now_unix() - ts))
}

pub fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(n.saturating_sub(1)).collect::<String>())
    }
}

/// Renders progress to stderr. On a terminal, layer progress is a live
/// block of lines redrawn in place; otherwise only milestones are printed.
pub fn progress_printer() -> ProgressFn {
    let tty = std::io::stderr().is_terminal();
    struct State {
        order: Vec<String>,
        lines: HashMap<String, String>,
        drawn: usize,
        last_done: HashMap<String, bool>,
    }
    let st = Arc::new(Mutex::new(State { order: vec![], lines: HashMap::new(), drawn: 0, last_done: HashMap::new() }));
    Arc::new(move |p| {
        let mut st = st.lock().unwrap_or_else(|e| e.into_inner());
        let mut err = std::io::stderr().lock();
        let clear = |st: &mut State, err: &mut std::io::StderrLock| {
            if tty && st.drawn > 0 {
                let _ = write!(err, "\x1b[{}A", st.drawn);
                for _ in 0..st.drawn {
                    let _ = writeln!(err, "\x1b[2K");
                }
                let _ = write!(err, "\x1b[{}A", st.drawn);
                st.drawn = 0;
            }
        };
        let redraw = |st: &mut State, err: &mut std::io::StderrLock| {
            if tty {
                for id in &st.order {
                    let _ = writeln!(err, "{}", st.lines[id]);
                }
                st.drawn = st.order.len();
            }
        };
        match p {
            Progress::Status { message } => {
                clear(&mut st, &mut err);
                let _ = writeln!(err, "{message}");
                redraw(&mut st, &mut err);
            }
            Progress::Layer { id, current, total, done } => {
                let short: String = id.trim_start_matches("sha256:").chars().take(12).collect();
                let text = if done {
                    format!("{short}: {}", green("Pull complete"))
                } else if total > 0 {
                    let pct = (current as f64 / total as f64).min(1.0);
                    let bar = (pct * 30.0) as usize;
                    format!(
                        "{short}: Downloading [{}{}] {}/{}",
                        "=".repeat(bar),
                        " ".repeat(30 - bar),
                        hullr_core::human_bytes(current),
                        hullr_core::human_bytes(total)
                    )
                } else {
                    format!("{short}: Downloading {}", hullr_core::human_bytes(current))
                };
                if !tty {
                    if done && !st.last_done.get(&short).copied().unwrap_or(false) {
                        let _ = writeln!(err, "{text}");
                        st.last_done.insert(short, true);
                    }
                    return;
                }
                clear(&mut st, &mut err);
                if !st.lines.contains_key(&short) {
                    st.order.push(short.clone());
                }
                st.lines.insert(short, text);
                redraw(&mut st, &mut err);
            }
            Progress::Step { step, total, text, cached } => {
                clear(&mut st, &mut err);
                st.order.clear();
                st.lines.clear();
                let tag = if cached { format!(" {}", yellow("CACHED")) } else { String::new() };
                let _ = writeln!(err, "{} {}{tag}", bold(&format!("[{step}/{total}]")), text);
            }
            Progress::Output { line, .. } => {
                clear(&mut st, &mut err);
                let _ = writeln!(err, "{}", dim(&format!("  | {line}")));
                redraw(&mut st, &mut err);
            }
        }
    })
}

/// Ask a yes/no question on the terminal (default no).
pub fn confirm(q: &str) -> bool {
    eprint!("{q} [y/N] ");
    let _ = std::io::stderr().flush();
    let mut s = String::new();
    std::io::stdin().read_line(&mut s).is_ok() && matches!(s.trim(), "y" | "Y" | "yes")
}
