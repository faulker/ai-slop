//! Linux container runtime. INTERFACE STUB: the signatures are the contract
//! hullr-engine uses; bodies are to be implemented. On non-Linux every call
//! returns `Error::Unsupported` (macOS runs the engine inside a Linux VM).

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
    None
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
}

impl Runtime {
    pub fn new(cfg: RuntimeConfig) -> Result<Self> {
        Ok(Self { cfg })
    }
    pub fn capabilities(&self) -> Capabilities {
        todo!()
    }
    /// Persist the spec and prepare the container dir.
    pub fn create(&self, _spec: &RunSpec) -> Result<()> {
        todo!()
    }
    /// Spawn the detached shim, which owns the container process, writes the
    /// log file, applies the restart policy, and serves attach. Returns once
    /// the process is running (or waiting for attach) or failed to start.
    pub async fn start(&self, _id: &str, _wait_for_attach: bool) -> Result<()> {
        todo!()
    }
    pub fn state(&self, _id: &str) -> Result<RuntimeState> {
        todo!()
    }
    /// Resolves when the container has exited for good (no pending restart).
    pub async fn wait(&self, _id: &str) -> Result<i32> {
        todo!()
    }
    pub fn kill(&self, _id: &str, _signal: i32) -> Result<()> {
        todo!()
    }
    /// stop signal, then SIGKILL after `timeout_secs`.
    pub async fn stop(&self, _id: &str, _timeout_secs: u32) -> Result<()> {
        todo!()
    }
    /// Frame stream (`hullr_core::stream`) to the main process stdio.
    pub async fn attach(&self, _id: &str) -> Result<UnixStream> {
        todo!()
    }
    pub async fn exec(&self, _id: &str, _spec: &ExecSpec) -> Result<UnixStream> {
        todo!()
    }
    pub fn stats(&self, _id: &str) -> Result<Stats> {
        todo!()
    }
    /// JSON-lines `hullr_core::LogLine` file.
    pub fn log_path(&self, _id: &str) -> PathBuf {
        todo!()
    }
    /// Delete a stopped container (unmount + remove its dir).
    pub fn remove(&self, _id: &str) -> Result<()> {
        todo!()
    }
    /// Ensure a named shared network namespace exists, publishing `ports`.
    pub fn ensure_sandbox(&self, _name: &str, _ports: &[PortMapping]) -> Result<()> {
        todo!()
    }
    pub fn remove_sandbox(&self, _name: &str) -> Result<()> {
        todo!()
    }
    /// Run one build step in a writable rootfs dir (no overlay), foreground,
    /// with host networking. Streams output; returns exit code.
    pub async fn run_in_rootfs(
        &self,
        _rootfs: &Path,
        _args: Vec<String>,
        _env: Vec<String>,
        _cwd: String,
        _user: Option<String>,
        _platform: &Platform,
        _output: &(dyn Fn(LogStream, &[u8]) + Send + Sync),
    ) -> Result<i32> {
        todo!()
    }
}

/// Register qemu-user binfmt handlers (needs root). Returns enabled arches.
pub fn enable_emulation() -> Result<Vec<String>> {
    todo!()
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
