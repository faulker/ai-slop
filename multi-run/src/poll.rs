//! The poller thread and its scheduling. Owns the `Tmux` handle and every
//! tmux subprocess, so all tmux latency lives off the main thread (DESIGN.md
//! section 6). Contains no ratatui types: captured output crosses the channel
//! as raw ANSI bytes and is decoded on main, next to the buffer that consumes
//! it.
//!
//! The scheduling decisions are pure functions (`content_interval`, `due`,
//! `due_requests`), separated from the thread body so the whole poll-rate
//! table is unit-testable without spawning tmux.

use std::collections::HashMap;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

use crossterm::event::{KeyEvent, MouseEvent};

use crate::tmux::parse::{PaneRecord, SessionRecord};
use crate::tmux::{CaptureMode, CaptureRequest, NewSessionSpec, Tmux};

/// Metadata poll period: 4 Hz, one batched call covering every tracked job
/// (DESIGN.md section 6 poll-rate table).
pub const META_INTERVAL: Duration = Duration::from_millis(250);

/// Content poll period for a visible, running tile: 10 Hz.
pub const FAST_INTERVAL: Duration = Duration::from_millis(100);

/// Content poll period for an offscreen tile: 1 Hz.
pub const SLOW_INTERVAL: Duration = Duration::from_millis(1000);

/// Content poll period for a visible tile whose job has finished and whose
/// content has stopped changing: 0.2 Hz. Dead panes do not change unless
/// resized, so backing them off costs nothing.
pub const IDLE_INTERVAL: Duration = Duration::from_millis(5000);

/// Past this many tiles, offscreen tiles stop being captured entirely. The
/// 4 Hz metadata poll still drives their "new output" indicator via
/// `history_size` / `cursor_y`, at no capture cost (DESIGN.md section 6).
pub const OFFSCREEN_CAPTURE_LIMIT: usize = 12;

/// How many consecutive unchanged captures mark a finished tile as idle and
/// drop it to `IDLE_INTERVAL`.
pub const STALE_POLLS: u32 = 3;

/// Messages from the input and poller threads to main.
pub enum ToMain {
    Key(KeyEvent),
    Resize(u16, u16),
    Mouse(MouseEvent),
    Meta {
        sessions: Vec<SessionRecord>,
        panes: Vec<PaneRecord>,
    },
    /// Raw `capture-pane -p -e` bytes per session name, decoded on main.
    Content {
        updates: Vec<(String, Vec<u8>)>,
    },
    TmuxError(String),
    /// Ack that the poller has stopped issuing tmux calls, for the attach
    /// handshake (DESIGN.md section 8).
    PollerPaused,
}

/// A one-shot tmux operation, issued from main and executed on the poller so
/// that main never blocks on a subprocess.
pub enum TmuxOp {
    Create(Box<NewSessionSpec>),
    KillSession(String),
    SendInterrupt(String),
    SendEof(String),
    /// Signal a session's pane process directly, for stop levels 2 and 3.
    /// `send-keys C-c` only reaches a process that reads its terminal, so a
    /// wedged or non-interactive job needs a real signal.
    Signal {
        session: String,
        signal: &'static str,
    },
}

/// Messages from main to the poller thread.
pub enum ToPoller {
    /// The current visible set and each tile's capture mode. Replaces the
    /// poller's whole view of what to capture.
    SetVisible(Vec<TileSpec>),
    ResizeBatch(Vec<(String, u16, u16)>),
    OneShot(TmuxOp),
    Pause,
    Resume,
    Shutdown,
}

/// What main tells the poller about one tracked tile.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TileSpec {
    pub session_name: String,
    /// On the current grid page, or the focused detail pane in list view.
    pub visible: bool,
    /// Job state, reduced to what scheduling depends on.
    pub kind: TileKind,
    pub mode: CaptureMode,
}

/// The scheduling-relevant part of a job's state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TileKind {
    Running,
    /// Finished, with a count of consecutive captures that produced identical
    /// bytes. Once that reaches `STALE_POLLS`, the tile backs off.
    Finished {
        stale: u32,
    },
    /// Content is frozen; never capture again.
    Vanished,
}

/// How often this tile's content should be captured, or `None` for never.
/// Pure, so the DESIGN.md section 6 poll-rate table is testable without tmux.
pub fn content_interval(spec: &TileSpec, total_tiles: usize) -> Option<Duration> {
    match spec.kind {
        // A vanished session is gone from the server: capturing would fail,
        // and the last content is all we will ever have.
        TileKind::Vanished => None,
        _ if !spec.visible => {
            if total_tiles > OFFSCREEN_CAPTURE_LIMIT {
                None
            } else {
                Some(SLOW_INTERVAL)
            }
        }
        TileKind::Running => Some(FAST_INTERVAL),
        TileKind::Finished { stale } => Some(if stale >= STALE_POLLS {
            IDLE_INTERVAL
        } else {
            FAST_INTERVAL
        }),
    }
}

