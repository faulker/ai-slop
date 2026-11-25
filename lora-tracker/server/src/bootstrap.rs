//! First run: writes a default `server.toml` when none exists, filling in
//! `[[device]]` entries from the tracker and gateway configs next to it.
//!
//! Trackers contribute their `device_key`; gateways contribute their `[[peers]]`.
//! When both name the same tracker, the tracker's own file wins (it is the sender).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Deserialize;

/// The parts of a `device.toml` the server cares about. Other fields are ignored.
#[derive(Deserialize)]
struct DeviceFile {
    device_id: u32,
    role: String,
    device_key: Option<String>,
    #[serde(default)]
    peers: Vec<Peer>,
}

/// A `[[peers]]` entry on a gateway.
#[derive(Deserialize)]
struct Peer {
    device_id: u32,
    key: String,
}

/// A device found while scanning, and where its key came from.
#[derive(Debug, Clone, PartialEq)]
pub struct Found {
    pub key: String,
    pub name: Option<String>,
    pub source: PathBuf,
    /// True when the key came from the tracker's own file rather than a gateway.
    from_tracker: bool,
}

/// What a scan produced: devices by id, plus human-readable warnings.
#[derive(Debug, Default)]
pub struct Scan {
    pub devices: BTreeMap<u32, Found>,
    pub warnings: Vec<String>,
}

/// Creates `path` with defaults and any keys found beside it. Prints what it did.
pub fn write_default(path: &Path) -> Result<()> {
    let dir = match path.parent() {
        Some(d) if !d.as_os_str().is_empty() => d,
        _ => Path::new("."),
    };
    let scan = scan_dir(dir, path)?;
    for w in &scan.warnings {
        eprintln!("warning: {w}");
    }
    write_secret(path, render(&scan).as_bytes())?;

    println!("created {} (it holds keys; keep it private)", path.display());
    for (id, d) in &scan.devices {
        println!("  device {id} from {}", d.source.display());
    }
    if scan.devices.is_empty() {
        println!("  no tracker or gateway configs found next to it; add [[device]] entries from `lora-tracker pair`");
    }
    println!("  map: http://<this machine>:8080/");
    Ok(())
}

/// Reads every `*.toml` in `dir` except `skip` and collects device keys.
pub fn scan_dir(dir: &Path, skip: &Path) -> Result<Scan> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .with_context(|| format!("reading {}", dir.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "toml") && p.file_name() != skip.file_name())
        .collect();
    files.sort();

    let mut scan = Scan::default();
    for file in files {
        let Ok(text) = std::fs::read_to_string(&file) else { continue };
        // Not every .toml here is a device config (e.g. Cargo.toml); skip those quietly.
        let Ok(dev) = toml::from_str::<DeviceFile>(&text) else { continue };
        match dev.role.as_str() {
            "tracker" => match dev.device_key {
                Some(key) => {
                    let name = file.file_stem().map(|s| s.to_string_lossy().into_owned());
                    add(&mut scan, dev.device_id, Found { key, name, source: file.clone(), from_tracker: true });
                }
                None => scan.warnings.push(format!(
                    "{} is a tracker with no device_key; run `lora-tracker pair` first",
                    file.display()
                )),
            },
            "gateway" => {
                for p in dev.peers {
                    add(&mut scan, p.device_id, Found { key: p.key, name: None, source: file.clone(), from_tracker: false });
                }
            }
            _ => {}
        }
    }
    Ok(scan)
}

/// Records a device, preferring a tracker's own key and warning on any disagreement.
fn add(scan: &mut Scan, id: u32, new: Found) {
    let Some(old) = scan.devices.get(&id) else {
        scan.devices.insert(id, new);
        return;
    };
    if old.key.eq_ignore_ascii_case(&new.key) {
        if new.from_tracker && !old.from_tracker {
            scan.devices.insert(id, new);
        }
        return;
    }
    let (keep, drop) = if new.from_tracker && !old.from_tracker { (&new, old) } else { (old, &new) };
    scan.warnings.push(format!(
        "device {id}: key in {} differs from {}; using {} (re-run `lora-tracker pair` and re-flash)",
        drop.source.display(),
        keep.source.display(),
        keep.source.display()
    ));
    if new.from_tracker && !old.from_tracker {
        scan.devices.insert(id, new);
    }
}

