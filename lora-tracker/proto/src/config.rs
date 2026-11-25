//! Device config blob and the rules for which uplink paths it enables.
//!
//! Every uplink section is optional. A missing section, or a missing
//! dependency such as the device key or server URL, disables only that path;
//! the firmware still runs everything else. [`Config::paths`] is the single
//! place that decides this, so firmware and tests agree.
//!
//! Blob layout: `b"LTCF"` | version u8 | postcard(Config).

use heapless::{String, Vec};
use serde::{Deserialize, Serialize};

use crate::Error;

/// Magic prefix of a packed config blob.
pub const MAGIC: [u8; 4] = *b"LTCF";
/// Current config blob version.
pub const VERSION: u8 = 1;
/// Upper bound on a packed config; the flash partition must be at least this big.
pub const MAX_CONFIG: usize = 1024;
/// Max known WiFi networks stored on the device.
pub const MAX_WIFI: usize = 4;
/// Max Find My public keys in the rotation list.
pub const MAX_FINDMY_KEYS: usize = 8;
/// Max paired trackers a gateway can decrypt.
pub const MAX_PEERS: usize = 4;
/// Flash offset of the `ltcfg` partition (see `firmware/partitions.csv`).
/// The CLI writes the packed blob here; the firmware reads it from here.
pub const CONFIG_FLASH_OFFSET: u32 = 0xE000;
/// Size of the `ltcfg` partition: one 4 KiB flash sector.
pub const CONFIG_PARTITION_SIZE: usize = 0x1000;
const _: () = assert!(MAX_CONFIG <= CONFIG_PARTITION_SIZE, "config must fit its partition");

/// What this board does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Role {
    /// Rides on the vehicle and reports its position.
    Tracker,
    /// Sits at home, receives LoRa reports, and forwards them over WiFi.
    Gateway,
}

/// Custom LoRa link settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoraConfig {
    pub frequency_hz: u32,
    pub tx_power_dbm: i8,
    /// Seconds between reports while moving.
    pub interval_s: u16,
    /// Seconds between reports while stationary.
    pub stationary_interval_s: u16,
}

/// One known WiFi network.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WifiNetwork {
    pub ssid: String<32>,
    /// Empty for an open network.
    pub password: String<64>,
}

/// WiFi uplink settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WifiConfig {
    /// Tried first, in order.
    pub known: Vec<WifiNetwork, MAX_WIFI>,
    /// Also try open networks after the known ones (tracker only).
    pub allow_open: bool,
}

/// Where WiFi and LTE uploads go.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerConfig {
    /// Base URL, e.g. `http://host:8080`; `/api/v1/report` is appended.
    pub url: String<128>,
}

/// Meshtastic fallback position packets.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MeshtasticConfig {
    pub channel_name: String<12>,
    /// 16 or 32 byte AES key for the private channel.
    pub psk: Vec<u8, 32>,
    pub interval_s: u16,
}

/// Find My (OpenHaystack-style) beacon keys.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FindMyConfig {
    /// P-224 public key x-coordinates, rotated through in order.
    pub public_keys: Vec<[u8; 28], MAX_FINDMY_KEYS>,
    pub rotate_s: u32,
}

/// SIM7080G LTE-M uplink settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LteConfig {
    pub apn: String<32>,
    pub interval_s: u16,
}

/// A tracker this gateway is paired with, and the key to read its reports.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Peer {
    pub device_id: u32,
    pub key: [u8; 32],
}

/// Full device config. Only `device_id` and `role` are required.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Config {
    pub device_id: u32,
    pub role: Role,
    /// ChaCha20-Poly1305 key for the custom protocol (LoRa, WiFi, LTE).
    pub device_key: Option<[u8; 32]>,
    pub lora: Option<LoraConfig>,
    pub wifi: Option<WifiConfig>,
    pub server: Option<ServerConfig>,
    pub meshtastic: Option<MeshtasticConfig>,
    pub findmy: Option<FindMyConfig>,
    pub lte: Option<LteConfig>,
    /// Gateway only: paired trackers. Without peers a gateway still forwards
    /// packets blind; it just can't decrypt them to display or filter.
    pub peers: Vec<Peer, MAX_PEERS>,
}

impl Config {
    /// The key for `device_id`'s reports: our own key, or a paired peer's.
    pub fn key_for(&self, device_id: u32) -> Option<&[u8; 32]> {
        if device_id == self.device_id {
            if let Some(k) = &self.device_key {
                return Some(k);
            }
        }
        self.peers.iter().find(|p| p.device_id == device_id).map(|p| &p.key)
    }
}

/// Why a path is off.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Disabled {
    /// Its config section is missing.
    NotConfigured,
    /// Needs `device_key`, which is missing.
    NoDeviceKey,
    /// Needs a `server` section, which is missing.
    NoServer,
    /// Section present but empty (no WiFi networks, no Find My keys).
    Empty,
    /// This path doesn't apply to the board's role.
    NotForRole,
}

