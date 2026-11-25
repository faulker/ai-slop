//! Container directory layout, persisted state, and the pure decisions the
//! shim makes about it (restart policy, backoff).
//!
//! `<containers_dir>/<id>/` holds:
//! `spec.json` (RunSpec), `state.json` (RuntimeState, written atomically by
//! the shim), `log.jsonl`, `shim.sock`, `shim.pid`, `shim.log` (shim
//! diagnostics), `stopped` (marker: stopped by the user), `init.json`
//! (config handed to the init), `rootfs/`, `upper/`, `work/`, `hosts`,
//! `resolv.conf`, `hostname`.

use crate::{NetworkMode, RunSpec, RuntimeState, Status};
use hullr_core::{Error, RestartPolicy, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct Layout {
    pub dir: PathBuf,
}

impl Layout {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }
    pub fn spec(&self) -> PathBuf {
        self.dir.join("spec.json")
    }
    pub fn state(&self) -> PathBuf {
        self.dir.join("state.json")
    }
    pub fn log(&self) -> PathBuf {
        self.dir.join("log.jsonl")
    }
    pub fn sock(&self) -> PathBuf {
        self.dir.join("shim.sock")
    }
    pub fn shim_pid(&self) -> PathBuf {
        self.dir.join("shim.pid")
    }
    pub fn shim_log(&self) -> PathBuf {
        self.dir.join("shim.log")
    }
    pub fn stopped(&self) -> PathBuf {
        self.dir.join("stopped")
    }
    pub fn init_cfg(&self) -> PathBuf {
        self.dir.join("init.json")
    }
    pub fn rootfs(&self) -> PathBuf {
        self.dir.join("rootfs")
    }
    pub fn upper(&self) -> PathBuf {
        self.dir.join("upper")
    }
    pub fn work(&self) -> PathBuf {
        self.dir.join("work")
    }
    pub fn copied_marker(&self) -> PathBuf {
        self.dir.join(".rootfs-copied")
    }
    pub fn file(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }

    pub fn read_spec(&self) -> Result<RunSpec> {
        let b = std::fs::read(self.spec()).map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => Error::NotFound(format!("container {}", self.id())),
            _ => e.into(),
        })?;
        Ok(serde_json::from_slice(&b)?)
    }

    pub fn id(&self) -> String {
        self.dir.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
    }

    /// Raw state as last written (no liveness check).
    pub fn read_state(&self) -> Result<RuntimeState> {
        let b = std::fs::read(self.state())?;
        let mut st: RuntimeState = serde_json::from_slice(&b)?;
        if self.stopped().exists() {
            st.stopped_by_user = true;
        }
        Ok(st)
    }

    pub fn write_state(&self, st: &RuntimeState) -> Result<()> {
        hullr_core::paths::write_atomic(&self.state(), &serde_json::to_vec_pretty(st)?)?;
        Ok(())
    }
}

pub fn initial_state() -> RuntimeState {
    RuntimeState {
        status: Status::Created,
        pid: None,
        exit_code: None,
        started_at: None,
        finished_at: None,
        restart_count: 0,
        stopped_by_user: false,
    }
}

/// Correct a persisted state for a shim that is no longer alive: a state
/// claiming to run is reported as exited (137 if no code is known).
pub fn reconcile(mut st: RuntimeState, shim_alive: bool) -> RuntimeState {
    if !shim_alive && matches!(st.status, Status::Running | Status::Restarting) {
        st.status = Status::Exited;
        st.pid = None;
        st.exit_code = Some(st.exit_code.unwrap_or(137));
        st.finished_at = st.finished_at.or(Some(hullr_core::now_unix()));
    }
    st
}

/// Should the shim start the process again after it exited with `code`?
/// `restarts` is how many restarts already happened.
pub fn should_restart(policy: RestartPolicy, code: i32, restarts: u32, stopped_by_user: bool) -> bool {
    if stopped_by_user {
        return false;
    }
    match policy {
        RestartPolicy::No => false,
        RestartPolicy::Always | RestartPolicy::UnlessStopped => true,
        RestartPolicy::OnFailure(max) => code != 0 && (max == 0 || restarts < max),
    }
}

/// Restart backoff: 100ms doubling to 10s, reset after a run of 10s or more.
#[derive(Debug, Clone)]
pub struct Backoff {
    next: Duration,
}

impl Default for Backoff {
    fn default() -> Self {
        Self { next: Duration::from_millis(100) }
    }
}

impl Backoff {
    pub const MAX: Duration = Duration::from_secs(10);

