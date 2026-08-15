//! The attach / detach round trip: hand the real terminal to
//! `tmux attach-session`, then take it back cleanly. The step order is
//! load-bearing and follows DESIGN.md section 8 exactly; each step cites the
//! research finding it guards.
//!
//! This module owns only the handoff. It does not decide which session to
//! attach to, and it does not mutate `App`; the caller applies the returned
//! `Outcome`.

use std::io::{self, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::time::{Duration, Instant};

use crossterm::execute;
use crossterm::terminal::{Clear, ClearType, disable_raw_mode, enable_raw_mode};

use crate::poll::{ToMain, ToPoller};
use crate::tmux::Tmux;
use crate::{enter_screen, leave_screen};

/// How long to wait for the poller's `Pause` ack before proceeding anyway.
/// Bounded so a wedged poller cannot hang the whole app on a keystroke.
const PAUSE_ACK_TIMEOUT: Duration = Duration::from_millis(500);

/// What happened, for the caller to fold into `App`.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Attached and detached normally.
    Detached,
    /// The session was already gone; the terminal was never torn down.
    Vanished,
    /// The attach itself failed. Carries a message for the error log.
    Failed(String),
}

/// Everything the round trip needs from the caller. Grouped into a struct
/// because the ordering constraints mean all of it is needed at once.
pub struct Handoff<'a> {
    pub tmux: &'a Tmux,
    pub session: &'a str,
    /// Every job session in the group, including `session`. All of them are
    /// un-pinned from their tile size before the handoff, because `C-n` /
    /// `C-p` can move the client to any of them without detaching first.
    pub sessions: &'a [String],
    pub to_poller: &'a Sender<ToPoller>,
    pub from_threads: &'a Receiver<ToMain>,
    /// Stop flag for the current input thread. Set to true to retire it.
    pub input_stop: &'a Arc<AtomicBool>,
}

/// Run the full handoff. Returns once the user has detached and the TUI owns
/// the terminal again.
///
/// The caller must, on return: respawn the input thread with a fresh stop
/// flag, send `ToPoller::Resume`, re-apply tile sizes for every tile, and
/// force a redraw. Those are the caller's because they need `App` and the
/// `Terminal`, which this module deliberately does not touch.
pub fn round_trip(h: Handoff<'_>) -> Outcome {
    // Step 1. Do not tear down a working terminal for a session that is not
    // there. `has-session` uses exact `=name` matching (gotcha C).
    match h.tmux.has_session(h.session) {
        Ok(true) => {}
        Ok(false) => return Outcome::Vanished,
        Err(e) => return Outcome::Failed(format!("cannot check session: {e}")),
    }

    // Step 2. No tmux subprocess should be in flight while the user owns the
    // terminal. Bounded wait: a missed ack is not worth hanging on.
    let _ = h.to_poller.send(ToPoller::Pause);
    wait_for_pause(h.from_threads, PAUSE_ACK_TIMEOUT);

    // Step 3. GOTCHA F: `tmux attach` shares our stdin. A crossterm reader
    // blocked in read(0, ..) steals keystrokes from it. The input thread's
    // 100ms poll makes this bounded; the caller joins it.
    h.input_stop.store(true, Ordering::Relaxed);
    std::thread::sleep(Duration::from_millis(120));

    // Steps 4 and 5. Item 5, fact 2: tmux unconditionally enters the alt
    // screen on attach and leaves it on detach. Still being in it when tmux
    // exits leaves us drawing into a buffer nobody sees. This uses the same
    // pair of helpers as `TerminalGuard`, so the set of modes the TUI turns
    // on has exactly one definition; when it did not, mouse reporting came
    // back on here and nothing on the exit path ever turned it off.
    let _ = disable_raw_mode();
    let mut stdout = io::stdout();
    leave_screen(&mut stdout);
    let _ = stdout.flush();

    // Step 6. GOTCHA E: `resize-window` left these windows at `manual`, which
    // would maroon the pane at tile size inside a full-size terminal. Every
    // session is un-pinned, not just the target: `C-n`/`C-p` switch the
    // client to another one without coming back through here first.
    if let Err(e) = h.tmux.unset_window_size(h.sessions) {
        // Not fatal: the attach still works, the pane is just the wrong size.
        let _ = e;
    }

    // Step 7. Blocking foreground child with inherited stdio. Print nothing
    // between here and the restore: tmux writes "[detached ...]" to the
    // primary screen on the way out, and step 9's Clear wipes it.
    let status = Tmux::command(&h.tmux.attach_argv(h.session)).status();

    // Steps 8 to 10. Always restore, whatever the attach did.
    let _ = enable_raw_mode();
    let _ = enter_screen(&mut stdout);
    let _ = execute!(stdout, Clear(ClearType::All));
    let _ = stdout.flush();

    match status {
        Ok(s) if s.success() => Outcome::Detached,
        Ok(s) => {
            // Distinguish "the session died while attached" from "the attach
            // failed". stderr was inherited, so the exit code is all we have.
            match h.tmux.has_session(h.session) {
                Ok(false) => Outcome::Vanished,
                _ => Outcome::Failed(format!("tmux attach exited with {s}")),
            }
        }
        Err(e) => Outcome::Failed(format!("cannot run tmux attach: {e}")),
    }
}

/// Drain messages until the poller acks the pause or the deadline passes.
/// Messages other than the ack are dropped: they describe a world the user is
/// about to change from inside tmux anyway, and the caller forces a fresh
/// poll on resume.
fn wait_for_pause(rx: &Receiver<ToMain>, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return;
        }
        match rx.recv_timeout(remaining) {
            Ok(ToMain::PollerPaused) => return,
            Ok(_) => continue,
            Err(_) => return,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    #[test]
    fn wait_for_pause_returns_on_the_ack() {
        let (tx, rx) = mpsc::channel();
        tx.send(ToMain::PollerPaused).unwrap();
        // Returning promptly rather than burning the full timeout is the
        // behaviour under test.
        let start = Instant::now();
        wait_for_pause(&rx, Duration::from_secs(5));
        assert!(start.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn wait_for_pause_skips_unrelated_messages() {
        let (tx, rx) = mpsc::channel();
        tx.send(ToMain::TmuxError("noise".into())).unwrap();
        tx.send(ToMain::PollerPaused).unwrap();
        let start = Instant::now();
        wait_for_pause(&rx, Duration::from_secs(5));
        assert!(start.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn wait_for_pause_gives_up_when_the_sender_is_gone() {
        let (tx, rx) = mpsc::channel::<ToMain>();
        drop(tx);
        wait_for_pause(&rx, Duration::from_secs(5));
    }

    #[test]
    fn wait_for_pause_respects_its_deadline() {
        let (_tx, rx) = mpsc::channel::<ToMain>();
        let start = Instant::now();
        wait_for_pause(&rx, Duration::from_millis(50));
        assert!(start.elapsed() >= Duration::from_millis(50));
        assert!(start.elapsed() < Duration::from_secs(2));
    }
}
