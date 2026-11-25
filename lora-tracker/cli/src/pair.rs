//! Pairing: one shared key written into a tracker's and a gateway's `device.toml`.
//!
//! Edits go through `toml_edit` so the user's comments and layout survive.

use anyhow::{bail, Context, Result};
use toml_edit::{value, ArrayOfTables, DocumentMut, Item, Table};
use tracker_proto::config as pc;

use crate::device_toml::{DeviceToml, Role};

/// Where the shared key comes from.
pub enum KeyChoice {
    /// Keep the tracker's current key, generating one only if it has none.
    Existing,
    /// Use this exact key (from `--key`).
    Explicit([u8; 32]),
    /// Replace any existing key with this freshly generated one.
    New([u8; 32]),
}

/// Parses a user-supplied key: 64 hex characters, spaces and colons allowed.
pub fn parse_key(s: &str) -> Result<[u8; 32]> {
    let cleaned: String = s.chars().filter(|c| !c.is_whitespace() && *c != ':').collect();
    let bytes = hex::decode(&cleaned).context("key must be hex (0-9, a-f)")?;
    let len = bytes.len();
    bytes
        .try_into()
        .map_err(|_| anyhow::anyhow!("key must be 32 bytes (64 hex characters), got {len} bytes"))
}

/// Parses and validates a device.toml, returning both the typed view and the editable doc.
fn load(name: &str, text: &str) -> Result<(DeviceToml, DocumentMut)> {
    let typed: DeviceToml = toml::from_str(text).with_context(|| format!("parsing {name}"))?;
    typed.to_config().with_context(|| format!("validating {name}"))?;
    let doc: DocumentMut = text.parse().with_context(|| format!("parsing {name}"))?;
    Ok((typed, doc))
}

/// Result of a pairing: the updated file contents and the key both now share.
pub struct Paired {
    pub tracker: String,
    pub gateway: String,
    pub key: [u8; 32],
    pub tracker_id: u32,
}

/// Pairs `tracker_text` with `gateway_text`. `fresh` supplies a random key
/// when [`KeyChoice::Existing`] finds none.
pub fn pair(tracker_text: &str, gateway_text: &str, choice: KeyChoice, fresh: [u8; 32]) -> Result<Paired> {
    let (tracker, mut tdoc) = load("tracker config", tracker_text)?;
    let (gateway, mut gdoc) = load("gateway config", gateway_text)?;
    if tracker.role != Role::Tracker {
        bail!("the first file must be a tracker (role = \"tracker\")");
    }
    if gateway.role != Role::Gateway {
        bail!("the second file must be a gateway (role = \"gateway\")");
    }

    let key = match choice {
        KeyChoice::Explicit(k) | KeyChoice::New(k) => k,
        KeyChoice::Existing => match &tracker.device_key {
            Some(k) => parse_key(k)?,
            None => fresh,
        },
    };
    let key_hex = hex::encode(key);
    tdoc["device_key"] = value(&key_hex);

    // Replace this tracker's peer entry if present, otherwise append one.
    let peers = gdoc
        .entry("peers")
        .or_insert(Item::ArrayOfTables(ArrayOfTables::new()))
        .as_array_of_tables_mut()
        .context("gateway config: `peers` must be [[peers]] tables")?;
    let existing = peers
        .iter_mut()
        .find(|t| t.get("device_id").and_then(|v| v.as_integer()) == Some(tracker.device_id as i64));
    match existing {
        Some(t) => t["key"] = value(&key_hex),
        None => {
            if peers.len() >= pc::MAX_PEERS {
                bail!("gateway already has {} paired trackers (the max); remove one first", pc::MAX_PEERS);
            }
            let mut t = Table::new();
            t["device_id"] = value(tracker.device_id as i64);
            t["key"] = value(&key_hex);
            peers.push(t);
        }
    }

    let (tracker_out, gateway_out) = (tdoc.to_string(), gdoc.to_string());
    // Re-validate what we're about to write, so a bad edit never reaches disk.
    load("updated tracker config", &tracker_out)?;
    load("updated gateway config", &gateway_out)?;
    Ok(Paired { tracker: tracker_out, gateway: gateway_out, key, tracker_id: tracker.device_id })
}

#[cfg(test)]
mod tests {
    use super::*;

    const TRACKER: &str = "# my tracker\ndevice_id = 10\nrole = \"tracker\"\n";
    const GATEWAY: &str = "# home\ndevice_id = 20\nrole = \"gateway\"\n";

    /// Reads the gateway's peer key for `id` back through the real conversion path.
    fn peer_key(gateway: &str, id: u32) -> Option<[u8; 32]> {
        let d: DeviceToml = toml::from_str(gateway).unwrap();
        d.to_config().unwrap().key_for(id).copied()
    }

    #[test]
    fn parse_key_accepts_common_formats() {
        let hex64 = "ab".repeat(32);
        assert_eq!(parse_key(&hex64).unwrap(), [0xAB; 32]);
        let spaced = ["ab"; 32].join(" ");
        assert_eq!(parse_key(&spaced).unwrap(), [0xAB; 32]);
        let colons = ["AB"; 32].join(":");
        assert_eq!(parse_key(&colons).unwrap(), [0xAB; 32]);
        assert!(parse_key("abcd").unwrap_err().to_string().contains("64 hex"));
        assert!(parse_key("zz").is_err());
    }

    #[test]
    fn generates_key_and_keeps_comments() {
        let p = pair(TRACKER, GATEWAY, KeyChoice::Existing, [5; 32]).unwrap();
        assert_eq!(p.key, [5; 32]);
        assert!(p.tracker.starts_with("# my tracker") && p.gateway.starts_with("# home"));
        let t: DeviceToml = toml::from_str(&p.tracker).unwrap();
        assert_eq!(t.to_config().unwrap().device_key, Some([5; 32]));
        assert_eq!(peer_key(&p.gateway, 10), Some([5; 32]));
    }

    #[test]
    fn existing_tracker_key_is_reused() {
        let tracker = format!("{TRACKER}device_key = \"{}\"\n", "11".repeat(32));
        let p = pair(&tracker, GATEWAY, KeyChoice::Existing, [5; 32]).unwrap();
        assert_eq!(p.key, [0x11; 32]);
        assert_eq!(peer_key(&p.gateway, 10), Some([0x11; 32]));
    }

    #[test]
    fn explicit_key_overrides_and_repairing_replaces_peer() {
        let first = pair(TRACKER, GATEWAY, KeyChoice::Existing, [5; 32]).unwrap();
        let again = pair(&first.tracker, &first.gateway, KeyChoice::Explicit([9; 32]), [0; 32]).unwrap();
        assert_eq!(peer_key(&again.gateway, 10), Some([9; 32]));
        assert_eq!(again.gateway.matches("[[peers]]").count(), 1);
    }

    #[test]
    fn rejects_wrong_roles_and_full_gateway() {
        assert!(pair(GATEWAY, GATEWAY, KeyChoice::Existing, [0; 32]).is_err());
        assert!(pair(TRACKER, TRACKER, KeyChoice::Existing, [0; 32]).is_err());

        let mut gw = GATEWAY.to_string();
        for i in 0..pc::MAX_PEERS {
            gw.push_str(&format!("[[peers]]\ndevice_id = {}\nkey = \"{}\"\n", 100 + i, "00".repeat(32)));
        }
        let err = pair(TRACKER, &gw, KeyChoice::Existing, [0; 32]).err().unwrap().to_string();
        assert!(err.contains("max"), "{err}");
    }
}
