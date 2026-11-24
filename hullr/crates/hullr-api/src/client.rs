//! `Api` implementation that talks to a daemon over its socket.

use crate::{read_line_raw, Reply, Request};
use futures::stream::BoxStream;
use futures::StreamExt;
use hullr_core::api::{AttachStream, EventStream, LogLineStream, ProgressFn};
use hullr_core::*;
use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use std::path::PathBuf;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

#[derive(Debug, Clone)]
pub struct Client {
    socket: PathBuf,
}

impl Client {
    pub fn new(socket: impl Into<PathBuf>) -> Self {
        Self { socket: socket.into() }
    }

    pub fn socket(&self) -> &PathBuf {
        &self.socket
    }

    /// True if a daemon answers on the socket.
    pub async fn ping(&self) -> bool {
        let fut = self.call::<String>("ping", Value::Null, None);
        matches!(tokio::time::timeout(std::time::Duration::from_secs(3), fut).await, Ok(Ok(_)))
    }

    async fn open(&self, method: &str, params: Value) -> Result<UnixStream> {
        let mut c = UnixStream::connect(&self.socket)
            .await
            .map_err(|e| Error::Io(format!("cannot reach the hullr daemon at {}: {e}", self.socket.display())))?;
        let mut line = serde_json::to_vec(&Request { method: method.into(), params })?;
        line.push(b'\n');
        c.write_all(&line).await?;
        Ok(c)
    }

    async fn call<T: DeserializeOwned>(&self, method: &str, params: Value, progress: Option<&ProgressFn>) -> Result<T> {
        let c = self.open(method, params).await?;
        let mut lines = BufReader::new(c).lines();
        while let Some(l) = lines.next_line().await? {
            match serde_json::from_str::<Reply>(&l)? {
                Reply::Progress(p) => {
                    if let Some(f) = progress {
                        f(p)
                    }
                }
                Reply::Item(_) => {}
                Reply::Ok(v) => return serde_json::from_value(v).map_err(Into::into),
                Reply::Err(e) => return Err(e),
            }
        }
        Err(Error::Io(format!("daemon closed the connection during '{method}'")))
    }

    async fn stream<T: DeserializeOwned + Send + 'static>(
        &self,
        method: &str,
        params: Value,
    ) -> Result<BoxStream<'static, Result<T>>> {
        let c = self.open(method, params).await?;
        let mut lines = BufReader::new(c).lines();
        // First reply says whether the stream opened.
        match lines.next_line().await? {
            Some(l) => match serde_json::from_str::<Reply>(&l)? {
                Reply::Ok(_) => {}
                Reply::Err(e) => return Err(e),
                _ => return Err(Error::Invalid("unexpected reply opening stream".into())),
            },
            None => return Err(Error::Io("daemon closed the stream".into())),
        }
        Ok(futures::stream::unfold(lines, |mut lines| async move {
            loop {
                let l = match lines.next_line().await {
                    Ok(Some(l)) => l,
                    Ok(None) => return None,
                    Err(e) => return Some((Err(e.into()), lines)),
                };
                match serde_json::from_str::<Reply>(&l) {
                    Ok(Reply::Item(v)) => return Some((serde_json::from_value(v).map_err(Into::into), lines)),
                    Ok(Reply::Err(e)) => return Some((Err(e), lines)),
                    Ok(_) => continue,
                    Err(e) => return Some((Err(e.into()), lines)),
                }
            }
        })
        .boxed())
    }

    async fn upgrade(&self, method: &str, params: Value) -> Result<AttachStream> {
        let mut c = self.open(method, params).await?;
        let l = read_line_raw(&mut c).await?.ok_or_else(|| Error::Io("daemon closed the connection".into()))?;
        match serde_json::from_str::<Reply>(&l)? {
            Reply::Ok(_) => Ok(Box::new(c)),
            Reply::Err(e) => Err(e),
            _ => Err(Error::Invalid("unexpected reply to attach".into())),
        }
    }
}

