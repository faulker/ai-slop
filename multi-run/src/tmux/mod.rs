//! tmux backend. Owns the private socket name, the generated config path, argv
//! construction for every tmux invocation, and dispatching those invocations.
//! Contains no UI state and no drawing.
//!
//! Every invocation in this file follows DESIGN.md section 5's catalogue.
//! Chained subcommands (multiple tmux commands run in one process, separated
//! by `;`) are built with a bare `;` as its own argv element, never a shell-
//! escaped `\;`: `std::process::Command` does not go through a shell, so a
//! literal `\;` would be passed to tmux verbatim and rejected as
//! `unknown command: \;`.
//!
//! ## Target colon note (verified against real tmux 3.7b, not just gotcha D)
//!
//! DESIGN.md section 5's catalogue writes several targets without a trailing
//! colon (e.g. `capture-pane -p -e -t =S`, `send-keys -t =S C-c`). Gotcha D
//! in tmux-research.md documents the trailing-colon requirement only for
//! `set-option`. Verified directly against the tmux 3.7b installed here:
//! `capture-pane` and `send-keys` also fail with `can't find pane: =S`
//! when given a bare `=S` (no colon), even though `has-session`,
//! `resize-window`, `display-message`, and `kill-session` all accept `=S`
//! with or without the colon. The colon form is accepted by every command
//! tested and rejected by none, so `target()` below always emits `=S:` for
//! every targeted invocation rather than special-casing per subcommand. This
//! is a real deviation from the DESIGN.md catalogue text, not from its
//! intent (exact-match targeting, gotcha C); worth folding back into the
//! design doc.
//!
//! ## Chained-command failure note
//!
//! Verified: if one subcommand in a `;`-chained invocation fails (e.g. a
//! `capture-pane` targeting a session that vanished mid-batch), tmux aborts
//! the rest of the chain rather than running the remaining subcommands. A
//! batched content poll that hits a vanished session therefore loses the
//! whole batch for that poll cycle, not just that one session's capture.
//! This is self-healing: DESIGN.md section 11 already has pollers retry on
//! `CommandFailed`/`NoSuchSession`, and the next metadata poll drops the
//! vanished session from the next content poll's request list. No API
//! change follows from this; it is documented here so a caller reading a
//! `NoSuchSession` out of `poll_content` understands why sibling captures in
//! the same call were lost too.

pub mod conf;
pub mod parse;

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use crate::cli::GroupId;
use crate::error::{self, TmuxError};

/// `list-sessions -f` filter that keeps only sessions carrying an `@mr_group`
/// user option, i.e. sessions multi-run created (DESIGN.md section 5,
/// invocation 3; research item 2). A session created by anything else on the
/// same socket does not have this option and is excluded server-side.
const MR_GROUP_FILTER: &str = "#{!=:#{@mr_group},}";

/// `display-message` format for the stale-server check: the server's own pid
/// and how many sessions it is holding, in one invocation. Both are server
/// scoped, so no target is needed.
const SERVER_STATUS_FORMAT: &str = "#{pid}\t#{server_sessions}";

/// `-F` format string for the session half of the metadata poll. Fields are
/// separated by a literal tab byte, written here as the Rust escape `\t`
/// (verified against tmux 3.7b that DESIGN.md section 5's `#{l:\t}` renders
/// as the two literal characters `\` and `t` rather than an escape, so this
/// build uses section 5's documented fallback: "embed an actual tab
/// character in the argv string"). `@mr_cmd` is last so a command string
/// containing a tab lands wholly in the final field when parsed with
/// `splitn` (`tmux/parse.rs`, DESIGN.md section 14 open question 2).
const SESSION_FORMAT: &str = "#{session_name}\t#{@mr_group}\t#{@mr_index}\t#{@mr_cwd}\t#{@mr_started}\t#{session_attached}\t#{@mr_label}\t#{@mr_cmd}";

/// `list-panes -a -f` filter that keeps only job panes, i.e. window 0 pane 0
/// of every session on the socket. A user who splits the pane or opens a
/// window with their own tmux prefix while attached would otherwise produce a
/// second row for the same session, and the join in `main.rs` would let that
/// row overwrite the job's real liveness and exit status. The address is
/// stable because our generated config pins `base-index` and
/// `pane-base-index` to 0.
const JOB_PANE_FILTER: &str = "#{&&:#{==:#{window_index},0},#{==:#{pane_index},0}}";

/// `-F` format string for the pane half of the metadata poll (`list-panes
/// -a`, filtered to job panes by `JOB_PANE_FILTER`; the surviving rows are
/// matched against the session set in Rust, per DESIGN.md section 5).
const PANE_FORMAT: &str = "#{session_name}\t#{pane_dead}\t#{pane_dead_status}\t#{pane_dead_signal}\t#{pane_dead_time}\t#{pane_pid}\t#{pane_width}\t#{pane_height}\t#{history_size}\t#{cursor_y}";

