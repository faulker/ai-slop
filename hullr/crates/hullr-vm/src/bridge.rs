//! Host side API bridge: the unix socket `Paths::socket()` on the Mac, each
//! connection piped to the daemon on guest vsock port 1024. The CLI, TUI and
//! GUI connect to it exactly as they would to the native daemon on Linux.

use crate::transport::{pump, Connector};
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::Arc;
use tokio::net::{UnixListener, UnixStream};

/// Bind the unix socket, replacing a stale socket file. Fails if another
/// process is still serving on it.
pub async fn bind_unix(path: &Path) -> io::Result<UnixListener> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    if path.exists() {
        if UnixStream::connect(path).await.is_ok() {
            return Err(io::Error::new(
                io::ErrorKind::AddrInUse,
                format!("{} is already being served", path.display()),
            ));
        }
        std::fs::remove_file(path)?;
    }
    let listener = UnixListener::bind(path)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    Ok(listener)
}

/// Accept forever; pipe each client to `port` on the guest. If the guest is
/// not reachable the client connection is closed immediately.
pub async fn serve<C: Connector>(listener: UnixListener, connector: Arc<C>, port: u32) -> io::Result<()> {
    loop {
        let (client, _) = listener.accept().await?;
        let connector = connector.clone();
        tokio::spawn(async move {
            match connector.connect(port).await {
                Ok(guest) => {
                    let _ = pump(client, guest).await;
                }
                Err(e) => eprintln!("hullr vm: api connection to guest port {port} failed: {e}"),
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::TcpConnector;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    #[tokio::test]
    async fn bridges_unix_to_guest() {
        // Fake guest daemon: echoes with a prefix.
        let guest = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let guest_addr = guest.local_addr().unwrap();
        tokio::spawn(async move {
            loop {
                let (mut s, _) = guest.accept().await.unwrap();
                tokio::spawn(async move {
                    let mut buf = vec![0; 5];
                    s.read_exact(&mut buf).await.unwrap();
                    s.write_all(b"pong:").await.unwrap();
                    s.write_all(&buf).await.unwrap();
                });
            }
        });

        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("hullr.sock");
        std::fs::write(&sock, b"stale").unwrap();
        let listener = bind_unix(&sock).await.unwrap();
        let mut c = TcpConnector::default();
        c.ports.insert(1024, guest_addr);
        tokio::spawn(serve(listener, Arc::new(c), 1024));

        // A second bind sees the live server.
        assert_eq!(bind_unix(&sock).await.unwrap_err().kind(), io::ErrorKind::AddrInUse);

        let mut client = UnixStream::connect(&sock).await.unwrap();
        client.write_all(b"hello").await.unwrap();
        let mut out = vec![0; 10];
        client.read_exact(&mut out).await.unwrap();
        assert_eq!(&out, b"pong:hello");

        // Unroutable port: the client sees EOF.
        let dir2 = tempfile::tempdir().unwrap();
        let sock2 = dir2.path().join("x.sock");
        let l2 = bind_unix(&sock2).await.unwrap();
        tokio::spawn(serve(l2, Arc::new(TcpConnector::default()), 1024));
        let mut c2 = UnixStream::connect(&sock2).await.unwrap();
        let mut buf = Vec::new();
        assert_eq!(c2.read_to_end(&mut buf).await.unwrap(), 0);
    }
}
