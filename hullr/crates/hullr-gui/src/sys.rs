//! Talking to the host desktop: opening URLs and a terminal window.

use std::process::{Command, Stdio};

/// Quote for a POSIX shell.
pub fn sh_quote(s: &str) -> String {
    if !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || "-_./:=@%+".contains(c)) {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', "'\\''"))
    }
}

/// The interactive command a terminal window should run.
pub fn exec_command_line(exe: &str, id: &str) -> String {
    format!("{} exec -it {} sh", sh_quote(exe), sh_quote(id))
}

/// Candidate commands (program + args) to open a terminal running
/// `hullr exec -it <id> sh`, in the order to try them.
pub fn terminal_candidates(os: &str, exe: &str, id: &str) -> Vec<Vec<String>> {
    let line = exec_command_line(exe, id);
    let v = |parts: &[&str]| parts.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    if os == "macos" {
        let script = format!(
            "tell application \"Terminal\"\nactivate\ndo script \"{}\"\nend tell",
            line.replace('\\', "\\\\").replace('"', "\\\"")
        );
        return vec![vec!["osascript".into(), "-e".into(), script]];
    }
    // A shell wrapper keeps the window open if the command fails.
    let keep = format!("{line}; echo; echo 'Press Enter to close'; read _");
    let mut out = vec![
        [v(&["x-terminal-emulator", "-e", "sh", "-c"]), vec![keep.clone()]].concat(),
        [v(&["gnome-terminal", "--", "sh", "-c"]), vec![keep.clone()]].concat(),
        [v(&["konsole", "-e", "sh", "-c"]), vec![keep.clone()]].concat(),
        [v(&["xfce4-terminal", "-e"]), vec![format!("sh -c {}", sh_quote(&keep))]].concat(),
        [v(&["xterm", "-e", "sh", "-c"]), vec![keep]].concat(),
    ];
    out.shrink_to_fit();
    out
}

/// Try each candidate until one starts. Returns the program that worked.
pub fn launch_first(candidates: &[Vec<String>]) -> Result<String, String> {
    for c in candidates {
        let Some((prog, args)) = c.split_first() else { continue };
        if Command::new(prog)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .is_ok()
        {
            return Ok(prog.clone());
        }
    }
    Err("No terminal program found. Run this yourself: hullr exec -it <container> sh".into())
}

pub fn open_shell_in_terminal(id: &str) -> Result<String, String> {
    let exe = std::env::current_exe()
        .ok()
        .and_then(|p| p.to_str().map(str::to_string))
        .filter(|p| std::path::Path::new(p).file_name().is_some_and(|n| n.to_string_lossy().starts_with("hullr")))
        .unwrap_or_else(|| "hullr".to_string());
    launch_first(&terminal_candidates(std::env::consts::OS, &exe, id))
}

/// Open a URL or folder with the desktop's default handler.
pub fn open_target(target: &str) -> Result<(), String> {
    let prog = if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
    Command::new(prog)
        .arg(target)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("Could not open {target} with {prog}: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoting() {
        assert_eq!(sh_quote("abc-1.2"), "abc-1.2");
        assert_eq!(sh_quote("a b"), "'a b'");
        assert_eq!(sh_quote("it's"), "'it'\\''s'");
        assert_eq!(sh_quote(""), "''");
    }

    #[test]
    fn command_line() {
        assert_eq!(exec_command_line("hullr", "abc123"), "hullr exec -it abc123 sh");
        assert_eq!(exec_command_line("/Apps/My hullr", "x"), "'/Apps/My hullr' exec -it x sh");
    }

    #[test]
    fn macos_uses_osascript_terminal() {
        let c = terminal_candidates("macos", "hullr", "abc");
        assert_eq!(c.len(), 1);
        assert_eq!(c[0][0], "osascript");
        assert!(c[0][2].contains("tell application \"Terminal\""));
        assert!(c[0][2].contains("do script \"hullr exec -it abc sh\""));
    }

    #[test]
    fn macos_script_escapes_quotes() {
        let c = terminal_candidates("macos", "/a\"b/hullr", "abc");
        assert!(c[0][2].contains("\\\""));
    }

    #[test]
    fn linux_fallback_chain_order() {
        let c = terminal_candidates("linux", "hullr", "abc");
        let progs: Vec<&str> = c.iter().map(|c| c[0].as_str()).collect();
        assert_eq!(progs, ["x-terminal-emulator", "gnome-terminal", "konsole", "xfce4-terminal", "xterm"]);
        assert!(c[0].last().unwrap().contains("hullr exec -it abc sh"));
    }

    #[test]
    fn launch_reports_missing_terminals() {
        let r = launch_first(&[vec!["/definitely/not/a/program".into()]]);
        assert!(r.unwrap_err().contains("No terminal program found"));
        assert!(launch_first(&[]).is_err());
    }
}