/// The exact text `display-message -p` is asked to print between the two
/// halves of the metadata poll, so the batched stdout can be split back into
/// "session rows" and "pane rows" (DESIGN.md section 5, invocation 3).
const PANES_MARKER: &str = "@@MR-PANES";

/// tmux socket + generated config, and every argv this app ever builds for
/// `tmux`. One `Tmux` per process; cheap to construct, holds no OS handles.
pub struct Tmux {
    socket: String,
    conf_path: PathBuf,
}

/// Everything needed to create one job's tmux session (DESIGN.md section 5,
/// invocation 2).
pub struct NewSessionSpec {
    /// The tmux session name, e.g. `mr-g1-1` (`JobId::tmux_name()`).
    pub session_name: String,
    pub cwd: PathBuf,
    /// Tile inner size at creation time. Use 80x24 if the layout is not
    /// known yet; the first debounced resize corrects it (DESIGN.md section
    /// 5, invocation 2 notes).
    pub width: u16,
    pub height: u16,
    pub command: String,
    pub group: GroupId,
    pub index: u16,
    pub started: u64,
    /// `@mr_label` is only written when this is `Some`.
    pub label: Option<String>,
}

/// Verdict of [`Tmux::server_cwd_state`].
///
/// The tmux server starts in the directory multi-run was launched from and
/// keeps it for life: it is a daemon that outlives the invocation which
/// started it, and `exit-empty off` (see `tmux/conf.rs`) means ours never
/// exits even with nothing left to hold. Delete or rename that directory later
/// and the server is left holding an unlinked inode. `getcwd` inside it fails,
/// and from then on every pane the server forks inherits the broken directory:
/// `new-session -c <dir>` does not rescue it, tmux reports an empty
/// `pane_current_path`, and each job's shell opens with `shell-init: error
/// retrieving current directory`, taking with it anything that calls `getcwd`
/// at shell startup, e.g. a version-manager hook such as fnm's.
///
/// Nothing can move a running process's working directory from the outside, so
/// the only cure is a new server, which is `main.rs`'s call to make: it is free
/// when the stale server is holding no sessions and destructive otherwise.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ServerCwd {
    /// No server on our socket, so nothing can be stale yet.
    NoServer,
    /// The server is standing in a directory that still exists.
    Live,
    /// The server's directory has been deleted or renamed. Every pane it
    /// creates from now on is broken. `sessions` is how much a restart costs.
    Gone { cwd: PathBuf, sessions: u32 },
    /// The check could not be made (a platform whose process table we cannot
    /// read, or output we could not parse). Treated as "carry on": a
    /// diagnostic must not become a new way to fail.
    Unknown,
}

/// A running process's working directory, or `None` where we cannot ask.
///
/// Linux publishes it as a symlink under `/proc`, which for a deleted
/// directory reads back with a " (deleted)" suffix. Everywhere else this goes
/// through `lsof`, whose `-F n` output prefixes each path with `n`, and which
/// keeps reporting the original path after a delete. Both leave the caller
/// with a path that fails to resolve, which is the signal it wants.
fn process_cwd(pid: u32) -> Option<PathBuf> {
    if cfg!(target_os = "linux") {
        return std::fs::read_link(format!("/proc/{pid}/cwd")).ok();
    }
    let out = Command::new("lsof")
        .args(["-a", "-p", &pid.to_string(), "-d", "cwd", "-F", "n"])
        .output()
        .ok()?;
    String::from_utf8(out.stdout).ok()?.lines().find_map(|line| {
        line.strip_prefix('n')
            .filter(|path| !path.is_empty())
            .map(PathBuf::from)
    })
}

/// The two record sets returned by one metadata poll (DESIGN.md section 5,
/// invocation 3). Joining them on `session_name` is the caller's job
/// (`poll.rs`), not this module's.
pub struct MetadataPoll {
    pub sessions: Vec<parse::SessionRecord>,
    pub panes: Vec<parse::PaneRecord>,
}

/// Which variant of `capture-pane` a content-poll request uses (DESIGN.md
/// section 5, invocation 4).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaptureMode {
    /// No `-S`/`-E`. The pane is already sized to the tile, so this returns
    /// exactly the visible screen (research items 3 and 4).
    Live,
    /// `-S -1`: recover the line the "pane is dead" banner scrolled off,
    /// needed on tmux below 3.3 where `remain-on-exit-format ""` does not
    /// exist (gotcha A).
    DeadDegraded,
    /// `-S -<offset> -E <height-1-offset>`: a scrollback view `offset` lines
    /// above the live bottom, yielding exactly `height` lines (DESIGN.md
    /// section 5).
    Scrollback { offset: u32, height: u16 },
}