/// Whether a path should start. Hardware probing happens later, in firmware.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathPlan {
    Enabled,
    Disabled(Disabled),
}

/// The plan for every path. BLE status is always on so an unconfigured
/// board can still be inspected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Paths {
    pub lora: PathPlan,
    pub wifi: PathPlan,
    pub lte: PathPlan,
    pub meshtastic: PathPlan,
    pub findmy: PathPlan,
    pub ble_status: PathPlan,
}

impl Config {
    /// Decides which paths to start, given only what is configured.
    pub fn paths(&self) -> Paths {
        use Disabled::*;
        use PathPlan::Enabled;
        let off = PathPlan::Disabled;
        let tracker = self.role == Role::Tracker;
        // Gateways forward opaque packets, so only trackers need the key.
        let key_ok = !tracker || self.device_key.is_some();

        let lora = match (&self.lora, key_ok) {
            (None, _) => off(NotConfigured),
            (Some(_), false) => off(NoDeviceKey),
            (Some(_), true) => Enabled,
        };
        let wifi = match &self.wifi {
            None => off(NotConfigured),
            Some(w) if w.known.is_empty() && !(tracker && w.allow_open) => off(Empty),
            Some(_) if self.server.is_none() => off(NoServer),
            Some(_) if !key_ok => off(NoDeviceKey),
            Some(_) => Enabled,
        };
        let lte = match &self.lte {
            _ if !tracker => off(NotForRole),
            None => off(NotConfigured),
            Some(_) if self.server.is_none() => off(NoServer),
            Some(_) if !key_ok => off(NoDeviceKey),
            Some(_) => Enabled,
        };
        let meshtastic = match &self.meshtastic {
            _ if !tracker => off(NotForRole),
            None => off(NotConfigured),
            Some(m) if m.psk.is_empty() => off(Empty),
            Some(_) => Enabled,
        };
        let findmy = match &self.findmy {
            _ if !tracker => off(NotForRole),
            None => off(NotConfigured),
            Some(f) if f.public_keys.is_empty() => off(Empty),
            Some(_) => Enabled,
        };
        Paths { lora, wifi, lte, meshtastic, findmy, ble_status: Enabled }
    }

    /// Packs the config into `out`, returning the blob length.
    pub fn encode(&self, out: &mut [u8]) -> Result<usize, Error> {
        if out.len() < MAGIC.len() + 1 {
            return Err(Error::BufferTooSmall);
        }
        out[..4].copy_from_slice(&MAGIC);
        out[4] = VERSION;
        let n = postcard::to_slice(self, &mut out[5..])
            .map_err(|e| match e {
                postcard::Error::SerializeBufferFull => Error::BufferTooSmall,
                _ => Error::Encode,
            })?
            .len();
        Ok(5 + n)
    }

