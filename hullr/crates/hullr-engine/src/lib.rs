//! The hullr engine: implements `hullr_core::Api` on top of the image store,
//! the runtime, the builder and the compose loader.
//!
//! The engine keeps no in-memory state that matters. Container records live
//! in `<root>/engine/containers/<id>.json`, runtime state is owned by each
//! container's shim, and events are appended to `<root>/engine/events.jsonl`.
//! That is what lets several `hullr` processes (CLI, TUI, daemon) share one
//! data root safely, with or without a daemon.

mod compose;
mod events;
mod health;
mod lock;
mod logs;
mod names;
mod records;
mod resolve;
mod volumes;

use futures::StreamExt;
use hullr_core::api::{AttachStream, EventKind, EventStream, LogLineStream};
use hullr_core::paths::Paths;
use hullr_core::*;
use hullr_image::{ImageStore, StoredImage};
use hullr_runtime::{Runtime, RuntimeConfig, Status};
use records::Record;
use std::path::PathBuf;
use std::sync::Arc;

pub use health::health_loop;

pub struct Engine {
    pub(crate) paths: Paths,
    pub(crate) images: ImageStore,
    pub(crate) runtime: Runtime,
    pub(crate) mode: String,
}

impl Engine {
    /// Open (creating if needed) the data root. `exe` is the hullr binary the
    /// runtime re-execs for its helpers. `mode` is reported by `info`.
    pub fn open(paths: Paths, exe: PathBuf, mode: &str) -> Result<Arc<Self>> {
        paths.ensure().context("creating data root")?;
        std::fs::create_dir_all(records::dir(&paths))?;
        let images = ImageStore::open(paths.images())?;
        let runtime = Runtime::new(RuntimeConfig {
            containers_dir: paths.containers(),
            sandboxes_dir: paths.sandboxes(),
            exe,
        })?;
        Ok(Arc::new(Self { paths, images, runtime, mode: mode.to_string() }))
    }

    pub fn paths(&self) -> &Paths {
        &self.paths
    }

    pub fn runtime(&self) -> &Runtime {
        &self.runtime
    }

    pub(crate) fn emit(&self, kind: EventKind, id: &str, name: &str) {
        events::append(&self.paths, kind, id, name);
    }

    /// Resolve an id, id prefix or name to a record.
    pub(crate) fn find_record(&self, key: &str) -> Result<Record> {
        records::find(&self.paths, key)
    }

    pub(crate) fn to_info(&self, r: &Record) -> ContainerInfo {
        let st = self.runtime.state(&r.id).ok();
        let state = match st.as_ref().map(|s| s.status) {
            Some(Status::Created) | None => ContainerState::Created,
            Some(Status::Running) => ContainerState::Running,
            Some(Status::Restarting) => ContainerState::Restarting,
            Some(Status::Exited) => ContainerState::Exited,
        };
        ContainerInfo {
            id: r.id.clone(),
            name: r.name.clone(),
            image: r.image.clone(),
            image_id: r.image_id.clone(),
            command: r.command.clone(),
            created: r.created,
            state,
            exit_code: st.as_ref().and_then(|s| s.exit_code),
            pid: st.as_ref().and_then(|s| s.pid),
            started_at: st.as_ref().and_then(|s| s.started_at),
            finished_at: st.as_ref().and_then(|s| s.finished_at),
            health: if state == ContainerState::Running { r.health } else { None },
            platform: r.platform.clone(),
            spec: r.spec.clone(),
        }
    }

    pub(crate) fn is_running(&self, id: &str) -> bool {
        matches!(
            self.runtime.state(id).map(|s| s.status),
            Ok(Status::Running) | Ok(Status::Restarting)
        )
    }

    /// Image ids used by any container record.
    pub(crate) fn images_in_use(&self) -> std::collections::HashSet<String> {
        records::list(&self.paths)
            .unwrap_or_default()
            .into_iter()
            .map(|r| r.image_id)
            .collect()
    }