/// One pane's capture request within a batched content poll.
pub struct CaptureRequest {
    pub session_name: String,
    pub mode: CaptureMode,
}

impl Tmux {
    /// Build a handle for the given private socket name and generated config
    /// path. Does not touch the filesystem or spawn anything; `tmux/conf.rs`
    /// owns writing the config file, this module only reads its path back.
    pub fn new(socket: impl Into<String>, conf_path: impl Into<PathBuf>) -> Self {
        Tmux {
            socket: socket.into(),
            conf_path: conf_path.into(),
        }
    }

    pub fn socket(&self) -> &str {
        &self.socket
    }

    pub fn conf_path(&self) -> &Path {
        &self.conf_path
    }

    /// Every call is `tmux -L <socket> -f <conf> <subcommand...>`. `-f` is
    /// honoured only when the server is first started (research headline
    /// finding; DESIGN.md section 5), so passing it unconditionally is
    /// harmless and removes a "have we bootstrapped yet" branch. This is the
    /// server-bootstrap half of the contract: the first invocation that
    /// causes tmux to start a server (typically `create_session`) is what
    /// actually applies `conf_path`; every later call just reconnects to the
    /// already-running server. Pinned by the
    /// `f_flag_does_not_reset_running_options` integration test.
    fn base(&self) -> Vec<String> {
        vec![
            "-L".into(),
            self.socket.clone(),
            "-f".into(),
            self.conf_path.display().to_string(),
        ]
    }

    /// Exact-match session target (gotcha C: tmux prefix-matches a bare
    /// name), with the trailing colon every targeted subcommand accepts. See
    /// the module doc comment for why this file uses the colon uniformly
    /// rather than only for `set-option` as gotcha D describes.
    fn target(session_name: &str) -> String {
        format!("={session_name}:")
    }

    /// The job's own pane, addressed explicitly rather than through the
    /// session's *current* pane. A user who splits the pane or opens another
    /// window while attached leaves the session pointing somewhere that is
    /// not the job; capturing, interrupting or signalling that would read and
    /// act on the wrong process. Window and pane 0 is
    /// where `new-session` puts the job, and `tmux/conf.rs` pins both bases
    /// to 0 so the address holds even under a user config that sets them.
    fn job_pane(session_name: &str) -> String {
        format!("={session_name}:0.0")
    }

    /// The window holding the job's pane, for the window-scoped calls
    /// (`resize-window`, `set-option -uw window-size`). Same reasoning as
    /// `job_pane`.
    fn job_window(session_name: &str) -> String {
        format!("={session_name}:0")
    }

    /// The single place a `tmux` child process is configured. `full` is the
    /// complete argv after the program name. `attach.rs` spawns through here
    /// too, since it needs the same setup with inherited stdio rather than
    /// captured output.
    ///
    /// No working directory is set, deliberately: the child inherits ours, and
    /// multi-run never chdirs, so the tmux server starts in the directory the
    /// user ran multi-run in. That directory is the server's for life (it is a
    /// daemon, and `exit-empty off` means ours never exits to shed it), which
    /// is what [`Tmux::server_cwd_state`] exists to keep an eye on.
    pub fn command(full: &[String]) -> Command {
        let mut cmd = Command::new("tmux");
        cmd.args(full);
        cmd
    }

    /// Run one possibly-chained tmux invocation and classify a non-zero exit
    /// via `error::classify`. `subcommand` is a short label (e.g.
    /// `"new-session"`) used only for diagnostics in the resulting
    /// `TmuxError::CommandFailed`; it does not affect what gets run. `args`
    /// is everything after `-L <socket> -f <conf>`.
    fn invoke(&self, subcommand: &str, args: Vec<String>) -> Result<Output, TmuxError> {
        let mut full = self.base();
        full.extend(args);
        let out = Self::command(&full).output().map_err(TmuxError::Io)?;
        if out.status.success() {
            Ok(out)
        } else {
            Err(error::classify(subcommand, &full, &out))
        }
    }

