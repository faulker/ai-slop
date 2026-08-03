//! Owns the typed tmux error class (`TmuxError`) and the single `classify`
//! function that turns a finished `tmux` process into one of its variants.
//! Everything outside this file that needs to branch on *why* a tmux call
//! failed goes through `classify`; nothing else should pattern-match stderr
//! text. Non-tmux errors elsewhere in the app use `anyhow` instead, per
//! DESIGN.md section 11.

use std::process::Output;

/// The one error class the app actually branches on. Every other error
/// path uses `anyhow::Error` with `.context()`.
#[derive(Debug, thiserror::Error)]
pub enum TmuxError {
    #[error("tmux is not installed")]
    NotInstalled,

    /// Benign in almost every context: it means zero sessions on this socket.
    #[error("no tmux server running on socket {socket}")]
    NoServer { socket: String },

    #[error("no such tmux session: {0}")]
    NoSuchSession(String),

    #[error("tmux {subcommand} failed (exit {code}): {stderr}")]
    CommandFailed {
        subcommand: String,
        code: i32,
        stderr: String,
        argv: Vec<String>,
    },

    #[error("could not parse tmux output: {0}")]
    Parse(String),

    #[error("tmux io error: {0}")]
    Io(#[from] std::io::Error),
}

/// Pull the value that follows a given flag (e.g. `-L`, `-t`) out of an argv
/// slice. Returns `None` if the flag is absent or has no following element.
fn arg_after(argv: &[String], flag: &str) -> Option<String> {
    argv.iter()
        .position(|a| a == flag)
        .and_then(|i| argv.get(i + 1))
        .map(|s| s.trim_start_matches('=').to_string())
}

/// Map a finished tmux process into a typed error. Research item 7: there is
/// no dedicated exit code for "no server", so we must match stderr text.
/// Matching is case-insensitive and checked in the order of the table in
/// DESIGN.md section 11: `NoServer` patterns first, then `NoSuchSession`
/// patterns, then a generic `CommandFailed` for anything else.
pub fn classify(subcommand: &str, argv: &[String], out: &Output) -> TmuxError {
    let stderr = String::from_utf8_lossy(&out.stderr);
    let lower = stderr.to_lowercase();

    if lower.contains("no server running") || lower.contains("error connecting to") {
        let socket = arg_after(argv, "-L").unwrap_or_else(|| "unknown".to_string());
        return TmuxError::NoServer { socket };
    }

    // "no current target" is a fourth stderr shape, verified against real
    // tmux 3.7b: with `exit-empty off` (our config), killing the *last*
    // session on a socket leaves the server running with zero sessions.
    // Targeting any specific session on that now-empty server (e.g. a
    // `has-session` right after the kill) fails this way instead of with
    // "can't find session", because tmux has no session left to fall back
    // to when resolving the target. Semantically it means the same thing a
    // caller cares about: the requested session is not there.
    if lower.contains("can't find session")
        || lower.contains("no such session")
        || lower.contains("session not found")
        || lower.contains("no current target")
    {
        // `tmux/mod.rs` always targets `=name:` (trailing colon; see its
        // module doc comment for why), so strip that too, not just the
        // leading `=` that `arg_after` already handles.
        let target = arg_after(argv, "-t")
            .map(|s| s.trim_end_matches(':').to_string())
            .unwrap_or_else(|| subcommand.to_string());
        return TmuxError::NoSuchSession(target);
    }

    TmuxError::CommandFailed {
        subcommand: subcommand.to_string(),
        code: out.status.code().unwrap_or(-1),
        stderr: stderr.trim().to_string(),
        argv: argv.to_vec(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::process::ExitStatusExt;
    use std::process::ExitStatus;

    /// Build a synthetic `Output` with the given exit code and stderr text,
    /// standing in for a real finished `tmux` process in tests.
    fn output_with_stderr(code: i32, stderr: &str) -> Output {
        Output {
            status: ExitStatus::from_raw(code << 8),
            stdout: Vec::new(),
            stderr: stderr.as_bytes().to_vec(),
        }
    }

    #[test]
    fn no_server_running_is_benign() {
        let argv = vec![
            "-L".to_string(),
            "multi-run".to_string(),
            "list-sessions".to_string(),
        ];
        let out = output_with_stderr(1, "no server running on /tmp/tmux-501/multi-run");
        let err = classify("list-sessions", &argv, &out);
        assert!(matches!(err, TmuxError::NoServer { socket } if socket == "multi-run"));
    }

    #[test]
    fn error_connecting_to_is_benign() {
        let argv = vec![
            "-L".to_string(),
            "multi-run".to_string(),
            "list-sessions".to_string(),
        ];
        let out = output_with_stderr(
            1,
            "error connecting to /tmp/tmux-501/multi-run (No such file or directory)",
        );
        let err = classify("list-sessions", &argv, &out);
        assert!(matches!(err, TmuxError::NoServer { socket } if socket == "multi-run"));
    }

    #[test]
    fn no_server_matching_is_case_insensitive() {
        let argv = vec!["-L".to_string(), "multi-run".to_string()];
        let out = output_with_stderr(1, "NO SERVER RUNNING on socket");
        let err = classify("has-session", &argv, &out);
        assert!(matches!(err, TmuxError::NoServer { .. }));
    }

    #[test]
    fn no_server_falls_back_when_socket_flag_missing() {
        let argv = vec!["has-session".to_string()];
        let out = output_with_stderr(1, "no server running");
        let err = classify("has-session", &argv, &out);
        assert!(matches!(err, TmuxError::NoServer { socket } if socket == "unknown"));
    }

    #[test]
    fn cant_find_session_is_no_such_session() {
        let argv = vec!["-t".to_string(), "=mr-k3f9-1".to_string()];
        let out = output_with_stderr(1, "can't find session: mr-k3f9-1");
        let err = classify("has-session", &argv, &out);
        assert!(matches!(err, TmuxError::NoSuchSession(t) if t == "mr-k3f9-1"));
    }

    #[test]
    fn no_such_session_variant_text_is_no_such_session() {
        let argv = vec!["-t".to_string(), "=mr-k3f9-1".to_string()];
        let out = output_with_stderr(1, "no such session: mr-k3f9-1");
        let err = classify("kill-session", &argv, &out);
        assert!(matches!(err, TmuxError::NoSuchSession(_)));
    }

    #[test]
    fn no_current_target_is_no_such_session() {
        // Verified against real tmux 3.7b: once the last session on a
        // socket is killed (with our `exit-empty off` config keeping the
        // now-empty server alive), targeting any specific session fails
        // this way rather than with "can't find session".
        let argv = vec!["-t".to_string(), "=mr-k3f9-1:".to_string()];
        let out = output_with_stderr(1, "no current target");
        let err = classify("has-session", &argv, &out);
        assert!(matches!(err, TmuxError::NoSuchSession(t) if t == "mr-k3f9-1"));
    }

    #[test]
    fn session_not_found_is_no_such_session() {
        let argv = vec!["-t".to_string(), "=mr-k3f9-1".to_string()];
        let out = output_with_stderr(1, "session not found: mr-k3f9-1");
        let err = classify("send-keys", &argv, &out);
        assert!(matches!(err, TmuxError::NoSuchSession(_)));
    }

    #[test]
    fn no_such_session_strips_the_trailing_colon_tmux_mod_always_sends() {
        // tmux/mod.rs's `target()` always emits `=name:`, not just `=name`.
        let argv = vec!["-t".to_string(), "=mr-k3f9-1:".to_string()];
        let out = output_with_stderr(1, "can't find session: mr-k3f9-1");
        let err = classify("has-session", &argv, &out);
        assert!(matches!(err, TmuxError::NoSuchSession(t) if t == "mr-k3f9-1"));
    }

    #[test]
    fn no_such_session_falls_back_to_subcommand_when_target_flag_missing() {
        let argv = vec!["kill-session".to_string()];
        let out = output_with_stderr(1, "no such session");
        let err = classify("kill-session", &argv, &out);
        assert!(matches!(err, TmuxError::NoSuchSession(t) if t == "kill-session"));
    }

    #[test]
    fn genuine_error_is_command_failed() {
        let argv = vec![
            "new-session".to_string(),
            "-d".to_string(),
            "-s".to_string(),
            "mr-k3f9-1".to_string(),
        ];
        let out = output_with_stderr(1, "duplicate session: mr-k3f9-1");
        let err = classify("new-session", &argv, &out);
        match err {
            TmuxError::CommandFailed {
                subcommand,
                code,
                stderr,
                argv: got_argv,
            } => {
                assert_eq!(subcommand, "new-session");
                assert_eq!(code, 1);
                assert_eq!(stderr, "duplicate session: mr-k3f9-1");
                assert_eq!(got_argv, argv);
            }
            other => panic!("expected CommandFailed, got {other:?}"),
        }
    }

    #[test]
    fn command_failed_uses_negative_one_when_no_exit_code() {
        // A process killed by a signal has no exit code (status.code() is None).
        let argv = vec!["capture-pane".to_string()];
        let out = Output {
            status: ExitStatus::from_raw(9), // terminated by signal 9, no exit code
            stdout: Vec::new(),
            stderr: b"killed".to_vec(),
        };
        let err = classify("capture-pane", &argv, &out);
        assert!(matches!(err, TmuxError::CommandFailed { code: -1, .. }));
    }

    #[test]
    fn error_messages_render_via_display() {
        let e = TmuxError::NotInstalled;
        assert_eq!(e.to_string(), "tmux is not installed");

        let e = TmuxError::NoServer {
            socket: "multi-run".to_string(),
        };
        assert_eq!(e.to_string(), "no tmux server running on socket multi-run");

        let e = TmuxError::NoSuchSession("mr-k3f9-1".to_string());
        assert_eq!(e.to_string(), "no such tmux session: mr-k3f9-1");

        let e = TmuxError::Parse("bad row".to_string());
        assert_eq!(e.to_string(), "could not parse tmux output: bad row");
    }
}
