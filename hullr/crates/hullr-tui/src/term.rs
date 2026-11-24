//! Interactive terminal pump for exec/attach streams.
//!
//! [`pump`] connects the local terminal to an [`AttachStream`]: stdin bytes
//! become `Stdin` frames, `Stdout`/`Stderr` frames are written to the local
//! stdout/stderr, and the terminal size is sent on start and on SIGWINCH.
//! The CLI reuses it for `hullr run -it`, `hullr exec -it` and `hullr attach`.

use anyhow::{anyhow, Result};
use hullr_core::api::AttachStream;
use hullr_core::stream::{read_frame, write_frame, Frame};
use std::io::Write;
use tokio::sync::mpsc;

/// Returned by [`pump`] when the user pressed the detach sequence
/// (ctrl-p then ctrl-q) and left the process running. It is `i32::MIN`, which
/// no real exit code or negative signal number can equal.
pub const DETACHED: i32 = i32::MIN;

const CTRL_P: u8 = 0x10;
const CTRL_Q: u8 = 0x11;

/// Restores the terminal's cooked mode when dropped, on every path including
/// panics (the destructor runs during unwinding).
pub struct RawGuard {
    active: bool,
}

impl RawGuard {
    /// Enable raw mode. A no-op guard is returned when stdin is not a
    /// terminal, so piping input still works.
    pub fn enable() -> Self {
        let active = crossterm::tty::IsTty::is_tty(&std::io::stdin())
            && crossterm::terminal::enable_raw_mode().is_ok();
        Self { active }
    }

    pub fn is_active(&self) -> bool {
        self.active
    }
}

impl Drop for RawGuard {
    fn drop(&mut self) {
        if self.active {
            let _ = crossterm::terminal::disable_raw_mode();
        }
    }
}

/// Detects Docker's detach sequence (ctrl-p ctrl-q) in a byte stream that may
/// be split across reads. Bytes that turn out not to be part of the sequence
/// are passed through unchanged.
#[derive(Debug, Default)]
pub struct DetachDetector {
    held_ctrl_p: bool,
}

impl DetachDetector {
    /// Returns the bytes to forward and whether the detach sequence completed.
    /// On detach, bytes after the sequence are discarded.
    pub fn feed(&mut self, input: &[u8]) -> (Vec<u8>, bool) {
        let mut out = Vec::with_capacity(input.len() + 1);
        for &b in input {
            if self.held_ctrl_p {
                self.held_ctrl_p = false;
                if b == CTRL_Q {
                    return (out, true);
                }
                out.push(CTRL_P);
            }
            if b == CTRL_P {
                self.held_ctrl_p = true;
            } else {
                out.push(b);
            }
        }
        (out, false)
    }

    /// A held ctrl-p that never got a partner (stdin closed).
    pub fn flush(&mut self) -> Vec<u8> {
        if std::mem::take(&mut self.held_ctrl_p) {
            vec![CTRL_P]
        } else {
            Vec::new()
        }
    }
}

/// Connect the local terminal to `stream` until the remote process exits.
/// Equivalent to [`pump_with`] with `stdin` forwarding enabled.
pub async fn pump(stream: AttachStream, tty: bool) -> Result<i32> {
    pump_with(stream, tty, true).await
}

/// Connect the local terminal to `stream` until the remote process exits.
///
/// With `tty` set (and `stdin` forwarding on) the terminal is put in raw mode
/// (restored on every path), the detach sequence ctrl-p ctrl-q is honoured,
/// and the terminal size is forwarded on start and on every SIGWINCH.
///
/// With `stdin == false` the local stdin is never read: `Frame::CloseStdin` is
/// sent immediately, raw mode is not entered and there is no detach handling
/// (like `docker run` without `-i`). The tty size is still forwarded.
///
/// Returns the process exit code, or [`DETACHED`] if the user detached. If the
/// stream ends without an `Exit` frame an error is returned. The stdin reader
/// is a detached thread, so a read blocked on the terminal never delays the
/// return (it stops within 50 ms anyway).
pub async fn pump_with(stream: AttachStream, tty: bool, stdin: bool) -> Result<i32> {
    let _raw = if tty && stdin { Some(RawGuard::enable()) } else { None };
    let raw_active = _raw.as_ref().is_some_and(|g| g.is_active());

    let (stdin_rx, stop) = if stdin {
        spawn_stdin_reader()
    } else {
        // A closed channel makes the pump send CloseStdin straight away.
        let (_tx, rx) = mpsc::unbounded_channel();
        (rx, std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true)))
    };
    let (resize_tx, resize_rx) = mpsc::unbounded_channel::<(u16, u16)>();
    if tty {
        if let Ok((c, r)) = crossterm::terminal::size() {
            let _ = resize_tx.send((c, r));
        }
        spawn_resize_watcher(resize_tx);
    }
    let io = PumpIo {
        stdin: stdin_rx,
        resize: resize_rx,
        out: Box::new(std::io::stdout()),
        err: Box::new(std::io::stderr()),
        detach_keys: tty && stdin,
        crlf_errors: raw_active,
    };
    let res = pump_io(stream, io).await;
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    res
}

