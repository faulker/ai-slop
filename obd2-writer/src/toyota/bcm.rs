use crate::error::Result;
use crate::protocol::uds;
use crate::transport::elm327::Elm327;

/// Low-level write to a DID using UDS WriteDataByIdentifier (0x2E).
/// Prefer `write_safety::verified_write_did` for safe writes with validation.
///
/// Flow:
/// 1. Set target ECU header
/// 2. Enter Extended Diagnostic Session (0x10 0x03)
/// 3. Write DID
#[allow(dead_code)]
pub async fn write_did(elm: &mut Elm327, did_hex: &str, data_hex: &str, ecu: &str) -> Result<()> {
    let did = u16::from_str_radix(did_hex.trim_start_matches("0x"), 16)
        .map_err(|_| crate::error::Error::Config(format!("invalid DID hex: {}", did_hex)))?;

    let data = hex::decode(data_hex.replace(' ', ""))?;

    // Set target ECU
    elm.set_header(ecu).await?;
    println!("Target ECU: {}", ecu);

    // Enter Extended Diagnostic Session
    println!("Entering Extended Diagnostic Session...");
    let session_req = uds::diagnostic_session_control(uds::SESSION_EXTENDED);
    let session_resp = uds::send_uds(elm, &session_req).await?;
    println!("Session: {}", uds::hex_string(&session_resp));

    // Write DID
    println!("Writing DID 0x{:04X} = {}...", did, data_hex);
    let write_req = uds::write_data_by_identifier(did, &data);
    let write_resp = uds::send_uds(elm, &write_req).await?;

    // Positive response: 0x6E + DID_HI + DID_LO
    if write_resp[0] == 0x6E {
        if write_resp.len() >= 3 {
            let echoed_did = ((write_resp[1] as u16) << 8) | (write_resp[2] as u16);
            if echoed_did != did {
                return Err(crate::error::Error::Protocol(format!(
                    "write response DID mismatch: expected 0x{:04X}, got 0x{:04X}",
                    did, echoed_did
                )));
            }
        }
        println!("Write successful.");
    } else {
        return Err(crate::error::Error::Protocol(format!(
            "unexpected write response: {}",
            uds::hex_string(&write_resp)
        )));
    }

    // Return to default session
    let default_req = uds::diagnostic_session_control(uds::SESSION_DEFAULT);
    let _ = uds::send_uds(elm, &default_req).await;

    Ok(())
}

/// Perform a security access handshake.
/// Returns Ok(()) if security access was granted.
#[allow(dead_code)]
pub async fn security_access(elm: &mut Elm327, level: u8, key_provider: &dyn SecurityKeyProvider) -> Result<()> {
    // Request seed
    println!("Requesting security seed (level {})...", level);
    let seed_req = uds::security_access_request_seed(level);
    let seed_resp = uds::send_uds(elm, &seed_req).await?;

    // seed_resp: [0x67, level, seed_bytes...]
    if seed_resp.len() < 3 {
        return Err(crate::error::Error::Protocol("seed response too short".into()));
    }

    let seed = &seed_resp[2..];
    println!("Seed: {}", uds::hex_string(seed));

    // Check for zero seed (already unlocked)
    if seed.iter().all(|&b| b == 0) {
        println!("ECU already unlocked (zero seed).");
        return Ok(());
    }

    // Compute key
    let key = key_provider.compute_key(seed, level)?;
    println!("Sending key...");

    let key_req = uds::security_access_send_key(level, &key);
    let key_resp = uds::send_uds(elm, &key_req).await?;

    if key_resp[0] == 0x67 {
        println!("Security access granted.");
    } else {
        println!("Response: {}", uds::hex_string(&key_resp));
    }

    Ok(())
}

/// Trait for providing seed-key computation.
#[allow(dead_code)]
pub trait SecurityKeyProvider {
    fn compute_key(&self, seed: &[u8], level: u8) -> crate::error::Result<Vec<u8>>;
}

/// Manual key provider — prompts the user to enter the key.
/// Note: `compute_key` is sync but reads stdin via `spawn_blocking` internally
/// when called from the async `security_access` function.
#[allow(dead_code)]
pub struct ManualKeyProvider;

impl SecurityKeyProvider for ManualKeyProvider {
    fn compute_key(&self, seed: &[u8], level: u8) -> crate::error::Result<Vec<u8>> {
        println!("Seed (level {}): {}", level, uds::hex_string(seed));
        println!("Enter key as hex bytes (e.g., AA BB CC DD):");

        let mut input = String::new();
        std::io::stdin()
            .read_line(&mut input)
            .map_err(crate::error::Error::Io)?;

        let key = hex::decode(input.trim().replace(' ', ""))?;
        Ok(key)
    }
}

/// Async wrapper that reads the key without blocking the tokio runtime.
pub async fn security_access_async_key(
    elm: &mut Elm327,
    level: u8,
) -> crate::error::Result<()> {
    // Request seed
    println!("Requesting security seed (level {})...", level);
    let seed_req = uds::security_access_request_seed(level);
    let seed_resp = uds::send_uds(elm, &seed_req).await?;

    if seed_resp.len() < 3 {
        return Err(crate::error::Error::Protocol("seed response too short".into()));
    }

    let seed = seed_resp[2..].to_vec();
    println!("Seed: {}", uds::hex_string(&seed));

    if seed.iter().all(|&b| b == 0) {
        println!("ECU already unlocked (zero seed).");
        return Ok(());
    }

    // Read key from stdin on a blocking thread
    let key = tokio::task::spawn_blocking(move || -> crate::error::Result<Vec<u8>> {
        println!("Enter key as hex bytes (e.g., AA BB CC DD):");
        let mut input = String::new();
        std::io::stdin()
            .read_line(&mut input)
            .map_err(crate::error::Error::Io)?;
        let key = hex::decode(input.trim().replace(' ', ""))?;
        Ok(key)
    })
    .await
    .map_err(|e| crate::error::Error::Config(format!("key input task failed: {}", e)))??;

    println!("Sending key...");
    let key_req = uds::security_access_send_key(level, &key);
    let key_resp = uds::send_uds(elm, &key_req).await?;

    if key_resp[0] == 0x67 {
        println!("Security access granted.");
    } else {
        println!("Response: {}", uds::hex_string(&key_resp));
    }

    Ok(())
}