    /// Whether the already-running server is still standing in a directory
    /// that exists. See [`ServerCwd`] for why this is worth a syscall.
    ///
    /// Runs only against a server that is already up; with none running there
    /// is nothing to be stale and the answer is [`ServerCwd::NoServer`]. The
    /// pid and session count come from one `display-message`, and the pid's
    /// working directory from the platform's process table.
    pub fn server_cwd_state(&self) -> ServerCwd {
        let out = match self.invoke(
            "display-message",
            vec![
                "display-message".into(),
                "-p".into(),
                SERVER_STATUS_FORMAT.into(),
            ],
        ) {
            Ok(out) => out,
            // Every other failure is someone else's to report: this check must
            // never be the thing that stops a launch.
            Err(_) => return ServerCwd::NoServer,
        };
        let text = String::from_utf8_lossy(&out.stdout);
        let (pid, sessions) = match text.trim().split_once('\t') {
            Some((pid, sessions)) => (pid.trim(), sessions.trim()),
            None => return ServerCwd::Unknown,
        };
        let (Ok(pid), Ok(sessions)) = (pid.parse::<u32>(), sessions.parse::<u32>()) else {
            return ServerCwd::Unknown;
        };
        match process_cwd(pid) {
            // A path the server still holds but that no longer resolves is
            // exactly the unlinked-directory case. Linux marks it by appending
            // " (deleted)", macOS reports the old path as-is; neither exists.
            Some(cwd) if !cwd.exists() => ServerCwd::Gone { cwd, sessions },
            Some(_) => ServerCwd::Live,
            None => ServerCwd::Unknown,
        }
    }

    /// Pick the next `GroupId` (DESIGN.md section 4): the smallest unused
    /// sequential `gN` (`g1`, `g2`, …) among the `@mr_group` values
    /// currently visible in `list-sessions`. If no server is running yet,
    /// there is nothing to collide with and `g1` is used.
    pub fn new_group_id(&self) -> Result<GroupId, TmuxError> {
        let existing: HashSet<String> = match self.poll_metadata() {
            Ok(meta) => meta.sessions.into_iter().map(|s| s.group).collect(),
            Err(TmuxError::NoServer { .. }) => HashSet::new(),
            Err(e) => return Err(e),
        };
        Ok(Self::next_sequential_group_id(&existing))
    }

    /// Smallest unused `gN` given the set of live group id strings.
    /// Exposed to unit tests; production callers go through `new_group_id`.
    fn next_sequential_group_id(existing: &HashSet<String>) -> GroupId {
        let mut n = 1u32;
        loop {
            let candidate = format!("g{n}");
            if !existing.contains(&candidate) {
                return GroupId::new(candidate);
            }
            n = n.saturating_add(1);
            if n == 0 {
                // Exhausted u32; return something total rather than looping.
                return GroupId::new("g1");
            }
        }
    }

    /// Create one job's tmux session plus its `@mr_*` metadata, all in one
    /// process (DESIGN.md section 5, invocation 2). Guards: item 1
    /// (`remain-on-exit` is already global via the `-f` config, so there is
    /// no race even for an instantly-exiting command), item 2 (`@mr_*`
    /// namespacing), profile loading (config `default-shell` is the
    /// `user-shell` wrapper that re-execs `$SHELL` with login/interactive
    /// flags), gotcha C (exact-match targeting for the `set-option` calls),
    /// gotcha D (the trailing colon on every target).
    pub fn create_session(&self, spec: &NewSessionSpec) -> Result<(), TmuxError> {
        let t = Self::target(&spec.session_name);
        let mut args: Vec<String> = vec![
            "new-session".into(),
            "-d".into(),
            "-s".into(),
            spec.session_name.clone(),
            "-c".into(),
            spec.cwd.display().to_string(),
            "-x".into(),
            spec.width.to_string(),
            "-y".into(),
            spec.height.to_string(),
            spec.command.clone(),
            ";".into(),
            "set-option".into(),
            "-t".into(),
            t.clone(),
            "@mr_group".into(),
            spec.group.as_str().to_string(),
            ";".into(),
            "set-option".into(),
            "-t".into(),
            t.clone(),
            "@mr_index".into(),
            spec.index.to_string(),
            ";".into(),
            "set-option".into(),
            "-t".into(),
            t.clone(),
            "@mr_cmd".into(),
            spec.command.clone(),
            ";".into(),
            "set-option".into(),
            "-t".into(),
            t.clone(),
            "@mr_cwd".into(),
            spec.cwd.display().to_string(),
            ";".into(),
            "set-option".into(),
            "-t".into(),
            t.clone(),
            "@mr_started".into(),
            spec.started.to_string(),
        ];
        if let Some(label) = &spec.label {
            args.push(";".into());
            args.extend([
                "set-option".into(),
                "-t".into(),
                t,
                "@mr_label".into(),
                label.clone(),
            ]);
        }
        self.invoke("new-session", args)?;
        Ok(())
    }

