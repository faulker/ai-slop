use std::io::Write;

use crate::error::{Error, Result};
use crate::protocol::uds;
use crate::transport::elm327::Elm327;

/// Result of scanning a single DID.
pub struct DiscoveredDid {
    pub did: u16,
    pub data: Vec<u8>,
    pub writable: bool,
}

/// Predefined scan ranges for Toyota BCM DID discovery.
pub static TOYOTA_BCM_RANGES: &[(&str, u16, u16)] = &[
    ("Body settings (common)", 0x0100, 0x01FF),
    ("BCM configuration", 0xB000, 0xB1FF),
    ("Body alternate range", 0xC000, 0xC0FF),
    ("Identification DIDs", 0xF100, 0xF1FF),
];

/// Parse a hex range string like "B000-B1FF" into (start, end).
pub fn parse_range(range_str: &str) -> Result<(u16, u16)> {
    let parts: Vec<&str> = range_str.split('-').collect();
    if parts.len() != 2 {
        return Err(Error::Config(format!(
            "invalid range '{}'. Expected format: START-END (e.g., B000-B1FF)",
            range_str
        )));
    }
    let start = u16::from_str_radix(parts[0].trim_start_matches("0x"), 16)
        .map_err(|_| Error::Config(format!("invalid hex start: '{}'", parts[0])))?;
    let end = u16::from_str_radix(parts[1].trim_start_matches("0x"), 16)
        .map_err(|_| Error::Config(format!("invalid hex end: '{}'", parts[1])))?;
    if start > end {
        return Err(Error::Config(format!(
            "start 0x{:04X} > end 0x{:04X}",
            start, end
        )));
    }
    Ok((start, end))
}

/// Scan a range of DIDs on a target ECU via Mode 22 reads.
/// Returns all DIDs that returned a positive response.
pub async fn scan_did_range(
    elm: &mut Elm327,
    ecu: &str,
    start: u16,
    end: u16,
    test_writable: bool,
) -> Result<Vec<DiscoveredDid>> {
    let total = (end as u32) - (start as u32) + 1;
    let mut found = Vec::new();

    elm.set_header(ecu).await?;

    // Use short timeout for faster scanning
    elm.at_command("ATST 19").await?;

    // Enter extended session (some DIDs only respond in extended)
    let session_req = uds::diagnostic_session_control(uds::SESSION_EXTENDED);
    match uds::send_uds(elm, &session_req).await {
        Ok(_) => {}
        Err(e) => {
            eprintln!("Warning: could not enter extended session: {}", e);
            eprintln!("Some DIDs may not respond in default session.");
        }
    }

    let mut keepalive_counter = 0u32;

    for did in start..=end {
        let progress = (did as u32) - (start as u32) + 1;
        print!(
            "\r  Scanning 0x{:04X} ({}/{})... found {} so far",
            did,
            progress,
            total,
            found.len()
        );
        std::io::stdout().flush().ok();

        // Send TesterPresent every 50 DIDs to keep the extended session alive
        keepalive_counter += 1;
        if keepalive_counter >= 50 {
            keepalive_counter = 0;
            let tp = uds::tester_present();
            let _ = uds::send_uds(elm, &tp).await;
        }

        let request = uds::read_data_by_identifier(did);
        match uds::send_uds(elm, &request).await {
            Ok(response) => {
                // Positive response: 0x62 + DID_HI + DID_LO + data
                if response.len() >= 3 && response[0] == 0x62 {
                    let data = response[3..].to_vec();

                    let writable = if test_writable && !data.is_empty() {
                        test_did_writable(elm, did, &data).await
                    } else {
                        false
                    };

                    println!(
                        "\r  FOUND: 0x{:04X} = {} ({} bytes){}",
                        did,
                        uds::hex_string(&data),
                        data.len(),
                        if writable { " [WRITABLE]" } else { "" }
                    );

                    found.push(DiscoveredDid {
                        did,
                        data,
                        writable,
                    });
                }
            }
            Err(_) => {
                // NRC (requestOutOfRange, serviceNotSupported, etc.) — DID doesn't exist, skip
            }
        }
    }

    println!(
        "\r  Scanned 0x{:04X}–0x{:04X}: {} DID(s) found.            ",
        start,
        end,
        found.len()
    );

    // Return to default session
    let _ = uds::send_uds(
        elm,
        &uds::diagnostic_session_control(uds::SESSION_DEFAULT),
    )
    .await;

    // Restore default timeout
    elm.at_command("ATST FF").await?;
    elm.set_header("7DF").await?;

    Ok(found)
}

