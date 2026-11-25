use std::io::Write;

use crate::error::Result;
use crate::protocol::uds;
use crate::transport::elm327::Elm327;

/// Tester header used for BMW diagnostics. The module address rides in the
/// first data byte (extended addressing) instead of the CAN ID.
pub const TESTER_HEADER: &str = "6F1";

/// Standard UDS identification DIDs read by `identify`. All read-only.
pub static ID_DIDS: &[(u16, &str)] = &[
    (0xF190, "VIN"),
    (0xF187, "Part number"),
    (0xF18A, "Supplier"),
    (0xF18C, "Serial number"),
    (0xF191, "Hardware version"),
    (0xF195, "Software version"),
    (0xF197, "System name"),
];

/// Diagnostic target string for a module address, e.g. 0x60 -> "6F1:60".
pub fn target(address: u8) -> String {
    format!("{}:{:02X}", TESTER_HEADER, address)
}

/// Probe every extended address in `start..=end` with TesterPresent and return
/// the ones that answered (positive or negative response). Read-only.
/// Skips the tester's own address (0xF1).
pub async fn sweep(elm: &mut Elm327, start: u8, end: u8) -> Result<Vec<u8>> {
    let mut found = Vec::new();

    // ~300ms per address keeps a full 00-FF sweep around a minute or two.
    elm.at_command("ATST 4B").await?;

    for address in start..=end {
        if address == 0xF1 {
            continue;
        }
        print!("\r  Probing {}... ", target(address));
        std::io::stdout().flush().ok();

        elm.set_header(&target(address)).await?;
        if let Ok(resp) = elm.send_obd("3E 00").await {
            if !resp.lines.is_empty() {
                println!("\r  FOUND module 0x{:02X}           ", address);
                found.push(address);
            }
        }
    }
    println!("\r  Swept 0x{:02X}-0x{:02X}: {} module(s) answered.        ", start, end, found.len());

    elm.at_command("ATST FF").await?;
    elm.set_header("7DF").await?;

    Ok(found)
}

/// Read the standard identification DIDs from one module in the default session.
/// Returns (label, raw bytes) for each DID the module answered.
pub async fn identify(elm: &mut Elm327, address: u8) -> Result<Vec<(&'static str, Vec<u8>)>> {
    let mut values = Vec::new();
    elm.set_header(&target(address)).await?;

    for (did, label) in ID_DIDS {
        if let Ok(resp) = uds::send_uds(elm, &uds::read_data_by_identifier(*did)).await {
            if resp.len() > 3 {
                values.push((*label, resp[3..].to_vec()));
            }
        }
    }

    elm.set_header("7DF").await?;
    Ok(values)
}

/// Render ID bytes as text when they are printable ASCII, otherwise as hex.
pub fn display_id(data: &[u8]) -> String {
    let trimmed: Vec<u8> = data.iter().copied().filter(|b| *b != 0x00 && *b != 0xFF).collect();
    if !trimmed.is_empty() && trimmed.iter().all(|b| b.is_ascii_graphic() || *b == b' ') {
        String::from_utf8_lossy(&trimmed).trim().to_string()
    } else {
        uds::hex_string(data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::elm327::parse_target;

    #[test]
    fn test_target_round_trips_through_parse_target() {
        assert_eq!(target(0x12), "6F1:12");
        assert_eq!(parse_target(&target(0x60)).unwrap(), ("6F1".to_string(), Some(0x60)));
    }

    #[test]
    fn test_display_id_ascii_and_hex() {
        assert_eq!(display_id(b"TESTVIN0000000001"), "TESTVIN0000000001");
        assert_eq!(display_id(&[0x41, 0x42, 0x00, 0x00]), "AB");
        assert_eq!(display_id(&[0x01, 0x9A]), "01 9A");
    }
}