    /// Unpacks a blob. Trailing bytes (erased flash, 0xFF) are ignored.
    pub fn decode(blob: &[u8]) -> Result<Config, Error> {
        if blob.len() < 5 {
            return Err(Error::Truncated);
        }
        if blob[..4] != MAGIC {
            return Err(Error::BadMagic);
        }
        if blob[4] != VERSION {
            return Err(Error::BadVersion(blob[4]));
        }
        postcard::take_from_bytes(&blob[5..])
            .map(|(cfg, _rest)| cfg)
            .map_err(|_| Error::Decode)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Disabled::*;
    use PathPlan::Enabled;

    /// A tracker with nothing but identity configured.
    fn bare(role: Role) -> Config {
        Config {
            device_id: 1,
            role,
            device_key: None,
            lora: None,
            wifi: None,
            server: None,
            meshtastic: None,
            findmy: None,
            lte: None,
            peers: Vec::new(),
        }
    }

    /// A tracker with every section filled in.
    fn full() -> Config {
        let mut known = Vec::new();
        known
            .push(WifiNetwork { ssid: String::try_from("home").unwrap(), password: String::try_from("pw").unwrap() })
            .unwrap();
        let mut keys = Vec::new();
        keys.push([3u8; 28]).unwrap();
        Config {
            device_key: Some([1u8; 32]),
            lora: Some(LoraConfig { frequency_hz: 915_000_000, tx_power_dbm: 14, interval_s: 30, stationary_interval_s: 600 }),
            wifi: Some(WifiConfig { known, allow_open: false }),
            server: Some(ServerConfig { url: String::try_from("http://example.invalid:8080").unwrap() }),
            meshtastic: Some(MeshtasticConfig {
                channel_name: String::try_from("trk").unwrap(),
                psk: Vec::from_slice(&[9u8; 16]).unwrap(),
                interval_s: 900,
            }),
            findmy: Some(FindMyConfig { public_keys: keys, rotate_s: 900 }),
            lte: Some(LteConfig { apn: String::try_from("hologram").unwrap(), interval_s: 300 }),
            ..bare(Role::Tracker)
        }
    }

    fn off(d: Disabled) -> PathPlan {
        PathPlan::Disabled(d)
    }

    #[test]
    fn full_tracker_enables_everything() {
        let p = full().paths();
        assert_eq!(p.lora, Enabled);
        assert_eq!(p.wifi, Enabled);
        assert_eq!(p.lte, Enabled);
        assert_eq!(p.meshtastic, Enabled);
        assert_eq!(p.findmy, Enabled);
        assert_eq!(p.ble_status, Enabled);
    }

    #[test]
    fn unconfigured_board_still_has_ble() {
        let p = bare(Role::Tracker).paths();
        assert_eq!(p.ble_status, Enabled);
        for plan in [p.lora, p.wifi, p.lte, p.meshtastic, p.findmy] {
            assert_eq!(plan, off(NotConfigured));
        }
    }

    #[test]
    fn missing_lte_leaves_others_running() {
        let cfg = Config { lte: None, ..full() };
        let p = cfg.paths();
        assert_eq!(p.lte, off(NotConfigured));
        assert_eq!((p.lora, p.wifi, p.meshtastic, p.findmy), (Enabled, Enabled, Enabled, Enabled));
    }

    #[test]
    fn missing_key_disables_only_custom_protocol_paths() {
        let p = Config { device_key: None, ..full() }.paths();
        assert_eq!((p.lora, p.wifi, p.lte), (off(NoDeviceKey), off(NoDeviceKey), off(NoDeviceKey)));
        assert_eq!((p.meshtastic, p.findmy, p.ble_status), (Enabled, Enabled, Enabled));
    }

    #[test]
    fn missing_server_disables_internet_paths_only() {
        let p = Config { server: None, ..full() }.paths();
        assert_eq!((p.wifi, p.lte), (off(NoServer), off(NoServer)));
        assert_eq!(p.lora, Enabled);
    }

    #[test]
    fn empty_sections() {
        let mut cfg = full();
        cfg.wifi.as_mut().unwrap().known.clear();
        cfg.findmy.as_mut().unwrap().public_keys.clear();
        cfg.meshtastic.as_mut().unwrap().psk.clear();
        let p = cfg.paths();
        assert_eq!((p.wifi, p.findmy, p.meshtastic), (off(Empty), off(Empty), off(Empty)));

        // Open-network scanning alone is enough for a tracker's WiFi path.
        cfg.wifi.as_mut().unwrap().allow_open = true;
        assert_eq!(cfg.paths().wifi, Enabled);
    }

    #[test]
    fn gateway_needs_no_key_and_skips_tracker_paths() {
        let cfg = Config { role: Role::Gateway, device_key: None, ..full() };
        let p = cfg.paths();
        assert_eq!((p.lora, p.wifi), (Enabled, Enabled));
        for plan in [p.lte, p.meshtastic, p.findmy] {
            assert_eq!(plan, off(NotForRole));
        }
        // allow_open doesn't apply to gateways.
        let mut cfg = cfg;
        let w = cfg.wifi.as_mut().unwrap();
        w.known.clear();
        w.allow_open = true;
        assert_eq!(cfg.paths().wifi, off(Empty));
    }

    #[test]
    fn key_lookup_covers_self_and_peers() {
        let mut gw = Config { role: Role::Gateway, device_id: 50, ..bare(Role::Gateway) };
        gw.peers.push(Peer { device_id: 7, key: [4u8; 32] }).unwrap();
        assert_eq!(gw.key_for(7), Some(&[4u8; 32]));
        assert_eq!(gw.key_for(8), None);
        assert_eq!(gw.key_for(50), None);
        let t = full();
        assert_eq!(t.key_for(t.device_id), Some(&[1u8; 32]));
    }

    #[test]
    fn max_config_fits_the_partition() {
        // A gateway with every peer slot used still packs.
        let mut gw = Config { role: Role::Gateway, ..full() };
        for i in 0..MAX_PEERS as u32 {
            gw.peers.push(Peer { device_id: i, key: [0xFF; 32] }).unwrap();
        }
        let mut buf = [0u8; MAX_CONFIG];
        assert!(gw.encode(&mut buf).is_ok());
    }

    #[test]
    fn blob_round_trip_with_erased_flash_tail() {
        let mut buf = [0xFFu8; MAX_CONFIG];
        let n = full().encode(&mut buf).unwrap();
        assert!(n < MAX_CONFIG);
        assert_eq!(Config::decode(&buf).unwrap(), full());
        assert_eq!(Config::decode(&buf[..n]).unwrap(), full());
    }

    #[test]
    fn blob_errors() {
        assert_eq!(Config::decode(&[0xFF; 64]), Err(Error::BadMagic));
        assert_eq!(Config::decode(b"LTC"), Err(Error::Truncated));
        let mut buf = [0u8; MAX_CONFIG];
        let n = full().encode(&mut buf).unwrap();
        buf[4] = 2;
        assert_eq!(Config::decode(&buf[..n]), Err(Error::BadVersion(2)));
        buf[4] = VERSION;
        assert_eq!(Config::decode(&buf[..8]), Err(Error::Decode));
        assert_eq!(full().encode(&mut [0u8; 16]), Err(Error::BufferTooSmall));
    }
}