#[async_trait::async_trait]
impl Api for Client {
    async fn info(&self) -> Result<SystemInfo> {
        self.call("info", Value::Null, None).await
    }
    async fn list_containers(&self, all: bool) -> Result<Vec<ContainerInfo>> {
        self.call("list_containers", json!({ "all": all }), None).await
    }
    async fn inspect_container(&self, id: &str) -> Result<ContainerInfo> {
        self.call("inspect_container", json!({ "id": id }), None).await
    }
    async fn create_container(&self, spec: ContainerSpec, progress: ProgressFn) -> Result<String> {
        self.call("create_container", json!({ "spec": spec }), Some(&progress)).await
    }
    async fn start_container(&self, id: &str, wait_for_attach: bool) -> Result<()> {
        self.call("start_container", json!({ "id": id, "wait_for_attach": wait_for_attach }), None).await
    }
    async fn stop_container(&self, id: &str, timeout_secs: u32) -> Result<()> {
        self.call("stop_container", json!({ "id": id, "timeout": timeout_secs }), None).await
    }
    async fn kill_container(&self, id: &str, signal: &str) -> Result<()> {
        self.call("kill_container", json!({ "id": id, "signal": signal }), None).await
    }
    async fn restart_container(&self, id: &str, timeout_secs: u32) -> Result<()> {
        self.call("restart_container", json!({ "id": id, "timeout": timeout_secs }), None).await
    }
    async fn remove_container(&self, id: &str, force: bool) -> Result<()> {
        self.call("remove_container", json!({ "id": id, "force": force }), None).await
    }
    async fn wait_container(&self, id: &str) -> Result<i32> {
        self.call("wait_container", json!({ "id": id }), None).await
    }
    async fn logs(&self, id: &str, opts: LogOptions) -> Result<LogLineStream> {
        self.stream("logs", json!({ "id": id, "opts": opts })).await
    }
    async fn stats(&self, id: &str) -> Result<Stats> {
        self.call("stats", json!({ "id": id }), None).await
    }
    async fn attach(&self, id: &str) -> Result<AttachStream> {
        self.upgrade("attach", json!({ "id": id })).await
    }
    async fn exec(&self, id: &str, spec: ExecSpec) -> Result<AttachStream> {
        self.upgrade("exec", json!({ "id": id, "spec": spec })).await
    }
    async fn list_images(&self) -> Result<Vec<ImageInfo>> {
        self.call("list_images", Value::Null, None).await
    }
    async fn inspect_image(&self, reference: &str) -> Result<ImageInfo> {
        self.call("inspect_image", json!({ "reference": reference }), None).await
    }
    async fn pull_image(&self, reference: &str, platform: Option<Platform>, progress: ProgressFn) -> Result<ImageInfo> {
        self.call("pull_image", json!({ "reference": reference, "platform": platform }), Some(&progress)).await
    }
    async fn push_image(&self, reference: &str, progress: ProgressFn) -> Result<()> {
        self.call("push_image", json!({ "reference": reference }), Some(&progress)).await
    }
    async fn tag_image(&self, source: &str, target: &str) -> Result<()> {
        self.call("tag_image", json!({ "source": source, "target": target }), None).await
    }
    async fn remove_image(&self, reference: &str, force: bool) -> Result<Vec<String>> {
        self.call("remove_image", json!({ "reference": reference, "force": force }), None).await
    }
    async fn build_image(&self, opts: BuildOptions, progress: ProgressFn) -> Result<ImageInfo> {
        self.call("build_image", json!({ "opts": opts }), Some(&progress)).await
    }
    async fn login(&self, registry: &str, username: &str, password: &str) -> Result<()> {
        self.call("login", json!({ "registry": registry, "username": username, "password": password }), None)
            .await
    }
    async fn list_volumes(&self) -> Result<Vec<VolumeInfo>> {
        self.call("list_volumes", Value::Null, None).await
    }
    async fn create_volume(&self, name: &str) -> Result<VolumeInfo> {
        self.call("create_volume", json!({ "name": name }), None).await
    }
    async fn remove_volume(&self, name: &str, force: bool) -> Result<()> {
        self.call("remove_volume", json!({ "name": name, "force": force }), None).await
    }
    async fn compose_up(&self, opts: ComposeOptions, progress: ProgressFn) -> Result<ComposeStatus> {
        self.call("compose_up", json!({ "opts": opts }), Some(&progress)).await
    }
    async fn compose_down(&self, opts: ComposeOptions, progress: ProgressFn) -> Result<()> {
        self.call("compose_down", json!({ "opts": opts }), Some(&progress)).await
    }
    async fn compose_ps(&self, opts: ComposeOptions) -> Result<ComposeStatus> {
        self.call("compose_ps", json!({ "opts": opts }), None).await
    }
    async fn compose_list(&self) -> Result<Vec<ComposeStatus>> {
        self.call("compose_list", Value::Null, None).await
    }
    async fn prune(&self, all_images: bool, volumes: bool) -> Result<PruneReport> {
        self.call("prune", json!({ "all_images": all_images, "volumes": volumes }), None).await
    }
    async fn events(&self) -> Result<EventStream> {
        self.stream("events", Value::Null).await
    }
}