/// The terminal-facing side of the pump, injectable for tests.
pub struct PumpIo {
    pub stdin: mpsc::UnboundedReceiver<Vec<u8>>,
    pub resize: mpsc::UnboundedReceiver<(u16, u16)>,
    pub out: Box<dyn Write + Send>,
    pub err: Box<dyn Write + Send>,
    pub detach_keys: bool,
    pub crlf_errors: bool,
}

/// The pump loop, separated from the real terminal so it can be tested.
pub async fn pump_io(stream: AttachStream, mut io: PumpIo) -> Result<i32> {
    let (mut rd, mut wr) = tokio::io::split(stream);
    let (ftx, mut frx) = mpsc::unbounded_channel::<Frame>();
    // A dedicated writer keeps a full pipe in one direction from stalling the
    // other direction.
    let writer = tokio::spawn(async move {
        while let Some(f) = frx.recv().await {
            if write_frame(&mut wr, &f).await.is_err() {
                break;
            }
        }
    });

    let mut detector = DetachDetector::default();
    let mut stdin_open = true;
    let mut resize_open = true;
    let mut server_error: Option<String> = None;

    let result: Result<i32> = loop {
        tokio::select! {
            frame = read_frame(&mut rd) => match frame {
                Ok(Some(Frame::Stdout(d))) => {
                    let _ = io.out.write_all(&d);
                    let _ = io.out.flush();
                }
                Ok(Some(Frame::Stderr(d))) => {
                    let _ = io.err.write_all(&d);
                    let _ = io.err.flush();
                }
                Ok(Some(Frame::Error(e))) => server_error = Some(e),
                Ok(Some(Frame::Exit(code))) => break Ok(code),
                Ok(Some(_)) => {}
                Ok(None) => break Err(anyhow!("connection lost before the process exited")),
                Err(e) => break Err(anyhow!("stream error: {e}")),
            },
            data = io.stdin.recv(), if stdin_open => match data {
                Some(bytes) => {
                    let (fwd, detached) = if io.detach_keys {
                        detector.feed(&bytes)
                    } else {
                        (bytes, false)
                    };
                    if !fwd.is_empty() {
                        let _ = ftx.send(Frame::Stdin(fwd));
                    }
                    if detached {
                        break Ok(DETACHED);
                    }
                }
                None => {
                    stdin_open = false;
                    let tail = detector.flush();
                    if !tail.is_empty() {
                        let _ = ftx.send(Frame::Stdin(tail));
                    }
                    let _ = ftx.send(Frame::CloseStdin);
                }
            },
            size = io.resize.recv(), if resize_open => match size {
                Some((cols, rows)) => { let _ = ftx.send(Frame::Resize { cols, rows }); }
                None => resize_open = false,
            },
        }
    };

    drop(ftx);
    if result.is_ok() {
        // Let queued frames (such as the last stdin bytes) flush.
        let _ = tokio::time::timeout(std::time::Duration::from_millis(200), writer).await;
    } else {
        writer.abort();
    }
    if let Some(e) = server_error {
        let nl = if io.crlf_errors { "\r\n" } else { "\n" };
        let _ = write!(io.err, "{nl}error: {e}{nl}");
        let _ = io.err.flush();
    }
    result
}

