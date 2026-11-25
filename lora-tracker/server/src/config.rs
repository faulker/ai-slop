//! `server.toml`: where to listen, where the database lives, and each tracker's key.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::Deserialize;

/// The file as written by the user.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawConfig {
    #[serde(default = "default_listen")]
    listen: SocketAddr,
    #[serde(default = "default_database")]
    database: PathBuf,
    #[serde(default)]
    device: Vec<RawDevice>,
}

/// One `[[device]]` entry, as printed by `lora-tracker pair`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawDevice {
    device_id: u32,
    key: String,
    name: Option<String>,
}

/// A tracker the server will accept reports from.
#[derive(Debug, Clone)]
pub struct Device {
    pub key: [u8; 32],
    pub name: String,
}

/// Validated server settings.
#[derive(Debug, Clone)]
pub struct Config {
    pub listen: SocketAddr,
    pub database: PathBuf,
    pub devices: HashMap<u32, Device>,
}

/// Default bind address: every interface, so boards on the LAN can reach it.
fn default_listen() -> SocketAddr {
    "0.0.0.0:8080".parse().unwrap()
}

/// Default database file name, resolved next to `server.toml`.
fn default_database() -> PathBuf {
    PathBuf::from("tracker.db")
}

/// Reads and validates `server.toml`. A relative `database` is resolved against the file's directory.
pub fn load(path: &Path) -> Result<Config> {
    let text = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let mut cfg = parse(&text).with_context(|| format!("in {}", path.display()))?;
    if cfg.database.is_relative() {
        let dir = path.parent().unwrap_or(Path::new("."));
        cfg.database = dir.join(&cfg.database);
    }
    Ok(cfg)
}

/// Parses and validates config text without touching the filesystem.
pub fn parse(text: &str) -> Result<Config> {
    let raw: RawConfig = toml::from_str(text)?;
    let mut devices = HashMap::new();
    for d in raw.device {
        let key = parse_key(&d.key).with_context(|| format!("device {}", d.device_id))?;
        let name = d.name.unwrap_or_else(|| format!("Tracker {}", d.device_id));
        if devices.insert(d.device_id, Device { key, name }).is_some() {
            bail!("device_id {} is listed twice", d.device_id);
        }
    }
    Ok(Config { listen: raw.listen, database: raw.database, devices })
}

/// Parses a 64-character hex key.
fn parse_key(s: &str) -> Result<[u8; 32]> {
    let bytes = hex::decode(s.trim()).context("key must be hex")?;
    let len = bytes.len();
    bytes.try_into().map_err(|_| anyhow::anyhow!("key must be 32 bytes (64 hex characters), got {len} bytes"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: &str = "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";

    #[test]
    fn minimal_file_uses_defaults() {
        let cfg = parse("").unwrap();
        assert_eq!(cfg.listen, default_listen());
        assert_eq!(cfg.database, PathBuf::from("tracker.db"));
        assert!(cfg.devices.is_empty());
    }

    #[test]
    fn pair_output_parses_as_is() {
        let cfg = parse(&format!("[[device]]\ndevice_id = 7\nkey = \"{KEY}\"\n")).unwrap();
        let d = &cfg.devices[&7];
        assert_eq!(d.key[1], 1);
        assert_eq!(d.name, "Tracker 7");
    }

    #[test]
    fn rejects_bad_keys_duplicates_and_typos() {
        assert!(parse("[[device]]\ndevice_id = 1\nkey = \"abcd\"\n").is_err());
        let dup = format!("[[device]]\ndevice_id = 1\nkey = \"{KEY}\"\n[[device]]\ndevice_id = 1\nkey = \"{KEY}\"\n");
        assert!(parse(&dup).is_err());
        assert!(parse("listn = \"0.0.0.0:1\"\n").is_err());
    }

    #[test]
    fn relative_database_resolves_next_to_config() {
        let dir = std::env::temp_dir().join(format!("lt-srv-cfg-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("server.toml");
        std::fs::write(&path, "database = \"data.db\"\n").unwrap();
        let cfg = load(&path).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(cfg.database, dir.join("data.db"));
    }
}
