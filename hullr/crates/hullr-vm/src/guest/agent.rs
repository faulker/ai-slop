//! The guest agent (`hullr __guest-agent`), started by the VM's `/init`.
//!
//! * vsock 1025 (control): each host connection gets the full published port
//!   set as `add` lines, then diffs as the set changes. The host may send
//!   `{"op":"shutdown"}` to power the guest off cleanly.
//! * vsock 1026 (forward): each connection starts with `tcp <port>\n` and is
//!   then piped to `127.0.0.1:<port>` (falling back to `[::1]:<port>`).
//!
//! The serving functions are generic over [`Acceptor`] so they can be tested
//! over TCP.

use super::ports::{diff, read_published, Published};
use crate::protocol::{read_forward_header, ControlMsg, Op, Proto};
use crate::transport::{pump, Acceptor, Stream};
use std::io;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::sync::watch;

/// Maps a forwarded port to the guest addresses to try, in order.
pub type Dialer = Arc<dyn Fn(u16) -> Vec<SocketAddr> + Send + Sync>;

/// Called when the host asks for a shutdown.
pub type ShutdownFn = Arc<dyn Fn() + Send + Sync>;

/// The production dialer: guest loopback, IPv4 then IPv6.
pub fn guest_loopback() -> Dialer {
    Arc::new(|port| vec![SocketAddr::from(([127, 0, 0, 1], port)), SocketAddr::from((std::net::Ipv6Addr::LOCALHOST, port))])
}

/// Serve the forward port: accept, read the header, connect, pump.
pub async fn serve_forward<A: Acceptor>(acceptor: A, dial: Dialer) -> io::Result<()> {
    loop {
        let stream = acceptor.accept().await?;
        let dial = dial.clone();
        tokio::spawn(async move {
            if let Err(e) = forward_one(stream, dial).await {
                eprintln!("hullr guest-agent: forward failed: {e}");
            }
        });
    }
}

async fn forward_one<S: Stream>(mut stream: S, dial: Dialer) -> io::Result<()> {
    let (proto, port) = read_forward_header(&mut stream).await?;
    if proto != Proto::Tcp {
        return Err(io::Error::new(io::ErrorKind::Unsupported, "udp forwarding is not supported"));
    }
    let mut last = io::Error::new(io::ErrorKind::ConnectionRefused, "no address to dial");
    for addr in dial(port) {
        match TcpStream::connect(addr).await {
            Ok(target) => {
                let _ = target.set_nodelay(true);
                pump(stream, target).await?;
                return Ok(());
            }
            Err(e) => last = e,
        }
    }
    Err(last)
}

/// Serve the control port: every connection receives the published set and
/// its changes; a `shutdown` line calls `on_shutdown`.
pub async fn serve_control<A: Acceptor>(acceptor: A, published: watch::Receiver<Published>, on_shutdown: ShutdownFn) -> io::Result<()> {
    loop {
        let stream = acceptor.accept().await?;
        tokio::spawn(control_conn(stream, published.clone(), on_shutdown.clone()));
    }
}

async fn control_conn<S: Stream>(stream: S, mut published: watch::Receiver<Published>, on_shutdown: ShutdownFn) {
    let (rd, mut wr) = tokio::io::split(stream);
    let mut lines = BufReader::new(rd).lines();
    let mut sent = Published::new();
    loop {
        let current = published.borrow_and_update().clone();
        for msg in diff(&sent, &current) {
            if wr.write_all(msg.to_line().as_bytes()).await.is_err() {
                return;
            }
        }
        sent = current;
        tokio::select! {
            changed = published.changed() => if changed.is_err() { return },
            line = lines.next_line() => match line {
                Ok(Some(line)) => {
                    if let Ok(ControlMsg { op: Op::Shutdown, .. }) = ControlMsg::parse(&line) {
                        on_shutdown();
                    }
                }
                _ => return,
            },
        }
    }
}

/// Re-read the published port set every `interval`, notifying on change.
pub async fn poll_published(tx: watch::Sender<Published>, interval: Duration) {
    loop {
        let now = tokio::task::spawn_blocking(read_published).await.unwrap_or_default();
        tx.send_if_modified(|cur| {
            if *cur != now {
                *cur = now;
                true
            } else {
                false
            }
        });
        tokio::time::sleep(interval).await;
    }
}

/// Run the agent until an error. Linux guests only.
pub async fn run_guest_agent() -> hullr_core::Result<()> {
    #[cfg(target_os = "linux")]
    {
        use super::vsock::VsockListener;
        use crate::protocol::{CONTROL_PORT, FORWARD_PORT};
        use hullr_core::Context;

        let control = VsockListener::bind(CONTROL_PORT).context("vsock listen on control port")?;
        let forward = VsockListener::bind(FORWARD_PORT).context("vsock listen on forward port")?;
        let (tx, rx) = watch::channel(Published::new());
        tokio::spawn(poll_published(tx, Duration::from_secs(1)));
        tokio::spawn(async move {
            if let Err(e) = serve_forward(forward, guest_loopback()).await {
                eprintln!("hullr guest-agent: forward listener failed: {e}");
            }
        });
        let shutdown: ShutdownFn = Arc::new(|| {
            std::thread::spawn(power_off);
        });
        serve_control(control, rx, shutdown).await.context("control listener")?;
        Ok(())
    }
    #[cfg(not(target_os = "linux"))]
    {
        Err(hullr_core::Error::Unsupported("the guest agent only runs inside the Linux VM".into()))
    }
}

/// Flush and power off. PID 1 in the VM is busybox init, which on SIGUSR2
/// runs its shutdown actions, terminates every process, syncs and powers
/// off. If that does not happen within 15s, power off directly.
#[cfg(target_os = "linux")]
fn power_off() {
    eprintln!("hullr guest-agent: shutdown requested");
    // SAFETY: plain syscalls without memory arguments.
    unsafe {
        libc::sync();
        if libc::kill(1, libc::SIGUSR2) == 0 {
            std::thread::sleep(Duration::from_secs(15));
        }
        libc::sync();
        libc::reboot(libc::RB_POWER_OFF);
    }
}