/// Start a thread that forwards stdin bytes. It polls with a timeout so it
/// can be stopped, and never steals input after the pump returns.
fn spawn_stdin_reader() -> (mpsc::UnboundedReceiver<Vec<u8>>, std::sync::Arc<std::sync::atomic::AtomicBool>) {
    use std::sync::atomic::{AtomicBool, Ordering};
    let (tx, rx) = mpsc::unbounded_channel();
    let stop = std::sync::Arc::new(AtomicBool::new(false));
    let stop2 = stop.clone();
    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        while !stop2.load(Ordering::Relaxed) {
            match read_stdin_timeout(&mut buf, 50) {
                ReadOutcome::Data(n) => {
                    if tx.send(buf[..n].to_vec()).is_err() {
                        break;
                    }
                }
                ReadOutcome::Timeout => {}
                ReadOutcome::Eof => break,
            }
        }
        // Dropping tx signals EOF to the pump.
    });
    (rx, stop)
}

enum ReadOutcome {
    Data(usize),
    Timeout,
    Eof,
}

#[cfg(unix)]
fn read_stdin_timeout(buf: &mut [u8], timeout_ms: i32) -> ReadOutcome {
    let mut fds = libc::pollfd { fd: 0, events: libc::POLLIN, revents: 0 };
    // SAFETY: `fds` is a valid pollfd and we pass a count of one.
    let r = unsafe { libc::poll(&mut fds, 1, timeout_ms) };
    if r < 0 {
        let e = std::io::Error::last_os_error();
        return if e.kind() == std::io::ErrorKind::Interrupted {
            ReadOutcome::Timeout
        } else {
            ReadOutcome::Eof
        };
    }
    if r == 0 {
        return ReadOutcome::Timeout;
    }
    // SAFETY: `buf` is valid for `buf.len()` bytes.
    let n = unsafe { libc::read(0, buf.as_mut_ptr().cast(), buf.len()) };
    match n {
        n if n > 0 => ReadOutcome::Data(n as usize),
        0 => ReadOutcome::Eof,
        _ => {
            let e = std::io::Error::last_os_error();
            if matches!(e.kind(), std::io::ErrorKind::Interrupted | std::io::ErrorKind::WouldBlock) {
                ReadOutcome::Timeout
            } else {
                ReadOutcome::Eof
            }
        }
    }
}

#[cfg(not(unix))]
fn read_stdin_timeout(buf: &mut [u8], _timeout_ms: i32) -> ReadOutcome {
    use std::io::Read;
    match std::io::stdin().read(buf) {
        Ok(0) | Err(_) => ReadOutcome::Eof,
        Ok(n) => ReadOutcome::Data(n),
    }
}

#[cfg(unix)]
fn spawn_resize_watcher(tx: mpsc::UnboundedSender<(u16, u16)>) {
    use tokio::signal::unix::{signal, SignalKind};
    let Ok(mut sig) = signal(SignalKind::window_change()) else { return };
    tokio::spawn(async move {
        while sig.recv().await.is_some() {
            if let Ok((c, r)) = crossterm::terminal::size() {
                if tx.send((c, r)).is_err() {
                    break;
                }
            }
        }
    });
}