/// Test if a DID is writable by writing its current value back.
/// This is non-destructive: we write exactly what we just read.
async fn test_did_writable(elm: &mut Elm327, did: u16, current_data: &[u8]) -> bool {
    let write_req = uds::write_data_by_identifier(did, current_data);
    match uds::send_uds(elm, &write_req).await {
        Ok(resp) => resp.first().copied() == Some(0x6E),
        Err(_) => false,
    }
}

/// Run a full scan with multiple ranges and save results to a file.
pub async fn scan_and_save(
    elm: &mut Elm327,
    ecu: &str,
    ranges: &[(u16, u16)],
    test_writable: bool,
    output_path: Option<&str>,
) -> Result<Vec<DiscoveredDid>> {
    let mut all_found = Vec::new();

    println!("DID Discovery Scan on ECU 0x{}", ecu);
    println!("Ranges: {}", ranges.iter()
        .map(|(s, e)| format!("0x{:04X}–0x{:04X}", s, e))
        .collect::<Vec<_>>()
        .join(", "));
    if test_writable {
        println!("Writability testing: ENABLED (writes current value back to test)");
    }
    println!();

    for (start, end) in ranges {
        let found = scan_did_range(elm, ecu, *start, *end, test_writable).await?;
        all_found.extend(found);
    }

    println!("\n=== Scan Summary ===");
    println!("ECU: 0x{}", ecu);
    println!("Total DIDs found: {}", all_found.len());

    if !all_found.is_empty() {
        println!();
        println!("{:<10} {:<6} {:<40} {}", "DID", "Bytes", "Value", "Writable");
        println!("{}", "-".repeat(70));
        for d in &all_found {
            println!(
                "0x{:04X}    {:<6} {:<40} {}",
                d.did,
                d.data.len(),
                uds::hex_string(&d.data),
                if d.writable { "YES" } else { "-" }
            );
        }
    }

    // Save to file if requested
    if let Some(path) = output_path {
        save_scan_results(ecu, &all_found, path)?;
        println!("\nResults saved to: {}", path);
    }

    Ok(all_found)
}

/// Save scan results to a TOML-compatible file that can be merged into toyota_dids.toml.
fn save_scan_results(ecu: &str, dids: &[DiscoveredDid], path: &str) -> Result<()> {
    let mut content = String::new();
    content.push_str(&format!(
        "# DID Scan Results — ECU 0x{}\n",
        ecu
    ));
    content.push_str(&format!(
        "# Generated: {}\n",
        chrono_timestamp()
    ));
    content.push_str(&format!("# Found {} DIDs\n\n", dids.len()));

    for d in dids {
        content.push_str("[[did]]\n");
        content.push_str(&format!("id = 0x{:04X}\n", d.did));
        content.push_str(&format!(
            "name = \"Unknown DID 0x{:04X}\"\n",
            d.did
        ));
        content.push_str("unit = \"raw\"\n");
        content.push_str("formula = \"A\"\n");
        content.push_str(&format!("ecu = \"{}\"\n", ecu));
        if d.writable {
            content.push_str("writable = true\n");
            content.push_str(&format!("data_length = {}\n", d.data.len()));
        }
        content.push_str(&format!(
            "description = \"Discovered by scan. Current value: {}\"\n",
            uds::hex_string(&d.data)
        ));
        content.push_str("category = \"discovered\"\n");
        content.push('\n');
    }

    std::fs::write(path, content)
        .map_err(|e| Error::Config(format!("failed to write scan results: {}", e)))?;
    Ok(())
}

fn chrono_timestamp() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    format!("{}", secs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_range_valid() {
        let (start, end) = parse_range("B000-B1FF").unwrap();
        assert_eq!(start, 0xB000);
        assert_eq!(end, 0xB1FF);
    }

    #[test]
    fn test_parse_range_with_0x_prefix() {
        let (start, end) = parse_range("0xF100-0xF1FF").unwrap();
        assert_eq!(start, 0xF100);
        assert_eq!(end, 0xF1FF);
    }

    #[test]
    fn test_parse_range_invalid_format() {
        assert!(parse_range("B000").is_err());
        assert!(parse_range("B000-B1FF-C000").is_err());
        assert!(parse_range("ZZZZ-FFFF").is_err());
    }

    #[test]
    fn test_parse_range_reversed() {
        assert!(parse_range("FFFF-0000").is_err());
    }

    #[test]
    fn test_toyota_bcm_ranges_valid() {
        for (_, start, end) in TOYOTA_BCM_RANGES {
            assert!(start <= end, "range 0x{:04X}-0x{:04X} is reversed", start, end);
        }
    }
}
