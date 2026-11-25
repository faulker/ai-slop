use std::collections::BTreeMap;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::transport::elm327::Elm327;

/// Values observed at each byte position of one CAN ID, as sorted unique lists.
/// Index 0 is byte 0 of the frame.
pub type ByteValues = Vec<Vec<u8>>;

/// Aggregated passive capture: for every CAN ID seen, the set of values each
/// byte position took. Frame order and timing are intentionally discarded;
/// the diff only needs to know which bytes were stable and what they were.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MonitorCapture {
    pub label: String,
    pub timestamp: u64,
    /// Filter values (e.g. "300") of windows where the adapter reported BUFFER FULL.
    pub truncated_windows: Vec<String>,
    pub frames: BTreeMap<String, ByteValues>,
}

impl MonitorCapture {
    pub fn new(label: &str) -> Self {
        Self {
            label: label.to_string(),
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            truncated_windows: Vec::new(),
            frames: BTreeMap::new(),
        }
    }

    /// Fold one frame into the per-byte value sets.
    pub fn record(&mut self, id: &str, data: &[u8]) {
        let bytes = self.frames.entry(id.to_uppercase()).or_default();
        if bytes.len() < data.len() {
            bytes.resize(data.len(), Vec::new());
        }
        for (i, b) in data.iter().enumerate() {
            if let Err(pos) = bytes[i].binary_search(b) {
                bytes[i].insert(pos, *b);
            }
        }
    }

    pub fn load(path: &str) -> Result<Self> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| Error::Config(format!("failed to read {}: {}", path, e)))?;
        serde_json::from_str(&content)
            .map_err(|e| Error::Config(format!("failed to parse {}: {}", path, e)))
    }

    pub fn save(&self, path: &str) -> Result<()> {
        let content = serde_json::to_string_pretty(self)
            .map_err(|e| Error::Config(format!("failed to serialize capture: {}", e)))?;
        std::fs::write(path, content)
            .map_err(|e| Error::Config(format!("failed to write {}: {}", path, e)))
    }
}

/// Parse one ATMA output line (`7B0 01 02 03 04 05 06 07 08`) into (id, bytes).
/// Returns None for status lines such as BUFFER FULL, STOPPED, or command echoes.
pub fn parse_frame_line(line: &str) -> Option<(String, Vec<u8>)> {
    let mut parts = line.split_whitespace();
    let id = parts.next()?;
    // 11-bit IDs print as 3 hex chars, 29-bit as 8.
    if !(id.len() == 3 || id.len() == 8) || !id.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let mut data = Vec::with_capacity(8);
    for tok in parts {
        if tok.len() != 2 {
            return None;
        }
        data.push(u8::from_str_radix(tok, 16).ok()?);
    }
    Some((id.to_uppercase(), data))
}

/// Capture the whole 11-bit ID space in eight 0x100-wide windows so a busy
/// 500 kbps bus does not overrun the adapter's serial link in any one window.
pub async fn capture(elm: &mut Elm327, seconds_per_window: u64, label: &str) -> Result<MonitorCapture> {
    let mut cap = MonitorCapture::new(label);
    let window = Duration::from_secs(seconds_per_window);

    for block in 0u16..8 {
        let filter = format!("{:03X}", block << 8);
        print!("  Window 0x{}-0x{:03X} ({}s)... ", filter, (block << 8) | 0xFF, seconds_per_window);
        std::io::Write::flush(&mut std::io::stdout()).ok();

        let (lines, truncated) = elm.monitor_frames(&filter, "700", window).await?;
        let mut count = 0usize;
        for line in &lines {
            if let Some((id, data)) = parse_frame_line(line) {
                cap.record(&id, &data);
                count += 1;
            }
        }
        if truncated {
            cap.truncated_windows.push(filter.clone());
        }
        println!("{} frames{}", count, if truncated { " [BUFFER FULL, truncated]" } else { "" });
    }

    println!("  Captured {} distinct CAN ID(s).", cap.frames.len());
    Ok(cap)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_frame_line_full() {
        let (id, data) = parse_frame_line("7B0 01 02 03 04 05 06 07 08").unwrap();
        assert_eq!(id, "7B0");
        assert_eq!(data, vec![1, 2, 3, 4, 5, 6, 7, 8]);
    }

    #[test]
    fn test_parse_frame_line_short_and_lowercase() {
        let (id, data) = parse_frame_line("0d1 ff 00").unwrap();
        assert_eq!(id, "0D1");
        assert_eq!(data, vec![0xFF, 0x00]);
    }

    #[test]
    fn test_parse_frame_line_ignores_status() {
        assert!(parse_frame_line("BUFFER FULL").is_none());
        assert!(parse_frame_line("STOPPED").is_none());
        assert!(parse_frame_line("ATMA").is_none());
        assert!(parse_frame_line("OK").is_none());
        assert!(parse_frame_line("").is_none());
        assert!(parse_frame_line("7B0 01 GG").is_none());
    }

    #[test]
    fn test_record_accumulates_unique_sorted_values() {
        let mut cap = MonitorCapture::new("t");
        cap.record("140", &[0x10, 0x01]);
        cap.record("140", &[0x10, 0x02]);
        cap.record("140", &[0x10, 0x01, 0xAA]);
        let bytes = &cap.frames["140"];
        assert_eq!(bytes[0], vec![0x10]);
        assert_eq!(bytes[1], vec![0x01, 0x02]);
        assert_eq!(bytes[2], vec![0xAA]);
    }

    #[test]
    fn test_capture_json_round_trip() {
        let mut cap = MonitorCapture::new("before");
        cap.record("7B0", &[1, 2]);
        cap.truncated_windows.push("300".into());
        let json = serde_json::to_string(&cap).unwrap();
        let back: MonitorCapture = serde_json::from_str(&json).unwrap();
        assert_eq!(back, cap);
    }
}
