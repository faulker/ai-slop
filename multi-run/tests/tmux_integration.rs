//! Integration tests for the tmux backend (`src/tmux/mod.rs`), run against a
//! real tmux on throwaway private sockets. Covers DESIGN.md section 12's
//! integration test list. Every test starts with
//! `let Some(_) = tmux_or_skip() else { return; };` so `cargo test` still
//! succeeds on a machine without tmux (or with one below the 2.9 floor).
//!
//! ## Why `#[path]` instead of `use multi_run::...`
//!
//! `Cargo.toml` declares only a `[[bin]]` target, no `[lib]` (out of this
//! milestone's scope to add). An integration test binary can only import a
//! crate's public API through a library target, so the handful of source
//! files these tests need are pulled in directly with `#[path]`, mirroring
//! their real module tree under `src/`. This recompiles those files into the
//! test binary (including their own `#[cfg(test)]` unit tests, which harmlessly
//! run a second time under this binary's name) but requires no change to
//! `Cargo.toml` and no duplication of their logic.
//!
//! ## Isolation
//!
//! Every test gets a unique socket name (`mrtest-<tag>-<pid>-<nanos>`) and a
//! throwaway config file under the OS temp dir, via `TestServer`. Nothing
//! here ever touches the default socket or a socket named `multi-run`; the
//! user's own tmux sessions on other sockets are never referenced. `TestServer`
//! kills its server and removes its config file in `Drop`, which runs even
//! when an assertion panics.

// These modules are compiled into this test binary via `#[path]`, so anything
// the *binary* uses but these tests do not looks dead here. That is expected,
// not a signal, hence the allow on each.
#[allow(dead_code)]
#[path = "../src/cli.rs"]
mod cli;
#[allow(dead_code)]
#[path = "../src/error.rs"]
mod error;
#[allow(dead_code)]
#[path = "../src/preflight.rs"]
mod preflight;
#[allow(dead_code)]
#[path = "../src/tmux/mod.rs"]
mod tmux;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use cli::GroupId;
use error::TmuxError;
use tmux::{CaptureMode, CaptureRequest, NewSessionSpec, Tmux};

/// Current unix time in seconds, for `NewSessionSpec::started`.
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

/// A nanosecond-resolution counter used only to make throwaway socket/config
/// names unique.
fn nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos()
}

/// Run `tmux -V`, parse it with the real `preflight` module, and return the
/// parsed version when it meets the `Tier::Unsupported` floor (DESIGN.md
/// section 10: below 2.9 there is no `resize-window`/`window-size`, so this
/// whole test suite cannot run). Prints a one-line skip notice and returns
/// `None` otherwise, which every test treats as "return immediately,
/// counted as passed" so `cargo test` succeeds on a machine without tmux.
fn tmux_or_skip() -> Option<preflight::TmuxVersion> {
    let raw = match preflight::tmux_version_string() {
        Ok(raw) => raw,
        Err(_) => {
            eprintln!("skipping tmux integration test: tmux not found on PATH");
            return None;
        }
    };
    let version = match preflight::parse_version(&raw) {
        Ok(v) => v,
        Err(_) => {
            eprintln!("skipping tmux integration test: could not parse tmux version {raw:?}");
            return None;
        }
    };
    if preflight::tier_for(&version) == preflight::Tier::Unsupported {
        eprintln!(
            "skipping tmux integration test: tmux {version} is below the minimum supported version"
        );
        return None;
    }
    Some(version)
}

/// Isolates one test on its own throwaway tmux socket and generated config,
/// and guarantees cleanup on the way out, including on panic (DESIGN.md
/// section 12). `tag` only affects the socket name's readability in `tmux
/// ls` while a suite run is in flight; uniqueness comes from the pid and a
/// nanosecond timestamp, so parallel `cargo test` runs never collide with
/// each other, with a previous run, or with the user's own sockets.
struct TestServer {
    socket: String,
    conf_path: PathBuf,
    tmux: Tmux,
}

impl TestServer {
    /// Write a throwaway config (reusing the real `tmux::conf::generate`, so
    /// these tests exercise the same config content the app itself would
    /// write) and build a `Tmux` handle for it. Does not start a server;
    /// the first real tmux invocation (typically `create_session`) does
    /// that implicitly.
    fn new(tag: &str) -> Self {
        let socket = format!("mrtest-{tag}-{}-{}", std::process::id(), nanos());
        let conf_path = std::env::temp_dir().join(format!("{socket}.conf"));
        let conf_text = tmux::conf::generate(tmux::conf::ConfOptions::default());
        std::fs::write(&conf_path, conf_text).expect("write throwaway tmux.conf for test");
        let tmux = Tmux::new(socket.clone(), conf_path.clone());
        TestServer {
            socket,
            conf_path,
            tmux,
        }
    }
}

impl Drop for TestServer {
    /// Runs even when an assertion panics, which is exactly why this is
    /// `Drop` and not a teardown call at the end of each test (DESIGN.md
    /// section 12). Never touches any socket but this test's own.
    fn drop(&mut self) {
        let _ = Command::new("tmux")
            .args(["-L", &self.socket, "kill-server"])
            .output();
        let _ = std::fs::remove_file(&self.conf_path);
        // kill-server stops the server but leaves its socket file behind, so
        // remove it too; otherwise every test run leaves a dozen dead sockets
        // in the tmux socket directory forever.
        if let Some(sock) = socket_path(&self.socket) {
            let _ = std::fs::remove_file(sock);
        }
    }
}

/// Where tmux puts a named socket: `$TMUX_TMPDIR`, else `/tmp`, then
/// `tmux-<uid>/<name>`. Returns `None` when the uid cannot be read, in which
/// case the caller simply skips the cleanup.
fn socket_path(socket: &str) -> Option<PathBuf> {
    let base = std::env::var("TMUX_TMPDIR").unwrap_or_else(|_| "/tmp".to_string());
    let uid = Command::new("id").arg("-u").output().ok()?;
    let uid = String::from_utf8(uid.stdout).ok()?.trim().to_string();
    Some(PathBuf::from(base).join(format!("tmux-{uid}")).join(socket))
}

/// Build a `NewSessionSpec` for a small pane with sensible test defaults
/// (40x10, no label, `cwd` under the OS temp dir so tests never depend on
/// the invoking shell's directory).
fn mk_spec(session_name: &str, group: &str, index: u16, command: &str) -> NewSessionSpec {
    NewSessionSpec {
        session_name: session_name.to_string(),
        cwd: std::env::temp_dir(),
        width: 40,
        height: 10,
        command: command.to_string(),
        group: GroupId::new(group),
        index,
        started: now(),
        label: None,
    }
}

