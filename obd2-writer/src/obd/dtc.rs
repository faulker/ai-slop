use crate::error::{Error, Result};
use crate::protocol::isotp;
use crate::transport::elm327::Elm327;

/// Decode a 2-byte DTC into the standard format (e.g., P0300, C0100, B1234, U0100).
fn decode_dtc(byte1: u8, byte2: u8) -> String {
    let prefix = match (byte1 >> 6) & 0x03 {
        0 => 'P', // Powertrain
        1 => 'C', // Chassis
        2 => 'B', // Body
        3 => 'U', // Network
        _ => '?',
    };

    let digit1 = (byte1 >> 4) & 0x03;
    let digit2 = byte1 & 0x0F;
    let digit3 = (byte2 >> 4) & 0x0F;
    let digit4 = byte2 & 0x0F;

    format!("{}{}{:X}{:X}{:X}", prefix, digit1, digit2, digit3, digit4)
}

/// Read DTCs using Mode 03 (show stored DTCs).
pub async fn read_dtcs(elm: &mut Elm327) -> Result<()> {
    let response = elm.send_obd("03").await?;
    let payload = isotp::reassemble(&response.lines)?;

    if payload.is_empty() {
        println!("No DTCs stored.");
        return Ok(());
    }

    // Mode 03 response: [0x43, num_dtcs, DTC1_byte1, DTC1_byte2, DTC2_byte1, ...]
    if payload[0] != 0x43 {
        return Err(Error::Protocol(format!(
            "unexpected DTC response SID: 0x{:02X}",
            payload[0]
        )));
    }

    if payload.len() < 2 {
        println!("No DTCs stored.");
        return Ok(());
    }

    // First byte after SID is count of DTCs
    let num_dtcs = payload[1] as usize;
    let dtc_bytes = &payload[2..];

    if num_dtcs == 0 {
        println!("No DTCs stored.");
        return Ok(());
    }

    println!("Stored DTCs ({}):", num_dtcs);
    for i in 0..num_dtcs {
        let offset = i * 2;
        if offset + 1 >= dtc_bytes.len() {
            break;
        }
        let code = decode_dtc(dtc_bytes[offset], dtc_bytes[offset + 1]);
        println!("  {}", code);
    }

    Ok(())
}

/// Clear all DTCs using OBD Mode 04 (consistent with Mode 03 read).
pub async fn clear_dtcs(elm: &mut Elm327) -> Result<()> {
    println!("Clearing all DTCs...");

    let response = elm.send_obd("04").await?;
    let payload = isotp::reassemble(&response.lines)?;

    if !payload.is_empty() && payload[0] == 0x44 {
        println!("DTCs cleared successfully.");
    } else {
        let hex = payload.iter().map(|b| format!("{:02X}", b)).collect::<Vec<_>>().join(" ");
        println!("Unexpected response: {}", hex);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_decode_dtc_powertrain() {
        assert_eq!(decode_dtc(0x03, 0x00), "P0300");
    }

    #[test]
    fn test_decode_dtc_body() {
        assert_eq!(decode_dtc(0x81, 0x23), "B0123");
    }

    #[test]
    fn test_decode_dtc_chassis() {
        assert_eq!(decode_dtc(0x41, 0x00), "C0100");
    }

    #[test]
    fn test_decode_dtc_network() {
        assert_eq!(decode_dtc(0xC1, 0x00), "U0100");
    }
}
