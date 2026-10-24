//! Tiny HTTP file server that serves the HLS playlist and segments to the Chromecast.

use std::fs::File;
use std::net::{IpAddr, SocketAddr, UdpSocket};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use anyhow::{anyhow, Context};
use tiny_http::{Header, Method, Request, Response, Server};

/// Serves files from a directory on an OS-chosen port in a background thread.
pub struct HlsServer {
    server: Arc<Server>,
    stop: Arc<AtomicBool>,
    port: u16,
    thread: Option<JoinHandle<()>>,
}

impl HlsServer {
    /// Binds 0.0.0.0 on an OS-chosen port and serves files from `dir`.
    pub fn start(dir: PathBuf) -> anyhow::Result<HlsServer> {
        let server = Server::http("0.0.0.0:0")
            .map_err(|e| anyhow!("could not start the HTTP server: {e}"))?;
        let port = server
            .server_addr()
            .to_ip()
            .map(|a: SocketAddr| a.port())
            .context("HTTP server has no IP address")?;
        let server = Arc::new(server);
        let stop = Arc::new(AtomicBool::new(false));
        let thread = {
            let server = server.clone();
            let stop = stop.clone();
            std::thread::Builder::new()
                .name("cast-http".into())
                .spawn(move || serve_loop(&server, &dir, &stop))
                .context("could not spawn the HTTP server thread")?
        };
        Ok(HlsServer {
            server,
            stop,
            port,
            thread: Some(thread),
        })
    }

    /// The port the server is listening on.
    pub fn port(&self) -> u16 {
        self.port
    }

    /// Stops the server and joins its thread.
    pub fn stop(mut self) {
        self.shutdown();
    }

    /// Signals the loop to exit and waits for the thread.
    fn shutdown(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        self.server.unblock();
        if let Some(t) = self.thread.take() {
            if t.join().is_err() {
                log::error!("HTTP server thread panicked");
            }
        }
    }
}

impl Drop for HlsServer {
    /// Ensures the thread is stopped if `stop` was not called.
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Accepts requests until the stop flag is set.
fn serve_loop(server: &Server, dir: &Path, stop: &AtomicBool) {
    while !stop.load(Ordering::SeqCst) {
        match server.recv_timeout(Duration::from_millis(200)) {
            Ok(Some(request)) => handle(request, dir),
            Ok(None) => {}
            Err(e) => {
                log::error!("HTTP server receive failed: {e}");
                break;
            }
        }
    }
}

/// Builds a header from static ASCII strings.
fn header(name: &str, value: &str) -> Header {
    Header::from_bytes(name.as_bytes(), value.as_bytes()).expect("static header is valid")
}

/// Answers one request: OPTIONS preflight, GET/HEAD of a plain file name in `dir`.
fn handle(request: Request, dir: &Path) {
    let cors = header("Access-Control-Allow-Origin", "*");
    let result = if *request.method() == Method::Options {
        let resp = Response::empty(204)
            .with_header(cors)
            .with_header(header("Access-Control-Allow-Methods", "GET, HEAD, OPTIONS"))
            .with_header(header("Access-Control-Allow-Headers", "*"));
        request.respond(resp)
    } else if !matches!(request.method(), Method::Get | Method::Head) {
        request.respond(Response::empty(405).with_header(cors))
    } else {
        // Strip any query string, then only accept a plain file name.
        let path = request.url().split('?').next().unwrap_or("");
        let name = path.strip_prefix('/').unwrap_or(path).to_string();
        let file = if is_plain_name(&name) {
            File::open(dir.join(&name)).ok()
        } else {
            None
        };
        match file {
            Some(file) => {
                let mut resp = Response::from_file(file)
                    .with_header(cors)
                    .with_header(header("Content-Type", content_type_for(&name)));
                if name.ends_with(".m3u8") {
                    resp = resp.with_header(header("Cache-Control", "no-cache"));
                }
                request.respond(resp)
            }
            None => request.respond(
                Response::from_string("not found")
                    .with_status_code(404)
                    .with_header(cors),
            ),
        }
    };
    if let Err(e) = result {
        log::debug!("HTTP response failed: {e}");
    }
}

/// True for a non-empty file name with no separators or parent references.
fn is_plain_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.contains('/')
        && !name.contains('\\')
        && !name.contains('\0')
        && !name.contains("%")
}