/// Poll `tmux.poll_metadata()` until the named session's pane satisfies
/// `pred`, or panic after `timeout`. Used to wait out the small, real delay
/// between issuing `new-session`/a signal and tmux's next `list-panes`
/// reflecting it, without a fixed sleep that would be either flaky or slow.
fn wait_for_pane(
    tmux: &Tmux,
    session_name: &str,
    timeout: Duration,
    pred: impl Fn(&tmux::parse::PaneRecord) -> bool,
) -> tmux::parse::PaneRecord {
    let deadline = Instant::now() + timeout;
    loop {
        if let Ok(meta) = tmux.poll_metadata() {
            if let Some(p) = meta.panes.iter().find(|p| p.session_name == session_name) {
                if pred(p) {
                    return p.clone();
                }
            }
        }
        if Instant::now() >= deadline {
            panic!("timed out waiting for pane {session_name:?} to satisfy the predicate");
        }
        std::thread::sleep(Duration::from_millis(30));
    }
}

// ---- create_and_read_back_metadata: item 2, @mr_* round trip -------------

#[test]
fn create_and_read_back_metadata() {
    let Some(_) = tmux_or_skip() else { return };
    let server = TestServer::new("meta");
    let spec = NewSessionSpec {
        label: Some("integration test".to_string()),
        ..mk_spec("mr-tst1-1", "tst1", 1, "sleep 30")
    };
    server.tmux.create_session(&spec).expect("create_session");

    let meta = server.tmux.poll_metadata().expect("poll_metadata");
    assert_eq!(meta.sessions.len(), 1);
    let s = &meta.sessions[0];
    assert_eq!(s.session_name, "mr-tst1-1");
    assert_eq!(s.group, "tst1");
    assert_eq!(s.index, 1);
    assert_eq!(s.label.as_deref(), Some("integration test"));
    assert_eq!(s.cmd, "sleep 30");

    assert_eq!(meta.panes.len(), 1);
    assert_eq!(meta.panes[0].session_name, "mr-tst1-1");
    assert!(!meta.panes[0].dead);
}

// ---- unrelated_session_is_filtered_out: item 2 ----------------------------

#[test]
fn unrelated_session_is_filtered_out() {
    let Some(_) = tmux_or_skip() else { return };
    let server = TestServer::new("decoy");
    server
        .tmux
        .create_session(&mk_spec("mr-tst2-1", "tst2", 1, "sleep 30"))
        .unwrap();
    // A decoy session created without going through `create_session` at
    // all, so it never gets an `@mr_group` option.
    Command::new("tmux")
        .args([
            "-L",
            &server.socket,
            "-f",
            &server.conf_path.display().to_string(),
            "new-session",
            "-d",
            "-s",
            "decoy-no-group",
            "-x",
            "40",
            "-y",
            "10",
            "sleep 30",
        ])
        .status()
        .expect("spawn decoy session");

    let meta = server.tmux.poll_metadata().unwrap();
    assert_eq!(meta.sessions.len(), 1);
    assert_eq!(meta.sessions[0].session_name, "mr-tst2-1");
}

// ---- exit_42_reports_dead_status: item 1 -----------------------------------

#[test]
fn exit_42_reports_dead_status() {
    let Some(_) = tmux_or_skip() else { return };
    let server = TestServer::new("exit42");
    server
        .tmux
        .create_session(&mk_spec("mr-tst3-1", "tst3", 1, "exit 42"))
        .unwrap();

    let pane = wait_for_pane(&server.tmux, "mr-tst3-1", Duration::from_secs(2), |p| {
        p.dead
    });
    assert_eq!(pane.dead_status, Some(42));
    assert_eq!(pane.dead_signal, None);
}

// ---- sigterm_reports_dead_signal: item 1 -----------------------------------

#[test]
fn sigterm_reports_dead_signal() {
    let Some(_) = tmux_or_skip() else { return };
    let server = TestServer::new("sigterm");
    server
        .tmux
        .create_session(&mk_spec("mr-tst4-1", "tst4", 1, "sleep 60"))
        .unwrap();

    let alive = wait_for_pane(&server.tmux, "mr-tst4-1", Duration::from_secs(2), |p| {
        !p.dead && p.pane_pid.is_some()
    });
    let pid = alive.pane_pid.expect("pane_pid present once alive");
    Command::new("kill")
        .args(["-TERM", &pid.to_string()])
        .status()
        .expect("send SIGTERM");

    let dead = wait_for_pane(&server.tmux, "mr-tst4-1", Duration::from_secs(2), |p| {
        p.dead
    });
    assert_eq!(dead.dead_status, None);
    assert_eq!(dead.dead_signal.as_deref(), Some("term"));
}

// ---- instant_exit_is_not_a_race: item 1, 20 iterations ---------------------

#[test]
fn instant_exit_is_not_a_race() {
    let Some(_) = tmux_or_skip() else { return };
    let server = TestServer::new("race");
    for i in 0..20u16 {
        let name = format!("mr-tst5-{i}");
        server
            .tmux
            .create_session(&mk_spec(&name, "tst5", i, "exit 7"))
            .unwrap();
        let pane = wait_for_pane(&server.tmux, &name, Duration::from_secs(2), |p| p.dead);
        assert_eq!(pane.dead_status, Some(7), "iteration {i}");
        server.tmux.kill_session(&name).ok();
    }
}

// ---- banner_suppression_keeps_first_line: gotcha A, tmux >= 3.3 only ------

#[test]
fn banner_suppression_keeps_first_line() {
    let Some(version) = tmux_or_skip() else {
        return;
    };
    if preflight::tier_for(&version) != preflight::Tier::Full {
        eprintln!("skipping banner_suppression_keeps_first_line: needs tmux >= 3.3");
        return;
    }
    let server = TestServer::new("banner");
    server
        .tmux
        .create_session(&mk_spec(
            "mr-tst6-1",
            "tst6",
            1,
            "printf 'A\\nB\\n'; exit 0",
        ))
        .unwrap();
    wait_for_pane(&server.tmux, "mr-tst6-1", Duration::from_secs(2), |p| {
        p.dead
    });

    let blocks = server
        .tmux
        .poll_content(&[CaptureRequest {
            session_name: "mr-tst6-1".to_string(),
            mode: CaptureMode::Live,
        }])
        .unwrap();
    assert_eq!(blocks.len(), 1);
    assert!(
        blocks[0].content.starts_with('A'),
        "expected the first printed line to survive with the dead-pane banner suppressed, got {:?}",
        blocks[0].content
    );
}

// ---- has_session_is_exact: gotcha C ----------------------------------------

#[test]
fn has_session_is_exact() {
    let Some(_) = tmux_or_skip() else { return };
    let server = TestServer::new("hasexact");
    server
        .tmux
        .create_session(&mk_spec("mr-tst7-2", "tst7", 2, "sleep 30"))
        .unwrap();

    assert!(server.tmux.has_session("mr-tst7-2").unwrap());
    // "mr-tst7" is a strict prefix of the real session name; a naive
    // (non-exact) target match would find it, which is exactly gotcha C.
    assert!(!server.tmux.has_session("mr-tst7").unwrap());
}

