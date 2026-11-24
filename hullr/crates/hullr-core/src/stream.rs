//! Framed byte-stream protocol for attach and exec.
//!
//! Used between the CLI and the runtime's shim (directly, or proxied by the
//! daemon). Each frame is `tag: u8`, `len: u32 BE`, `payload`.

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Frame {
    /// Client -> container stdin.
    Stdin(Vec<u8>),
    /// Client closed stdin (EOF).
    CloseStdin,
    /// Container stdout (or the pty) -> client.
    Stdout(Vec<u8>),
    /// Container stderr -> client (never used with a tty).
    Stderr(Vec<u8>),
    /// Terminal resize from the client.
    Resize { cols: u16, rows: u16 },
    /// The process exited; last frame the server sends.
    Exit(i32),
    /// Server-side error text; followed by Exit.
    Error(String),
}

const T_STDIN: u8 = 0;
const T_STDOUT: u8 = 1;
const T_STDERR: u8 = 2;
const T_EXIT: u8 = 3;
const T_RESIZE: u8 = 4;
const T_CLOSE_STDIN: u8 = 5;
const T_ERROR: u8 = 6;

/// Largest payload accepted, to bound memory on a corrupt stream.
pub const MAX_FRAME: usize = 1 << 20;

impl Frame {
    pub fn encode(&self) -> Vec<u8> {
        let (tag, payload): (u8, Vec<u8>) = match self {
            Frame::Stdin(d) => (T_STDIN, d.clone()),
            Frame::CloseStdin => (T_CLOSE_STDIN, vec![]),
            Frame::Stdout(d) => (T_STDOUT, d.clone()),
            Frame::Stderr(d) => (T_STDERR, d.clone()),
            Frame::Resize { cols, rows } => {
                let mut v = cols.to_be_bytes().to_vec();
                v.extend_from_slice(&rows.to_be_bytes());
                (T_RESIZE, v)
            }
            Frame::Exit(c) => (T_EXIT, c.to_be_bytes().to_vec()),
            Frame::Error(s) => (T_ERROR, s.as_bytes().to_vec()),
        };
        let mut out = Vec::with_capacity(5 + payload.len());
        out.push(tag);
        out.extend_from_slice(&(payload.len() as u32).to_be_bytes());
        out.extend_from_slice(&payload);
        out
    }

    fn decode(tag: u8, p: Vec<u8>) -> std::io::Result<Self> {
        let bad = || std::io::Error::new(std::io::ErrorKind::InvalidData, "bad frame");
        Ok(match tag {
            T_STDIN => Frame::Stdin(p),
            T_CLOSE_STDIN => Frame::CloseStdin,
            T_STDOUT => Frame::Stdout(p),
            T_STDERR => Frame::Stderr(p),
            T_RESIZE if p.len() == 4 => Frame::Resize {
                cols: u16::from_be_bytes([p[0], p[1]]),
                rows: u16::from_be_bytes([p[2], p[3]]),
            },
            T_EXIT if p.len() == 4 => Frame::Exit(i32::from_be_bytes([p[0], p[1], p[2], p[3]])),
            T_ERROR => Frame::Error(String::from_utf8_lossy(&p).into_owned()),
            _ => return Err(bad()),
        })
    }
}

pub async fn write_frame<W: AsyncWrite + Unpin>(w: &mut W, f: &Frame) -> std::io::Result<()> {
    w.write_all(&f.encode()).await?;
    w.flush().await
}

/// Read one frame; `Ok(None)` on clean EOF.
pub async fn read_frame<R: AsyncRead + Unpin>(r: &mut R) -> std::io::Result<Option<Frame>> {
    let mut hdr = [0u8; 5];
    match r.read_exact(&mut hdr).await {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    let len = u32::from_be_bytes([hdr[1], hdr[2], hdr[3], hdr[4]]) as usize;
    if len > MAX_FRAME {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "frame too large"));
    }
    let mut p = vec![0u8; len];
    r.read_exact(&mut p).await?;
    Frame::decode(hdr[0], p).map(Some)
}

/// Blocking variants, for code that runs before/without a tokio runtime
/// (the shim's hot loop, the container init).
pub mod blocking {
    use super::*;
    use std::io::{Read, Write};

    pub fn write_frame<W: Write>(w: &mut W, f: &Frame) -> std::io::Result<()> {
        w.write_all(&f.encode())?;
        w.flush()
    }

    pub fn read_frame<R: Read>(r: &mut R) -> std::io::Result<Option<Frame>> {
        let mut hdr = [0u8; 5];
        match r.read_exact(&mut hdr) {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
            Err(e) => return Err(e),
        }
        let len = u32::from_be_bytes([hdr[1], hdr[2], hdr[3], hdr[4]]) as usize;
        if len > MAX_FRAME {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "frame too large"));
        }
        let mut p = vec![0u8; len];
        r.read_exact(&mut p)?;
        Frame::decode(hdr[0], p).map(Some)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn roundtrip() {
        let frames = vec![
            Frame::Stdin(b"ls\n".to_vec()),
            Frame::Resize { cols: 80, rows: 24 },
            Frame::Stdout(vec![1, 2, 3]),
            Frame::Error("boom".into()),
            Frame::Exit(-9),
            Frame::CloseStdin,
        ];
        let mut buf = Vec::new();
        for f in &frames {
            write_frame(&mut buf, f).await.unwrap();
        }
        let mut r = &buf[..];
        for f in &frames {
            assert_eq!(read_frame(&mut r).await.unwrap().as_ref(), Some(f));
        }
        assert_eq!(read_frame(&mut r).await.unwrap(), None);
    }
}
