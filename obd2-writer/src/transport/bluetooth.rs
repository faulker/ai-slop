//! Bluetooth Classic connection management for the OBDLink adapter on macOS.
//!
//! macOS only brings up an SPP link when a program opens the serial port, and
//! drops it shortly after the port closes, which makes a sleepy adapter look
//! dead. Forcing the baseband connection up first with `blueutil` makes the
//! serial port answer reliably and keeps the link alive between CLI commands.

use std::process::Command;
use std::time::Duration;

use serde::Deserialize;

use crate::error::{Error, Result};

const BLUEUTIL_URL: &str = "https://github.com/toy/blueutil";
const CONNECT_ATTEMPTS: usize = 3;

/// One entry from `blueutil --paired --format json`.
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct BtDevice {
    pub address: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub connected: bool,
    #[serde(default)]
    pub paired: bool,
}

/// Locate the blueutil binary, checking PATH and the usual Homebrew prefixes.
fn find_blueutil() -> Result<String> {
    for candidate in ["blueutil", "/opt/homebrew/bin/blueutil", "/usr/local/bin/blueutil"] {
        if Command::new(candidate).arg("--version").output().is_ok() {
            return Ok(candidate.to_string());
        }
    }
    Err(Error::Config(format!(
        "blueutil not found. Install it with `brew install blueutil` ({})",
        BLUEUTIL_URL
    )))
}

fn run_blueutil(bin: &str, args: &[&str]) -> Result<String> {
    let output = Command::new(bin)
        .args(args)
        .output()
        .map_err(|e| Error::Config(format!("failed to run blueutil: {}", e)))?;
    if !output.status.success() {
        return Err(Error::Config(format!(
            "blueutil {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// Parse the JSON array blueutil prints for `--paired` or `--inquiry`.
pub fn parse_device_list(json: &str) -> Result<Vec<BtDevice>> {
    serde_json::from_str(json)
        .map_err(|e| Error::Config(format!("failed to parse blueutil output: {}", e)))
}

/// Pick the OBDLink adapter out of a device list by name.
pub fn find_obdlink(devices: &[BtDevice]) -> Option<&BtDevice> {
    devices.iter().find(|d| d.name.to_lowercase().contains("obdlink"))
}

/// Find the paired OBDLink and make sure its Bluetooth link is up, retrying a
/// few times because a sleeping adapter needs a moment after its button is
/// pressed. If it is not paired yet, run a discovery scan and pair it.
pub fn ensure_obdlink_connected() -> Result<BtDevice> {
    let bin = find_blueutil()?;

    if run_blueutil(&bin, &["--power"])?.trim() != "1" {
        return Err(Error::Config("Bluetooth is powered off".into()));
    }

    let paired = parse_device_list(&run_blueutil(&bin, &["--paired", "--format", "json"])?)?;
    let device = match find_obdlink(&paired) {
        Some(d) => d.clone(),
        None => pair_obdlink(&bin)?,
    };

    if device.connected {
        println!("Bluetooth: {} already connected", device.name);
        return Ok(device);
    }

    for attempt in 1..=CONNECT_ATTEMPTS {
        println!(
            "Bluetooth: connecting to {} (attempt {}/{})...",
            device.name, attempt, CONNECT_ATTEMPTS
        );
        if run_blueutil(&bin, &["--connect", &device.address]).is_ok()
            && run_blueutil(&bin, &["--is-connected", &device.address])?.trim() == "1"
        {
            return Ok(BtDevice { connected: true, ..device });
        }
        if attempt < CONNECT_ATTEMPTS {
            std::thread::sleep(Duration::from_secs(3));
        }
    }

    Err(Error::Config(format!(
        "could not connect to {}. Is the ignition on and the adapter awake (LED blinking)? \
         Press its button to wake it, and make sure no phone app is holding the connection.",
        device.name
    )))
}

/// Discover nearby devices and pair the OBDLink. The adapter must be in
/// pairing mode (hold its button until the LED blinks) for this to succeed.
fn pair_obdlink(bin: &str) -> Result<BtDevice> {
    println!("Bluetooth: no paired OBDLink found. Scanning for 10s (put the adapter in pairing mode)...");
    let found = parse_device_list(&run_blueutil(bin, &["--inquiry", "10", "--format", "json"])?)?;
    let device = find_obdlink(&found)
        .cloned()
        .ok_or_else(|| Error::Config("no OBDLink adapter discovered".into()))?;
    println!("Bluetooth: pairing with {}...", device.name);
    run_blueutil(bin, &["--pair", &device.address])?;
    Ok(BtDevice { paired: true, ..device })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"[
        {"address":"10-94-bb-ac-c8-2e","favourite":false,"name":"Trackpad","connected":false,"paired":true},
        {"address":"80-04-3e-80-13-ca","paired":true,"RSSI":-49,"connected":true,"name":"OBDLink MX+ 23907","slave":false}
    ]"#;

    #[test]
    fn test_parse_device_list_tolerates_extra_fields() {
        let devices = parse_device_list(SAMPLE).unwrap();
        assert_eq!(devices.len(), 2);
        assert_eq!(devices[1].address, "80-04-3e-80-13-ca");
        assert!(devices[1].connected);
    }

    #[test]
    fn test_parse_device_list_missing_name() {
        let devices = parse_device_list(r#"[{"address":"aa-bb","paired":true}]"#).unwrap();
        assert_eq!(devices[0].name, "");
        assert!(!devices[0].connected);
    }

    #[test]
    fn test_find_obdlink_by_name() {
        let devices = parse_device_list(SAMPLE).unwrap();
        assert_eq!(find_obdlink(&devices).unwrap().name, "OBDLink MX+ 23907");
        assert!(find_obdlink(&devices[..1]).is_none());
    }

    #[test]
    fn test_parse_device_list_rejects_garbage() {
        assert!(parse_device_list("not json").is_err());
    }
}