// ---- set_option_needs_trailing_colon: gotcha D -----------------------------

#[test]
fn set_option_needs_trailing_colon() {
    let Some(_) = tmux_or_skip() else { return };
    let server = TestServer::new("setopt");
    server
        .tmux
        .create_session(&mk_spec("mr-tst8-1", "tst8", 1, "sleep 30"))
        .unwrap();

    let without_colon = Command::new("tmux")
        .args([
            "-L",
            &server.socket,
            "set-option",
            "-t",
            "=mr-tst8-1",
            "@probe",
            "x",
        ])
        .output()
        .unwrap();
    assert!(
        !without_colon.status.success(),
        "gotcha D: set-option without the trailing colon should fail"
    );

    let with_colon = Command::new("tmux")
        .args([
            "-L",
            &server.socket,
            "set-option",
            "-t",
            "=mr-tst8-1:",
            "@probe",
            "x",
        ])
        .output()
        .unwrap();
    assert!(with_colon.status.success());
}

/// Verified deviation from DESIGN.md section 5's catalogue text (see
/// `tmux/mod.rs`'s module doc comment): gotcha D describes the trailing
/// colon as a `set-option` quirk, but `capture-pane` and `send-keys` on this
/// tmux also fail without it. Not in DESIGN.md section 12's named list, but
/// added because it pins the reason `Tmux::target()` always emits the colon.
#[test]
fn capture_pane_and_send_keys_also_need_the_trailing_colon() {
    let Some(_) = tmux_or_skip() else { return };
    let server = TestServer::new("colonpane");
    server
        .tmux
        .create_session(&mk_spec("mr-tst9-1", "tst9", 1, "sleep 30"))
        .unwrap();

    let cap_without = Command::new("tmux")
        .args([
            "-L",
            &server.socket,
            "capture-pane",
            "-p",
            "-t",
            "=mr-tst9-1",
        ])
        .output()
        .unwrap();
    assert!(!cap_without.status.success());
    let cap_with = Command::new("tmux")
        .args([
            "-L",
            &server.socket,
            "capture-pane",
            "-p",
            "-t",
            "=mr-tst9-1:",
        ])
        .output()
        .unwrap();
    assert!(cap_with.status.success());

    let keys_without = Command::new("tmux")
        .args([
            "-L",
            &server.socket,
            "send-keys",
            "-t",
            "=mr-tst9-1",
            "-l",
            "x",
        ])
        .output()
        .unwrap();
    assert!(!keys_without.status.success());
    let keys_with = Command::new("tmux")
        .args([
            "-L",
            &server.socket,
            "send-keys",
            "-t",
            "=mr-tst9-1:",
            "-l",
            "x",
        ])
        .output()
        .unwrap();
    assert!(keys_with.status.success());
}

// ---- resize_window_sets_manual_and_unset_restores: gotcha E ---------------

#[test]
fn resize_window_sets_manual_and_unset_restores() {
    let Some(_) = tmux_or_skip() else { return };
    let server = TestServer::new("resizemanual");
    server
        .tmux
        .create_session(&mk_spec("mr-tst10-1", "tst10", 1, "sleep 30"))
        .unwrap();

    server
        .tmux
        .resize_windows(&[("mr-tst10-1".to_string(), 30, 8)])
        .unwrap();
    let after_resize = Command::new("tmux")
        .args([
            "-L",
            &server.socket,
            "show-options",
            "-w",
            "-t",
            "=mr-tst10-1:",
            "window-size",
        ])
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8_lossy(&after_resize.stdout).trim(),
        "window-size manual"
    );

    server
        .tmux
        .unset_window_size(&["mr-tst10-1".to_string()])
        .unwrap();
    let after_unset = Command::new("tmux")
        .args([
            "-L",
            &server.socket,
            "show-options",
            "-w",
            "-t",
            "=mr-tst10-1:",
            "window-size",
        ])
        .output()
        .unwrap();
    // `-uw` clears the window-scope override, so it no longer appears in a
    // window-scoped `show-options -w` at all (it now inherits the global
    // default instead of reporting "manual").
    assert_ne!(
        String::from_utf8_lossy(&after_unset.stdout).trim(),
        "window-size manual"
    );
}

// ---- resize_changes_pane_dimensions: item 4 --------------------------------

#[test]
fn resize_changes_pane_dimensions() {
    let Some(_) = tmux_or_skip() else { return };
    let server = TestServer::new("resizedims");
    server
        .tmux
        .create_session(&mk_spec("mr-tst11-1", "tst11", 1, "sleep 30"))
        .unwrap();

    server
        .tmux
        .resize_windows(&[("mr-tst11-1".to_string(), 33, 9)])
        .unwrap();
    let meta = server.tmux.poll_metadata().unwrap();
    let pane = meta
        .panes
        .iter()
        .find(|p| p.session_name == "mr-tst11-1")
        .unwrap();
    assert_eq!((pane.width, pane.height), (33, 9));
}

// ---- batched_capture_splits_on_markers: item 3 -----------------------------

#[test]
fn batched_capture_splits_on_markers() {
    let Some(_) = tmux_or_skip() else { return };
    let server = TestServer::new("batchcap");
    for (i, text) in ["one", "two", "three"].iter().enumerate() {
        let name = format!("mr-tst12-{}", i + 1);
        server
            .tmux
            .create_session(&mk_spec(
                &name,
                "tst12",
                (i + 1) as u16,
                &format!("printf {text}; sleep 30"),
            ))
            .unwrap();
    }
    // Give the three panes' printf a moment to land before capturing.
    wait_for_pane(&server.tmux, "mr-tst12-3", Duration::from_secs(2), |p| {
        !p.dead
    });
    std::thread::sleep(Duration::from_millis(200));

    let requests: Vec<CaptureRequest> = (1..=3)
        .map(|i| CaptureRequest {
            session_name: format!("mr-tst12-{i}"),
            mode: CaptureMode::Live,
        })
        .collect();
    let blocks = server.tmux.poll_content(&requests).unwrap();
    assert_eq!(blocks.len(), 3);
    assert_eq!(blocks[0].session_name, "mr-tst12-1");
    assert!(blocks[0].content.contains("one"));
    assert_eq!(blocks[1].session_name, "mr-tst12-2");
    assert!(blocks[1].content.contains("two"));
    assert_eq!(blocks[2].session_name, "mr-tst12-3");
    assert!(blocks[2].content.contains("three"));
}

// ---- capture_returns_exact_pane_height: items 3, 4 -------------------------

