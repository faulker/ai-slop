//! Lean Linux container runtime: namespaces, overlayfs, a per-container
//! shim, pasta/slirp4netns networking, qemu binfmt emulation, best-effort
//! cgroup v2. No daemon: each container's detached `__shim` owns it, and
//! every method here works from the files in the container dir.
//!
//! Modules: `shim` (supervisor), `init` (namespaces + pid 1), `mounts`
//! (rootfs), `ns` (user namespaces, setns), `net`, `exec`, `sandbox`,
//! `emulation`, `cgroup`, `stats`, `state` (layout + persisted state),
//! `user` (passwd/group), `util`.
//!
//! On non-Linux every call returns `Error::Unsupported` (macOS runs the
//! engine inside a Linux VM).

#![cfg_attr(not(target_os = "linux"), allow(unused_variables, dead_code))]

pub mod emulation;
pub mod net;
pub mod state;
pub mod user;

#[cfg(target_os = "linux")]
mod cgroup;
#[cfg(target_os = "linux")]
mod exec;
#[cfg(target_os = "linux")]
mod init;
#[cfg(target_os = "linux")]
mod mounts;
#[cfg(target_os = "linux")]
mod ns;
#[cfg(target_os = "linux")]
mod runtime;
#[cfg(target_os = "linux")]
mod sandbox;
#[cfg(target_os = "linux")]
mod shim;
#[cfg(target_os = "linux")]
mod stats;
#[cfg(target_os = "linux")]
mod util;

use hullr_core::{
    ExecSpec, LogStream, MountKind, NetworkMode, Platform, PortMapping, RestartPolicy, Result, Stats,
};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tokio::net::UnixStream;

/// Call first thing in `main`, before any threads or tokio. If the process
/// was re-exec'd as an internal helper (`__shim`, `__init`, `__exec`,
/// `__sandbox`), runs it and returns its exit code; otherwise `None`.
pub fn hidden_entry() -> Option<i32> {
    #[cfg(target_os = "linux")]
    {
        let args: Vec<std::ffi::OsString> = std::env::args_os().collect();
        let role = args.get(1)?.to_str()?;
        let arg = |i: usize| args.get(i).map(PathBuf::from).unwrap_or_default();
        let code = match role {
            "__shim" => shim::main(&arg(2), args.iter().skip(3).any(|a| a == "--wait-attach")),
            "__init" => init::main(&arg(2)),
            "__exec" => {
                let pid = args.get(3).and_then(|p| p.to_str()?.parse().ok()).unwrap_or(0);
                exec::main(&arg(2), pid)
            }
            "__sandbox" => sandbox::main(&arg(2)),
            "__rm" => runtime::rm_main(&arg(2)),
            _ => return None,
        };
        Some(code)
    }
    #[cfg(not(target_os = "linux"))]
    {
        None
    }
}