    pub fn next_delay(&mut self, ran_for: Duration) -> Duration {
        if ran_for >= Duration::from_secs(10) {
            self.next = Duration::from_millis(100);
        }
        let d = self.next;
        self.next = (self.next * 2).min(Self::MAX);
        d
    }
}

/// How the init should set up networking. Decided by the shim at each run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NetPlan {
    /// Stay in the host network namespace.
    Host,
    /// New network namespace (lo up); the shim may attach pasta/slirp.
    New,
    /// Join the user (if different) and network namespace of this pid.
    Join(u32),
}

/// Everything `__init` needs, written to a JSON file by the shim (or by
/// `run_in_rootfs`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InitConfig {
    pub spec: RunSpec,
    /// The mount point that becomes `/`.
    pub rootfs: PathBuf,
    /// Overlay upper/work dirs. `None` means `rootfs` is used directly.
    pub overlay: Option<(PathBuf, PathBuf)>,
    /// Marker file for the copy fallback (copy lower into rootfs once).
    pub copied_marker: Option<PathBuf>,
    /// Host files bind-mounted over container paths (`/etc/hosts`, ...).
    pub etc_files: Vec<(PathBuf, String)>,
    pub net: NetPlan,
}

/// Which namespaces / features a network mode needs from the shim.
pub fn wants_user_networking(mode: &NetworkMode) -> bool {
    matches!(mode, NetworkMode::Private)
}

pub fn is_pid_alive(pid: u32) -> bool {
    #[cfg(target_os = "linux")]
    {
        if pid == 0 {
            return false;
        }
        // A zombie still answers kill(0); check /proc state too.
        match std::fs::read_to_string(format!("/proc/{pid}/stat")) {
            Ok(s) => !s.rsplit_once(')').map(|(_, r)| r.trim_start().starts_with('Z')).unwrap_or(false),
            Err(_) => false,
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = pid;
        false
    }
}

/// Pid of the shim from `shim.pid` if it is alive and really is our shim.
pub fn shim_pid(layout: &Layout) -> Option<u32> {
    let pid: u32 = std::fs::read_to_string(layout.shim_pid()).ok()?.trim().parse().ok()?;
    if !is_pid_alive(pid) {
        return None;
    }
    let cmd = std::fs::read(format!("/proc/{pid}/cmdline")).ok()?;
    let has = |needle: &[u8]| cmd.split(|b| *b == 0).any(|a| a == needle);
    (has(b"__shim") && has(layout.dir.as_os_str().as_encoded_bytes())).then_some(pid)
}

pub fn path_str(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restart_policy() {
        use RestartPolicy::*;
        assert!(!should_restart(No, 1, 0, false));
        assert!(should_restart(Always, 0, 50, false));
        assert!(!should_restart(Always, 0, 0, true));
        assert!(should_restart(UnlessStopped, 0, 0, false));
        assert!(!should_restart(OnFailure(2), 0, 0, false));
        assert!(should_restart(OnFailure(2), 1, 1, false));
        assert!(!should_restart(OnFailure(2), 1, 2, false));
        assert!(should_restart(OnFailure(0), 1, 1000, false));
    }

    #[test]
    fn backoff() {
        let mut b = Backoff::default();
        let short = Duration::from_millis(5);
        let d: Vec<u64> = (0..9).map(|_| b.next_delay(short).as_millis() as u64).collect();
        assert_eq!(d, vec![100, 200, 400, 800, 1600, 3200, 6400, 10000, 10000]);
        assert_eq!(b.next_delay(Duration::from_secs(11)).as_millis(), 100);
    }

    #[test]
    fn reconcile_dead_shim() {
        let mut st = initial_state();
        st.status = Status::Running;
        st.pid = Some(42);
        let r = reconcile(st.clone(), false);
        assert_eq!((r.status, r.exit_code, r.pid), (Status::Exited, Some(137), None));
        let r = reconcile(st.clone(), true);
        assert_eq!(r.status, Status::Running);
        let created = reconcile(initial_state(), false);
        assert_eq!(created.status, Status::Created);
    }

    #[test]
    fn state_roundtrip_and_marker() {
        let d = tempfile::tempdir().unwrap();
        let l = Layout::new(d.path());
        let mut st = initial_state();
        st.restart_count = 3;
        l.write_state(&st).unwrap();
        assert_eq!(l.read_state().unwrap(), st);
        std::fs::write(l.stopped(), "").unwrap();
        assert!(l.read_state().unwrap().stopped_by_user);
    }
}