/// Is a tile due for capture, given when it was last captured? A tile never
/// captured is always due.
pub fn due(last: Option<Instant>, interval: Duration, now: Instant) -> bool {
    match last {
        None => true,
        Some(t) => now.duration_since(t) >= interval,
    }
}

/// Which tiles to capture on this tick, in `specs` order. Split out from the
/// thread body so the batching decision is testable.
pub fn due_requests(
    specs: &[TileSpec],
    last: &HashMap<String, Instant>,
    now: Instant,
) -> Vec<CaptureRequest> {
    let total = specs.len();
    specs
        .iter()
        .filter_map(|spec| {
            let interval = content_interval(spec, total)?;
            if !due(last.get(&spec.session_name).copied(), interval, now) {
                return None;
            }
            Some(CaptureRequest {
                session_name: spec.session_name.clone(),
                mode: spec.mode,
            })
        })
        .collect()
}

/// How long the poller waits for a control message before doing tmux work.
/// Short enough to stay responsive to `ToPoller`, and shorter than the
/// fastest interval it must honour.
const TICK: Duration = Duration::from_millis(25);

/// Run the poller loop until `ToPoller::Shutdown` or a disconnected channel.
/// Owns `tmux` for the life of the thread.
pub fn run(tmux: Tmux, rx: Receiver<ToPoller>, tx: Sender<ToMain>) {
    let mut specs: Vec<TileSpec> = Vec::new();
    let mut last_capture: HashMap<String, Instant> = HashMap::new();
    let mut last_meta: Option<Instant> = None;
    let mut paused = false;

    loop {
        // Drain every pending control message before doing tmux work, so a
        // Pause or Shutdown takes effect on this tick rather than after
        // another round of subprocesses.
        loop {
            match rx.recv_timeout(TICK) {
                Ok(ToPoller::Shutdown) | Err(RecvTimeoutError::Disconnected) => return,
                Ok(ToPoller::SetVisible(next)) => {
                    // Drop capture timestamps for sessions we no longer
                    // track, so the map cannot grow without bound.
                    last_capture.retain(|name, _| next.iter().any(|s| &s.session_name == name));
                    specs = next;
                }
                Ok(ToPoller::ResizeBatch(batch)) => {
                    if let Err(e) = tmux.resize_windows(&batch) {
                        let _ = tx.send(ToMain::TmuxError(e.to_string()));
                    }
                }
                Ok(ToPoller::OneShot(op)) => {
                    if let Err(e) = run_op(&tmux, op) {
                        let _ = tx.send(ToMain::TmuxError(e));
                    }
                }
                Ok(ToPoller::Pause) => {
                    paused = true;
                    let _ = tx.send(ToMain::PollerPaused);
                }
                Ok(ToPoller::Resume) => {
                    paused = false;
                    // Force an immediate refresh: panes may have changed size
                    // and content while we were detached.
                    last_capture.clear();
                    last_meta = None;
                }
                Err(RecvTimeoutError::Timeout) => break,
            }
        }

        if paused {
            continue;
        }

        let now = Instant::now();

        if due(last_meta, META_INTERVAL, now) {
            last_meta = Some(now);
            match tmux.poll_metadata() {
                Ok(meta) => {
                    let _ = tx.send(ToMain::Meta {
                        sessions: meta.sessions,
                        panes: meta.panes,
                    });
                }
                Err(e) => {
                    let _ = tx.send(ToMain::TmuxError(e.to_string()));
                }
            }
        }

        let requests = due_requests(&specs, &last_capture, now);
        if requests.is_empty() {
            continue;
        }
        match tmux.poll_content(&requests) {
            Ok(blocks) => {
                let updates: Vec<(String, Vec<u8>)> = blocks
                    .into_iter()
                    .map(|b| (b.session_name, b.content.into_bytes()))
                    .collect();
                for (name, _) in &updates {
                    last_capture.insert(name.clone(), now);
                }
                if !updates.is_empty() {
                    let _ = tx.send(ToMain::Content { updates });
                }
            }
            Err(e) => {
                // A vanished session aborts the rest of the chained batch
                // (see tmux/mod.rs). Mark every requested tile as attempted
                // so one bad session cannot spin the loop at full rate.
                for req in &requests {
                    last_capture.insert(req.session_name.clone(), now);
                }
                let _ = tx.send(ToMain::TmuxError(e.to_string()));
            }
        }
    }
}