    /// Batched metadata poll: session rows filtered to ours, then every pane
    /// on the socket (DESIGN.md section 5, invocation 3; also used for
    /// `--list` and the startup orphan scan, invocation 14). Guards: item 2
    /// (`@mr_group` filter), gotcha C (exact match is irrelevant here since
    /// nothing is targeted; this call touches every session on the socket by
    /// design). "Empty result is not an error" (research item 7): a `rc != 0`
    /// classified as `TmuxError::NoServer` collapses to an empty poll rather
    /// than propagating, since research verified both a truly absent socket
    /// and a socket whose server has exited report this way; the more common
    /// shape with `exit-empty off` is `rc == 0` with the marker line and
    /// empty session rows, handled by the normal parse path below.
    pub fn poll_metadata(&self) -> Result<MetadataPoll, TmuxError> {
        let args: Vec<String> = vec![
            "list-sessions".into(),
            "-f".into(),
            MR_GROUP_FILTER.into(),
            "-F".into(),
            SESSION_FORMAT.into(),
            ";".into(),
            "display-message".into(),
            "-p".into(),
            PANES_MARKER.into(),
            ";".into(),
            "list-panes".into(),
            "-a".into(),
            "-f".into(),
            JOB_PANE_FILTER.into(),
            "-F".into(),
            PANE_FORMAT.into(),
        ];
        let out = match self.invoke("list-sessions", args) {
            Ok(out) => out,
            // No server at all, and the `exit-empty off` case where the
            // server outlives its last session: there, `list-sessions`
            // succeeds with empty output but the chained `list-panes -a`
            // fails with "no current target" (verified against tmux 3.7b),
            // which classifies as NoSuchSession. Both mean "nothing is
            // running", which is an empty result, not an error.
            Err(TmuxError::NoServer { .. }) | Err(TmuxError::NoSuchSession(_)) => {
                return Ok(MetadataPoll {
                    sessions: Vec::new(),
                    panes: Vec::new(),
                });
            }
            Err(e) => return Err(e),
        };

        let raw = String::from_utf8_lossy(&out.stdout);
        let marker_line = format!("{PANES_MARKER}\n");
        let (sessions_raw, panes_raw) = raw.split_once(&marker_line).ok_or_else(|| {
            TmuxError::Parse(format!(
                "metadata poll marker {PANES_MARKER:?} not found in output"
            ))
        })?;

        let sessions =
            parse::parse_session_records(sessions_raw).map_err(|e| TmuxError::Parse(e.0))?;
        let panes = parse::parse_pane_records(panes_raw).map_err(|e| TmuxError::Parse(e.0))?;
        Ok(MetadataPoll { sessions, panes })
    }

    /// Batched content poll: one `display-message` marker plus one
    /// `capture-pane` per request, in one process (DESIGN.md section 5,
    /// invocation 4). Guards: item 3 (`-e` for SGR sequences, never `-J`,
    /// never relying on `-N` for padding), item 4 (no `-S`/`-E` in `Live`
    /// mode because the pane is already sized to the tile), gotcha A
    /// (`DeadDegraded` mode). See the module doc comment for what happens
    /// when one session in the batch has vanished. An empty `requests` slice
    /// returns an empty result without spawning tmux at all.
    pub fn poll_content(
        &self,
        requests: &[CaptureRequest],
    ) -> Result<Vec<parse::CaptureBlock>, TmuxError> {
        if requests.is_empty() {
            return Ok(Vec::new());
        }

        let session_order: Vec<String> = requests.iter().map(|r| r.session_name.clone()).collect();
        let args = Self::capture_argv(requests);

        let out = self.invoke("capture-pane", args)?;
        let raw = String::from_utf8_lossy(&out.stdout);
        Ok(parse::split_captures(&raw, &session_order))
    }

    /// The argument list `poll_content` hands to tmux, split out so the
    /// `-S`/`-E` arithmetic is testable without a running server.
    fn capture_argv(requests: &[CaptureRequest]) -> Vec<String> {
        let mut args: Vec<String> = Vec::new();
        for (i, req) in requests.iter().enumerate() {
            if i > 0 {
                args.push(";".into());
            }
            args.extend([
                "display-message".into(),
                "-p".into(),
                format!("@@MR:{}", req.session_name),
                ";".into(),
                "capture-pane".into(),
                "-p".into(),
                "-e".into(),
                "-t".into(),
                Self::job_pane(&req.session_name),
            ]);
            match req.mode {
                CaptureMode::Live => {}
                CaptureMode::DeadDegraded => args.extend(["-S".into(), "-1".into()]),
                CaptureMode::Scrollback { offset, height } => {
                    // `-E` goes negative as soon as the view sits more than one
                    // screen back; that is a valid history line number, so it
                    // must not be clamped to 0. Clamping widens the window to
                    // `offset + 1` lines, and `ui::tile::decode` keeps the last
                    // `height` of those, which is the live bottom again.
                    let end = height as i64 - 1 - offset as i64;
                    args.extend([
                        "-S".into(),
                        format!("-{offset}"),
                        "-E".into(),
                        end.to_string(),
                    ]);
                }
            }
        }
        args
    }

