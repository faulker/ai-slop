//! A canned `Api` for tests.

use crate::testutil::*;
use async_trait::async_trait;
use hullr_core::api::*;
use hullr_core::*;
use std::sync::Mutex;

pub struct MockApi {
    pub containers: Vec<ContainerInfo>,
    pub images: Vec<ImageInfo>,
    pub volumes: Vec<VolumeInfo>,
    pub calls: Mutex<Vec<String>>,
}

impl MockApi {
    pub fn sample() -> Self {
        Self {
            containers: vec![container("web", ContainerState::Running), container("db", ContainerState::Exited)],
            images: vec![image("docker.io/library/nginx:latest")],
            volumes: vec![volume("pgdata")],
            calls: Mutex::new(vec![]),
        }
    }

    fn record(&self, s: impl Into<String>) {
        self.calls.lock().unwrap().push(s.into());
    }

    pub fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
}

fn unsupported<T>() -> Result<T> {
    Err(Error::Unsupported("mock".into()))
}

#[async_trait]
impl Api for MockApi {
    async fn info(&self) -> Result<SystemInfo> {
        Ok(SystemInfo {
            version: "0.1.0".into(),
            os: "linux".into(),
            host_platform: platform(),
            mode: "in-process".into(),
            data_root: "/var/lib/hullr".into(),
            rootless: true,
            emulated_platforms: vec![],
            containers: 2,
            running: 1,
            images: 1,
            warnings: vec![],
        })
    }
    async fn list_containers(&self, all: bool) -> Result<Vec<ContainerInfo>> {
        Ok(self.containers.iter().filter(|c| all || c.state == ContainerState::Running).cloned().collect())
    }
    async fn inspect_container(&self, _id: &str) -> Result<ContainerInfo> {
        unsupported()
    }
    async fn create_container(&self, spec: ContainerSpec, progress: ProgressFn) -> Result<String> {
        progress(Progress::Status { message: "creating".into() });
        self.record(format!("create {}", spec.image));
        Ok("c0ffee".into())
    }
    async fn start_container(&self, id: &str, _w: bool) -> Result<()> {
        self.record(format!("start {id}"));
        Ok(())
    }
    async fn stop_container(&self, id: &str, _t: u32) -> Result<()> {
        self.record(format!("stop {id}"));
        Ok(())
    }
    async fn kill_container(&self, _id: &str, _s: &str) -> Result<()> {
        unsupported()
    }
    async fn restart_container(&self, id: &str, _t: u32) -> Result<()> {
        self.record(format!("restart {id}"));
        Ok(())
    }
    async fn remove_container(&self, id: &str, force: bool) -> Result<()> {
        self.record(format!("rm {id} force={force}"));
        Ok(())
    }
    async fn wait_container(&self, _id: &str) -> Result<i32> {
        unsupported()
    }
    async fn logs(&self, _id: &str, _o: LogOptions) -> Result<LogLineStream> {
        let lines = vec![
            Ok(LogLine { ts: 1, stream: LogStream::Stdout, line: "listening on :80".into() }),
            Ok(LogLine { ts: 2, stream: LogStream::Stderr, line: "warn: slow disk".into() }),
        ];
        Ok(Box::pin(futures::stream::iter(lines)))
    }
    async fn stats(&self, _id: &str) -> Result<Stats> {
        Ok(Stats { cpu_percent: 12.5, memory_bytes: 20_000_000, memory_limit: Some(512_000_000), pids: 4, ..Default::default() })
    }
    async fn attach(&self, _id: &str) -> Result<AttachStream> {
        unsupported()
    }
    async fn exec(&self, _id: &str, _s: ExecSpec) -> Result<AttachStream> {
        unsupported()
    }
    async fn list_images(&self) -> Result<Vec<ImageInfo>> {
        Ok(self.images.clone())
    }
    async fn inspect_image(&self, _r: &str) -> Result<ImageInfo> {
        unsupported()
    }
    async fn pull_image(&self, r: &str, _p: Option<Platform>, progress: ProgressFn) -> Result<ImageInfo> {
        progress(Progress::Layer { id: "abc".into(), current: 5, total: 10, done: false });
        self.record(format!("pull {r}"));
        Ok(image(r))
    }
    async fn push_image(&self, _r: &str, _p: ProgressFn) -> Result<()> {
        unsupported()
    }
    async fn tag_image(&self, _s: &str, _t: &str) -> Result<()> {
        unsupported()
    }
    async fn remove_image(&self, r: &str, _f: bool) -> Result<Vec<String>> {
        self.record(format!("rmi {r}"));
        Ok(vec![r.into()])
    }
    async fn build_image(&self, _o: BuildOptions, _p: ProgressFn) -> Result<ImageInfo> {
        unsupported()
    }
    async fn login(&self, _r: &str, _u: &str, _p: &str) -> Result<()> {
        unsupported()
    }
    async fn list_volumes(&self) -> Result<Vec<VolumeInfo>> {
        Ok(self.volumes.clone())
    }
    async fn create_volume(&self, name: &str) -> Result<VolumeInfo> {
        self.record(format!("volume create {name}"));
        Ok(volume(name))
    }
    async fn remove_volume(&self, name: &str, _f: bool) -> Result<()> {
        self.record(format!("volume rm {name}"));
        Ok(())
    }
    async fn compose_up(&self, _o: ComposeOptions, _p: ProgressFn) -> Result<ComposeStatus> {
        Ok(ComposeStatus { project: "demo".into(), services: vec![] })
    }
    async fn compose_down(&self, _o: ComposeOptions, _p: ProgressFn) -> Result<()> {
        Ok(())
    }
    async fn compose_ps(&self, _o: ComposeOptions) -> Result<ComposeStatus> {
        unsupported()
    }
    async fn compose_list(&self) -> Result<Vec<ComposeStatus>> {
        Ok(vec![])
    }
    async fn prune(&self, a: bool, v: bool) -> Result<PruneReport> {
        self.record(format!("prune {a} {v}"));
        Ok(PruneReport::default())
    }
    async fn events(&self) -> Result<EventStream> {
        unsupported()
    }
}