/// Execute one one-shot tmux operation, flattening the error to a display
/// string for the error log.
fn run_op(tmux: &Tmux, op: TmuxOp) -> Result<(), String> {
    let result = match op {
        TmuxOp::Create(spec) => tmux.create_session(&spec),
        TmuxOp::KillSession(name) => tmux.kill_session(&name),
        TmuxOp::SendInterrupt(name) => tmux.send_keys_interrupt(&name),
        TmuxOp::SendEof(name) => tmux.send_keys_eof(&name),
        TmuxOp::Signal { session, signal } => tmux.signal_pane(&session, signal),
    };
    result.map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(visible: bool, kind: TileKind) -> TileSpec {
        TileSpec {
            session_name: "mr-k3f9-1".into(),
            visible,
            kind,
            mode: CaptureMode::Live,
        }
    }

    #[test]
    fn visible_running_polls_fast() {
        assert_eq!(
            content_interval(&spec(true, TileKind::Running), 4),
            Some(FAST_INTERVAL)
        );
    }

    #[test]
    fn visible_finished_but_still_changing_polls_fast() {
        let s = spec(
            true,
            TileKind::Finished {
                stale: STALE_POLLS - 1,
            },
        );
        assert_eq!(content_interval(&s, 4), Some(FAST_INTERVAL));
    }

    #[test]
    fn visible_finished_and_stale_backs_off() {
        let s = spec(true, TileKind::Finished { stale: STALE_POLLS });
        assert_eq!(content_interval(&s, 4), Some(IDLE_INTERVAL));
    }

    #[test]
    fn vanished_is_never_captured_even_when_visible() {
        assert_eq!(content_interval(&spec(true, TileKind::Vanished), 4), None);
    }

    #[test]
    fn offscreen_polls_slow_under_the_limit() {
        let s = spec(false, TileKind::Running);
        assert_eq!(
            content_interval(&s, OFFSCREEN_CAPTURE_LIMIT),
            Some(SLOW_INTERVAL)
        );
    }

    #[test]
    fn offscreen_stops_being_captured_past_the_limit() {
        let s = spec(false, TileKind::Running);
        assert_eq!(content_interval(&s, OFFSCREEN_CAPTURE_LIMIT + 1), None);
    }

    #[test]
    fn offscreen_rate_wins_over_finished_backoff() {
        // Visibility is checked before kind, so a finished offscreen tile
        // under the limit uses the offscreen rate, not the idle rate.
        let s = spec(false, TileKind::Finished { stale: STALE_POLLS });
        assert_eq!(content_interval(&s, 4), Some(SLOW_INTERVAL));
    }

    #[test]
    fn never_captured_is_always_due() {
        assert!(due(None, FAST_INTERVAL, Instant::now()));
    }

    #[test]
    fn recently_captured_is_not_due() {
        let now = Instant::now();
        assert!(!due(Some(now), FAST_INTERVAL, now));
    }

    #[test]
    fn due_once_the_interval_has_elapsed() {
        let now = Instant::now();
        assert!(due(Some(now - FAST_INTERVAL), FAST_INTERVAL, now));
    }

    #[test]
    fn due_requests_skips_vanished_and_not_yet_due() {
        let now = Instant::now();
        let specs = vec![
            TileSpec {
                session_name: "a".into(),
                visible: true,
                kind: TileKind::Running,
                mode: CaptureMode::Live,
            },
            TileSpec {
                session_name: "b".into(),
                visible: true,
                kind: TileKind::Vanished,
                mode: CaptureMode::Live,
            },
            TileSpec {
                session_name: "c".into(),
                visible: true,
                kind: TileKind::Running,
                mode: CaptureMode::Live,
            },
        ];
        let mut last = HashMap::new();
        last.insert("c".to_string(), now); // just captured, not due

        let reqs = due_requests(&specs, &last, now);
        let names: Vec<&str> = reqs.iter().map(|r| r.session_name.as_str()).collect();
        assert_eq!(names, vec!["a"]);
    }

    #[test]
    fn due_requests_preserves_spec_order() {
        let now = Instant::now();
        let specs: Vec<TileSpec> = ["a", "b", "c"]
            .iter()
            .map(|n| TileSpec {
                session_name: (*n).into(),
                visible: true,
                kind: TileKind::Running,
                mode: CaptureMode::Live,
            })
            .collect();
        let reqs = due_requests(&specs, &HashMap::new(), now);
        let names: Vec<&str> = reqs.iter().map(|r| r.session_name.as_str()).collect();
        assert_eq!(names, vec!["a", "b", "c"]);
    }
}
