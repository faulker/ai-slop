//! On-disk layout. Everything lives under one root (`$HULLR_HOME`, default
//! `~/.hullr`, or `/var/lib/hullr` when running as root on Linux).
//!
//! ```text
//! <root>/
//!   hullr.sock            daemon API socket
//!   images/               owned by hullr-image
//!   containers/<id>/      owned by hullr-runtime (rootfs, state, logs, shim socket)
//!   volumes/<name>/data   named volumes
//!   sandboxes/<name>/     shared network sandboxes
//!   build/                builder scratch + cache index
//!   engine/               engine metadata (container records)
//! ```

use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct Paths {
    pub root: PathBuf,
}

impl Paths {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Resolve the default root from the environment.
    pub fn from_env() -> Self {
        if let Some(p) = std::env::var_os("HULLR_HOME") {
            return Self::new(p);
        }
        #[cfg(target_os = "linux")]
        {
            // SAFETY: geteuid has no preconditions.
            if unsafe { libc_geteuid() } == 0 {
                return Self::new("/var/lib/hullr");
            }
        }
        Self::new(dirs::home_dir().unwrap_or_else(|| PathBuf::from("/tmp")).join(".hullr"))
    }

    pub fn socket(&self) -> PathBuf {
        if let Some(p) = std::env::var_os("HULLR_SOCKET") {
            return p.into();
        }
        self.root.join("hullr.sock")
    }
    pub fn images(&self) -> PathBuf {
        self.root.join("images")
    }
    pub fn containers(&self) -> PathBuf {
        self.root.join("containers")
    }
    pub fn container(&self, id: &str) -> PathBuf {
        self.containers().join(id)
    }
    pub fn volumes(&self) -> PathBuf {
        self.root.join("volumes")
    }
    pub fn sandboxes(&self) -> PathBuf {
        self.root.join("sandboxes")
    }
    pub fn build(&self) -> PathBuf {
        self.root.join("build")
    }
    pub fn engine(&self) -> PathBuf {
        self.root.join("engine")
    }
    pub fn vm(&self) -> PathBuf {
        self.root.join("vm")
    }
    pub fn ensure(&self) -> std::io::Result<()> {
        for d in [
            self.root.clone(),
            self.images(),
            self.containers(),
            self.volumes(),
            self.sandboxes(),
            self.build(),
            self.engine(),
        ] {
            std::fs::create_dir_all(&d)?;
        }
        Ok(())
    }
}

#[cfg(target_os = "linux")]
extern "C" {
    #[link_name = "geteuid"]
    fn libc_geteuid() -> u32;
}

/// Write a file atomically (temp file + rename).
pub fn write_atomic(path: &Path, data: &[u8]) -> std::io::Result<()> {
    let dir = path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir)?;
    let tmp = dir.join(format!(
        ".{}.tmp{}",
        path.file_name().and_then(|n| n.to_str()).unwrap_or("f"),
        std::process::id()
    ));
    std::fs::write(&tmp, data)?;
    std::fs::rename(&tmp, path)
}
