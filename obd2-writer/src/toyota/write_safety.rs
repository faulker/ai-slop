use crate::error::{Error, Result};
use crate::protocol::uds;
use crate::toyota::backup::BackupStore;
use crate::toyota::enhanced_pids;
use crate::transport::elm327::Elm327;

/// Whitelist-based validation for DID writes.
pub struct DidWhitelist {
    dids: Vec<enhanced_pids::DidDefinition>,
}

impl DidWhitelist {
    pub fn load() -> Result<Self> {
        let dids = enhanced_pids::cached_dids().to_vec();
        Ok(Self { dids })
    }

    /// Validate that a DID is whitelisted and the data meets constraints.
    pub fn validate(&self, did: u16, data: &[u8]) -> Result<()> {
        let def = self
            .dids
            .iter()
            .find(|d| d.id == did)
            .ok_or(Error::DidNotWhitelisted { did })?;

        if !def.writable {
            return Err(Error::DidNotWhitelisted { did });
        }

        if let Some(expected_len) = def.data_length {
            if data.len() != expected_len {
                return Err(Error::ValueOutOfRange {
                    did,
                    detail: format!(
                        "expected {} bytes, got {}",
                        expected_len,
                        data.len()
                    ),
                });
            }
        }

        if def.min_value.is_some() || def.max_value.is_some() {
            let value = data.iter().fold(0u64, |acc, &b| (acc << 8) | b as u64);
            if let Some(min) = def.min_value {
                if value < min {
                    return Err(Error::ValueOutOfRange {
                        did,
                        detail: format!("value 0x{:X} below minimum 0x{:X}", value, min),
                    });
                }
            }
            if let Some(max) = def.max_value {
                if value > max {
                    return Err(Error::ValueOutOfRange {
                        did,
                        detail: format!("value 0x{:X} above maximum 0x{:X}", value, max),
                    });
                }
            }
        }

        Ok(())
    }
}

/// Read the current value of a DID. Returns the data bytes (after the 3-byte header).
async fn read_did_value(elm: &mut Elm327, did: u16) -> Result<Vec<u8>> {
    let request = uds::read_data_by_identifier(did);
    let response = uds::send_uds(elm, &request).await?;
    if response.len() < 3 {
        return Err(Error::Protocol("read response too short".into()));
    }
    Ok(response[3..].to_vec())
}