    /// Find an image locally, pulling according to `policy`.
    pub(crate) async fn ensure_image(
        &self,
        reference: &str,
        platform: Option<&Platform>,
        policy: PullPolicy,
        progress: ProgressFn,
    ) -> Result<StoredImage> {
        if policy != PullPolicy::Always {
            if let Some(img) = self.images.find(reference, platform)? {
                return Ok(img);
            }
            if policy == PullPolicy::Never {
                return Err(Error::NotFound(format!("image '{reference}' (pull policy is never)")));
            }
        }
        let r = ImageRef::parse(reference)?;
        let want = platform.cloned().unwrap_or_else(Platform::host);
        progress(api::Progress::Status { message: format!("Pulling {}", r.short()) });
        let img = self.images.pull(&r, &want, progress).await?;
        self.emit(EventKind::ImagePulled, &img.info.id, &r.short());
        Ok(img)
    }

    /// Remove exited `--rm` containers whose owner is gone. Called lazily by
    /// listings and periodically by the daemon.
    pub fn collect_garbage(&self) {
        for r in records::list(&self.paths).unwrap_or_default() {
            if r.spec.auto_remove {
                if let Ok(st) = self.runtime.state(&r.id) {
                    if st.status == Status::Exited {
                        let _ = self.remove_now(&r);
                    }
                }
            }
        }
    }

    fn remove_now(&self, r: &Record) -> Result<()> {
        match self.runtime.remove(&r.id) {
            Ok(()) | Err(Error::NotFound(_)) => {}
            Err(e) => return Err(e),
        }
        records::delete(&self.paths, &r.id)?;
        self.emit(EventKind::ContainerRemoved, &r.id, &r.name);
        Ok(())
    }

    /// Restart containers whose restart policy says they should be running
    /// but whose shim is gone (e.g. after a reboot). Used by the daemon.
    pub async fn restore_restart_policies(&self) {
        for r in records::list(&self.paths).unwrap_or_default() {
            let wants = matches!(r.spec.restart, RestartPolicy::Always | RestartPolicy::UnlessStopped);
            if !wants {
                continue;
            }
            if let Ok(st) = self.runtime.state(&r.id) {
                if st.status == Status::Exited && !st.stopped_by_user {
                    let _ = self.runtime.start(&r.id, false).await;
                }
            }
        }
    }
}

#[async_trait::async_trait]
impl Api for Engine {
    async fn info(&self) -> Result<SystemInfo> {
        let caps = self.runtime.capabilities();
        let recs = records::list(&self.paths)?;
        let running = recs.iter().filter(|r| self.is_running(&r.id)).count();
        Ok(SystemInfo {
            version: env!("CARGO_PKG_VERSION").to_string(),
            os: std::env::consts::OS.to_string(),
            host_platform: Platform::host(),
            mode: self.mode.clone(),
            data_root: self.paths.root.clone(),
            rootless: caps.rootless,
            emulated_platforms: caps.emulated.iter().map(|p| p.to_string()).collect(),
            containers: recs.len(),
            running,
            images: self.images.list().map(|l| l.len()).unwrap_or(0),
            warnings: caps.warnings,
        })
    }

    async fn list_containers(&self, all: bool) -> Result<Vec<ContainerInfo>> {
        self.collect_garbage();
        let mut out: Vec<ContainerInfo> = records::list(&self.paths)?
            .iter()
            .map(|r| self.to_info(r))
            .filter(|c| all || c.state == ContainerState::Running || c.state == ContainerState::Restarting)
            .collect();
        out.sort_by_key(|c| std::cmp::Reverse(c.created));
        Ok(out)
    }

    async fn inspect_container(&self, id: &str) -> Result<ContainerInfo> {
        Ok(self.to_info(&self.find_record(id)?))
    }

    async fn create_container(&self, spec: ContainerSpec, progress: ProgressFn) -> Result<String> {
        resolve::create(self, spec, progress).await
    }

    async fn start_container(&self, id: &str, wait_for_attach: bool) -> Result<()> {
        let r = self.find_record(id)?;
        if self.is_running(&r.id) {
            return Ok(());
        }
        if let NetworkMode::Sandbox(name) = &r.spec.network {
            // Standalone containers in a sandbox publish their own ports.
            self.runtime.ensure_sandbox(name, &r.spec.ports)?;
        }
        self.runtime.start(&r.id, wait_for_attach).await?;
        records::set_health(&self.paths, &r.id, r.spec.healthcheck.as_ref().map(|_| Health::Starting))?;
        self.emit(EventKind::ContainerStarted, &r.id, &r.name);
        Ok(())
    }

