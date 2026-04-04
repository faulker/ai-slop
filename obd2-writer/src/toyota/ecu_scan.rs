use std::io::Write;

use crate::error::Result;
use crate::transport::elm327::Elm327;

pub struct EcuInfo {
    pub name: &'static str,
    pub tx_address: &'static str,
}

/// Known Toyota 3rd-gen Tacoma ECU addresses.
/// Sources: commaai/opendbc, Toyota service manual, community verification.
pub static KNOWN_ECUS: &[EcuInfo] = &[
    EcuInfo { name: "ECM (Engine)", tx_address: "7E0" },
    EcuInfo { name: "TCM (Transmission)", tx_address: "7E1" },
    EcuInfo { name: "TCM #2 (Transmission)", tx_address: "701" },
    EcuInfo { name: "ECM #2", tx_address: "7E2" },
    EcuInfo { name: "ABS/VSC", tx_address: "7B0" },
    EcuInfo { name: "ABS/Brake #2", tx_address: "730" },
    EcuInfo { name: "BCM (Body/Gateway)", tx_address: "750" },
    EcuInfo { name: "Gateway", tx_address: "760" },
    EcuInfo { name: "SRS (Airbag)", tx_address: "780" },
    EcuInfo { name: "SRS #2 (Airbag)", tx_address: "784" },
    EcuInfo { name: "Parking Assist", tx_address: "790" },
    EcuInfo { name: "EPS (Power Steering)", tx_address: "7A0" },
    EcuInfo { name: "Steering Angle Sensor", tx_address: "7B3" },
    EcuInfo { name: "Combination Meter", tx_address: "7C0" },
    EcuInfo { name: "Instrument Cluster", tx_address: "7C1" },
    EcuInfo { name: "HVAC", tx_address: "7C4" },
    EcuInfo { name: "Headlamp Leveling", tx_address: "745" },
];

/// Scan all known ECU addresses by sending TesterPresent.
/// Uses a short timeout to avoid long waits on non-responding ECUs.
pub async fn scan_ecus(elm: &mut Elm327) -> Result<Vec<&'static EcuInfo>> {
    let mut found = Vec::new();

    // Set moderate timeout for scanning (~300ms per ECU)
    elm.at_command("ATST 4B").await?;

    // Warmup: broadcast TesterPresent to ensure CAN bus is active
    elm.set_header("7DF").await?;
    let _ = elm.send_obd("3E 00").await;

    for ecu in KNOWN_ECUS {
        print!("  Scanning {} (0x{})... ", ecu.name, ecu.tx_address);
        std::io::stdout().flush().ok();

        elm.set_header(ecu.tx_address).await?;

        // Explicitly set expected receive address (TX + 8)
        let tx_val = u16::from_str_radix(ecu.tx_address, 16).unwrap_or(0);
        let rx_addr = format!("{:03X}", tx_val + 8);
        let _ = elm.at_command(&format!("ATCRA {}", rx_addr)).await;

        // Use send_obd directly — a negative UDS response still means ECU is present
        match elm.send_obd("3E 00").await {
            Ok(resp) if !resp.lines.is_empty() => {
                println!("FOUND");
                found.push(ecu);
            }
            Ok(_) => {
                println!("empty response");
            }
            Err(_) => {
                println!("no response");
            }
        }
    }

    // Restore default timeout and broadcast header
    elm.at_command("ATST FF").await?;
    elm.at_command("ATAR").await?;
    elm.set_header("7DF").await?;

    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_known_ecus_not_empty() {
        assert!(!KNOWN_ECUS.is_empty());
    }

    #[test]
    fn test_known_ecus_have_valid_hex_addresses() {
        for ecu in KNOWN_ECUS {
            assert!(
                !ecu.tx_address.is_empty(),
                "ECU {} has empty address",
                ecu.name
            );
            assert!(
                ecu.tx_address.chars().all(|c| c.is_ascii_hexdigit()),
                "ECU {} has non-hex address: {}",
                ecu.name,
                ecu.tx_address
            );
        }
    }
}
