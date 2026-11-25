//! Host side of port forwarding.
//!
//! The guest daemon publishes container ports inside the VM. The guest agent
//! announces them over the control channel (guest vsock port 1025); for each
//! `add` the host listens on the same TCP port and forwards every accepted
//! connection over a fresh vsock stream to guest port 1026, prefixed with the
//! `tcp <port>\n` header. UDP is not forwarded yet.

use crate::protocol::{write_forward_header, ControlMsg, Op, Proto, CONTROL_PORT, FORWARD_PORT};
use crate::transport::{pump, Connector};
use std::collections::HashMap;
use std::io;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

/// Active host listeners, keyed by port.
pub struct Forwarder<C: Connector> {
    connector: Arc<C>,
    active: Mutex<HashMap<u16, (String, JoinHandle<()>)>>,
}

impl<C: Connector> Forwarder<C> {
    pub fn new(connector: Arc<C>) -> Self {
        Self { connector, active: Mutex::new(HashMap::new()) }
    }

    /// Listen on `host_ip:port` and forward to the same port in the guest.
    /// Re-adding with the same address is a no-op; a new address replaces the
    /// old listener.
    pub async fn add(&self, host_ip: &str, port: u16) -> io::Result<()> {
        let ip = if host_ip.is_empty() { "0.0.0.0" } else { host_ip };
        {
            let mut active = self.active.lock().unwrap();
            match active.get(&port) {
                Some((cur, _)) if cur == ip => return Ok(()),
                Some(_) => {
                    if let Some((_, h)) = active.remove(&port) {
                        h.abort();
                    }
                }
                None => {}
            }
        }
        let listener = TcpListener::bind((ip, port)).await?;
        let connector = self.connector.clone();
        let task = tokio::spawn(async move {
            loop {
                let Ok((client, _)) = listener.accept().await else {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    continue;
                };
                let _ = client.set_nodelay(true);
                let connector = connector.clone();
                tokio::spawn(async move {
                    let mut guest = match connector.connect(FORWARD_PORT).await {
                        Ok(g) => g,
                        Err(e) => {
                            eprintln!("hullr vm: forward of port {port} failed: {e}");
                            return;
                        }
                    };
                    if write_forward_header(&mut guest, Proto::Tcp, port).await.is_ok() {
                        let _ = pump(client, guest).await;
                    }
                });
            }
        });
        if let Some((_, old)) = self.active.lock().unwrap().insert(port, (ip.to_string(), task)) {
            old.abort();
        }
        Ok(())
    }

    /// Stop listening on `port`.
    pub fn remove(&self, port: u16) {
        if let Some((_, h)) = self.active.lock().unwrap().remove(&port) {
            h.abort();
        }
    }

    /// Stop all listeners (the control channel dropped; the guest re-announces
    /// everything on reconnect).
    pub fn clear(&self) {
        for (_, (_, h)) in self.active.lock().unwrap().drain() {
            h.abort();
        }
    }

    /// Currently forwarded `(host_ip, port)` pairs, sorted by port.
    pub fn active(&self) -> Vec<(String, u16)> {
        let mut v: Vec<_> = self.active.lock().unwrap().iter().map(|(p, (ip, _))| (ip.clone(), *p)).collect();
        v.sort_by_key(|(_, p)| *p);
        v
    }

    /// Apply one control message from the guest.
    pub async fn apply(&self, msg: &ControlMsg) {
        if msg.proto != Proto::Tcp {
            eprintln!("hullr vm: ignoring {} port {}: only tcp is forwarded", msg.proto.as_str(), msg.port);
            return;
        }
        match msg.op {
            Op::Add => {
                if let Err(e) = self.add(&msg.host_ip, msg.port).await {
                    eprintln!("hullr vm: cannot listen on {}:{}: {e}", msg.host_ip, msg.port);
                }
            }
            Op::Remove => self.remove(msg.port),
            Op::Shutdown => {}
        }
    }
}

impl<C: Connector> Drop for Forwarder<C> {
    fn drop(&mut self) {
        self.clear();
    }
}

/// Keep a control connection to the guest agent open, applying its messages
/// to `fwd` and sending `outbound` messages (such as shutdown) to it.
/// Reconnects every second while the guest is unreachable. Never returns.
pub async fn run_control<C: Connector>(connector: Arc<C>, fwd: Arc<Forwarder<C>>, mut outbound: mpsc::Receiver<ControlMsg>) {
    loop {
        if let Ok(stream) = connector.connect(CONTROL_PORT).await {
            control_session(stream, &fwd, &mut outbound).await;
            fwd.clear();
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

async fn control_session<C: Connector>(stream: C::Stream, fwd: &Forwarder<C>, outbound: &mut mpsc::Receiver<ControlMsg>) {
    let (rd, mut wr) = tokio::io::split(stream);
    let mut lines = BufReader::new(rd).lines();
    let mut outbound_open = true;
    loop {
        tokio::select! {
            line = lines.next_line() => match line {
                Ok(Some(line)) if line.trim().is_empty() => {}
                Ok(Some(line)) => match ControlMsg::parse(&line) {
                    Ok(msg) => fwd.apply(&msg).await,
                    Err(e) => eprintln!("hullr vm: bad control line {line:?}: {e}"),
                },
                _ => return,
            },
            msg = outbound.recv(), if outbound_open => match msg {
                Some(msg) => {
                    if wr.write_all(msg.to_line().as_bytes()).await.is_err() {
                        return;
                    }
                }
                None => outbound_open = false,
            },
        }
    }
}