    async fn stop_container(&self, id: &str, timeout_secs: u32) -> Result<()> {
        let r = self.find_record(id)?;
        if self.is_running(&r.id) {
            self.runtime.stop(&r.id, timeout_secs).await?;
            self.emit(EventKind::ContainerDied, &r.id, &r.name);
        }
        Ok(())
    }

    async fn kill_container(&self, id: &str, signal: &str) -> Result<()> {
        let r = self.find_record(id)?;
        if !self.is_running(&r.id) {
            return Err(Error::Conflict(format!("container {} is not running", r.name)));
        }
        self.runtime.kill(&r.id, hullr_runtime::parse_signal(signal)?)
    }

    async fn restart_container(&self, id: &str, timeout_secs: u32) -> Result<()> {
        self.stop_container(id, timeout_secs).await?;
        self.start_container(id, false).await
    }

    async fn remove_container(&self, id: &str, force: bool) -> Result<()> {
        let r = self.find_record(id)?;
        if self.is_running(&r.id) {
            if !force {
                return Err(Error::Conflict(format!(
                    "container {} is running; stop it first or use --force",
                    r.name
                )));
            }
            self.runtime.kill(&r.id, 9)?;
            let _ = tokio::time::timeout(std::time::Duration::from_secs(10), self.runtime.wait(&r.id)).await;
        }
        self.remove_now(&r)
    }

    async fn wait_container(&self, id: &str) -> Result<i32> {
        let r = self.find_record(id)?;
        self.runtime.wait(&r.id).await
    }

    async fn logs(&self, id: &str, opts: LogOptions) -> Result<LogLineStream> {
        let r = self.find_record(id)?;
        logs::stream(self.runtime.log_path(&r.id), opts, {
            let id = r.id.clone();
            let paths = self.paths.clone();
            // `follow` ends once the container is gone or exited for good.
            move || {
                let p = paths.container(&id).join("state.json");
                match std::fs::read(&p).ok().and_then(|b| serde_json::from_slice::<hullr_runtime::RuntimeState>(&b).ok()) {
                    Some(s) => matches!(s.status, Status::Running | Status::Restarting | Status::Created),
                    None => false,
                }
            }
        })
    }

    async fn stats(&self, id: &str) -> Result<Stats> {
        let r = self.find_record(id)?;
        if !self.is_running(&r.id) {
            return Ok(Stats::default());
        }
        self.runtime.stats(&r.id)
    }

    async fn attach(&self, id: &str) -> Result<AttachStream> {
        let r = self.find_record(id)?;
        Ok(Box::new(self.runtime.attach(&r.id).await?))
    }

    async fn exec(&self, id: &str, spec: ExecSpec) -> Result<AttachStream> {
        let r = self.find_record(id)?;
        if !self.is_running(&r.id) {
            return Err(Error::Conflict(format!("container {} is not running", r.name)));
        }
        Ok(Box::new(self.runtime.exec(&r.id, &spec).await?))
    }

    async fn list_images(&self) -> Result<Vec<ImageInfo>> {
        let mut l = self.images.list()?;
        l.sort_by_key(|c| std::cmp::Reverse(c.created));
        Ok(l)
    }

    async fn inspect_image(&self, reference: &str) -> Result<ImageInfo> {
        Ok(self.images.get(reference)?.info)
    }

    async fn pull_image(&self, reference: &str, platform: Option<Platform>, progress: ProgressFn) -> Result<ImageInfo> {
        Ok(self.ensure_image(reference, platform.as_ref(), PullPolicy::Always, progress).await?.info)
    }

    async fn push_image(&self, reference: &str, progress: ProgressFn) -> Result<()> {
        let r = ImageRef::parse(reference)?;
        self.images.push(&r, progress).await
    }

    async fn tag_image(&self, source: &str, target: &str) -> Result<()> {
        let img = self.images.get(source)?;
        self.images.tag(&img.info.id, &ImageRef::parse(target)?)
    }

    async fn remove_image(&self, reference: &str, force: bool) -> Result<Vec<String>> {
        let used = self.images_in_use();
        let removed = self.images.remove(reference, force, &|id| used.contains(id))?;
        for id in &removed {
            self.emit(EventKind::ImageRemoved, id, reference);
        }
        Ok(removed)
    }