    /// Resize a batch of tiles in one process, one `resize-window` per tile
    /// (DESIGN.md section 5, invocation 5; item 4). Guard: gotcha E, this
    /// silently sets each window's `window-size` to `manual`, which is why
    /// callers only include tiles whose size actually changed. An empty
    /// batch is a no-op that does not spawn tmux.
    pub fn resize_windows(&self, targets: &[(String, u16, u16)]) -> Result<(), TmuxError> {
        if targets.is_empty() {
            return Ok(());
        }
        self.invoke("resize-window", Self::resize_argv(targets))?;
        Ok(())
    }

    /// The argument list `resize_windows` hands to tmux, split out so the
    /// targeting is testable without a running server.
    fn resize_argv(targets: &[(String, u16, u16)]) -> Vec<String> {
        let mut args: Vec<String> = Vec::new();
        for (i, (name, width, height)) in targets.iter().enumerate() {
            if i > 0 {
                args.push(";".into());
            }
            args.extend([
                "resize-window".into(),
                "-t".into(),
                Self::job_window(name),
                "-x".into(),
                width.to_string(),
                "-y".into(),
                height.to_string(),
            ]);
        }
        args
    }

    /// Unset the per-window `window-size` override before an attach
    /// (DESIGN.md section 5, invocation 6; section 8, step 6). Guards:
    /// gotcha E (falls back to the global `latest`, so an attaching client
    /// gets its own real size instead of the last tile size) and gotcha D
    /// (trailing colon; `-uw` still parses its target as a window/session
    /// target the same way `set-option` does).
    /// Takes every session rather than only the one being attached to,
    /// because `C-n`/`C-p` let the user switch sessions without detaching:
    /// a session still pinned to its tile size would paint a small window
    /// into a full-size terminal, leaving the TUI's old pixels around it.
    /// An empty batch is a no-op that does not spawn tmux.
    pub fn unset_window_size(&self, session_names: &[String]) -> Result<(), TmuxError> {
        if session_names.is_empty() {
            return Ok(());
        }
        let mut args: Vec<String> = Vec::new();
        for (i, name) in session_names.iter().enumerate() {
            if i > 0 {
                args.push(";".into());
            }
            args.extend([
                "set-option".into(),
                "-uw".into(),
                "-t".into(),
                Self::job_window(name),
                "window-size".into(),
            ]);
        }
        self.invoke("set-option", args)?;
        Ok(())
    }

    /// Build (but never run) the argv for `attach-session` (DESIGN.md
    /// section 5, invocation 7; section 8). The caller is responsible for
    /// running this with inherited stdio as a blocking foreground child;
    /// that terminal handoff belongs to `attach.rs`, not here. Returned argv
    /// includes the leading `-L <socket> -f <conf>` (i.e. this is everything
    /// that goes after the `tmux` binary name). Guards: item 5, gotchas C,
    /// E, F (the caller's responsibility once it has this argv), gotcha D.
    pub fn attach_argv(&self, session_name: &str) -> Vec<String> {
        let mut args = self.base();
        args.extend([
            "attach-session".into(),
            "-t".into(),
            Self::target(session_name),
        ]);
        args
    }

    /// Stop level 1: `send-keys C-c` (DESIGN.md section 5, invocation 9;
    /// item 6; gotcha C). This is not general keystroke proxying: only the
    /// narrow, catalogued key sends in this file exist.
    pub fn send_keys_interrupt(&self, session_name: &str) -> Result<(), TmuxError> {
        self.invoke(
            "send-keys",
            vec![
                "send-keys".into(),
                "-t".into(),
                Self::job_pane(session_name),
                "C-c".into(),
            ],
        )?;
        Ok(())
    }