#[test]
fn capture_returns_exact_pane_height() {
    let Some(_) = tmux_or_skip() else { return };
    let server = TestServer::new("capheight");
    let spec = NewSessionSpec {
        width: 30,
        height: 6,
        ..mk_spec("mr-tst13-1", "tst13", 1, "sleep 30")
    };
    server.tmux.create_session(&spec).unwrap();

    let blocks = server
        .tmux
        .poll_content(&[CaptureRequest {
            session_name: "mr-tst13-1".to_string(),
            mode: CaptureMode::Live,
        }])
        .unwrap();
    // `content` is lines joined with "\n" (`tmux/parse.rs::split_captures`),
    // so `split('\n')` is its exact inverse; `.lines()` is not usable here
    // because it silently drops a trailing empty segment, which a blank
    // pane's last row always is.
    assert_eq!(
        blocks[0].content.split('\n').count(),
        6,
        "no -S/-E: capture-pane must return exactly pane_height lines"
    );
}

// ---- scrollback capture keeps the tile full at every offset ---------------

/// Walks a real pane's scrollback the way `k`/wheel-up does and asserts every
/// step still yields exactly `height` lines of the expected content. The bug
/// this covers: past one screen back, a `-E` clamped to 0 silently returned
/// the live bottom; past the top of the history, tmux clamps `-S` and returns
/// a short capture, which the tile pads at the bottom (output looks eaten from
/// the bottom up).
#[test]
fn scrollback_capture_returns_a_full_tile_at_every_offset() {
    let Some(_) = tmux_or_skip() else { return };
    let server = TestServer::new("scrollbk");
    let height: u16 = 6;
    let spec = NewSessionSpec {
        width: 30,
        height,
        ..mk_spec(
            "mr-tst19-1",
            "tst19",
            1,
            "for i in $(seq 1 40); do echo line$i; done; sleep 30",
        )
    };
    server.tmux.create_session(&spec).unwrap();
    wait_for_pane(&server.tmux, "mr-tst19-1", Duration::from_secs(2), |p| {
        p.history_size > 0
    });
    std::thread::sleep(Duration::from_millis(200));

    let panes = server.tmux.poll_metadata().unwrap().panes;
    let history = panes[0].history_size;
    assert!(history > height as u32, "need history to scroll through");

    for offset in [1u32, height as u32, height as u32 + 5, history] {
        let blocks = server
            .tmux
            .poll_content(&[CaptureRequest {
                session_name: "mr-tst19-1".to_string(),
                mode: CaptureMode::Scrollback { offset, height },
            }])
            .unwrap();
        let lines: Vec<&str> = blocks[0].content.split('\n').collect();
        assert_eq!(
            lines.len(),
            height as usize,
            "offset {offset}: capture must stay exactly one tile tall"
        );
        assert!(
            lines.iter().any(|l| l.starts_with("line")),
            "offset {offset}: expected real output, got {lines:?}"
        );
    }

    // Consecutive offsets must actually move the window, not repeat the same
    // screen (the symptom of a clamped -E).
    let mut seen: Vec<String> = Vec::new();
    for offset in [height as u32, height as u32 * 2] {
        let blocks = server
            .tmux
            .poll_content(&[CaptureRequest {
                session_name: "mr-tst19-1".to_string(),
                mode: CaptureMode::Scrollback { offset, height },
            }])
            .unwrap();
        seen.push(blocks[0].content.clone());
    }
    assert_ne!(
        seen[0], seen[1],
        "scrolling further back must show new lines"
    );
}

// ---- tab_separator_is_a_real_tab: section 5 field separator ---------------

#[test]
fn tab_separator_is_a_real_tab() {
    let Some(_) = tmux_or_skip() else { return };
    let server = TestServer::new("tabsep");
    server
        .tmux
        .create_session(&mk_spec("mr-tst14-1", "tst14", 1, "sleep 30"))
        .unwrap();

    // Mirrors the literal-tab-byte format string `tmux/mod.rs` builds (its
    // SESSION_FORMAT constant is private, so this reproduces the same
    // mechanic rather than importing it): DESIGN.md section 5 says to emit
    // tabs with `#{l:\t}`, but that renders as two literal characters on
    // this tmux, so the format string carries a real 0x09 byte instead
    // (verified separately; see the module doc comment in tmux/mod.rs).
    let raw = Command::new("tmux")
        .args([
            "-L",
            &server.socket,
            "list-sessions",
            "-F",
            "#{session_name}\t#{@mr_group}",
        ])
        .output()
        .unwrap();
    assert!(raw.status.success());
    assert!(
        raw.stdout.contains(&0x09u8),
        "expected a literal tab byte (0x09) in list-sessions output"
    );

    // The full round trip through our own parser is the load-bearing check:
    // if the separator were not a real tab, `parse_session_records` would
    // fail with a field-count error instead of succeeding.
    let meta = server.tmux.poll_metadata().unwrap();
    assert_eq!(meta.sessions[0].group, "tst14");
}

// ---- no_server_stderr_is_classified_benign: item 7 -------------------------

#[test]
fn no_server_stderr_is_classified_benign() {
    let Some(_) = tmux_or_skip() else { return };
    let socket = format!("mrtest-noserver-{}-{}", std::process::id(), nanos());
    let conf_path = std::env::temp_dir().join(format!("{socket}.conf"));
    // Deliberately do not create this socket's server at all: it has never
    // existed, which is the "error connecting to" shape from research item 7.
    let tmux = Tmux::new(socket, conf_path);

    let meta = tmux
        .poll_metadata()
        .expect("NoServer must collapse to an empty poll, not an Err");
    assert!(meta.sessions.is_empty());
    assert!(meta.panes.is_empty());

    assert!(!tmux.has_session("anything").unwrap());
    assert!(matches!(
        tmux.kill_session("anything").unwrap_err(),
        TmuxError::NoServer { .. }
    ));
    assert!(
        tmux.kill_server().is_ok(),
        "kill-server on an absent server is treated as already-done, not an error"
    );
}

// ---- empty_but_running_server_polls_empty: regression -----------------------

/// Our config sets `exit-empty off`, so killing the last session leaves the
/// server alive with zero sessions. In that state `list-sessions` succeeds
/// with empty output but the chained `list-panes -a` fails with "no current
/// target", which classifies as `NoSuchSession`. That is still "nothing is
/// running", so the poll must return empty rather than erroring. Regression
/// test: this previously made `--list` fail with a confusing message once
/// every session had been killed.
#[test]
fn empty_but_running_server_polls_empty() {
    let Some(_) = tmux_or_skip() else { return };
    let server = TestServer::new("emptysrv");

    server
        .tmux
        .create_session(&mk_spec("mr-aaaa-1", "aaaa", 1, "sleep 30"))
        .expect("create the only session");
    server
        .tmux
        .kill_session("mr-aaaa-1")
        .expect("kill the only session");

    // The server must still be up, which is what makes this case distinct
    // from the never-existed one above.
    assert!(
        !server
            .tmux
            .has_session("mr-aaaa-1")
            .expect("has-session on an empty server"),
        "the session is gone"
    );

    let meta = server
        .tmux
        .poll_metadata()
        .expect("an empty but running server must poll empty, not error");
    assert!(meta.sessions.is_empty());
    assert!(meta.panes.is_empty());
}