/// Safe write with read-back verification, whitelist checking, and automatic backup.
///
/// Flow:
/// 1. Set ECU header, enter extended session
/// 2. Read current DID value (also validates DID exists) — dry-run stops here
/// 3. Validate against whitelist
/// 4. Backup original value
/// 5. Write new value
/// 6. Read back and verify
/// 7. On verification failure: rollback to original value
/// 8. Return to default session
pub async fn verified_write_did(
    elm: &mut Elm327,
    did_hex: &str,
    data_hex: &str,
    ecu: &str,
    dry_run: bool,
) -> Result<()> {
    let did = u16::from_str_radix(did_hex.trim_start_matches("0x"), 16)
        .map_err(|_| Error::Config(format!("invalid DID hex: {}", did_hex)))?;
    let data = hex::decode(data_hex.replace(' ', ""))?;

    // Set target ECU
    elm.set_header(ecu).await?;
    println!("Target ECU: {}", ecu);

    // Enter extended session
    println!("Entering Extended Diagnostic Session...");
    let session_req = uds::diagnostic_session_control(uds::SESSION_EXTENDED);
    let session_resp = uds::send_uds(elm, &session_req).await?;
    println!("Session: {}", uds::hex_string(&session_resp));

    // Step 1: Read current value (validates DID exists)
    println!("Reading current value of DID 0x{:04X}...", did);
    let original_data = match read_did_value(elm, did).await {
        Ok(d) => {
            println!("Current value: {}", uds::hex_string(&d));
            d
        }
        Err(e) => {
            // Return to default session before reporting
            let _ = uds::send_uds(
                elm,
                &uds::diagnostic_session_control(uds::SESSION_DEFAULT),
            )
            .await;
            return Err(Error::Protocol(format!(
                "DID 0x{:04X} not readable ({}). Aborting write.",
                did, e
            )));
        }
    };

    // Dry-run: report what would happen and stop
    if dry_run {
        println!();
        println!("=== DRY RUN ===");
        println!("DID 0x{:04X} exists on ECU {}", did, ecu);
        println!("Current value: {}", uds::hex_string(&original_data));
        println!("Would write:   {}", uds::hex_string(&data));

        // Check whitelist
        let whitelist = DidWhitelist::load()?;
        match whitelist.validate(did, &data) {
            Ok(()) => println!("Whitelist:     PASS"),
            Err(e) => println!("Whitelist:     FAIL ({})", e),
        }

        let _ = uds::send_uds(
            elm,
            &uds::diagnostic_session_control(uds::SESSION_DEFAULT),
        )
        .await;
        println!("=== No changes made ===");
        return Ok(());
    }

    // Step 2: Whitelist validation
    let whitelist = DidWhitelist::load()?;
    whitelist.validate(did, &data)?;
    println!("Whitelist check: PASS");

    // Step 3: Backup original value
    let mut backup_store = BackupStore::load()?;
    backup_store.record(ecu, did, &original_data)?;
    backup_store.save()?;
    println!("Original value backed up.");

    // Step 4: Write
    println!("Writing DID 0x{:04X} = {}...", did, uds::hex_string(&data));
    let write_req = uds::write_data_by_identifier(did, &data);
    let write_resp = uds::send_uds(elm, &write_req).await?;

    if write_resp[0] != 0x6E {
        let _ = uds::send_uds(
            elm,
            &uds::diagnostic_session_control(uds::SESSION_DEFAULT),
        )
        .await;
        return Err(Error::Protocol(format!(
            "unexpected write response: {}",
            uds::hex_string(&write_resp)
        )));
    }

    // Verify DID echo in response
    if write_resp.len() >= 3 {
        let echoed_did = ((write_resp[1] as u16) << 8) | (write_resp[2] as u16);
        if echoed_did != did {
            let _ = uds::send_uds(
                elm,
                &uds::diagnostic_session_control(uds::SESSION_DEFAULT),
            )
            .await;
            return Err(Error::Protocol(format!(
                "write response DID mismatch: expected 0x{:04X}, got 0x{:04X}",
                did, echoed_did
            )));
        }
    }

    // Step 5: Read-back verification
    println!("Verifying write...");
    let readback = read_did_value(elm, did).await?;
    println!("Read-back value: {}", uds::hex_string(&readback));

    let matches = if readback.len() >= data.len() {
        readback[..data.len()] == data[..]
    } else {
        false
    };

    if !matches {
        // Rollback
        println!("VERIFICATION FAILED — attempting rollback...");
        let rollback_req = uds::write_data_by_identifier(did, &original_data);
        match uds::send_uds(elm, &rollback_req).await {
            Ok(resp) if resp[0] == 0x6E => {
                println!("Rollback successful — original value restored.");
            }
            Ok(resp) => {
                let _ = uds::send_uds(
                    elm,
                    &uds::diagnostic_session_control(uds::SESSION_DEFAULT),
                )
                .await;
                return Err(Error::RollbackFailed(format!(
                    "unexpected rollback response: {}. ECU may be in an inconsistent state.",
                    uds::hex_string(&resp)
                )));
            }
            Err(e) => {
                let _ = uds::send_uds(
                    elm,
                    &uds::diagnostic_session_control(uds::SESSION_DEFAULT),
                )
                .await;
                return Err(Error::RollbackFailed(format!(
                    "rollback write failed: {}. ECU may be in an inconsistent state.",
                    e
                )));
            }
        }

        let _ = uds::send_uds(
            elm,
            &uds::diagnostic_session_control(uds::SESSION_DEFAULT),
        )
        .await;

        return Err(Error::WriteVerificationFailed {
            expected: uds::hex_string(&data),
            actual: uds::hex_string(&readback),
        });
    }

    println!("Write verified successfully.");

    // Return to default session
    let _ = uds::send_uds(
        elm,
        &uds::diagnostic_session_control(uds::SESSION_DEFAULT),
    )
    .await;

    Ok(())
}

