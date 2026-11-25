//! The contract every front end (CLI, TUI, GUI) programs against.

use crate::*;
use futures::stream::BoxStream;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::io::{AsyncRead, AsyncWrite};

/// Progress reported by long operations (pull, build, compose up).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Progress {
    /// A one-line status message.
    Status { message: String },
    /// Layer download progress. `total` is 0 when unknown.
    Layer { id: String, current: u64, total: u64, done: bool },
    /// Build step `step` of `total`, e.g. `RUN apk add curl`.
    Step { step: usize, total: usize, text: String, cached: bool },
    /// Raw output from a build step or compose service.
    Output { source: String, line: String },
}

pub type ProgressFn = Arc<dyn Fn(Progress) + Send + Sync>;

/// A progress sink that drops everything.
pub fn no_progress() -> ProgressFn {
    Arc::new(|_| {})
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Event {
    pub ts: i64,
    pub kind: EventKind,
    /// Container id, image id or project name.
    pub id: String,
    #[serde(default)]
    pub name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    ContainerCreated,
    ContainerStarted,
    ContainerDied,
    ContainerRemoved,
    ImagePulled,
    ImageBuilt,
    ImageRemoved,
    VolumeCreated,
    VolumeRemoved,
}

pub type EventStream = BoxStream<'static, Result<Event>>;
pub type LogLineStream = BoxStream<'static, Result<LogLine>>;

/// A bidirectional stream speaking `stream::Frame`s.
pub trait AttachIo: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> AttachIo for T {}
pub type AttachStream = Box<dyn AttachIo>;

#[async_trait::async_trait]
pub trait Api: Send + Sync {
    async fn info(&self) -> Result<SystemInfo>;

    // ---- containers ----
    async fn list_containers(&self, all: bool) -> Result<Vec<ContainerInfo>>;
    /// Accepts id, id prefix or name.
    async fn inspect_container(&self, id: &str) -> Result<ContainerInfo>;
    /// Pulls the image if the spec's pull policy says so. Returns the id.
    async fn create_container(&self, spec: ContainerSpec, progress: ProgressFn) -> Result<String>;
    /// With `wait_for_attach`, the process does not exec until the first
    /// attach connects (so no output is lost), or 30s pass.
    async fn start_container(&self, id: &str, wait_for_attach: bool) -> Result<()>;
    async fn stop_container(&self, id: &str, timeout_secs: u32) -> Result<()>;
    async fn kill_container(&self, id: &str, signal: &str) -> Result<()>;
    async fn restart_container(&self, id: &str, timeout_secs: u32) -> Result<()>;
    async fn remove_container(&self, id: &str, force: bool) -> Result<()>;
    /// Wait for exit; returns the exit code.
    async fn wait_container(&self, id: &str) -> Result<i32>;
    async fn logs(&self, id: &str, opts: LogOptions) -> Result<LogLineStream>;
    async fn stats(&self, id: &str) -> Result<Stats>;
    /// Attach to the container's main process stdio.
    async fn attach(&self, id: &str) -> Result<AttachStream>;
    /// Run a new process in a running container.
    async fn exec(&self, id: &str, spec: ExecSpec) -> Result<AttachStream>;

    // ---- images ----
    async fn list_images(&self) -> Result<Vec<ImageInfo>>;
    async fn inspect_image(&self, reference: &str) -> Result<ImageInfo>;
    async fn pull_image(&self, reference: &str, platform: Option<Platform>, progress: ProgressFn) -> Result<ImageInfo>;
    async fn push_image(&self, reference: &str, progress: ProgressFn) -> Result<()>;
    async fn tag_image(&self, source: &str, target: &str) -> Result<()>;
    /// Returns untagged/deleted ids.
    async fn remove_image(&self, reference: &str, force: bool) -> Result<Vec<String>>;
    async fn build_image(&self, opts: BuildOptions, progress: ProgressFn) -> Result<ImageInfo>;
    /// Store registry credentials (in `~/.docker/config.json` format).
    async fn login(&self, registry: &str, username: &str, password: &str) -> Result<()>;

    // ---- volumes ----
    async fn list_volumes(&self) -> Result<Vec<VolumeInfo>>;
    async fn create_volume(&self, name: &str) -> Result<VolumeInfo>;
    async fn remove_volume(&self, name: &str, force: bool) -> Result<()>;

    // ---- compose ----
    async fn compose_up(&self, opts: ComposeOptions, progress: ProgressFn) -> Result<ComposeStatus>;
    async fn compose_down(&self, opts: ComposeOptions, progress: ProgressFn) -> Result<()>;
    async fn compose_ps(&self, opts: ComposeOptions) -> Result<ComposeStatus>;
    /// All projects that have containers.
    async fn compose_list(&self) -> Result<Vec<ComposeStatus>>;

    // ---- system ----
    async fn prune(&self, all_images: bool, volumes: bool) -> Result<PruneReport>;
    async fn events(&self) -> Result<EventStream>;
}
