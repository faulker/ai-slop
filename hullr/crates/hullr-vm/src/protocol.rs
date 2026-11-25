//! Host/guest wire formats over vsock.
//!
//! | guest port | direction            | content                                  |
//! |-----------:|----------------------|------------------------------------------|
//! | 1024       | host connects        | the hullr JSON API (served by the daemon) |
//! | 1025       | host connects        | control: JSON lines, see [`ControlMsg`]   |
//! | 1026       | host connects        | one forwarded connection per stream       |
//!
//! Control channel: after the host connects, the guest agent sends one
//! `add` line per currently published port, then `add`/`remove` lines as
//! listening sockets come and go. The host may send `{"op":"shutdown"}` to
//! make the guest sync its disks and power off.
//!
//! Forward channel: the host writes a header line `tcp <port>\n`, then the
//! stream carries raw bytes to and from `127.0.0.1:<port>` in the guest.
//! UDP forwarding is not supported yet: published UDP ports are not announced
//! and a `udp` header is rejected.

use serde::{Deserialize, Serialize};
use std::io;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// The daemon's API port inside the guest.
pub const API_PORT: u32 = 1024;
/// The guest agent's control port.
pub const CONTROL_PORT: u32 = 1025;
/// The guest agent's forwarding port.
pub const FORWARD_PORT: u32 = 1026;

/// Header lines are short; anything longer is a protocol error.
const MAX_HEADER: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Op {
    Add,
    Remove,
    Shutdown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Proto {
    #[default]
    Tcp,
    Udp,
}

impl Proto {
    pub fn as_str(self) -> &'static str {
        match self {
            Proto::Tcp => "tcp",
            Proto::Udp => "udp",
        }
    }
}

/// One control line, e.g.
/// `{"op":"add","proto":"tcp","host_ip":"0.0.0.0","port":8080}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControlMsg {
    pub op: Op,
    #[serde(default)]
    pub proto: Proto,
    #[serde(default)]
    pub host_ip: String,
    #[serde(default)]
    pub port: u16,
}

impl ControlMsg {
    pub fn add(host_ip: &str, port: u16) -> Self {
        Self { op: Op::Add, proto: Proto::Tcp, host_ip: host_ip.into(), port }
    }
    pub fn remove(host_ip: &str, port: u16) -> Self {
        Self { op: Op::Remove, proto: Proto::Tcp, host_ip: host_ip.into(), port }
    }
    pub fn shutdown() -> Self {
        Self { op: Op::Shutdown, proto: Proto::Tcp, host_ip: String::new(), port: 0 }
    }
    /// Serialize as one newline-terminated JSON line.
    pub fn to_line(&self) -> String {
        let mut s = serde_json::to_string(self).expect("ControlMsg serializes");
        s.push('\n');
        s
    }
    pub fn parse(line: &str) -> io::Result<Self> {
        serde_json::from_str(line.trim()).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
    }
}

/// The forward header line for a port.
pub fn forward_header(proto: Proto, port: u16) -> String {
    format!("{} {port}\n", proto.as_str())
}

/// Parse a forward header line (without or with its trailing newline).
pub fn parse_forward_header(line: &str) -> io::Result<(Proto, u16)> {
    let bad = || io::Error::new(io::ErrorKind::InvalidData, format!("bad forward header {line:?}"));
    let (proto, port) = line.trim_end_matches('\n').split_once(' ').ok_or_else(bad)?;
    let proto = match proto {
        "tcp" => Proto::Tcp,
        "udp" => Proto::Udp,
        _ => return Err(bad()),
    };
    let port: u16 = port.parse().map_err(|_| bad())?;
    if port == 0 {
        return Err(bad());
    }
    Ok((proto, port))
}

/// Write the forward header.
pub async fn write_forward_header<W: AsyncWrite + Unpin>(w: &mut W, proto: Proto, port: u16) -> io::Result<()> {
    w.write_all(forward_header(proto, port).as_bytes()).await
}

/// Read one header line byte by byte, so no payload bytes are consumed.
pub async fn read_forward_header<R: AsyncRead + Unpin>(r: &mut R) -> io::Result<(Proto, u16)> {
    let mut line = Vec::with_capacity(16);
    loop {
        let b = r.read_u8().await?;
        if b == b'\n' {
            break;
        }
        line.push(b);
        if line.len() > MAX_HEADER {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "forward header too long"));
        }
    }
    let line = String::from_utf8(line).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    parse_forward_header(&line)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn control_lines() {
        let m = ControlMsg::add("0.0.0.0", 8080);
        assert_eq!(m.to_line(), "{\"op\":\"add\",\"proto\":\"tcp\",\"host_ip\":\"0.0.0.0\",\"port\":8080}\n");
        assert_eq!(ControlMsg::parse(&m.to_line()).unwrap(), m);
        assert_eq!(ControlMsg::parse(r#"{"op":"shutdown"}"#).unwrap().op, Op::Shutdown);
        assert!(ControlMsg::parse(r#"{"op":"explode"}"#).is_err());
    }

    #[test]
    fn header_parse() {
        assert_eq!(parse_forward_header("tcp 8080\n").unwrap(), (Proto::Tcp, 8080));
        assert_eq!(parse_forward_header("udp 53").unwrap(), (Proto::Udp, 53));
        for bad in ["", "tcp", "tcp 0", "tcp 70000", "sctp 1", "tcp  80", "tcp 80 x"] {
            assert!(parse_forward_header(bad).is_err(), "{bad:?}");
        }
        assert_eq!(forward_header(Proto::Tcp, 443), "tcp 443\n");
    }

    #[tokio::test]
    async fn header_does_not_eat_payload() {
        let (mut a, mut b) = tokio::io::duplex(64);
        write_forward_header(&mut a, Proto::Tcp, 8080).await.unwrap();
        a.write_all(b"GET /").await.unwrap();
        drop(a);
        assert_eq!(read_forward_header(&mut b).await.unwrap(), (Proto::Tcp, 8080));
        let mut rest = String::new();
        b.read_to_string(&mut rest).await.unwrap();
        assert_eq!(rest, "GET /");
    }

    #[tokio::test]
    async fn header_too_long() {
        let (mut a, mut b) = tokio::io::duplex(256);
        a.write_all(&[b'x'; 100]).await.unwrap();
        assert!(read_forward_header(&mut b).await.is_err());
    }
}