/// Restore a previously backed-up DID value.
pub async fn restore_did(elm: &mut Elm327, did_hex: &str, ecu: &str) -> Result<()> {
    let did = u16::from_str_radix(did_hex.trim_start_matches("0x"), 16)
        .map_err(|_| Error::Config(format!("invalid DID hex: {}", did_hex)))?;

    let store = BackupStore::load()?;
    let entry = store.get(ecu, did).ok_or_else(|| {
        Error::Config(format!(
            "no backup found for DID 0x{:04X} on ECU {}",
            did, ecu
        ))
    })?;

    let backup_data_hex = entry.original_data.clone();
    println!(
        "Restoring DID 0x{:04X} on ECU {} to backed-up value: {}",
        did, ecu, backup_data_hex
    );

    // Use verified_write_did for the restore (it will create its own backup of the current value,
    // but that won't overwrite the existing one since record() preserves the first backup)
    let data_hex = backup_data_hex.replace(' ', "");
    verified_write_did(elm, did_hex, &data_hex, ecu, false).await
}

/// Print all backup entries.
pub fn print_backups() -> Result<()> {
    let store = BackupStore::load()?;
    let entries = store.list();
    if entries.is_empty() {
        println!("No backups stored.");
        return Ok(());
    }
    println!("{:<12} {:<8} {:<20} {}", "ECU:DID", "DID", "Original Value", "Timestamp");
    println!("{}", "-".repeat(60));
    for (key, entry) in entries {
        println!(
            "{:<12} {:<8} {:<20} {}",
            key, entry.did, entry.original_data, entry.timestamp
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_whitelist_rejects_unknown_did() {
        let whitelist = DidWhitelist { dids: vec![] };
        assert!(whitelist.validate(0xFFFF, &[0x01]).is_err());
    }

    #[test]
    fn test_whitelist_rejects_non_writable() {
        let whitelist = DidWhitelist {
            dids: vec![enhanced_pids::DidDefinition {
                id: 0x0100,
                name: "Test".into(),
                unit: "".into(),
                formula: "A".into(),
                ecu: "7E0".into(),
                writable: false,
                data_length: None,
                min_value: None,
                max_value: None,
            }],
        };
        assert!(whitelist.validate(0x0100, &[0x01]).is_err());
    }

    #[test]
    fn test_whitelist_accepts_writable() {
        let whitelist = DidWhitelist {
            dids: vec![enhanced_pids::DidDefinition {
                id: 0x1234,
                name: "Test".into(),
                unit: "".into(),
                formula: "A".into(),
                ecu: "7E0".into(),
                writable: true,
                data_length: None,
                min_value: None,
                max_value: None,
            }],
        };
        assert!(whitelist.validate(0x1234, &[0x01]).is_ok());
    }

    #[test]
    fn test_whitelist_checks_data_length() {
        let whitelist = DidWhitelist {
            dids: vec![enhanced_pids::DidDefinition {
                id: 0x1234,
                name: "Test".into(),
                unit: "".into(),
                formula: "A".into(),
                ecu: "7E0".into(),
                writable: true,
                data_length: Some(2),
                min_value: None,
                max_value: None,
            }],
        };
        assert!(whitelist.validate(0x1234, &[0x01]).is_err());
        assert!(whitelist.validate(0x1234, &[0x01, 0x02]).is_ok());
    }

    #[test]
    fn test_whitelist_checks_value_range() {
        let whitelist = DidWhitelist {
            dids: vec![enhanced_pids::DidDefinition {
                id: 0x1234,
                name: "Test".into(),
                unit: "".into(),
                formula: "A".into(),
                ecu: "7E0".into(),
                writable: true,
                data_length: Some(1),
                min_value: Some(0x00),
                max_value: Some(0x0F),
            }],
        };
        assert!(whitelist.validate(0x1234, &[0x05]).is_ok());
        assert!(whitelist.validate(0x1234, &[0x10]).is_err());
    }
}
