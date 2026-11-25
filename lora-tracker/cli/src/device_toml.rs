//! Human-editable `device.toml` schema and its conversion to the packed
//! `tracker_proto::config::Config`.
//!
//! Byte fields (keys, PSKs) are hex strings so the file stays readable.
//! Every section except `device_id` and `role` is optional, matching the proto
//! config: leave one out and only that path is disabled.

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use tracker_proto::config as pc;
use tracker_proto::heapless;

/// Board role as written in TOML.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Tracker,
    Gateway,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Lora {
    pub frequency_hz: u32,
    pub tx_power_dbm: i8,
    pub interval_s: u16,
    pub stationary_interval_s: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WifiNetwork {
    pub ssid: String,
    #[serde(default)]
    pub password: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Wifi {
    #[serde(default)]
    pub known: Vec<WifiNetwork>,
    #[serde(default)]
    pub allow_open: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Server {
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Meshtastic {
    pub channel_name: String,
    /// Hex, 16 or 32 bytes.
    pub psk: String,
    pub interval_s: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FindMy {
    /// Hex P-224 public key x-coordinates, 28 bytes each.
    pub public_keys: Vec<String>,
    pub rotate_s: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Lte {
    pub apn: String,
    pub interval_s: u16,
}

/// A paired tracker, on a gateway. Written by `lora-tracker pair`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Peer {
    pub device_id: u32,
    /// Hex, 32 bytes; the same value as that tracker's `device_key`.
    pub key: String,
}

/// Top-level `device.toml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceToml {
    pub device_id: u32,
    pub role: Role,
    /// Hex, 32 bytes. Trackers need it for LoRa/WiFi/LTE; the server needs the same value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lora: Option<Lora>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wifi: Option<Wifi>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server: Option<Server>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meshtastic: Option<Meshtastic>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub findmy: Option<FindMy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lte: Option<Lte>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub peers: Vec<Peer>,
}

/// Decodes a hex string into exactly `N` bytes.
fn hex_array<const N: usize>(field: &str, s: &str) -> Result<[u8; N]> {
    let bytes = hex::decode(s.trim()).with_context(|| format!("{field}: not valid hex"))?;
    bytes
        .try_into()
        .map_err(|b: Vec<u8>| anyhow::anyhow!("{field}: expected {N} bytes, got {}", b.len()))
}

/// Copies a string into a fixed-capacity heapless string.
fn bounded<const N: usize>(field: &str, s: &str) -> Result<heapless::String<N>> {
    heapless::String::try_from(s).map_err(|_| anyhow::anyhow!("{field}: longer than {N} bytes"))
}

impl DeviceToml {
    /// Validates and converts into the proto config that gets packed to flash.
    pub fn to_config(&self) -> Result<pc::Config> {
        let device_key = self
            .device_key
            .as_deref()
            .map(|k| hex_array::<32>("device_key", k))
            .transpose()?;

        let lora = self.lora.as_ref().map(|l| pc::LoraConfig {
            frequency_hz: l.frequency_hz,
            tx_power_dbm: l.tx_power_dbm,
            interval_s: l.interval_s,
            stationary_interval_s: l.stationary_interval_s,
        });

        let wifi = match &self.wifi {
            None => None,
            Some(w) => {
                if w.known.len() > pc::MAX_WIFI {
                    bail!("wifi.known: at most {} networks", pc::MAX_WIFI);
                }
                let mut known = heapless::Vec::new();
                for n in &w.known {
                    let net = pc::WifiNetwork {
                        ssid: bounded("wifi.known.ssid", &n.ssid)?,
                        password: bounded("wifi.known.password", &n.password)?,
                    };
                    known.push(net).expect("length checked above");
                }
                Some(pc::WifiConfig { known, allow_open: w.allow_open })
            }
        };

        let server = self
            .server
            .as_ref()
            .map(|s| Ok::<_, anyhow::Error>(pc::ServerConfig { url: bounded("server.url", &s.url)? }))
            .transpose()?;

        let meshtastic = match &self.meshtastic {
            None => None,
            Some(m) => {
                let psk = hex::decode(m.psk.trim()).context("meshtastic.psk: not valid hex")?;
                if psk.len() != 16 && psk.len() != 32 {
                    bail!("meshtastic.psk: expected 16 or 32 bytes, got {}", psk.len());
                }
                Some(pc::MeshtasticConfig {
                    channel_name: bounded("meshtastic.channel_name", &m.channel_name)?,
                    psk: heapless::Vec::from_slice(&psk).expect("length checked above"),
                    interval_s: m.interval_s,
                })
            }
        };

        let findmy = match &self.findmy {
            None => None,
            Some(f) => {
                if f.public_keys.len() > pc::MAX_FINDMY_KEYS {
                    bail!("findmy.public_keys: at most {}", pc::MAX_FINDMY_KEYS);
                }
                let mut public_keys = heapless::Vec::new();
                for k in &f.public_keys {
                    public_keys
                        .push(hex_array::<28>("findmy.public_keys", k)?)
                        .expect("length checked above");
                }
                Some(pc::FindMyConfig { public_keys, rotate_s: f.rotate_s })
            }
        };

        let lte = self
            .lte
            .as_ref()
            .map(|l| Ok::<_, anyhow::Error>(pc::LteConfig { apn: bounded("lte.apn", &l.apn)?, interval_s: l.interval_s }))
            .transpose()?;

        if self.peers.len() > pc::MAX_PEERS {
            bail!("peers: at most {}", pc::MAX_PEERS);
        }
        let mut peers = heapless::Vec::new();
        for p in &self.peers {
            let peer = pc::Peer { device_id: p.device_id, key: hex_array::<32>("peers.key", &p.key)? };
            peers.push(peer).expect("length checked above");
        }

        Ok(pc::Config {
            peers,
            device_id: self.device_id,
            role: match self.role {
                Role::Tracker => pc::Role::Tracker,
                Role::Gateway => pc::Role::Gateway,
            },
            device_key,
            lora,
            wifi,
            server,
            meshtastic,
            findmy,
            lte,
        })
    }
}

/// Renders a fresh `device.toml`: generated secrets filled in, every optional
/// section present but commented out so the user enables only what they have.
pub fn render_template(device_id: u32, role: Role, device_key: Option<[u8; 32]>, mesh_psk: [u8; 16]) -> String {
    let role_s = match role {
        Role::Tracker => "tracker",
        Role::Gateway => "gateway",
    };
    let mut out = format!("device_id = {device_id}\nrole = \"{role_s}\"\n");
    if let Some(k) = device_key {
        out.push_str(&format!(
            "# Keep secret. Shared with the paired gateway and the server (`lora-tracker pair`).\ndevice_key = \"{}\"\n",
            hex::encode(k)
        ));
    } else {
        out.push_str("# Paired trackers live in [[peers]] tables, managed by `lora-tracker pair`.\n");
    }
    out.push_str(
        "\n# Every section below is optional. Uncomment the ones you have hardware or\n\
         # accounts for; anything left out is simply disabled and the rest still run.\n\
         \n\
         # [lora]\n\
         # frequency_hz = 915000000\n\
         # tx_power_dbm = 14\n\
         # interval_s = 30\n\
         # stationary_interval_s = 600\n\
         \n\
         # [server]\n\
         # url = \"http://your-server:8080\"\n\
         \n\
         # [wifi]\n\
         # allow_open = false\n\
         # [[wifi.known]]\n\
         # ssid = \"your-ssid\"\n\
         # password = \"your-password\"\n",
    );
    if role == Role::Tracker {
        out.push_str(&format!(
            "\n\
             # [meshtastic]\n\
             # channel_name = \"tracker\"\n\
             # psk = \"{}\"\n\
             # interval_s = 900\n\
             \n\
             # [findmy]\n\
             # public_keys = []\n\
             # rotate_s = 900\n\
             \n\
             # [lte]\n\
             # apn = \"your-carrier-apn\"\n\
             # interval_s = 300\n",
            hex::encode(mesh_psk)
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: [u8; 32] = [0xAB; 32];

    /// Uncomments every `# ` line after the header, simulating a user enabling all sections.
    fn uncomment_all(t: &str) -> String {
        t.lines()
            .map(|l| match l.strip_prefix("# ") {
                Some(rest) if rest.starts_with('[') || rest.contains(" = ") => rest,
                _ => l,
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn fresh_template_parses_with_only_required_fields() {
        let t = render_template(77, Role::Tracker, Some(KEY), [1; 16]);
        let d: DeviceToml = toml::from_str(&t).unwrap();
        let cfg = d.to_config().unwrap();
        assert_eq!(cfg.device_id, 77);
        assert_eq!(cfg.device_key, Some(KEY));
        assert!(cfg.lora.is_none() && cfg.wifi.is_none() && cfg.lte.is_none());
    }

    #[test]
    fn fully_uncommented_template_parses_and_packs() {
        let t = uncomment_all(&render_template(5, Role::Tracker, Some(KEY), [2; 16]));
        let d: DeviceToml = toml::from_str(&t).unwrap();
        let cfg = d.to_config().unwrap();
        assert_eq!(cfg.lora.as_ref().unwrap().frequency_hz, 915_000_000);
        assert_eq!(cfg.wifi.as_ref().unwrap().known[0].ssid.as_str(), "your-ssid");
        assert_eq!(cfg.meshtastic.as_ref().unwrap().psk.as_slice(), &[2; 16]);
        assert_eq!(cfg.lte.as_ref().unwrap().apn.as_str(), "your-carrier-apn");

        let mut buf = [0u8; pc::MAX_CONFIG];
        let n = cfg.encode(&mut buf).unwrap();
        assert_eq!(pc::Config::decode(&buf[..n]).unwrap(), cfg);
    }

    #[test]
    fn gateway_template_has_no_key_or_tracker_sections() {
        let t = render_template(9, Role::Gateway, None, [0; 16]);
        assert!(!t.contains("device_key") && !t.contains("[lte]") && !t.contains("[findmy]"));
        let d: DeviceToml = toml::from_str(&uncomment_all(&t)).unwrap();
        assert_eq!(d.to_config().unwrap().role, pc::Role::Gateway);
    }

    #[test]
    fn validation_errors_name_the_field() {
        let base = "device_id = 1\nrole = \"tracker\"\n";
        let cases = [
            ("device_key = \"abcd\"", "device_key"),
            ("device_key = \"zz\"", "device_key"),
            ("[meshtastic]\nchannel_name = \"x\"\npsk = \"00\"\ninterval_s = 1", "meshtastic.psk"),
            ("[meshtastic]\nchannel_name = \"waytoolongname\"\npsk = \"00000000000000000000000000000000\"\ninterval_s = 1", "meshtastic.channel_name"),
            ("[findmy]\npublic_keys = [\"00\"]\nrotate_s = 1", "findmy.public_keys"),
        ];
        for (extra, field) in cases {
            let d: DeviceToml = toml::from_str(&format!("{base}{extra}")).unwrap();
            let err = d.to_config().unwrap_err().to_string();
            assert!(err.contains(field), "{extra:?} gave {err:?}");
        }
    }

    #[test]
    fn too_many_wifi_networks() {
        let mut t = String::from("device_id = 1\nrole = \"tracker\"\n[wifi]\n");
        for i in 0..=pc::MAX_WIFI {
            t.push_str(&format!("[[wifi.known]]\nssid = \"n{i}\"\n"));
        }
        let d: DeviceToml = toml::from_str(&t).unwrap();
        assert!(d.to_config().unwrap_err().to_string().contains("wifi.known"));
    }
}
