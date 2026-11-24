//! A tiny transport abstraction so the bridge and port forwarding logic do not
//! depend on vsock.
//!
//! In production the host [`Connector`] dials guest vsock ports through
//! `VZVirtioSocketDevice` (macOS) and the guest [`Acceptor`] is an AF_VSOCK
//! listener (Linux). Tests use [`TcpConnector`] and `TcpListener`, mapping
//! each vsock port to a loopback TCP address.

use std::collections::HashMap;
use std::future::Future;
use std::io;
use std::net::SocketAddr;
use tokio::io::{AsyncRead, AsyncWrite};

/// A bidirectional byte stream.
pub trait Stream: AsyncRead + AsyncWrite + Unpin + Send + 'static {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send + 'static> Stream for T {}

/// Opens a stream to a guest port.
pub trait Connector: Send + Sync + 'static {
    type Stream: Stream;
    fn connect(&self, port: u32) -> impl Future<Output = io::Result<Self::Stream>> + Send;
}

/// Accepts streams on one port.
pub trait Acceptor: Send + Sync + 'static {
    type Stream: Stream;
    fn accept(&self) -> impl Future<Output = io::Result<Self::Stream>> + Send;
}

impl Acceptor for tokio::net::TcpListener {
    type Stream = tokio::net::TcpStream;
    async fn accept(&self) -> io::Result<Self::Stream> {
        tokio::net::TcpListener::accept(self).await.map(|(s, _)| s)
    }
}

impl Acceptor for tokio::net::UnixListener {
    type Stream = tokio::net::UnixStream;
    async fn accept(&self) -> io::Result<Self::Stream> {
        tokio::net::UnixListener::accept(self).await.map(|(s, _)| s)
    }
}

/// Test and development connector: "vsock port" -> TCP address.
#[derive(Debug, Clone, Default)]
pub struct TcpConnector {
    pub ports: HashMap<u32, SocketAddr>,
}

impl Connector for TcpConnector {
    type Stream = tokio::net::TcpStream;
    async fn connect(&self, port: u32) -> io::Result<Self::Stream> {
        let addr = self.ports.get(&port).copied().ok_or_else(|| {
            io::Error::new(io::ErrorKind::ConnectionRefused, format!("no route for port {port}"))
        })?;
        tokio::net::TcpStream::connect(addr).await
    }
}

/// Copy bytes both ways until both directions hit EOF. Returns bytes copied
/// (a to b, b to a).
pub async fn pump<A: Stream, B: Stream>(mut a: A, mut b: B) -> io::Result<(u64, u64)> {
    tokio::io::copy_bidirectional(&mut a, &mut b).await
}