    /// Send EOF: `send-keys C-d` (DESIGN.md section 5, invocation 10; item 6).
    /// Stop levels 2 and 3: signal the pane's foreground process group
    /// directly. `send-keys C-c` only reaches a process that is reading its
    /// terminal, so a job that ignores SIGINT or never reads stdin needs a
    /// real signal. Reads `#{pane_pid}` and signals its process group, so a
    /// shell's children die with it rather than being orphaned. The pane
    /// itself survives under `remain-on-exit`, so output stays readable.
    /// Guards: gotchas C and D via `target()`.
    pub fn signal_pane(&self, session_name: &str, signal: &str) -> Result<(), TmuxError> {
        let out = self.invoke(
            "display-message",
            vec![
                "display-message".into(),
                "-p".into(),
                "-t".into(),
                Self::job_pane(session_name),
                "#{pane_pid}".into(),
            ],
        )?;
        let pid = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if pid.is_empty() || !pid.chars().all(|c| c.is_ascii_digit()) {
            return Err(TmuxError::Parse(format!(
                "pane_pid for {session_name} was {pid:?}"
            )));
        }

        // Negative pid targets the process group. If the job already reaped
        // its children the group may be gone, which is not an error worth
        // surfacing: the next poll reports the pane as dead either way.
        let status = Command::new("kill")
            .args([format!("-{signal}"), format!("-{pid}")])
            .output()
            .map_err(|e| TmuxError::Parse(format!("cannot run kill: {e}")))?;
        if !status.status.success() {
            // Fall back to the single process before giving up.
            let _ = Command::new("kill")
                .args([format!("-{signal}"), pid])
                .output();
        }
        Ok(())
    }

    pub fn send_keys_eof(&self, session_name: &str) -> Result<(), TmuxError> {
        self.invoke(
            "send-keys",
            vec![
                "send-keys".into(),
                "-t".into(),
                Self::job_pane(session_name),
                "C-d".into(),
            ],
        )?;
        Ok(())
    }

    /// Dismiss/kill one session (DESIGN.md section 5, invocation 11; item 7;
    /// gotcha C). Errors (including `NoSuchSession` if the session already
    /// vanished) are returned rather than swallowed; the caller decides how
    /// to reconcile that against its model.
    pub fn kill_session(&self, session_name: &str) -> Result<(), TmuxError> {
        self.invoke(
            "kill-session",
            vec![
                "kill-session".into(),
                "-t".into(),
                Self::target(session_name),
            ],
        )?;
        Ok(())
    }

    /// Kill everything on this socket (DESIGN.md section 5, invocation 12;
    /// item 7; `--kill all`). A server that is already gone is treated as
    /// success, since the end state ("nothing running on this socket") is
    /// what the caller asked for either way.
    pub fn kill_server(&self) -> Result<(), TmuxError> {
        match self.invoke("kill-server", vec!["kill-server".into()]) {
            Ok(_) => Ok(()),
            Err(TmuxError::NoServer { .. }) => Ok(()),
            Err(e) => Err(e),
        }
    }

    /// Re-read the generated config into an already-running server. `-f` is
    /// only honoured when the server first starts, so without this a config
    /// upgrade (new key bindings, the status bar) would not reach a server
    /// left running by an earlier version until `--restart-server`. Options
    /// that only apply at creation time, such as `base-index`, still only
    /// affect sessions and windows made after this point. "No server" means
    /// there is nothing to update, which is success.
    pub fn source_conf(&self) -> Result<(), TmuxError> {
        match self.invoke(
            "source-file",
            vec![
                "source-file".into(),
                "-q".into(),
                self.conf_path.display().to_string(),
            ],
        ) {
            Ok(_) => Ok(()),
            Err(TmuxError::NoServer { .. }) => Ok(()),
            Err(e) => Err(e),
        }
    }

