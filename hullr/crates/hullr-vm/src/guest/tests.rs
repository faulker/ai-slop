//! Loopback test of the whole port forwarding path, with TCP standing in for
//! vsock: guest agent (control + forward) <-> host Forwarder + control loop.

use super::agent::{serve_control, serve_forward, Dialer, ShutdownFn};
use super::ports::Published;
use crate::forward::{run_control, Forwarder};
use crate::protocol::{ControlMsg, CONTROL_PORT, FORWARD_PORT};
use crate::transport::TcpConnector;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, watch};

async fn wait_for(mut cond: impl FnMut() -> bool) {
    for _ in 0..100 {
        if cond() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("condition not reached");
}

async fn echo_server() -> SocketAddr {
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let (mut s, _) = l.accept().await.unwrap();
            tokio::spawn(async move {
                let (mut r, mut w) = s.split();
                let _ = tokio::io::copy(&mut r, &mut w).await;
            });
        }
    });
    addr
}

async fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0").await.unwrap().local_addr().unwrap().port()
}

#[tokio::test]
async fn publish_forward_unpublish() {
    // Guest side.
    let app = echo_server().await;
    let host_port = free_port().await;
    let control = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let forward = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut connector = TcpConnector::default();
    connector.ports.insert(CONTROL_PORT, control.local_addr().unwrap());
    connector.ports.insert(FORWARD_PORT, forward.local_addr().unwrap());

    // The guest "localhost:<host_port>" is really the echo server, so the
    // host listener and the guest app do not collide on one machine.
    let dial: Dialer = Arc::new(move |port| if port == host_port { vec![app] } else { vec![] });
    let (tx, rx) = watch::channel(Published::new());
    let shut = Arc::new(AtomicBool::new(false));
    let shut2 = shut.clone();
    let on_shutdown: ShutdownFn = Arc::new(move || shut2.store(true, Ordering::SeqCst));
    tokio::spawn(serve_control(control, rx, on_shutdown));
    tokio::spawn(serve_forward(forward, dial));

    // Host side.
    let connector = Arc::new(connector);
    let fwd = Arc::new(Forwarder::new(connector.clone()));
    let (out_tx, out_rx) = mpsc::channel(4);
    tokio::spawn(run_control(connector, fwd.clone(), out_rx));

    // Publish: the host starts listening on the same port.
    tx.send(Published::from([(host_port, "127.0.0.1".to_string())])).unwrap();
    wait_for(|| fwd.active() == vec![("127.0.0.1".to_string(), host_port)]).await;

    let mut c = TcpStream::connect(("127.0.0.1", host_port)).await.unwrap();
    c.write_all(b"hello through the vm").await.unwrap();
    let mut buf = vec![0; 20];
    c.read_exact(&mut buf).await.unwrap();
    assert_eq!(&buf, b"hello through the vm");
    // Half close propagates: the echo side sees EOF and closes too.
    c.shutdown().await.unwrap();
    let mut rest = Vec::new();
    assert_eq!(c.read_to_end(&mut rest).await.unwrap(), 0);

    // Host to guest control message.
    out_tx.send(ControlMsg::shutdown()).await.unwrap();
    wait_for(|| shut.load(Ordering::SeqCst)).await;

    // Unpublish: the host listener goes away.
    tx.send(Published::new()).unwrap();
    wait_for(|| fwd.active().is_empty()).await;
    let mut gone = false;
    for _ in 0..50 {
        if TcpStream::connect(("127.0.0.1", host_port)).await.is_err() {
            gone = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(gone, "host port still accepting after remove");
}

#[tokio::test]
async fn late_control_connection_gets_full_state() {
    let control = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = control.local_addr().unwrap();
    let (tx, rx) = watch::channel(Published::from([(8080, "0.0.0.0".to_string())]));
    tokio::spawn(serve_control(control, rx, Arc::new(|| {})));

    let s = TcpStream::connect(addr).await.unwrap();
    let mut lines = tokio::io::BufReader::new(s);
    let mut line = String::new();
    use tokio::io::AsyncBufReadExt;
    lines.read_line(&mut line).await.unwrap();
    assert_eq!(ControlMsg::parse(&line).unwrap(), ControlMsg::add("0.0.0.0", 8080));

    tx.send(Published::from([(9090, "127.0.0.1".to_string())])).unwrap();
    line.clear();
    lines.read_line(&mut line).await.unwrap();
    assert_eq!(ControlMsg::parse(&line).unwrap(), ControlMsg::remove("0.0.0.0", 8080));
    line.clear();
    lines.read_line(&mut line).await.unwrap();
    assert_eq!(ControlMsg::parse(&line).unwrap(), ControlMsg::add("127.0.0.1", 9090));
}

#[tokio::test]
async fn udp_header_is_rejected() {
    let forward = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = forward.local_addr().unwrap();
    tokio::spawn(serve_forward(forward, Arc::new(|_| vec![])));
    let mut s = TcpStream::connect(addr).await.unwrap();
    s.write_all(b"udp 53\n").await.unwrap();
    let mut buf = Vec::new();
    assert_eq!(s.read_to_end(&mut buf).await.unwrap(), 0);
}
