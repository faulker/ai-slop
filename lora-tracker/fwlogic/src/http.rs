//! Just enough HTTP/1.0 for the board: parse the `[server]` URL, write a
//! request head, and read the status and body back. Some servers (ip-api.com)
//! answer HTTP/1.1 keep-alive anyway and never close, so the board stops
//! reading once [`complete`] says the whole response is in.

use core::fmt::Write;

use heapless::String;

/// The parts of an `http://host[:port][/base]` URL.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Url<'a> {
    pub host: &'a str,
    pub port: u16,
    /// Path prefix without a trailing slash; empty for the root.
    pub base: &'a str,
}

/// Why a server URL can't be used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UrlError {
    /// `https://` needs TLS, which this firmware doesn't carry.
    Https,
    /// Not `http://...`, or the host or port is unusable.
    Invalid,
}

/// Parses the `[server] url` from the config.
pub fn parse_url(s: &str) -> Result<Url<'_>, UrlError> {
    if s.starts_with("https://") {
        return Err(UrlError::Https);
    }
    let rest = s.strip_prefix("http://").ok_or(UrlError::Invalid)?;
    let (authority, base) = match rest.find('/') {
        Some(i) => (&rest[..i], rest[i..].trim_end_matches('/')),
        None => (rest, ""),
    };
    let (host, port) = match authority.rsplit_once(':') {
        Some((h, p)) => (h, p.parse().map_err(|_| UrlError::Invalid)?),
        None => (authority, 80),
    };
    if host.is_empty() || port == 0 {
        return Err(UrlError::Invalid);
    }
    Ok(Url { host, port, base })
}

/// Parses a dotted IPv4 literal, so the board can skip DNS for `http://192.168.1.5`.
pub fn ipv4(host: &str) -> Option<[u8; 4]> {
    let mut out = [0u8; 4];
    let mut parts = host.split('.');
    for b in &mut out {
        *b = parts.next()?.parse().ok()?;
    }
    parts.next().is_none().then_some(out)
}

/// A request head. `body_len` adds an octet-stream `Content-Length` for POSTs.
pub fn request_head(method: &str, host: &str, port: u16, path: &str, body_len: Option<usize>) -> String<320> {
    let mut h = String::new();
    let _ = write!(h, "{method} {path} HTTP/1.0\r\nHost: {host}");
    if port != 80 {
        let _ = write!(h, ":{port}");
    }
    let _ = write!(h, "\r\nUser-Agent: lora-tracker\r\nConnection: close\r\n");
    if let Some(n) = body_len {
        let _ = write!(h, "Content-Type: application/octet-stream\r\nContent-Length: {n}\r\n");
    }
    let _ = h.push_str("\r\n");
    h
}

/// The status code from `HTTP/1.x NNN ...`.
pub fn status(resp: &[u8]) -> Option<u16> {
    let line = resp.split(|b| *b == b'\r').next()?;
    let line = core::str::from_utf8(line).ok()?;
    let mut parts = line.split(' ');
    parts.next()?.strip_prefix("HTTP/")?;
    parts.next()?.parse().ok()
}

/// Whether `resp` already holds the whole response: the headers plus
/// `Content-Length` bytes of body. 204 and 304 have no body. Without a
/// length, only the server closing the connection ends it.
pub fn complete(resp: &[u8]) -> bool {
    let Some(at) = resp.windows(4).position(|w| w == b"\r\n\r\n") else { return false };
    if matches!(status(resp), Some(204 | 304)) {
        return true;
    }
    let Ok(head) = core::str::from_utf8(&resp[..at]) else { return false };
    let len = head.split("\r\n").find_map(|l| {
        let (name, value) = l.split_once(':')?;
        name.trim().eq_ignore_ascii_case("content-length").then(|| value.trim().parse::<usize>().ok())?
    });
    len.is_some_and(|n| resp.len() - (at + 4) >= n)
}

/// Everything after the blank line that ends the headers.
pub fn body(resp: &[u8]) -> Option<&[u8]> {
    let at = resp.windows(4).position(|w| w == b"\r\n\r\n")?;
    Some(&resp[at + 4..])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_urls() {
        assert_eq!(parse_url("http://192.168.1.216:8080"), Ok(Url { host: "192.168.1.216", port: 8080, base: "" }));
        assert_eq!(parse_url("http://tracker.lan/sub/"), Ok(Url { host: "tracker.lan", port: 80, base: "/sub" }));
        assert_eq!(parse_url("https://x"), Err(UrlError::Https));
        assert_eq!(parse_url("ftp://x"), Err(UrlError::Invalid));
        assert_eq!(parse_url("http://:80"), Err(UrlError::Invalid));
        assert_eq!(parse_url("http://x:notaport"), Err(UrlError::Invalid));
    }

    #[test]
    fn ipv4_literals() {
        assert_eq!(ipv4("192.168.1.216"), Some([192, 168, 1, 216]));
        assert_eq!(ipv4("tracker.lan"), None);
        assert_eq!(ipv4("1.2.3.4.5"), None);
        assert_eq!(ipv4("1.2.3.256"), None);
    }

    #[test]
    fn writes_heads() {
        let h = request_head("POST", "h", 8080, "/ingest", Some(40));
        assert!(h.starts_with("POST /ingest HTTP/1.0\r\nHost: h:8080\r\n"));
        assert!(h.contains("Content-Length: 40\r\n"));
        assert!(h.ends_with("\r\n\r\n"));
        let g = request_head("GET", "ip-api.com", 80, "/json/", None);
        assert!(g.contains("Host: ip-api.com\r\n") && !g.contains("Content-Length"));
    }

    #[test]
    fn knows_when_a_keep_alive_response_is_done() {
        let head = "HTTP/1.1 200 OK\r\ncontent-length: 4\r\nConnection: keep-alive\r\n\r\n";
        assert!(!complete(head.as_bytes()));
        assert!(!complete(format!("{head}{{}}").as_bytes()));
        assert!(complete(format!("{head}{{\"\"}}").as_bytes()));
        assert!(complete(b"HTTP/1.1 204 No Content\r\nDate: x\r\n\r\n"));
        assert!(!complete(b"HTTP/1.0 200 OK\r\n\r\nbody without length"));
        assert!(!complete(b"HTTP/1.1 200 OK\r\nContent-Len"));
    }

    #[test]
    fn reads_status_and_body() {
        let r = b"HTTP/1.1 204 No Content\r\nDate: x\r\n\r\n";
        assert_eq!(status(r), Some(204));
        assert_eq!(body(r), Some(&b""[..]));
        assert_eq!(body(b"HTTP/1.0 200 OK\r\n\r\n{}"), Some(&b"{}"[..]));
        assert_eq!(status(b"garbage"), None);
    }
}