    /// Cheap liveness check (DESIGN.md section 5, invocation 13; item 7;
    /// gotcha C). Both "no such session" and "no server at all" resolve to
    /// `Ok(false)`, since from the caller's point of view a session that
    /// cannot be found is a session that is not there to attach to; any
    /// other tmux failure still propagates as `Err`.
    pub fn has_session(&self, session_name: &str) -> Result<bool, TmuxError> {
        match self.invoke(
            "has-session",
            vec![
                "has-session".into(),
                "-t".into(),
                Self::target(session_name),
            ],
        ) {
            Ok(_) => Ok(true),
            Err(TmuxError::NoSuchSession(_)) | Err(TmuxError::NoServer { .. }) => Ok(false),
            Err(e) => Err(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmux() -> Tmux {
        Tmux::new(
            "multi-run-test",
            PathBuf::from("/tmp/multi-run-test/tmux.conf"),
        )
    }

    // ---- pure argv construction, no tmux required ----

    #[test]
    fn base_includes_socket_and_conf_flags() {
        let t = tmux();
        assert_eq!(
            t.base(),
            vec![
                "-L",
                "multi-run-test",
                "-f",
                "/tmp/multi-run-test/tmux.conf"
            ]
        );
    }

    #[test]
    fn target_is_exact_match_with_trailing_colon() {
        assert_eq!(Tmux::target("mr-k3f9-1"), "=mr-k3f9-1:");
    }

    /// The server takes its lifelong working directory from whichever of our
    /// invocations starts it, and that is meant to be the directory multi-run
    /// itself was started in. Leaving the child's directory unset is what
    /// achieves it, since multi-run never chdirs.
    #[test]
    fn tmux_children_inherit_our_working_directory() {
        let cmd = Tmux::command(&["list-sessions".to_string()]);
        assert_eq!(cmd.get_current_dir(), None);
    }

    #[test]
    fn attach_argv_builds_full_command_without_running_it() {
        let t = tmux();
        let argv = t.attach_argv("mr-k3f9-1");
        assert_eq!(
            argv,
            vec![
                "-L",
                "multi-run-test",
                "-f",
                "/tmp/multi-run-test/tmux.conf",
                "attach-session",
                "-t",
                "=mr-k3f9-1:",
            ]
        );
    }

    #[test]
    fn job_targets_pin_window_and_pane_zero() {
        assert_eq!(Tmux::job_pane("mr-k3f9-1"), "=mr-k3f9-1:0.0");
        assert_eq!(Tmux::job_window("mr-k3f9-1"), "=mr-k3f9-1:0");
    }

    #[test]
    fn capture_targets_the_job_pane_not_the_current_one() {
        // A window opened while attached becomes the session's current
        // window; capturing the session target would then draw that window
        // into the job's tile.
        let argv = Tmux::capture_argv(&[CaptureRequest {
            session_name: "mr-k3f9-1".into(),
            mode: CaptureMode::Live,
        }]);
        let t = argv.iter().position(|a| a == "-t").unwrap();
        assert_eq!(argv[t + 1], "=mr-k3f9-1:0.0");
    }

    #[test]
    fn resize_targets_the_job_window() {
        let argv = Tmux::resize_argv(&[("mr-k3f9-1".to_string(), 80, 24)]);
        let t = argv.iter().position(|a| a == "-t").unwrap();
        assert_eq!(argv[t + 1], "=mr-k3f9-1:0");
    }

    #[test]
    fn scrollback_capture_asks_for_exactly_one_tile_of_lines() {
        let argv = Tmux::capture_argv(&[CaptureRequest {
            session_name: "mr-k3f9-1".into(),
            mode: CaptureMode::Scrollback {
                offset: 5,
                height: 20,
            },
        }]);
        let s = argv.iter().position(|a| a == "-S").unwrap();
        assert_eq!(argv[s + 1], "-5");
        assert_eq!(argv[s + 3], "14", "-E = height - 1 - offset");
    }

    #[test]
    fn scrollback_end_stays_negative_beyond_one_screen_back() {
        // Regression: clamping -E to 0 made the request `offset + 1` lines
        // tall, so the tile fell back to showing the live bottom no matter
        // how far up the user scrolled.
        let argv = Tmux::capture_argv(&[CaptureRequest {
            session_name: "mr-k3f9-1".into(),
            mode: CaptureMode::Scrollback {
                offset: 100,
                height: 20,
            },
        }]);
        let s = argv.iter().position(|a| a == "-S").unwrap();
        assert_eq!(argv[s + 1], "-100");
        assert_eq!(argv[s + 3], "-81");
    }

    #[test]
    fn socket_and_conf_path_getters_round_trip_constructor_args() {
        let t = tmux();
        assert_eq!(t.socket(), "multi-run-test");
        assert_eq!(t.conf_path(), Path::new("/tmp/multi-run-test/tmux.conf"));
    }

    #[test]
    fn next_sequential_group_id_starts_at_g1() {
        let existing = HashSet::new();
        assert_eq!(Tmux::next_sequential_group_id(&existing).as_str(), "g1");
    }

    #[test]
    fn next_sequential_group_id_fills_holes() {
        let existing: HashSet<String> = ["g1", "g3"].into_iter().map(str::to_string).collect();
        assert_eq!(Tmux::next_sequential_group_id(&existing).as_str(), "g2");
    }

    #[test]
    fn next_sequential_group_id_ignores_legacy_ids() {
        // A leftover four-character group does not block g1.
        let existing: HashSet<String> = ["k3f9"].into_iter().map(str::to_string).collect();
        assert_eq!(Tmux::next_sequential_group_id(&existing).as_str(), "g1");
    }

    #[test]
    fn resize_windows_with_empty_batch_does_not_need_a_socket() {
        // No tmux process is spawned for an empty batch, so this must
        // succeed even though `tmux` may not be installed on this machine.
        let t = tmux();
        assert!(t.resize_windows(&[]).is_ok());
    }

    #[test]
    fn poll_content_with_empty_requests_does_not_need_a_socket() {
        let t = tmux();
        assert_eq!(t.poll_content(&[]).unwrap(), Vec::new());
    }
}
