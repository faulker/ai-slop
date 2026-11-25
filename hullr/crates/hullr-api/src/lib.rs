//! The daemon wire protocol.
//!
//! One request per connection, which keeps both sides trivial:
//!
//! 1. Client writes one JSON line: `{"method": "...", "params": {...}}`.
//! 2. Server writes zero or more JSON lines `{"progress": ...}` or
//!    `{"item": ...}` (for streams), then one final `{"ok": ...}` or
//!    `{"err": {...}}` line.
//! 3. Streaming methods (`logs`, `events`) reply `{"ok": null}` once the
//!    stream is open (or `{"err": ...}`), then send `{"item": ...}` lines
//!    until the connection closes; a mid-stream failure is an `{"err"}` line.
//! 4. For `attach` and `exec` the reply is `{"ok": null}` and the
//!    connection then carries raw `hullr_core::stream` frames both ways.

mod client;
mod server;
pub mod vsock;

pub use client::Client;
pub use server::{serve, serve_listener, Listen};

use hullr_core::api::Progress;
use hullr_core::Error;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct Request {
    pub method: String,
    #[serde(default)]
    pub params: serde_json::Value,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Reply {
    Progress(Progress),
    Item(serde_json::Value),
    Ok(serde_json::Value),
    Err(Error),
}

/// Read one `\n`-terminated line a byte at a time, so nothing after it
/// (e.g. attach frames) is consumed. `None` on EOF before any byte.
pub(crate) async fn read_line_raw<R: tokio::io::AsyncRead + Unpin>(r: &mut R) -> std::io::Result<Option<String>> {
    use tokio::io::AsyncReadExt;
    let mut buf = Vec::new();
    let mut b = [0u8; 1];
    loop {
        if r.read(&mut b).await? == 0 {
            return if buf.is_empty() {
                Ok(None)
            } else {
                Err(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "truncated line"))
            };
        }
        if b[0] == b'\n' {
            return Ok(Some(String::from_utf8_lossy(&buf).into_owned()));
        }
        buf.push(b[0]);
        if buf.len() > 16 << 20 {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "line too long"));
        }
    }
}