// ---- f_flag_does_not_reset_running_options: section 5 open question 5 -----

#[test]
fn f_flag_does_not_reset_running_options() {
    let Some(_) = tmux_or_skip() else { return };
    let server = TestServer::new("fflag");
    // Bootstraps the server using server.tmux's `-f` config, which sets
    // `remain-on-exit on`.
    server
        .tmux
        .create_session(&mk_spec("mr-tst16-1", "tst16", 1, "sleep 30"))
        .unwrap();

    // Flip the option at runtime, bypassing the config file entirely.
    Command::new("tmux")
        .args([
            "-L",
            &server.socket,
            "set-option",
            "-g",
            "remain-on-exit",
            "off",
        ])
        .status()
        .unwrap();

    // Every `Tmux` method still passes the same `-f <conf>` (base()); if `-f`
    // re-applied the config on every call, this poll would silently flip
    // the option back to "on".
    server.tmux.poll_metadata().unwrap();

    let show = Command::new("tmux")
        .args(["-L", &server.socket, "show-options", "-g", "remain-on-exit"])
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8_lossy(&show.stdout).trim(),
        "remain-on-exit off"
    );
}

// ---- command_with_tab_survives_round_trip: section 14 open question 2 -----

#[test]
fn command_with_tab_survives_round_trip() {
    let Some(_) = tmux_or_skip() else { return };
    let server = TestServer::new("cmdtab");
    let cmd_with_tab = "printf 'a\tb'; sleep 30"; // a real tab byte, embedded in the command text itself
    server
        .tmux
        .create_session(&mk_spec("mr-tst17-1", "tst17", 1, cmd_with_tab))
        .unwrap();

    let meta = server.tmux.poll_metadata().unwrap();
    let s = meta
        .sessions
        .iter()
        .find(|s| s.session_name == "mr-tst17-1")
        .unwrap();
    assert_eq!(
        s.cmd, cmd_with_tab,
        "@mr_cmd is the last field; splitn(8) must recover the embedded tab intact"
    );
}

// ---- session creation survives command exit with a readable pane ---------

#[test]
fn session_survives_command_exit_with_readable_pane() {
    let Some(_) = tmux_or_skip() else { return };
    let server = TestServer::new("survive");
    server
        .tmux
        .create_session(&mk_spec("mr-tst19-1", "tst19", 1, "echo done; exit 0"))
        .unwrap();

    let pane = wait_for_pane(&server.tmux, "mr-tst19-1", Duration::from_secs(2), |p| {
        p.dead
    });
    assert_eq!(pane.dead_status, Some(0));
    // remain-on-exit keeps the session (and its pane output) around instead
    // of tearing it down the instant the command exits (research item 1).
    assert!(server.tmux.has_session("mr-tst19-1").unwrap());

    let blocks = server
        .tmux
        .poll_content(&[CaptureRequest {
            session_name: "mr-tst19-1".to_string(),
            mode: CaptureMode::Live,
        }])
        .unwrap();
    assert!(blocks[0].content.contains("done"));
}

// ---- captured content matches what was printed -----------------------------

#[test]
fn captured_content_matches_what_was_printed() {
    let Some(_) = tmux_or_skip() else { return };
    let server = TestServer::new("capmatch");
    server
        .tmux
        .create_session(&mk_spec(
            "mr-tst18-1",
            "tst18",
            1,
            "printf 'hello multi-run'; sleep 30",
        ))
        .unwrap();
    wait_for_pane(&server.tmux, "mr-tst18-1", Duration::from_secs(2), |p| {
        !p.dead
    });
    std::thread::sleep(Duration::from_millis(200));

    let blocks = server
        .tmux
        .poll_content(&[CaptureRequest {
            session_name: "mr-tst18-1".to_string(),
            mode: CaptureMode::Live,
        }])
        .unwrap();
    assert_eq!(blocks.len(), 1);
    assert!(blocks[0].content.starts_with("hello multi-run"));
}

// ---- kill-session removes the session --------------------------------------

#[test]
fn kill_session_removes_the_session() {
    let Some(_) = tmux_or_skip() else { return };
    let server = TestServer::new("killsess");
    server
        .tmux
        .create_session(&mk_spec("mr-tst20-1", "tst20", 1, "sleep 30"))
        .unwrap();
    assert!(server.tmux.has_session("mr-tst20-1").unwrap());

    server.tmux.kill_session("mr-tst20-1").unwrap();
    assert!(!server.tmux.has_session("mr-tst20-1").unwrap());
}

// ---- GroupId generation avoids a live collision ---------------------------

#[test]
fn new_group_id_avoids_colliding_with_a_live_group() {
    let Some(_) = tmux_or_skip() else { return };
    let server = TestServer::new("groupid");
    let existing = server.tmux.new_group_id().unwrap();
    let name = format!("mr-{}-1", existing.as_str());
    server
        .tmux
        .create_session(&mk_spec(&name, existing.as_str(), 1, "sleep 30"))
        .unwrap();

    for _ in 0..20 {
        let id = server.tmux.new_group_id().unwrap();
        assert_ne!(id.as_str(), existing.as_str());
    }
}

// ---- job-pane pinning: an extra window must not shadow the job -----------

/// Open a second window in `session`, the way a user can with their own tmux
/// prefix while attached, and leave it selected. Returns once tmux has
/// created it.
fn open_extra_window(server: &TestServer, session: &str, command: &str) {
    let status = Command::new("tmux")
        .args([
            "-L",
            &server.socket,
            "new-window",
            "-t",
            &format!("={session}:"),
            command,
        ])
        .status()
        .expect("spawn extra window");
    assert!(status.success(), "new-window failed for {session}");
}

#[test]
fn extra_window_does_not_shadow_the_job_pane_in_the_metadata_poll() {
    let Some(_) = tmux_or_skip() else { return };
    let server = TestServer::new("shellwin");
    server
        .tmux
        .create_session(&mk_spec("mr-tst21-1", "tst21", 1, "exit 7"))
        .unwrap();
    wait_for_pane(&server.tmux, "mr-tst21-1", Duration::from_secs(5), |p| {
        p.dead
    });

    // The job has finished; the user attaches and opens a window of their
    // own. The poll must still describe the job, not that window.
    open_extra_window(&server, "mr-tst21-1", "sleep 30");

    let meta = server.tmux.poll_metadata().unwrap();
    let rows: Vec<_> = meta
        .panes
        .iter()
        .filter(|p| p.session_name == "mr-tst21-1")
        .collect();
    assert_eq!(rows.len(), 1, "one row per job session, got {rows:?}");
    assert!(rows[0].dead, "the job pane is still the dead one");
    assert_eq!(rows[0].dead_status, Some(7));
}

