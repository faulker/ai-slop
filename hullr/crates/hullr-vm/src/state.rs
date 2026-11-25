//! Files of the detached VM host process and the status/stop logic built on
//! them.
//!
//! ```text
//! <root>/vm/
//!   config.json   VmConfig
//!   disk.img      sparse raw data disk (ext4, formatted by the guest)
//!   vm.lock       flock held by the VM host process for its whole life
//!   vm.pid        pid of the VM host process
//!   state         "starting" until the guest API answers, then "running"
//!   console.log   guest serial console (kernel + init output)
//!   host.log      stdout/stderr of the VM host process
//!   kernel        guest kernel (installed by `vm/build.sh --install`)
//!   initrd        guest initramfs
//! ```
//!
//! Liveness is decided by the lock, not the pid, so a stale pid file left by
//! a crash (and a reused pid) never reads as running.

use hullr_core::paths::{write_atomic, Paths};
use hullr_core::{Error, Result};
use std::fs::File;
use std::os::fd::AsRawFd;
use std::path::PathBuf;
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VmStatus {
    Stopped,
    Starting,
    Running { pid: u32 },
}

/// Paths of the files under `<root>/vm`.
#[derive(Debug, Clone)]
pub struct VmFiles {
    pub dir: PathBuf,
}

impl VmFiles {
    pub fn new(paths: &Paths) -> Self {
        Self { dir: paths.vm() }
    }
    pub fn lock(&self) -> PathBuf {
        self.dir.join("vm.lock")
    }
    pub fn pid(&self) -> PathBuf {
        self.dir.join("vm.pid")
    }
    pub fn state(&self) -> PathBuf {
        self.dir.join("state")
    }
    pub fn disk(&self) -> PathBuf {
        self.dir.join("disk.img")
    }
    pub fn console_log(&self) -> PathBuf {
        self.dir.join("console.log")
    }
    pub fn host_log(&self) -> PathBuf {
        self.dir.join("host.log")
    }
    pub fn kernel(&self) -> PathBuf {
        self.dir.join("kernel")
    }
    pub fn initrd(&self) -> PathBuf {
        self.dir.join("initrd")
    }
}

/// An exclusive, non-blocking flock on `vm.lock`. Released on drop.
pub struct VmLock {
    _file: File,
}

impl VmLock {
    /// Take the lock, or return `None` if another process holds it.
    pub fn try_acquire(files: &VmFiles) -> Result<Option<Self>> {
        std::fs::create_dir_all(&files.dir)?;
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(files.lock())?;
        // SAFETY: flock on a valid, owned fd; no memory is involved.
        let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
        if rc == 0 {
            Ok(Some(Self { _file: file }))
        } else {
            let err = std::io::Error::last_os_error();
            if err.raw_os_error() == Some(libc::EWOULDBLOCK) {
                Ok(None)
            } else {
                Err(err.into())
            }
        }
    }
}

/// Record the host process pid and its state ("starting" or "running").
pub fn write_state(files: &VmFiles, state: &str) -> Result<()> {
    write_atomic(&files.pid(), format!("{}\n", std::process::id()).as_bytes())?;
    write_atomic(&files.state(), format!("{state}\n").as_bytes())?;
    Ok(())
}

/// Remove the pid and state files (the lock file stays; it is harmless).
pub fn clear_state(files: &VmFiles) {
    let _ = std::fs::remove_file(files.pid());
    let _ = std::fs::remove_file(files.state());
}

fn read_pid(files: &VmFiles) -> Option<u32> {
    std::fs::read_to_string(files.pid()).ok()?.trim().parse().ok()
}

/// Is the VM host process alive, and how far has it got?
pub fn status(paths: &Paths) -> VmStatus {
    let files = VmFiles::new(paths);
    match VmLock::try_acquire(&files) {
        // Nobody holds the lock: no VM, whatever the pid file says.
        Ok(Some(_lock)) => return VmStatus::Stopped,
        Ok(None) => {}
        Err(_) => return VmStatus::Stopped,
    }
    let state = std::fs::read_to_string(files.state()).unwrap_or_default();
    match (state.trim(), read_pid(&files)) {
        ("running", Some(pid)) => VmStatus::Running { pid },
        _ => VmStatus::Starting,
    }
}

fn signal(pid: u32, sig: libc::c_int) {
    // SAFETY: kill has no memory preconditions; a stale pid only yields ESRCH.
    unsafe {
        libc::kill(pid as libc::pid_t, sig);
    }
}

async fn wait_stopped(paths: &Paths, timeout: Duration) -> bool {
    let deadline = tokio::time::Instant::now() + timeout;
    while tokio::time::Instant::now() < deadline {
        if status(paths) == VmStatus::Stopped {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    status(paths) == VmStatus::Stopped
}

/// Stop the VM: SIGTERM asks the host process to shut the guest down cleanly
/// (sync and power off); after 30s it is killed.
pub async fn stop(paths: &Paths) -> Result<()> {
    let files = VmFiles::new(paths);
    if status(paths) == VmStatus::Stopped {
        clear_state(&files);
        return Ok(());
    }
    let pid = read_pid(&files).ok_or_else(|| {
        Error::Runtime(format!("vm is running but {} is missing", files.pid().display()))
    })?;
    signal(pid, libc::SIGTERM);
    if wait_stopped(paths, Duration::from_secs(30)).await {
        return Ok(());
    }
    signal(pid, libc::SIGKILL);
    if wait_stopped(paths, Duration::from_secs(5)).await {
        clear_state(&files);
        let _ = std::fs::remove_file(paths.socket());
        return Ok(());
    }
    Err(Error::Runtime(format!("vm host process {pid} did not exit")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_follows_the_lock() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path());
        let files = VmFiles::new(&paths);
        assert_eq!(status(&paths), VmStatus::Stopped);

        // A stale pid file without the lock still reads as stopped.
        std::fs::create_dir_all(&files.dir).unwrap();
        std::fs::write(files.pid(), "1\n").unwrap();
        std::fs::write(files.state(), "running\n").unwrap();
        assert_eq!(status(&paths), VmStatus::Stopped);

        // flock locks are per open file description, so a second open in the
        // same process conflicts just like another process would.
        let lock = VmLock::try_acquire(&files).unwrap().unwrap();
        assert!(VmLock::try_acquire(&files).unwrap().is_none());
        write_state(&files, "starting").unwrap();
        assert_eq!(status(&paths), VmStatus::Starting);
        write_state(&files, "running").unwrap();
        assert_eq!(status(&paths), VmStatus::Running { pid: std::process::id() });
        drop(lock);
        assert_eq!(status(&paths), VmStatus::Stopped);
    }

    #[tokio::test]
    async fn stop_when_stopped_is_ok() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path());
        stop(&paths).await.unwrap();
    }
}