#[cfg(not(unix))]
fn spawn_resize_watcher(_tx: mpsc::UnboundedSender<(u16, u16)>) {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Default)]
    struct Sink(Arc<Mutex<Vec<u8>>>);
    impl Write for Sink {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn detach_sequence() {
        let mut d = DetachDetector::default();
        assert_eq!(d.feed(b"ab"), (b"ab".to_vec(), false));
        assert_eq!(d.feed(&[CTRL_P]), (vec![], false));
        assert_eq!(d.feed(&[CTRL_Q, b'x']), (vec![], true));
    }

    #[test]
    fn detach_passthrough_when_not_followed() {
        let mut d = DetachDetector::default();
        assert_eq!(d.feed(&[CTRL_P, b'a']), (vec![CTRL_P, b'a'], false));
        assert_eq!(d.feed(&[CTRL_P]), (vec![], false));
        assert_eq!(d.flush(), vec![CTRL_P]);
        assert_eq!(d.flush(), Vec::<u8>::new());
        // ctrl-p ctrl-p ctrl-q: first is forwarded, second pairs with ctrl-q.
        assert_eq!(d.feed(&[CTRL_P, CTRL_P, CTRL_Q]), (vec![CTRL_P], true));
    }

    fn io(
        stdin: mpsc::UnboundedReceiver<Vec<u8>>,
        resize: mpsc::UnboundedReceiver<(u16, u16)>,
        out: Sink,
        err: Sink,
        detach: bool,
    ) -> PumpIo {
        PumpIo {
            stdin,
            resize,
            out: Box::new(out),
            err: Box::new(err),
            detach_keys: detach,
            crlf_errors: false,
        }
    }

    #[tokio::test]
    async fn pumps_output_and_returns_exit_code() {
        let (client, mut server) = tokio::io::duplex(4096);
        let (_stx, srx) = mpsc::unbounded_channel();
        let (_rtx, rrx) = mpsc::unbounded_channel();
        let (out, err) = (Sink::default(), Sink::default());
        tokio::spawn(async move {
            write_frame(&mut server, &Frame::Stdout(b"hello".to_vec())).await.unwrap();
            write_frame(&mut server, &Frame::Stderr(b"warn".to_vec())).await.unwrap();
            write_frame(&mut server, &Frame::Exit(3)).await.unwrap();
        });
        let code = pump_io(Box::new(client), io(srx, rrx, out.clone(), err.clone(), false))
            .await
            .unwrap();
        assert_eq!(code, 3);
        assert_eq!(&*out.0.lock().unwrap(), b"hello");
        assert_eq!(&*err.0.lock().unwrap(), b"warn");
    }

    #[tokio::test]
    async fn forwards_stdin_resize_and_eof() {
        let (client, mut server) = tokio::io::duplex(4096);
        let (stx, srx) = mpsc::unbounded_channel();
        let (rtx, rrx) = mpsc::unbounded_channel();
        stx.send(b"ls\n".to_vec()).unwrap();
        rtx.send((100, 40)).unwrap();
        drop(stx);
        let server_task = tokio::spawn(async move {
            let mut seen = Vec::new();
            while seen.len() < 3 {
                match read_frame(&mut server).await.unwrap() {
                    Some(f) => seen.push(f),
                    None => break,
                }
            }
            write_frame(&mut server, &Frame::Exit(0)).await.unwrap();
            seen
        });
        let code = pump_io(Box::new(client), io(srx, rrx, Sink::default(), Sink::default(), true))
            .await
            .unwrap();
        assert_eq!(code, 0);
        let seen = server_task.await.unwrap();
        assert!(seen.contains(&Frame::Stdin(b"ls\n".to_vec())));
        assert!(seen.contains(&Frame::Resize { cols: 100, rows: 40 }));
        assert!(seen.contains(&Frame::CloseStdin));
    }

    #[tokio::test]
    async fn detach_returns_sentinel() {
        let (client, _server) = tokio::io::duplex(4096);
        let (stx, srx) = mpsc::unbounded_channel();
        let (_rtx, rrx) = mpsc::unbounded_channel();
        stx.send(vec![CTRL_P, CTRL_Q]).unwrap();
        let code = pump_io(Box::new(client), io(srx, rrx, Sink::default(), Sink::default(), true))
            .await
            .unwrap();
        assert_eq!(code, DETACHED);
    }

    #[tokio::test]
    async fn no_stdin_sends_close_immediately() {
        let (client, mut server) = tokio::io::duplex(4096);
        let t = tokio::spawn(async move {
            let f = read_frame(&mut server).await.unwrap().unwrap();
            write_frame(&mut server, &Frame::Exit(0)).await.unwrap();
            f
        });
        // Not a tty, so no size frame comes first; stdin is never read.
        let code = pump_with(Box::new(client), false, false).await.unwrap();
        assert_eq!(code, 0);
        assert_eq!(t.await.unwrap(), Frame::CloseStdin);
    }

    #[tokio::test]
    async fn closed_stream_is_an_error() {
        let (client, server) = tokio::io::duplex(64);
        drop(server);
        let (_stx, srx) = mpsc::unbounded_channel();
        let (_rtx, rrx) = mpsc::unbounded_channel();
        let r = pump_io(Box::new(client), io(srx, rrx, Sink::default(), Sink::default(), false)).await;
        assert!(r.is_err());
    }
}