/// Content type for a served file name.
pub fn content_type_for(name: &str) -> &'static str {
    if name.ends_with(".m3u8") {
        "application/vnd.apple.mpegurl"
    } else if name.ends_with(".ts") {
        "video/mp2t"
    } else {
        "application/octet-stream"
    }
}

/// The local IP this machine uses to reach `peer` (UDP connect, no packets are sent).
pub fn local_ip_for(peer: IpAddr) -> anyhow::Result<IpAddr> {
    let bind = if peer.is_ipv4() {
        "0.0.0.0:0"
    } else {
        "[::]:0"
    };
    let socket = UdpSocket::bind(bind).context("could not open a UDP socket")?;
    socket
        .connect((peer, 9))
        .with_context(|| format!("no route to {peer}"))?;
    Ok(socket
        .local_addr()
        .context("could not read local address")?
        .ip())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpStream;

    /// Sends an HTTP/1.0 request and returns the raw response text.
    fn http(port: u16, method: &str, path: &str) -> String {
        let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        write!(s, "{method} {path} HTTP/1.0\r\n\r\n").unwrap();
        let mut buf = Vec::new();
        s.read_to_end(&mut buf).unwrap();
        String::from_utf8_lossy(&buf).into_owned()
    }

    /// Extension mapping.
    #[test]
    fn content_types() {
        assert_eq!(
            content_type_for("stream.m3u8"),
            "application/vnd.apple.mpegurl"
        );
        assert_eq!(content_type_for("seg00001.ts"), "video/mp2t");
        assert_eq!(content_type_for("x.bin"), "application/octet-stream");
    }

    /// Serves files with CORS and content type, 404s misses, rejects traversal.
    #[test]
    fn serves_files_safely() {
        let outer = tempfile::tempdir().unwrap();
        let dir = outer.path().join("hls");
        std::fs::create_dir(&dir).unwrap();
        std::fs::write(dir.join("stream.m3u8"), "#EXTM3U\n").unwrap();
        std::fs::write(dir.join("seg1.ts"), [1u8, 2, 3]).unwrap();
        std::fs::write(outer.path().join("secret.txt"), "secret").unwrap();
        let server = HlsServer::start(dir).unwrap();
        let port = server.port();

        let r = http(port, "GET", "/stream.m3u8").to_lowercase();
        assert!(
            r.starts_with("http/1.0 200") || r.starts_with("http/1.1 200"),
            "{r}"
        );
        assert!(r.contains("access-control-allow-origin: *"));
        assert!(r.contains("content-type: application/vnd.apple.mpegurl"));
        assert!(r.contains("cache-control: no-cache"));
        assert!(r.contains("#extm3u"));

        let r = http(port, "GET", "/seg1.ts").to_lowercase();
        assert!(r.contains("content-type: video/mp2t"));

        let r = http(port, "GET", "/missing.ts");
        assert!(r.contains(" 404"), "{r}");
        assert!(r.to_lowercase().contains("access-control-allow-origin: *"));

        for p in [
            "/../secret.txt",
            "/..%2fsecret.txt",
            "/%2e%2e/secret.txt",
            "//etc/passwd",
        ] {
            let r = http(port, "GET", p);
            assert!(r.contains(" 404"), "{p}: {r}");
            assert!(!r.contains("secret\n") && !r.ends_with("secret"));
        }

        let r = http(port, "OPTIONS", "/stream.m3u8").to_lowercase();
        assert!(r.contains(" 204"), "{r}");
        assert!(r.contains("access-control-allow-origin: *"));
        server.stop();
    }

    /// Loopback peers resolve to a local address of the same family.
    #[test]
    fn local_ip_for_loopback() {
        let ip = local_ip_for("127.0.0.1".parse().unwrap()).unwrap();
        assert!(ip.is_ipv4());
    }
}
