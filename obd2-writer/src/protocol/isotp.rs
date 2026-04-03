use crate::error::{Error, Result};

/// Reassemble multi-frame ISO-TP response from ELM327 output lines.
/// Each line is a CAN frame with header, e.g.:
///   "7E8 10 14 62 01 00 AA BB"  (first frame)
///   "7E8 21 CC DD EE FF 00 11"  (consecutive frame)
///   "7E8 22 22 33 44 55 66 77"  (consecutive frame)
///
/// For single-frame responses:
///   "7E8 04 41 0C 1A F8"
///
/// Returns the reassembled payload bytes (after stripping headers and PCI bytes).
pub fn reassemble(lines: &[String]) -> Result<Vec<u8>> {
    if lines.is_empty() {
        return Err(Error::Protocol("no response lines to reassemble".into()));
    }

    // Parse each line into (header, data_bytes)
    let frames: Vec<(String, Vec<u8>)> = lines
        .iter()
        .map(|line| parse_frame(line))
        .collect::<Result<Vec<_>>>()?;

    if frames.is_empty() {
        return Err(Error::Protocol("no valid frames found".into()));
    }

    let (_, first_data) = &frames[0];
    if first_data.is_empty() {
        return Err(Error::Protocol("empty frame data".into()));
    }

    let pci_type = (first_data[0] & 0xF0) >> 4;

    match pci_type {
        // Single frame
        0 => {
            let length = (first_data[0] & 0x0F) as usize;
            if length == 0 || length > first_data.len() - 1 {
                return Err(Error::Protocol(format!(
                    "invalid single frame length: {}",
                    length
                )));
            }
            Ok(first_data[1..1 + length].to_vec())
        }
        // First frame (multi-frame)
        1 => {
            let total_length =
                (((first_data[0] & 0x0F) as usize) << 8) | (first_data[1] as usize);
            let mut payload = Vec::with_capacity(total_length);

            let source_header = &frames[0].0;

            // First frame data starts at byte 2
            payload.extend_from_slice(&first_data[2..]);

            // Consecutive frames — validate sequence numbers and source ECU
            let mut expected_seq: u8 = 1;
            for (header, data) in frames.iter().skip(1) {
                if data.is_empty() {
                    continue;
                }
                if header != source_header {
                    return Err(Error::Protocol(format!(
                        "ISO-TP source ECU mismatch: expected {}, got {}",
                        source_header, header
                    )));
                }
                let cf_pci = (data[0] & 0xF0) >> 4;
                if cf_pci != 2 {
                    continue; // skip non-consecutive frames
                }
                let seq = data[0] & 0x0F;
                if seq != expected_seq {
                    return Err(Error::Protocol(format!(
                        "ISO-TP consecutive frame out of order: expected seq {}, got {}",
                        expected_seq, seq
                    )));
                }
                expected_seq = (expected_seq + 1) & 0x0F; // wraps 0xF → 0
                // Consecutive frame data starts at byte 1 (after sequence number)
                payload.extend_from_slice(&data[1..]);
            }

            // Trim to actual length
            payload.truncate(total_length);
            Ok(payload)
        }
        _ => Err(Error::Protocol(format!("unexpected PCI type: {}", pci_type))),
    }
}

/// Parse a single ELM327 response line into (header, data_bytes).
/// Input: "7E8 04 41 0C 1A F8"
/// Output: ("7E8", [0x04, 0x41, 0x0C, 0x1A, 0xF8])
fn parse_frame(line: &str) -> Result<(String, Vec<u8>)> {
    let parts: Vec<&str> = line.split_whitespace().collect();
    if parts.len() < 2 {
        return Err(Error::Protocol(format!("malformed frame: {}", line)));
    }

    let header = parts[0].to_string();
    let data: Vec<u8> = parts[1..]
        .iter()
        .map(|s| u8::from_str_radix(s, 16).map_err(|_| Error::Protocol(format!("invalid hex byte: {}", s))))
        .collect::<Result<Vec<_>>>()?;

    Ok((header, data))
}

/// Extract just the data bytes from a single-frame response line.
/// For simple Mode 01 responses where you know it's a single frame.
pub fn extract_single_frame_data(lines: &[String]) -> Result<Vec<u8>> {
    reassemble(lines)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_single_frame() {
        let lines = vec!["7E8 04 41 0C 1A F8".to_string()];
        let result = reassemble(&lines).unwrap();
        assert_eq!(result, vec![0x41, 0x0C, 0x1A, 0xF8]);
    }

    #[test]
    fn test_multi_frame() {
        let lines = vec![
            "7E8 10 0A 62 01 00 AA BB".to_string(),
            "7E8 21 CC DD EE FF 00".to_string(),
        ];
        let result = reassemble(&lines).unwrap();
        // Total length = 10 bytes
        // First frame payload: [0x62, 0x01, 0x00, 0xAA, 0xBB] = 5 bytes
        // Consecutive frame: [0xCC, 0xDD, 0xEE, 0xFF, 0x00] = 5 bytes
        // Total = 10 bytes
        assert_eq!(result.len(), 10);
        assert_eq!(result[0], 0x62); // UDS service + 0x40
    }

    #[test]
    fn test_empty_lines() {
        let lines: Vec<String> = vec![];
        assert!(reassemble(&lines).is_err());
    }

    #[test]
    fn test_multi_frame_bad_sequence() {
        let lines = vec![
            "7E8 10 0A 62 01 00 AA BB".to_string(),
            "7E8 23 CC DD EE FF 00".to_string(), // seq 3 instead of 1
        ];
        let result = reassemble(&lines);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("out of order"));
    }

    #[test]
    fn test_multi_frame_source_mismatch() {
        let lines = vec![
            "7E8 10 0A 62 01 00 AA BB".to_string(),
            "7C8 21 CC DD EE FF 00".to_string(), // different source ECU
        ];
        let result = reassemble(&lines);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("source ECU mismatch"));
    }

    #[test]
    fn test_multi_frame_sequence_wraps() {
        // Sequence wraps from 0xF to 0x0
        let mut lines = vec![
            "7E8 10 64 62 01 00 AA BB".to_string(), // FF=100 bytes, 5 data bytes
        ];
        // Need 95 more bytes in consecutive frames (6 data bytes each = 16 frames)
        for i in 1u8..=16 {
            let seq = i & 0x0F;
            lines.push(format!("7E8 2{:X} 01 02 03 04 05 06", seq));
        }
        let result = reassemble(&lines);
        assert!(result.is_ok());
        assert_eq!(result.unwrap().len(), 100);
    }
}