#[test]
fn capture_reads_the_job_pane_while_another_window_is_current() {
    let Some(_) = tmux_or_skip() else { return };
    let server = TestServer::new("shellcap");
    server
        .tmux
        .create_session(&mk_spec(
            "mr-tst22-1",
            "tst22",
            1,
            "echo job-output-marker; sleep 30",
        ))
        .unwrap();
    wait_for_pane(&server.tmux, "mr-tst22-1", Duration::from_secs(5), |p| {
        p.cursor_y > 0
    });

    open_extra_window(&server, "mr-tst22-1", "echo shell-output-marker; sleep 30");

    let blocks = server
        .tmux
        .poll_content(&[CaptureRequest {
            session_name: "mr-tst22-1".into(),
            mode: CaptureMode::Live,
        }])
        .unwrap();
    assert_eq!(blocks.len(), 1);
    assert!(
        blocks[0].content.contains("job-output-marker"),
        "tile drew {:?} instead of the job pane",
        blocks[0].content
    );
    assert!(!blocks[0].content.contains("shell-output-marker"));
}

#[test]
fn interrupt_reaches_the_job_pane_not_the_current_window() {
    let Some(_) = tmux_or_skip() else { return };
    let server = TestServer::new("shellstop");
    server
        .tmux
        .create_session(&mk_spec("mr-tst23-1", "tst23", 1, "sleep 30"))
        .unwrap();
    wait_for_pane(&server.tmux, "mr-tst23-1", Duration::from_secs(5), |p| {
        !p.dead
    });
    open_extra_window(&server, "mr-tst23-1", "sleep 30");

    // `s` in the TUI while another window happens to be current must still
    // stop the job.
    server.tmux.send_keys_interrupt("mr-tst23-1").unwrap();
    let pane = wait_for_pane(&server.tmux, "mr-tst23-1", Duration::from_secs(5), |p| {
        p.dead
    });
    assert!(pane.dead);

    // And the other window is untouched: it is not in the filtered poll, so
    // ask tmux directly.
    let out = Command::new("tmux")
        .args([
            "-L",
            &server.socket,
            "list-panes",
            "-t",
            "=mr-tst23-1:1",
            "-F",
            "#{pane_dead}",
        ])
        .output()
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "0");
}

// ---- the generated config's key bindings and status bar are real ----------

#[test]
fn generated_conf_binds_the_documented_keys_and_shows_the_status_bar() {
    let Some(_) = tmux_or_skip() else { return };
    let server = TestServer::new("keys");
    server
        .tmux
        .create_session(&mk_spec("mr-tst24-1", "tst24", 1, "sleep 30"))
        .unwrap();

    let keys = Command::new("tmux")
        .args(["-L", &server.socket, "list-keys", "-T", "root"])
        .output()
        .unwrap();
    let keys = String::from_utf8_lossy(&keys.stdout);
    // tmux prints the detach key back as the escaped `C-\\`.
    assert!(
        keys.contains("C-\\\\") && keys.contains("detach-client"),
        "no detach binding in:\n{keys}"
    );
    for line in ["C-n", "C-p"] {
        assert!(keys.contains(line), "missing {line} binding in:\n{keys}");
    }

    let status = Command::new("tmux")
        .args(["-L", &server.socket, "show-options", "-gv", "status"])
        .output()
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&status.stdout).trim(), "on");

    // The bar renders: expand the format the way an attached client would.
    let bar = Command::new("tmux")
        .args([
            "-L",
            &server.socket,
            "display-message",
            "-t",
            "=mr-tst24-1:",
            "-p",
            "#{E:status-format[0]}",
        ])
        .output()
        .unwrap();
    let bar = String::from_utf8_lossy(&bar.stdout);
    // Style directives sit between each key and its label, so check the
    // pieces. The single backslash here is the point: the config's `\\`
    // inside a double-quoted value must reach the bar as one character.
    for piece in ["Ctrl+\\ ", "detach", "Ctrl+n ", "next", "Ctrl+p ", "prev"] {
        assert!(
            bar.contains(piece),
            "status bar is missing {piece:?}, rendered as {bar:?}"
        );
    }
    assert!(bar.contains("sleep 30"), "status bar should name the job");
    assert!(
        bar.contains("mr-tst24-1"),
        "status bar should name the session"
    );
}

// ---- source_conf reaches a server that is already running -----------------

#[test]
fn source_conf_updates_an_already_running_server() {
    let Some(_) = tmux_or_skip() else { return };
    let server = TestServer::new("sourceconf");
    server
        .tmux
        .create_session(&mk_spec("mr-tst26-1", "tst26", 1, "sleep 30"))
        .unwrap();

    // Stand in for a config upgrade: the server started with the old file,
    // so `-f` will never be read again.
    let mut conf = std::fs::read_to_string(&server.conf_path).unwrap();
    conf.push_str("bind-key -n F12 display-message mr-source-conf-marker\n");
    std::fs::write(&server.conf_path, &conf).unwrap();

    let before = Command::new("tmux")
        .args(["-L", &server.socket, "list-keys", "-T", "root"])
        .output()
        .unwrap();
    assert!(!String::from_utf8_lossy(&before.stdout).contains("mr-source-conf-marker"));

    server.tmux.source_conf().unwrap();

    let after = Command::new("tmux")
        .args(["-L", &server.socket, "list-keys", "-T", "root"])
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&after.stdout).contains("mr-source-conf-marker"));
}

#[test]
fn source_conf_without_a_server_is_not_an_error() {
    let Some(_) = tmux_or_skip() else { return };
    let server = TestServer::new("sourcenoserver");
    server
        .tmux
        .source_conf()
        .expect("no server is not a failure");
}