/// Renders the server.toml text: the commented defaults plus one `[[device]]` per find.
fn render(scan: &Scan) -> String {
    let mut s = String::from(
        "# lora-tracker-server config, generated on first run. Holds keys: keep it private.\n\
         \n\
         # Boards use `[server] url = \"http://<this machine>:8080\"`.\n\
         listen = \"0.0.0.0:8080\"\n\
         \n\
         # SQLite file, relative to this file.\n\
         database = \"tracker.db\"\n"
    );
    for (id, d) in &scan.devices {
        s.push_str(&format!("\n# from {}\n[[device]]\ndevice_id = {id}\nkey = \"{}\"\n", d.source.display(), d.key));
        if let Some(name) = &d.name {
            s.push_str(&format!("name = {}\n", toml::Value::String(name.clone())));
        }
    }
    s
}

/// Writes a file readable only by the owner (0600 on Unix).
fn write_secret(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut f = opts.open(path).with_context(|| format!("creating {}", path.display()))?;
    f.write_all(bytes).with_context(|| format!("writing {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config;

    const K1: &str = "1111111111111111111111111111111111111111111111111111111111111111";
    const K2: &str = "2222222222222222222222222222222222222222222222222222222222222222";

    /// A fresh, empty temp directory unique to `name`.
    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("lt-boot-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn collects_trackers_and_gateway_peers_and_skips_other_toml() {
        let d = tmp("collect");
        std::fs::write(d.join("tracker.toml"), format!("device_id = 1\nrole = \"tracker\"\ndevice_key = \"{K1}\"\n[lora]\nfrequency_hz = 915000000\n")).unwrap();
        std::fs::write(d.join("gateway.toml"), format!("device_id = 9\nrole = \"gateway\"\n[[peers]]\ndevice_id = 2\nkey = \"{K2}\"\n")).unwrap();
        std::fs::write(d.join("Cargo.toml"), "[package]\nname = \"x\"\n").unwrap();
        let scan = scan_dir(&d, &d.join("server.toml")).unwrap();
        std::fs::remove_dir_all(&d).unwrap();
        assert_eq!(scan.devices.len(), 2);
        assert_eq!(scan.devices[&1].name.as_deref(), Some("tracker"));
        assert_eq!(scan.devices[&2].key, K2);
        assert!(scan.warnings.is_empty());
    }

    #[test]
    fn tracker_key_wins_over_mismatched_gateway_peer() {
        let d = tmp("conflict");
        // "a-gateway" sorts first, so the tracker has to override an existing entry.
        std::fs::write(d.join("a-gateway.toml"), format!("device_id = 9\nrole = \"gateway\"\n[[peers]]\ndevice_id = 1\nkey = \"{K2}\"\n")).unwrap();
        std::fs::write(d.join("tracker.toml"), format!("device_id = 1\nrole = \"tracker\"\ndevice_key = \"{K1}\"\n")).unwrap();
        let scan = scan_dir(&d, &d.join("server.toml")).unwrap();
        std::fs::remove_dir_all(&d).unwrap();
        assert_eq!(scan.devices[&1].key, K1);
        assert_eq!(scan.warnings.len(), 1);
    }

    #[test]
    fn unpaired_tracker_warns() {
        let d = tmp("nokey");
        std::fs::write(d.join("tracker.toml"), "device_id = 1\nrole = \"tracker\"\n").unwrap();
        let scan = scan_dir(&d, &d.join("server.toml")).unwrap();
        std::fs::remove_dir_all(&d).unwrap();
        assert!(scan.devices.is_empty());
        assert_eq!(scan.warnings.len(), 1);
    }

    #[test]
    fn written_file_is_valid_private_and_never_overwrites() {
        let d = tmp("write");
        std::fs::write(d.join("truck.toml"), format!("device_id = 4\nrole = \"tracker\"\ndevice_key = \"{K1}\"\n")).unwrap();
        let path = d.join("server.toml");
        write_default(&path).unwrap();
        let cfg = config::load(&path).unwrap();
        let second = write_default(&path);
        #[cfg(unix)]
        let mode = {
            use std::os::unix::fs::PermissionsExt;
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777
        };
        std::fs::remove_dir_all(&d).unwrap();
        assert_eq!(cfg.devices[&4].name, "truck");
        assert!(second.is_err());
        #[cfg(unix)]
        assert_eq!(mode, 0o600);
    }
}
