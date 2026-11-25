use std::io::Write;

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::protocol::uds;
use crate::toyota::did_scan;
use crate::transport::elm327::Elm327;

/// One DID's value at capture time. `did` and `data` are hex strings so the
/// JSON is readable by eye ("0100", "01 A0").
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SnapshotEntry {
    pub did: String,
    pub data: String,
}

/// All DIDs read from one ECU in a single pass.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Snapshot {
    pub label: String,
    pub timestamp: u64,
    pub ecu: String,
    pub entries: Vec<SnapshotEntry>,
}

impl Snapshot {
    /// Build a snapshot from raw (did, data) pairs.
    pub fn new(label: &str, ecu: &str, values: Vec<(u16, Vec<u8>)>) -> Self {
        Self {
            label: label.to_string(),
            timestamp: unix_now(),
            ecu: ecu.to_uppercase(),
            entries: values
                .into_iter()
                .map(|(did, data)| SnapshotEntry {
                    did: format!("{:04X}", did),
                    data: uds::hex_string(&data),
                })
                .collect(),
        }
    }

    /// DIDs in this snapshot, parsed back to u16.
    pub fn dids(&self) -> Result<Vec<u16>> {
        self.entries
            .iter()
            .map(|e| {
                u16::from_str_radix(&e.did, 16)
                    .map_err(|_| Error::Config(format!("invalid DID in snapshot: '{}'", e.did)))
            })
            .collect()
    }

    pub fn load(path: &str) -> Result<Self> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| Error::Config(format!("failed to read {}: {}", path, e)))?;
        serde_json::from_str(&content)
            .map_err(|e| Error::Config(format!("failed to parse {}: {}", path, e)))
    }

    pub fn save(&self, path: &str) -> Result<()> {
        let content = serde_json::to_string_pretty(self)
            .map_err(|e| Error::Config(format!("failed to serialize snapshot: {}", e)))?;
        std::fs::write(path, content)
            .map_err(|e| Error::Config(format!("failed to write {}: {}", path, e)))
    }
}

/// Discover DIDs in the given ranges on `ecu` and record their values as a
/// baseline snapshot. Writability testing is deliberately not exposed here.
pub async fn scan_to_snapshot(
    elm: &mut Elm327,
    ecu: &str,
    ranges: &[(u16, u16)],
    label: &str,
) -> Result<Snapshot> {
    let mut values = Vec::new();
    for (start, end) in ranges {
        let found = did_scan::scan_did_range(elm, ecu, *start, *end, false).await?;
        values.extend(found.into_iter().map(|d| (d.did, d.data)));
    }
    Ok(Snapshot::new(label, ecu, values))
}

/// Re-read exactly the DIDs listed in `baseline` on the same ECU.
pub async fn resnapshot(elm: &mut Elm327, baseline: &Snapshot, label: &str) -> Result<Snapshot> {
    let dids = baseline.dids()?;
    let values = read_did_list(elm, &baseline.ecu, &dids).await?;
    Ok(Snapshot::new(label, &baseline.ecu, values))
}

/// Read an explicit list of DIDs via Mode 22 in an extended session.
/// DIDs that return a negative response are omitted from the result.
pub async fn read_did_list(
    elm: &mut Elm327,
    ecu: &str,
    dids: &[u16],
) -> Result<Vec<(u16, Vec<u8>)>> {
    let mut values = Vec::with_capacity(dids.len());

    elm.set_header(ecu).await?;
    elm.at_command("ATST 19").await?;

    let session_req = uds::diagnostic_session_control(uds::SESSION_EXTENDED);
    if let Err(e) = uds::send_uds(elm, &session_req).await {
        eprintln!("Warning: could not enter extended session: {}", e);
    }

    for (i, did) in dids.iter().enumerate() {
        print!("\r  Reading 0x{:04X} ({}/{})", did, i + 1, dids.len());
        std::io::stdout().flush().ok();

        // Keep the extended session alive during long lists.
        if i % 50 == 49 {
            let _ = uds::send_uds(elm, &uds::tester_present()).await;
        }

        let request = uds::read_data_by_identifier(*did);
        if let Ok(response) = uds::send_uds(elm, &request).await {
            if response.len() >= 3 && response[0] == 0x62 {
                values.push((*did, response[3..].to_vec()));
            }
        }
    }
    println!("\r  Read {} of {} DID(s).                    ", values.len(), dids.len());

    let _ = uds::send_uds(elm, &uds::diagnostic_session_control(uds::SESSION_DEFAULT)).await;
    elm.at_command("ATST FF").await?;
    elm.set_header("7DF").await?;

    Ok(values)
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_snapshot_formats_hex() {
        let snap = Snapshot::new("before", "7e0", vec![(0x0100, vec![0x01, 0xA0])]);
        assert_eq!(snap.ecu, "7E0");
        assert_eq!(snap.entries[0].did, "0100");
        assert_eq!(snap.entries[0].data, "01 A0");
        assert_eq!(snap.dids().unwrap(), vec![0x0100]);
    }

    #[test]
    fn test_snapshot_json_round_trip() {
        let snap = Snapshot::new("before", "787", vec![(0xF190, vec![0x41]), (0x0001, vec![])]);
        let json = serde_json::to_string(&snap).unwrap();
        let back: Snapshot = serde_json::from_str(&json).unwrap();
        assert_eq!(back, snap);
    }

    #[test]
    fn test_snapshot_rejects_bad_did() {
        let snap = Snapshot {
            label: "x".into(),
            timestamp: 0,
            ecu: "7E0".into(),
            entries: vec![SnapshotEntry { did: "ZZZZ".into(), data: String::new() }],
        };
        assert!(snap.dids().is_err());
    }
}
