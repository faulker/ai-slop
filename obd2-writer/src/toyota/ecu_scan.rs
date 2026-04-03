use std::io::Write;

use crate::error::Result;
use crate::protocol::uds;
use crate::transport::elm327::Elm327;

pub struct EcuInfo {
    pub name: &'static str,
    pub tx_address: &'static str,
}

/// Known Toyota 3rd-gen Tacoma ECU addresses.
pub static KNOWN_ECUS: &[EcuInfo] = &[
    EcuInfo { name: "ECM (Engine)", tx_address: "7E0" },
    EcuInfo { name: "TCM (Transmission)", tx_address: "7E1" },
    EcuInfo { name: "ECM #2", tx_address: "7E2" },
    EcuInfo { name: "ABS/VSC", tx_address: "7B0" },
    EcuInfo { name: "BCM (Body)", tx_address: "750" },
    EcuInfo { name: "Gateway", tx_address: "760" },
    EcuInfo { name: "SRS (Airbag)", tx_address: "770" },
    EcuInfo { name: "EPS (Power Steering)", tx_address: "7A0" },
    EcuInfo { name: "A/C", tx_address: "7C0" },
    EcuInfo { name: "Instrument Cluster", tx_address: "7C1" },
    EcuInfo { name: "Combination Meter", tx_address: "701" },
    EcuInfo { name: "Parking Assist", tx_address: "790" },
    EcuInfo { name: "Headlamp Leveling", tx_address: "745" },
    EcuInfo { name: "Occupant Detection", tx_address: "780" },
];

/// Scan all known ECU addresses by sending TesterPresent.
/// Uses a short timeout to avoid long waits on non-responding ECUs.
pub async fn scan_ecus(elm: &mut Elm327) -> Result<Vec<&'static EcuInfo>> {
    let mut found = Vec::new();

    // Set short timeout for scanning (~100ms per ECU)
    elm.at_command("ATST 19").await?;

    for ecu in KNOWN_ECUS {
        print!("  Scanning {} (0x{})... ", ecu.name, ecu.tx_address);
        std::io::stdout().flush().ok();

        elm.set_header(ecu.tx_address).await?;
        let request = uds::tester_present();
        match uds::send_uds(elm, &request).await {
            Ok(resp) => {
                if !resp.is_empty() && resp[0] == 0x7E {
                    println!("FOUND");
                    found.push(ecu);
                } else {
                    println!("unexpected response: {}", uds::hex_string(&resp));
                }
            }
            Err(_) => {
                println!("no response");
            }
        }
    }

    // Restore default timeout and broadcast header
    elm.at_command("ATST FF").await?;
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
