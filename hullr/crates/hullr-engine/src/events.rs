//! Cross-process event log: JSON lines appended to `engine/events.jsonl`.

use futures::Stream;
use hullr_core::api::{Event, EventKind};
use hullr_core::paths::Paths;
use hullr_core::Result;
use std::io::{BufRead, Seek, SeekFrom, Write};
use std::path::PathBuf;

/// Rotate when the log grows past this.
const MAX_LOG: u64 = 4 << 20;

fn path(paths: &Paths) -> PathBuf {
    paths.engine().join("events.jsonl")
}

pub fn append(paths: &Paths, kind: EventKind, id: &str, name: &str) {
    let ev = Event { ts: hullr_core::now_unix(), kind, id: id.to_string(), name: name.to_string() };
    let p = path(paths);
    if std::fs::metadata(&p).map(|m| m.len() > MAX_LOG).unwrap_or(false) {
        let _ = std::fs::rename(&p, p.with_extension("jsonl.1"));
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&p) {
        if let Ok(mut line) = serde_json::to_vec(&ev) {
            line.push(b'\n');
            // One write() with O_APPEND keeps lines whole across processes.
            let _ = f.write_all(&line);
        }
    }
}

/// Stream events appended from now on.
pub fn follow(paths: &Paths) -> impl Stream<Item = Result<Event>> + Send + 'static {
    let p = path(paths);
    let start = std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
    futures::stream::unfold((p, start, Vec::<Event>::new()), |(p, mut pos, mut pending)| async move {
        loop {
            if let Some(ev) = pending.pop() {
                return Some((Ok(ev), (p, pos, pending)));
            }
            let len = std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
            if len < pos {
                pos = 0; // rotated
            }
            if len > pos {
                if let Ok(mut f) = std::fs::File::open(&p) {
                    let _ = f.seek(SeekFrom::Start(pos));
                    let mut r = std::io::BufReader::new(f);
                    let mut line = String::new();
                    let mut got = Vec::new();
                    while let Ok(n) = r.read_line(&mut line) {
                        if n == 0 || !line.ends_with('\n') {
                            break;
                        }
                        pos += n as u64;
                        if let Ok(ev) = serde_json::from_str::<Event>(line.trim_end()) {
                            got.push(ev);
                        }
                        line.clear();
                    }
                    got.reverse();
                    pending = got;
                    continue;
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt;

    #[tokio::test]
    async fn append_and_follow() {
        let t = tempfile::tempdir().unwrap();
        let p = Paths::new(t.path());
        std::fs::create_dir_all(p.engine()).unwrap();
        append(&p, EventKind::ContainerCreated, "old", "old");
        let mut s = Box::pin(follow(&p));
        append(&p, EventKind::ContainerStarted, "a", "web");
        append(&p, EventKind::ContainerDied, "a", "web");
        let e1 = s.next().await.unwrap().unwrap();
        let e2 = s.next().await.unwrap().unwrap();
        assert_eq!(e1.kind, EventKind::ContainerStarted);
        assert_eq!(e2.kind, EventKind::ContainerDied);
    }
}
