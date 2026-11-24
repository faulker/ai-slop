//! Reading (and following) a container's JSON-lines log file.

use futures::StreamExt;
use hullr_core::api::LogLineStream;
use hullr_core::{LogLine, LogOptions, Result};
use std::io::{BufRead, Seek, SeekFrom};
use std::path::PathBuf;

/// `alive()` tells a follower whether more output can still arrive.
pub fn stream(
    path: PathBuf,
    opts: LogOptions,
    alive: impl Fn() -> bool + Send + Sync + 'static,
) -> Result<LogLineStream> {
    let (mut lines, pos) = read_from(&path, 0);
    if let Some(n) = opts.tail {
        let skip = lines.len().saturating_sub(n);
        lines.drain(..skip);
    }
    let initial = futures::stream::iter(lines.into_iter().map(Ok));
    if !opts.follow {
        return Ok(initial.boxed());
    }
    struct St {
        path: PathBuf,
        pos: u64,
        pending: Vec<LogLine>,
        alive: std::sync::Arc<dyn Fn() -> bool + Send + Sync>,
    }
    let st = St { path, pos, pending: Vec::new(), alive: std::sync::Arc::new(alive) };
    let tail = futures::stream::unfold(st, |mut st| async move {
        loop {
            if let Some(l) = st.pending.pop() {
                return Some((Ok(l), st));
            }
            // Check liveness before reading so output written right before
            // exit is never lost.
            let alive = (st.alive)();
            let (mut got, new_pos) = read_from(&st.path, st.pos);
            st.pos = new_pos;
            if !got.is_empty() {
                got.reverse();
                st.pending = got;
                continue;
            }
            if !alive {
                return None;
            }
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }
    });
    Ok(initial.chain(tail).boxed())
}

/// Read complete lines from byte offset `pos`; returns lines and new offset.
fn read_from(path: &PathBuf, pos: u64) -> (Vec<LogLine>, u64) {
    let mut out = Vec::new();
    let Ok(mut f) = std::fs::File::open(path) else { return (out, pos) };
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    let mut pos = if len < pos { 0 } else { pos };
    if f.seek(SeekFrom::Start(pos)).is_err() {
        return (out, pos);
    }
    let mut r = std::io::BufReader::new(f);
    let mut line = String::new();
    while let Ok(n) = r.read_line(&mut line) {
        if n == 0 || !line.ends_with('\n') {
            break;
        }
        pos += n as u64;
        if let Ok(l) = serde_json::from_str::<LogLine>(line.trim_end()) {
            out.push(l);
        }
        line.clear();
    }
    (out, pos)
}

#[cfg(test)]
mod tests {
    use super::*;
    use hullr_core::LogStream;
    use std::io::Write;

    fn write(p: &PathBuf, text: &str) {
        let mut f = std::fs::OpenOptions::new().create(true).append(true).open(p).unwrap();
        let l = LogLine { ts: 1, stream: LogStream::Stdout, line: text.into() };
        writeln!(f, "{}", serde_json::to_string(&l).unwrap()).unwrap();
    }

    #[tokio::test]
    async fn tail_and_follow() {
        let t = tempfile::tempdir().unwrap();
        let p = t.path().join("log.jsonl");
        for i in 0..5 {
            write(&p, &format!("l{i}"));
        }
        let s = stream(p.clone(), LogOptions { tail: Some(2), ..Default::default() }, || false).unwrap();
        let got: Vec<String> = s.map(|l| l.unwrap().line).collect().await;
        assert_eq!(got, vec!["l3", "l4"]);

        let alive = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
        let a2 = alive.clone();
        let mut s = stream(p.clone(), LogOptions { follow: true, tail: Some(0), ..Default::default() }, move || {
            a2.load(std::sync::atomic::Ordering::SeqCst)
        })
        .unwrap();
        write(&p, "new");
        assert_eq!(s.next().await.unwrap().unwrap().line, "new");
        alive.store(false, std::sync::atomic::Ordering::SeqCst);
        assert!(s.next().await.is_none());
    }
}
