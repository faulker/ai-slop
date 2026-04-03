use crate::error::{self, Error, Result};
use crate::protocol::isotp;
use crate::transport::elm327::Elm327;

// UDS Service IDs
pub const DIAGNOSTIC_SESSION_CONTROL: u8 = 0x10;
#[allow(dead_code)]
pub const ECU_RESET: u8 = 0x11;
#[allow(dead_code)]
pub const CLEAR_DTC: u8 = 0x14;
#[allow(dead_code)]
pub const READ_DTC_INFO: u8 = 0x19;
pub const READ_DATA_BY_ID: u8 = 0x22;
pub const SECURITY_ACCESS: u8 = 0x27;
pub const WRITE_DATA_BY_ID: u8 = 0x2E;
#[allow(dead_code)]
pub const ROUTINE_CONTROL: u8 = 0x31;
pub const TESTER_PRESENT: u8 = 0x3E;
pub const NEGATIVE_RESPONSE: u8 = 0x7F;

// Session types
pub const SESSION_DEFAULT: u8 = 0x01;
pub const SESSION_PROGRAMMING: u8 = 0x02;
pub const SESSION_EXTENDED: u8 = 0x03;

// --- Request builders ---

pub fn diagnostic_session_control(session: u8) -> Vec<u8> {
    vec![DIAGNOSTIC_SESSION_CONTROL, session]
}

pub fn security_access_request_seed(level: u8) -> Vec<u8> {
    vec![SECURITY_ACCESS, level]
}

pub fn security_access_send_key(level: u8, key: &[u8]) -> Vec<u8> {
    let mut cmd = vec![SECURITY_ACCESS, level + 1];
    cmd.extend_from_slice(key);
    cmd
}

pub fn read_data_by_identifier(did: u16) -> Vec<u8> {
    vec![READ_DATA_BY_ID, (did >> 8) as u8, (did & 0xFF) as u8]
}

pub fn write_data_by_identifier(did: u16, data: &[u8]) -> Vec<u8> {
    let mut cmd = vec![WRITE_DATA_BY_ID, (did >> 8) as u8, (did & 0xFF) as u8];
    cmd.extend_from_slice(data);
    cmd
}

#[allow(dead_code)]
pub fn clear_all_dtcs() -> Vec<u8> {
    vec![CLEAR_DTC, 0xFF, 0xFF, 0xFF]
}

#[allow(dead_code)]
pub fn read_dtc_by_status_mask(mask: u8) -> Vec<u8> {
    vec![READ_DTC_INFO, 0x02, mask]
}

#[allow(dead_code)]
pub fn routine_control(sub_function: u8, routine_id: u16, data: &[u8]) -> Vec<u8> {
    let mut cmd = vec![
        ROUTINE_CONTROL,
        sub_function,
        (routine_id >> 8) as u8,
        (routine_id & 0xFF) as u8,
    ];
    cmd.extend_from_slice(data);
    cmd
}

pub fn tester_present() -> Vec<u8> {
    vec![TESTER_PRESENT, 0x00]
}

// --- Response parsing ---

/// Send a UDS request and parse the response.
/// Handles negative responses and responsePending (NRC 0x78).
pub async fn send_uds(elm: &mut Elm327, request: &[u8]) -> Result<Vec<u8>> {
    let hex_cmd = request
        .iter()
        .map(|b| format!("{:02X}", b))
        .collect::<Vec<_>>()
        .join(" ");

    let response = elm.send_obd(&hex_cmd).await?;
    let mut payload = isotp::reassemble(&response.lines)?;

    // Handle responsePending (NRC 0x78): ECU acknowledged the request but
    // needs more time. Read the pending response from the port without
    // retransmitting the request (retransmitting can cause ECU lockout).
    for _ in 0..20 {
        if payload.is_empty() {
            return Err(Error::Protocol("empty UDS response".into()));
        }

        if payload[0] != NEGATIVE_RESPONSE {
            break;
        }

        if payload.len() < 3 {
            return Err(Error::Protocol("truncated negative response".into()));
        }

        let nrc = payload[2];
        if nrc != 0x78 {
            break; // Not responsePending — fall through to error handling below
        }

        // NRC 0x78: wait briefly, then read the next response (do NOT resend)
        tracing::debug!("NRC 0x78 responsePending — waiting for ECU...");
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        let pending = elm.read_pending_response().await?;
        payload = isotp::reassemble(&pending.lines)?;
    }

    if payload.is_empty() {
        return Err(Error::Protocol("empty UDS response".into()));
    }

    // Check for negative response
    if payload[0] == NEGATIVE_RESPONSE {
        if payload.len() < 3 {
            return Err(Error::Protocol("truncated negative response".into()));
        }
        let service = payload[1];
        let nrc = payload[2];
        let nrc_name = error::nrc_name(nrc).to_string();
        return Err(Error::UdsNegativeResponse {
            service,
            nrc,
            nrc_name,
        });
    }

    // Positive response: service ID + 0x40
    let expected_response_sid = request[0] + 0x40;
    if payload[0] != expected_response_sid {
        return Err(Error::Protocol(format!(
            "unexpected response SID: expected 0x{:02X}, got 0x{:02X}",
            expected_response_sid, payload[0]
        )));
    }

    Ok(payload)
}

/// Format bytes as a hex string for display.
pub fn hex_string(data: &[u8]) -> String {
    data.iter().map(|b| format!("{:02X}", b)).collect::<Vec<_>>().join(" ")
}