    async fn build_image(&self, opts: BuildOptions, progress: ProgressFn) -> Result<ImageInfo> {
        let host = BuildHostImpl { engine: self };
        let img = hullr_build::build(&opts, &host, &self.paths.build(), progress).await?;
        self.emit(EventKind::ImageBuilt, &img.info.id, &opts.tags.join(","));
        Ok(img.info)
    }

    async fn login(&self, registry: &str, username: &str, password: &str) -> Result<()> {
        self.images.login(registry, username, password).await
    }

    async fn list_volumes(&self) -> Result<Vec<VolumeInfo>> {
        volumes::list(&self.paths)
    }

    async fn create_volume(&self, name: &str) -> Result<VolumeInfo> {
        let (v, created) = volumes::ensure(&self.paths, name, Default::default())?;
        if created {
            self.emit(EventKind::VolumeCreated, name, name);
        }
        Ok(v)
    }

    async fn remove_volume(&self, name: &str, force: bool) -> Result<()> {
        let users: Vec<String> = records::list(&self.paths)?
            .into_iter()
            .filter(|r| r.volume_names().iter().any(|v| v == name))
            .map(|r| r.name)
            .collect();
        if !users.is_empty() && !force {
            return Err(Error::Conflict(format!("volume {name} is used by: {}", users.join(", "))));
        }
        volumes::remove(&self.paths, name)?;
        self.emit(EventKind::VolumeRemoved, name, name);
        Ok(())
    }

    async fn compose_up(&self, opts: ComposeOptions, progress: ProgressFn) -> Result<ComposeStatus> {
        compose::up(self, opts, progress).await
    }

    async fn compose_down(&self, opts: ComposeOptions, progress: ProgressFn) -> Result<()> {
        compose::down(self, opts, progress).await
    }

    async fn compose_ps(&self, opts: ComposeOptions) -> Result<ComposeStatus> {
        compose::ps(self, &opts)
    }

    async fn compose_list(&self) -> Result<Vec<ComposeStatus>> {
        compose::list(self)
    }

    async fn prune(&self, all_images: bool, prune_volumes: bool) -> Result<PruneReport> {
        let mut report = PruneReport::default();
        for r in records::list(&self.paths)? {
            if !self.is_running(&r.id) && self.remove_now(&r).is_ok() {
                report.containers.push(r.id);
            }
        }
        let used = self.images_in_use();
        let (imgs, bytes) = self.images.prune(all_images, &|id| used.contains(id))?;
        report.images = imgs;
        report.bytes_reclaimed += bytes;
        if prune_volumes {
            let in_use: std::collections::HashSet<String> = records::list(&self.paths)?
                .iter()
                .flat_map(|r| r.volume_names())
                .collect();
            for v in volumes::list(&self.paths)? {
                if !in_use.contains(&v.name) {
                    report.bytes_reclaimed += volumes::size(&v.path);
                    if volumes::remove(&self.paths, &v.name).is_ok() {
                        report.volumes.push(v.name);
                    }
                }
            }
        }
        Ok(report)
    }

    async fn events(&self) -> Result<EventStream> {
        Ok(events::follow(&self.paths).boxed())
    }
}

/// Adapter giving the builder access to the store and runtime.
struct BuildHostImpl<'a> {
    engine: &'a Engine,
}

#[async_trait::async_trait]
impl hullr_build::BuildHost for BuildHostImpl<'_> {
    fn store(&self) -> &ImageStore {
        &self.engine.images
    }

    async fn base_image(&self, reference: &str, platform: &Platform, pull_always: bool, progress: ProgressFn) -> Result<StoredImage> {
        let policy = if pull_always { PullPolicy::Always } else { PullPolicy::Missing };
        self.engine.ensure_image(reference, Some(platform), policy, progress).await
    }

    async fn run(
        &self,
        rootfs: &std::path::Path,
        args: Vec<String>,
        env: Vec<String>,
        cwd: String,
        user: Option<String>,
        platform: &Platform,
        output: &(dyn for<'b> Fn(LogStream, &'b [u8]) + Send + Sync),
    ) -> Result<i32> {
        self.engine.runtime.run_in_rootfs(rootfs, args, env, cwd, user, platform, output).await
    }
}
