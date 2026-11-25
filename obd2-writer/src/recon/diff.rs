use std::collections::BTreeMap;

use crate::error::{Error, Result};
use crate::recon::monitor::MonitorCapture;
use crate::recon::snapshot::Snapshot;

/// A DID whose value differs between two snapshots.
#[derive(Debug, PartialEq)]
pub struct DidChange {
    pub did: String,
    pub before: String,
    pub after: String,
    /// Byte offsets that differ (or extend past the shorter value).
    pub offsets: Vec<usize>,
}

#[derive(Debug, Default, PartialEq)]
pub struct SnapshotDiff {
    pub changed: Vec<DidChange>,
    pub only_before: Vec<String>,
    pub only_after: Vec<String>,
}

/// Compare two DID snapshots of the same ECU.
pub fn diff_snapshots(before: &Snapshot, after: &Snapshot) -> SnapshotDiff {
    let a: BTreeMap<&str, &str> = before.entries.iter().map(|e| (e.did.as_str(), e.data.as_str())).collect();
    let b: BTreeMap<&str, &str> = after.entries.iter().map(|e| (e.did.as_str(), e.data.as_str())).collect();
    let mut diff = SnapshotDiff::default();

    for (did, data_a) in &a {
        match b.get(did) {
            None => diff.only_before.push(did.to_string()),
            Some(data_b) if data_a != data_b => diff.changed.push(DidChange {
                did: did.to_string(),
                before: data_a.to_string(),
                after: data_b.to_string(),
                offsets: changed_offsets(data_a, data_b),
            }),
            Some(_) => {}
        }
    }
    for did in b.keys() {
        if !a.contains_key(did) {
            diff.only_after.push(did.to_string());
        }
    }
    diff
}

/// Byte offsets where two space-separated hex strings differ.
fn changed_offsets(a: &str, b: &str) -> Vec<usize> {
    let a: Vec<&str> = a.split_whitespace().collect();
    let b: Vec<&str> = b.split_whitespace().collect();
    (0..a.len().max(b.len()))
        .filter(|&i| a.get(i) != b.get(i))
        .collect()
}

/// One byte of one CAN ID that was stable in both captures but at different values.
#[derive(Debug, PartialEq)]
pub struct ByteChange {
    pub id: String,
    pub byte: usize,
    pub before: u8,
    pub after: u8,
}

#[derive(Debug, Default, PartialEq)]
pub struct CaptureDiff {
    pub changed: Vec<ByteChange>,
    pub only_before: Vec<String>,
    pub only_after: Vec<String>,
}

/// Compare two passive captures. A byte is reported only when it held a single
/// value throughout each capture and those values differ; bytes that vary
/// within a capture (counters, checksums, live sensors) are ignored.
pub fn diff_captures(before: &MonitorCapture, after: &MonitorCapture) -> CaptureDiff {
    let mut diff = CaptureDiff::default();

    for (id, bytes_a) in &before.frames {
        let Some(bytes_b) = after.frames.get(id) else {
            diff.only_before.push(id.clone());
            continue;
        };
        for (i, (va, vb)) in bytes_a.iter().zip(bytes_b.iter()).enumerate() {
            if let ([a], [b]) = (va.as_slice(), vb.as_slice()) {
                if a != b {
                    diff.changed.push(ByteChange { id: id.clone(), byte: i, before: *a, after: *b });
                }
            }
        }
    }
    for id in after.frames.keys() {
        if !before.frames.contains_key(id) {
            diff.only_after.push(id.clone());
        }
    }
    diff
}

/// Load two JSON files, detect whether they are snapshots or captures, and print the diff.
pub fn diff_files(path_a: &str, path_b: &str) -> Result<()> {
    let kind_a = detect_kind(path_a)?;
    let kind_b = detect_kind(path_b)?;
    if kind_a != kind_b {
        return Err(Error::Config(format!(
            "cannot diff a {} against a {}",
            kind_a, kind_b
        )));
    }

    match kind_a {
        "snapshot" => {
            let a = Snapshot::load(path_a)?;
            let b = Snapshot::load(path_b)?;
            if a.ecu != b.ecu {
                eprintln!("Warning: snapshots are from different ECUs ({} vs {})", a.ecu, b.ecu);
            }
            print_snapshot_diff(&a, &b, &diff_snapshots(&a, &b));
        }
        _ => {
            let a = MonitorCapture::load(path_a)?;
            let b = MonitorCapture::load(path_b)?;
            print_capture_diff(&a, &b, &diff_captures(&a, &b));
        }
    }
    Ok(())
}

/// Peek at the JSON shape: snapshots have `entries`, captures have `frames`.
fn detect_kind(path: &str) -> Result<&'static str> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| Error::Config(format!("failed to read {}: {}", path, e)))?;
    let value: serde_json::Value = serde_json::from_str(&content)
        .map_err(|e| Error::Config(format!("failed to parse {}: {}", path, e)))?;
    if value.get("entries").is_some() {
        Ok("snapshot")
    } else if value.get("frames").is_some() {
        Ok("capture")
    } else {
        Err(Error::Config(format!("{} is neither a snapshot nor a capture", path)))
    }
}