#[derive(Debug, Clone)]
pub struct RuntimeConfig {
    /// `Paths::containers()`
    pub containers_dir: PathBuf,
    /// `Paths::sandboxes()`
    pub sandboxes_dir: PathBuf,
    /// The hullr binary to re-exec for helpers (usually current_exe()).
    pub exe: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedMount {
    pub kind: MountKind,
    /// Host path (bind / volume data dir). Empty for tmpfs.
    pub source: PathBuf,
    pub target: String,
    pub read_only: bool,
}

/// Fully resolved container config (engine already merged image config).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunSpec {
    pub id: String,
    pub hostname: String,
    /// Flattened image rootfs, read-only lower layer.
    pub lower: PathBuf,
    pub args: Vec<String>,
    pub env: Vec<String>,
    pub cwd: String,
    pub user: Option<String>,
    pub mounts: Vec<ResolvedMount>,
    pub ports: Vec<PortMapping>,
    pub network: NetworkMode,
    pub extra_hosts: Vec<(String, String)>,
    pub tty: bool,
    pub interactive: bool,
    pub platform: Platform,
    pub restart: RestartPolicy,
    pub stop_signal: Option<String>,
    pub memory_limit: Option<u64>,
    pub cpus: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Created,
    Running,
    Restarting,
    Exited,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeState {
    pub status: Status,
    pub pid: Option<u32>,
    pub exit_code: Option<i32>,
    pub started_at: Option<i64>,
    pub finished_at: Option<i64>,
    pub restart_count: u32,
    /// Stopped via stop/kill (restart policy must not restart it).
    pub stopped_by_user: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Capabilities {
    pub rootless: bool,
    pub overlay: bool,
    /// pasta or slirp4netns found (private networking + port publishing).
    pub user_networking: bool,
    pub cgroups: bool,
    /// Foreign platforms runnable now (binfmt registered).
    pub emulated: Vec<Platform>,
    pub warnings: Vec<String>,
}

pub struct Runtime {
    #[allow(dead_code)]
    cfg: RuntimeConfig,
    #[cfg(target_os = "linux")]
    tracker: stats::Tracker,
}

#[cfg(not(target_os = "linux"))]
fn unsupported<T>() -> Result<T> {
    Err(hullr_core::Error::Unsupported("containers run only on Linux (use the hullr VM)".into()))
}

/// Dispatch to the Linux implementation, or fail elsewhere.
macro_rules! imp {
    ($e:expr) => {{
        #[cfg(target_os = "linux")]
        {
            $e
        }
        #[cfg(not(target_os = "linux"))]
        {
            unsupported()
        }
    }};
}

impl Runtime {
    pub fn new(cfg: RuntimeConfig) -> Result<Self> {
        Ok(Self {
            cfg,
            #[cfg(target_os = "linux")]
            tracker: stats::Tracker::default(),
        })
    }
    pub fn capabilities(&self) -> Capabilities {
        #[cfg(target_os = "linux")]
        {
            runtime::capabilities(self)
        }
        #[cfg(not(target_os = "linux"))]
        {
            Capabilities::default()
        }
    }
    /// Persist the spec and prepare the container dir.
    pub fn create(&self, spec: &RunSpec) -> Result<()> {
        imp!(runtime::create(self, spec))
    }
    /// Spawn the detached shim, which owns the container process, writes the
    /// log file, applies the restart policy, and serves attach. Returns once
    /// the process is running (or waiting for attach) or failed to start.
    pub async fn start(&self, id: &str, wait_for_attach: bool) -> Result<()> {
        imp!(runtime::start(self, id, wait_for_attach).await)
    }
    pub fn state(&self, id: &str) -> Result<RuntimeState> {
        imp!(runtime::state(self, id))
    }
    /// Resolves when the container has exited for good (no pending restart).
    pub async fn wait(&self, id: &str) -> Result<i32> {
        imp!(runtime::wait(self, id).await)
    }
    pub fn kill(&self, id: &str, signal: i32) -> Result<()> {
        imp!(runtime::kill(self, id, signal))
    }
    /// stop signal, then SIGKILL after `timeout_secs`.
    pub async fn stop(&self, id: &str, timeout_secs: u32) -> Result<()> {
        imp!(runtime::stop(self, id, timeout_secs).await)
    }
    /// Frame stream (`hullr_core::stream`) to the main process stdio.
    pub async fn attach(&self, id: &str) -> Result<UnixStream> {
        imp!(runtime::attach(self, id).await)
    }
    pub async fn exec(&self, id: &str, spec: &ExecSpec) -> Result<UnixStream> {
        imp!(runtime::exec(self, id, spec).await)
    }
    pub fn stats(&self, id: &str) -> Result<Stats> {
        imp!(runtime::stats(self, id))
    }
    /// JSON-lines `hullr_core::LogLine` file.
    pub fn log_path(&self, id: &str) -> PathBuf {
        state::Layout::new(self.cfg.containers_dir.join(id)).log()
    }
    /// Delete a stopped container (unmount + remove its dir).
    pub fn remove(&self, id: &str) -> Result<()> {
        imp!(runtime::remove(self, id))
    }
    /// Ensure a named shared network namespace exists, publishing `ports`.
    pub fn ensure_sandbox(&self, name: &str, ports: &[PortMapping]) -> Result<()> {
        imp!(runtime::ensure_sandbox(self, name, ports))
    }
    pub fn remove_sandbox(&self, name: &str) -> Result<()> {
        imp!(runtime::remove_sandbox(self, name))
    }
    /// Run one build step in a writable rootfs dir (no overlay), foreground,
    /// with host networking. Streams output; returns exit code.
    pub async fn run_in_rootfs(
        &self,
        rootfs: &Path,
        args: Vec<String>,
        env: Vec<String>,
        cwd: String,
        user: Option<String>,
        platform: &Platform,
        output: &(dyn Fn(LogStream, &[u8]) + Send + Sync),
    ) -> Result<i32> {
        #[cfg(not(target_os = "linux"))]
        let _ = (rootfs, args, env, cwd, user, platform, output);
        imp!(runtime::run_in_rootfs(self, rootfs, args, env, cwd, user, platform, output).await)
    }
}

/// Register qemu-user binfmt handlers (needs root). Returns enabled arches.
pub fn enable_emulation() -> Result<Vec<String>> {
    imp!(emulation::enable())
}

/// Map a signal name (`SIGTERM`, `TERM`, `15`) to its number.
pub fn parse_signal(s: &str) -> Result<i32> {
    let s = s.trim().to_ascii_uppercase();
    let s = s.strip_prefix("SIG").unwrap_or(&s);
    if let Ok(n) = s.parse() {
        return Ok(n);
    }
    Ok(match s {
        "HUP" => 1,
        "INT" => 2,
        "QUIT" => 3,
        "KILL" => 9,
        "USR1" => 10,
        "USR2" => 12,
        "TERM" => 15,
        "STOP" => 19,
        "CONT" => 18,
        "WINCH" => 28,
        _ => return Err(hullr_core::Error::Invalid(format!("unknown signal '{s}'"))),
    })
}
