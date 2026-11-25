//! A tiny cross-process lock (flock) for operations that must not race,
//! such as picking a unique container name.

use hullr_core::{Context, Result};
use std::fs::File;
use std::os::fd::AsRawFd;
use std::path::Path;

pub struct FileLock {
    _file: File,
}

impl FileLock {
    pub fn acquire(path: &Path) -> Result<Self> {
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d)?;
        }
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(path)
            .context("opening lock file")?;
        // SAFETY: valid open fd; flock blocks until acquired and is released
        // when the fd closes (on drop).
        let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) };
        if rc != 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        Ok(Self { _file: file })
    }
}