/// Job panes must load the user's shell profile (e.g. `.zshrc`), matching a
/// normal terminal emulator. Uses a throwaway `ZDOTDIR` so the test never
/// touches the real home directory's rc files.
#[test]
fn job_pane_loads_user_shell_profile() {
    let Some(_) = tmux_or_skip() else { return };
    if !PathBuf::from("/bin/zsh").is_file() {
        eprintln!("skipping profile-load test: /bin/zsh not found");
        return;
    }

    let zdot = std::env::temp_dir().join(format!(
        "mr-zdot-{}-{}",
        std::process::id(),
        nanos()
    ));
    std::fs::create_dir_all(&zdot).unwrap();
    std::fs::write(zdot.join(".zshrc"), "export MR_PROFILE_PROBE=from-zshrc\n").unwrap();

    let socket = format!("mrtest-profile-{}-{}", std::process::id(), nanos());
    let conf_path = std::env::temp_dir().join(format!("{socket}.conf"));
    tmux::conf::ensure_with_user_shell(
        &conf_path,
        tmux::conf::ConfOptions {
            source_user_conf: false,
            tmux_3_3_plus: true,
            default_shell: PathBuf::from("/bin/sh"), // overwritten by ensure
        },
        Path::new("/bin/zsh"),
    )
    .expect("write conf + user-shell wrapper");

    let tmux = Tmux::new(socket.clone(), conf_path.clone());
    let out = std::env::temp_dir().join(format!("{socket}.out"));
    let _ = std::fs::remove_file(&out);

    // ZDOTDIR must reach the pane: the tmux server inherits this process env.
    // SAFETY: single-threaded test setup before any other test logic races on
    // env; restored below. Integration tests run in separate processes under
    // cargo by default for this binary's tests when not parallelized on env,
    // but we still restore to be polite.
    let prev_zdot = std::env::var_os("ZDOTDIR");
    // std::env::set_var is unsafe on Rust 2024; integration test accepts it.
    unsafe { std::env::set_var("ZDOTDIR", &zdot) };

    let cmd = format!(
        "printf '%s\\n' \"${{MR_PROFILE_PROBE:-unset}}\" > {}",
        out.display()
    );
    let create = tmux.create_session(&mk_spec("mr-profile-1", "prof", 1, &cmd));

    match prev_zdot {
        Some(v) => unsafe { std::env::set_var("ZDOTDIR", v) },
        None => unsafe { std::env::remove_var("ZDOTDIR") },
    }

    create.expect("create_session");

    let deadline = Instant::now() + Duration::from_secs(5);
    let mut body = String::new();
    while Instant::now() < deadline {
        if let Ok(text) = std::fs::read_to_string(&out) {
            body = text;
            if body.contains("from-zshrc") || body.contains("unset") {
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }

    let _ = Command::new("tmux")
        .args(["-L", &socket, "kill-server"])
        .output();
    let _ = std::fs::remove_file(&conf_path);
    let _ = std::fs::remove_file(tmux::conf::wrapper_path(&conf_path));
    let _ = std::fs::remove_file(&out);
    let _ = std::fs::remove_dir_all(&zdot);
    if let Some(sock) = socket_path(&socket) {
        let _ = std::fs::remove_file(sock);
    }

    assert_eq!(
        body.trim(),
        "from-zshrc",
        "job pane must source .zshrc via the user-shell wrapper"
    );
}

/// Read a running process's working directory, or `None` on a platform where
/// this test does not know how (in which case the caller skips the check).
/// Mirrors the production `tmux::process_cwd`, which is private; the point of
/// re-deriving it here is that the test agrees with the *system*, not with the
/// implementation it is checking.
fn process_cwd(pid: u32) -> Option<PathBuf> {
    if cfg!(target_os = "linux") {
        return std::fs::read_link(format!("/proc/{pid}/cwd")).ok();
    }
    let out = Command::new("lsof")
        .args(["-a", "-p", &pid.to_string(), "-d", "cwd", "-F", "n"])
        .output()
        .ok()?;
    String::from_utf8(out.stdout).ok()?.lines().find_map(|l| {
        l.strip_prefix('n')
            .filter(|p| !p.is_empty())
            .map(PathBuf::from)
    })
}

/// The server's pid, for tests that need to look at the process itself.
fn server_pid(socket: &str) -> u32 {
    let out = Command::new("tmux")
        .args(["-L", socket, "display-message", "-p", "#{pid}"])
        .output()
        .expect("read server pid");
    String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse()
        .expect("server pid is a number")
}

/// The tmux server takes its working directory from whichever multi-run
/// invocation starts it, and keeps it for life. That directory must be the one
/// multi-run was started in, which means our tmux children are spawned with no
/// directory override of their own.
#[test]
fn server_starts_in_the_directory_multi_run_was_started_in() {
    let Some(_) = tmux_or_skip() else { return };
    let srv = TestServer::new("startdir");

    // This is the invocation that starts the server, so its working directory
    // (this test process's, which is the crate root) is the server's.
    let spec = mk_spec("mr-start-1", "start", 1, "sleep 30");
    srv.tmux.create_session(&spec).expect("create session");
    wait_for_pane(&srv.tmux, "mr-start-1", Duration::from_secs(5), |p| !p.dead);

    let Some(cwd) = process_cwd(server_pid(&srv.socket)) else {
        eprintln!("skipping server cwd assertion: cannot read process cwd on this platform");
        return;
    };
    assert_eq!(
        std::fs::canonicalize(&cwd).expect("server cwd exists"),
        std::fs::canonicalize(std::env::current_dir().unwrap()).unwrap(),
        "the server must start in the directory multi-run was started in"
    );

    // Inheriting that directory must not cost a job its own: `-c` still
    // decides where the pane's shell starts.
    let out = Command::new("tmux")
        .args([
            "-L",
            &srv.socket,
            "display-message",
            "-p",
            "-t",
            "=mr-start-1:0.0",
            "#{pane_current_path}",
        ])
        .output()
        .expect("read pane_current_path");
    let pane_path = String::from_utf8_lossy(&out.stdout).trim().to_string();
    assert_eq!(
        std::fs::canonicalize(&pane_path).expect("pane path exists"),
        std::fs::canonicalize(&spec.cwd).expect("spec cwd exists"),
        "job pane must start in the directory passed as new-session -c"
    );
}

/// A live server standing in a directory that exists is not stale, and neither
/// is a socket with no server on it at all.
#[test]
fn server_cwd_state_is_live_for_a_healthy_server() {
    let Some(_) = tmux_or_skip() else { return };
    let srv = TestServer::new("cwdlive");

    assert_eq!(
        srv.tmux.server_cwd_state(),
        tmux::ServerCwd::NoServer,
        "nothing can be stale before a server exists"
    );

    srv.tmux
        .create_session(&mk_spec("mr-live-1", "live", 1, "sleep 30"))
        .expect("create session");
    wait_for_pane(&srv.tmux, "mr-live-1", Duration::from_secs(5), |p| !p.dead);

    match srv.tmux.server_cwd_state() {
        tmux::ServerCwd::Live => {}
        tmux::ServerCwd::Unknown => {
            eprintln!("skipping: cannot read process cwd on this platform")
        }
        other => panic!("healthy server reported as {other:?}"),
    }
}

/// The regression this check exists for: a server left standing in a directory
/// that is then deleted. Every pane it forks afterwards inherits the unlinked
/// directory, `getcwd` fails there, and the job's shell opens with
/// `shell-init: error retrieving current directory`. The state must be
/// reported as `Gone`, carrying the session count `main.rs` uses to decide
/// between replacing the server silently and telling the user about it.
#[test]
fn server_cwd_state_is_gone_once_its_directory_is_deleted() {
    let Some(_) = tmux_or_skip() else { return };
    let srv = TestServer::new("cwdgone");

    let doomed = std::env::temp_dir().join(format!("{}-doomed", srv.socket));
    std::fs::create_dir_all(&doomed).expect("create the doomed directory");

    // Start the server the way a multi-run run from `doomed` would, then take
    // the directory away, exactly as a rename or a `rm -rf` of a project would.
    let started = Command::new("tmux")
        .args([
            "-L",
            &srv.socket,
            "-f",
            &srv.conf_path.display().to_string(),
            "start-server",
        ])
        .current_dir(&doomed)
        .status()
        .expect("start server from the doomed directory");
    assert!(started.success(), "tmux start-server failed");
    std::fs::remove_dir(&doomed).expect("delete the doomed directory");

    let state = srv.tmux.server_cwd_state();
    if state == tmux::ServerCwd::Unknown {
        eprintln!("skipping: cannot read process cwd on this platform");
        return;
    }
    // The reported path is only checked by its final component: the deleted
    // directory can no longer be canonicalised, and macOS resolves the temp
    // dir through the /var -> /private/var symlink, so the two spellings of
    // the same directory never compare equal.
    match state {
        tmux::ServerCwd::Gone { cwd, sessions } => {
            assert_eq!(cwd.file_name(), doomed.file_name());
            assert_eq!(
                sessions, 0,
                "the session count is what decides whether replacing the server is free"
            );
        }
        other => panic!(
            "a server whose directory was deleted must be reported as Gone, got {other:?}"
        ),
    }

    // And the cure has to work: a replacement server started from a directory
    // that exists is healthy again.
    srv.tmux.kill_server().expect("kill the stale server");
    srv.tmux
        .create_session(&mk_spec("mr-gone-1", "gone", 1, "sleep 30"))
        .expect("create session on the replacement server");
    wait_for_pane(&srv.tmux, "mr-gone-1", Duration::from_secs(5), |p| !p.dead);
    assert_eq!(
        srv.tmux.server_cwd_state(),
        tmux::ServerCwd::Live,
        "the replacement server must be healthy"
    );
}

// ---- attached-session scrollback: mouse mode + copy-mode exit ------------

/// The single-session (attached) view has no TUI mouse handling of its own:
/// `attach.rs` tears the TUI's capture down before handing tmux the terminal.
/// So the wheel only scrolls if the real server actually took `mouse on` from
/// the generated config. A generated-string unit test cannot prove that; a
/// user config sourced ahead of ours could have set it off, and tmux could
/// reject the line outright.
#[test]
fn attached_session_has_mouse_and_scrollback_enabled() {
    let Some(_) = tmux_or_skip() else { return };
    let srv = TestServer::new("mouse");
    srv.tmux
        .create_session(&mk_spec("mr-mouse-1", "mouse", 1, "sleep 30"))
        .expect("create session");
    wait_for_pane(&srv.tmux, "mr-mouse-1", Duration::from_secs(5), |p| !p.dead);

    let show = |opt: &str| {
        let out = Command::new("tmux")
            .args(["-L", &srv.socket, "show-options", "-gv", opt])
            .output()
            .expect("tmux show-options");
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    };
    assert_eq!(show("mouse"), "on", "the wheel cannot scroll without this");
    assert_eq!(show("history-limit"), "50000");
}

/// The status bar tells the user "ESC exits scroll mode". Under `mode-keys
/// vi` tmux's stock `Escape` is `clear-selection`, which does not exit, so
/// the config binds both copy-mode tables. Assert the server really holds
/// `cancel` in each.
#[test]
fn escape_cancels_copy_mode_on_a_real_server() {
    let Some(_) = tmux_or_skip() else { return };
    let srv = TestServer::new("escape");
    srv.tmux
        .create_session(&mk_spec("mr-esc-1", "escape", 1, "sleep 30"))
        .expect("create session");
    wait_for_pane(&srv.tmux, "mr-esc-1", Duration::from_secs(5), |p| !p.dead);

    for table in ["copy-mode", "copy-mode-vi"] {
        let out = Command::new("tmux")
            .args(["-L", &srv.socket, "list-keys", "-T", table])
            .output()
            .expect("tmux list-keys");
        let keys = String::from_utf8_lossy(&out.stdout);
        let escape = keys
            .lines()
            .find(|l| l.contains(" Escape "))
            .unwrap_or_else(|| panic!("no Escape binding in table {table}"));
        assert!(
            escape.contains("cancel"),
            "Escape in {table} must cancel copy mode, got: {escape}"
        );
    }
}

/// End to end: the pane keeps real scrollback and copy mode can walk into
/// it. This is the behaviour the user actually wanted, one layer below the
/// wheel: `mouse on` only routes the wheel to `copy-mode -e`, so if copy
/// mode could not reach history, enabling the mouse would change nothing.
#[test]
fn copy_mode_reaches_scrollback_above_the_visible_pane() {
    let Some(_) = tmux_or_skip() else { return };
    let srv = TestServer::new("scrollback");
    // 60 lines of output through a 10-row pane: most of it is history.
    srv.tmux
        .create_session(&mk_spec(
            "mr-scroll-1",
            "scrollback",
            1,
            "for i in $(seq 1 60); do echo line-$i; done; sleep 30",
        ))
        .expect("create session");
    wait_for_pane(&srv.tmux, "mr-scroll-1", Duration::from_secs(5), |p| !p.dead);

    // Wait for the last line to land, then read back further than the pane
    // is tall. `-S -40` starts 40 lines above the visible region, which only
    // returns anything if history was retained.
    let deadline = Instant::now() + Duration::from_secs(5);
    let scrolled = loop {
        let out = Command::new("tmux")
            .args([
                "-L",
                &srv.socket,
                "capture-pane",
                "-p",
                "-S",
                "-40",
                "-t",
                "=mr-scroll-1:0.0",
            ])
            .output()
            .expect("tmux capture-pane");
        let scrolled = String::from_utf8_lossy(&out.stdout).to_string();
        if scrolled.contains("line-60") && scrolled.contains("line-25") {
            break scrolled;
        }
        assert!(
            Instant::now() < deadline,
            "timed out; captured scrollback was:\n{scrolled}"
        );
        std::thread::sleep(Duration::from_millis(50));
    };
    // line-25 scrolled off a 10-row pane long ago, so reading it back proves
    // the wheel has somewhere to go.
    assert!(scrolled.contains("line-25"));
}
