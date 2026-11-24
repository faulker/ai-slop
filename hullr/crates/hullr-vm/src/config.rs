//! VM settings, stored as JSON in `<root>/vm/config.json`.
//!
//! Missing fields fall back to [`VmConfig::default`], so a partial file such
//! as `{"memory_mib": 8192}` is valid. A missing or unreadable file yields the
//! defaults; changes take effect on the next VM start.

use hullr_core::paths::{write_atomic, Paths};
use hullr_core::{Error, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct VmConfig {
    /// Virtual CPUs.
    pub cpus: u32,
    /// Guest memory in MiB.
    pub memory_mib: u64,
    /// Size of the sparse data disk (`<root>/vm/disk.img`) in GiB. Growing it
    /// takes effect on the next boot; shrinking is ignored.
    pub disk_gib: u64,
    /// Expose Rosetta to the guest for fast linux/amd64 on Apple Silicon.
    pub rosetta: bool,
    /// Host directories shared into the guest at the same absolute path.
    pub shares: Vec<PathBuf>,
    /// Override the guest kernel (an uncompressed arm64 `Image` on Apple
    /// Silicon).
    pub kernel: Option<PathBuf>,
    /// Override the guest initramfs.
    pub initrd: Option<PathBuf>,
}

impl Default for VmConfig {
    fn default() -> Self {
        let host_cpus = std::thread::available_parallelism().map(|n| n.get() as u32).unwrap_or(2);
        let mut shares = Vec::new();
        if let Some(home) = dirs::home_dir() {
            shares.push(home);
        }
        shares.push(PathBuf::from("/tmp"));
        shares.push(PathBuf::from("/private/var/folders"));
        Self {
            cpus: host_cpus.clamp(1, 4),
            memory_mib: 4096,
            disk_gib: 64,
            rosetta: cfg!(all(target_os = "macos", target_arch = "aarch64")),
            shares,
            kernel: None,
            initrd: None,
        }
    }
}

impl VmConfig {
    /// Reject values the VM cannot boot with.
    pub fn validate(&self) -> Result<()> {
        if self.cpus == 0 {
            return Err(Error::Invalid("vm cpus must be at least 1".into()));
        }
        if self.memory_mib < 512 {
            return Err(Error::Invalid("vm memory_mib must be at least 512".into()));
        }
        if self.disk_gib < 1 {
            return Err(Error::Invalid("vm disk_gib must be at least 1".into()));
        }
        for s in &self.shares {
            if !s.is_absolute() {
                return Err(Error::Invalid(format!("vm share '{}' must be an absolute path", s.display())));
            }
        }
        Ok(())
    }
}

/// `<root>/vm/config.json`.
pub fn config_path(paths: &Paths) -> PathBuf {
    paths.vm().join("config.json")
}

/// Load the VM config, falling back to defaults when the file is missing or
/// invalid (an invalid file is reported on stderr, not fatal).
pub fn load_config(paths: &Paths) -> VmConfig {
    let path = config_path(paths);
    match std::fs::read(&path) {
        Ok(data) => match serde_json::from_slice(&data) {
            Ok(cfg) => cfg,
            Err(e) => {
                eprintln!("hullr: ignoring invalid {}: {e}", path.display());
                VmConfig::default()
            }
        },
        Err(_) => VmConfig::default(),
    }
}

/// Validate and write the VM config atomically.
pub fn save_config(paths: &Paths, cfg: &VmConfig) -> Result<()> {
    cfg.validate()?;
    let mut data = serde_json::to_vec_pretty(cfg)?;
    data.push(b'\n');
    write_atomic(&config_path(paths), &data)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_sane() {
        let c = VmConfig::default();
        assert!(c.cpus >= 1 && c.cpus <= 4);
        assert_eq!(c.memory_mib, 4096);
        assert_eq!(c.disk_gib, 64);
        assert!(c.shares.contains(&PathBuf::from("/tmp")));
        assert!(c.shares.contains(&PathBuf::from("/private/var/folders")));
        assert_eq!(c.rosetta, cfg!(all(target_os = "macos", target_arch = "aarch64")));
        c.validate().unwrap();
    }

    #[test]
    fn missing_file_gives_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path());
        assert_eq!(load_config(&paths), VmConfig::default());
    }

    #[test]
    fn save_then_load_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path());
        let cfg = VmConfig {
            cpus: 2,
            memory_mib: 2048,
            disk_gib: 10,
            rosetta: false,
            shares: vec!["/Users/me".into()],
            kernel: Some("/opt/k".into()),
            initrd: None,
        };
        save_config(&paths, &cfg).unwrap();
        assert!(config_path(&paths).exists());
        assert_eq!(load_config(&paths), cfg);
    }

    #[test]
    fn partial_file_fills_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path());
        std::fs::create_dir_all(paths.vm()).unwrap();
        std::fs::write(config_path(&paths), r#"{"memory_mib": 8192}"#).unwrap();
        let c = load_config(&paths);
        assert_eq!(c.memory_mib, 8192);
        assert_eq!(c.disk_gib, VmConfig::default().disk_gib);
    }

    #[test]
    fn invalid_file_gives_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path());
        std::fs::create_dir_all(paths.vm()).unwrap();
        std::fs::write(config_path(&paths), "not json").unwrap();
        assert_eq!(load_config(&paths), VmConfig::default());
    }

    #[test]
    fn validation_rejects_bad_values() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path());
        let bad = VmConfig { cpus: 0, ..VmConfig::default() };
        assert!(save_config(&paths, &bad).is_err());
        let bad = VmConfig { shares: vec!["relative".into()], ..VmConfig::default() };
        assert!(bad.validate().is_err());
    }
}