fn print_snapshot_diff(a: &Snapshot, b: &Snapshot, diff: &SnapshotDiff) {
    println!("DID diff on ECU 0x{}: '{}' -> '{}'", a.ecu, a.label, b.label);
    println!("  {} DIDs before, {} after, {} changed", a.entries.len(), b.entries.len(), diff.changed.len());
    for c in &diff.changed {
        println!("\n  0x{}  byte offset(s) {:?}", c.did, c.offsets);
        println!("    before: {}", c.before);
        println!("    after:  {}", c.after);
    }
    if !diff.only_before.is_empty() {
        println!("\n  Only in '{}': {}", a.label, diff.only_before.join(", "));
    }
    if !diff.only_after.is_empty() {
        println!("\n  Only in '{}': {}", b.label, diff.only_after.join(", "));
    }
    if diff.changed.is_empty() && diff.only_before.is_empty() && diff.only_after.is_empty() {
        println!("  No differences.");
    }
}

fn print_capture_diff(a: &MonitorCapture, b: &MonitorCapture, diff: &CaptureDiff) {
    println!("CAN broadcast diff: '{}' -> '{}'", a.label, b.label);
    println!("  {} IDs before, {} after", a.frames.len(), b.frames.len());
    for t in [&a.truncated_windows, &b.truncated_windows] {
        if !t.is_empty() {
            println!("  Note: truncated windows (BUFFER FULL) at filter(s) {}", t.join(", "));
        }
    }
    println!("  {} stable byte(s) changed value:", diff.changed.len());
    for c in &diff.changed {
        println!("    ID {} byte {}: 0x{:02X} -> 0x{:02X}", c.id, c.byte, c.before, c.after);
    }
    if !diff.only_before.is_empty() {
        println!("\n  IDs only in '{}': {}", a.label, diff.only_before.join(", "));
    }
    if !diff.only_after.is_empty() {
        println!("\n  IDs only in '{}': {}", b.label, diff.only_after.join(", "));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap(entries: &[(&str, &str)]) -> Snapshot {
        Snapshot {
            label: "s".into(),
            timestamp: 0,
            ecu: "7E0".into(),
            entries: entries
                .iter()
                .map(|(d, v)| crate::recon::snapshot::SnapshotEntry { did: d.to_string(), data: v.to_string() })
                .collect(),
        }
    }

    #[test]
    fn test_diff_snapshots_identical() {
        let a = snap(&[("0100", "01 02")]);
        assert_eq!(diff_snapshots(&a, &a), SnapshotDiff::default());
    }

    #[test]
    fn test_diff_snapshots_reports_changed_offset() {
        let a = snap(&[("0100", "01 02 03")]);
        let b = snap(&[("0100", "01 FF 03")]);
        let d = diff_snapshots(&a, &b);
        assert_eq!(d.changed.len(), 1);
        assert_eq!(d.changed[0].did, "0100");
        assert_eq!(d.changed[0].offsets, vec![1]);
    }

    #[test]
    fn test_diff_snapshots_length_change() {
        let a = snap(&[("0100", "01")]);
        let b = snap(&[("0100", "01 02")]);
        assert_eq!(diff_snapshots(&a, &b).changed[0].offsets, vec![1]);
    }

    #[test]
    fn test_diff_snapshots_one_sided() {
        let a = snap(&[("0100", "01"), ("0200", "02")]);
        let b = snap(&[("0100", "01"), ("0300", "03")]);
        let d = diff_snapshots(&a, &b);
        assert!(d.changed.is_empty());
        assert_eq!(d.only_before, vec!["0200"]);
        assert_eq!(d.only_after, vec!["0300"]);
    }

    #[test]
    fn test_diff_captures_stable_change_reported() {
        let mut a = MonitorCapture::new("a");
        let mut b = MonitorCapture::new("b");
        a.record("140", &[0x00, 0x10]);
        b.record("140", &[0x01, 0x10]);
        let d = diff_captures(&a, &b);
        assert_eq!(d.changed, vec![ByteChange { id: "140".into(), byte: 0, before: 0, after: 1 }]);
    }

    #[test]
    fn test_diff_captures_ignores_varying_bytes() {
        let mut a = MonitorCapture::new("a");
        let mut b = MonitorCapture::new("b");
        // Byte 0 is a counter in both captures, byte 1 is stable and equal.
        a.record("140", &[0x00, 0x10]);
        a.record("140", &[0x01, 0x10]);
        b.record("140", &[0x05, 0x10]);
        b.record("140", &[0x06, 0x10]);
        assert!(diff_captures(&a, &b).changed.is_empty());
    }

    #[test]
    fn test_diff_captures_one_sided_ids() {
        let mut a = MonitorCapture::new("a");
        let mut b = MonitorCapture::new("b");
        a.record("140", &[0]);
        b.record("141", &[0]);
        let d = diff_captures(&a, &b);
        assert_eq!(d.only_before, vec!["140"]);
        assert_eq!(d.only_after, vec!["141"]);
    }

    #[test]
    fn test_changed_offsets() {
        assert_eq!(changed_offsets("01 02 03", "01 02 03"), Vec::<usize>::new());
        assert_eq!(changed_offsets("01 02 03", "AA 02 BB"), vec![0, 2]);
    }
}
