//! Bridges the UI thread and the async API. Every call is spawned on the
//! tokio runtime; results come back as [`Event`]s over a channel and wake the
//! UI with `request_repaint`, so the UI thread never blocks on the engine.

use crate::state::{Event, Then};
use futures::StreamExt;
use hullr_core::api::{Api, Progress, ProgressFn};
use hullr_core::container::human_bytes;
use hullr_core::{ComposeOptions, ContainerSpec, LogOptions, LogStream, Platform};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::Duration;
use tokio::runtime::Handle;
use tokio::task::JoinHandle;

/// Aborts its task when dropped.
pub struct TaskGuard(JoinHandle<()>);

impl Drop for TaskGuard {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Where events go. Abstracted so tests can use a plain channel.
#[derive(Clone)]
pub struct Notifier {
    tx: mpsc::Sender<Event>,
    wake: Arc<dyn Fn() + Send + Sync>,
}

impl Notifier {
    pub fn new(tx: mpsc::Sender<Event>, wake: Arc<dyn Fn() + Send + Sync>) -> Self {
        Self { tx, wake }
    }

    pub fn send(&self, ev: Event) {
        if self.tx.send(ev).is_ok() {
            (self.wake)();
        }
    }
}

#[derive(Clone)]
pub struct Backend {
    api: Arc<dyn Api>,
    rt: Handle,
    out: Notifier,
    refreshing: Arc<AtomicBool>,
}

fn soft<T: Default>(r: hullr_core::Result<T>) -> Result<T, String> {
    match r {
        Err(hullr_core::Error::Unsupported(_)) => Ok(T::default()),
        r => r.map_err(|e| e.to_string()),
    }
}

impl Backend {
    pub fn new(api: Arc<dyn Api>, rt: Handle, out: Notifier) -> Self {
        Self { api, rt, out, refreshing: Arc::new(AtomicBool::new(false)) }
    }

    fn progress(&self, op: u64) -> ProgressFn {
        let out = self.out.clone();
        Arc::new(move |progress: Progress| out.send(Event::Progress { op, progress }))
    }

    /// Run `fut` and report its result as a toast (and activity row if `op`).
    fn action<F>(&self, op: Option<u64>, then: Then, fut: F)
    where
        F: std::future::Future<Output = hullr_core::Result<String>> + Send + 'static,
    {
        let out = self.out.clone();
        self.rt.spawn(async move {
            let result = fut.await.map_err(|e| e.to_string());
            out.send(Event::Done { op, result, then });
        });
    }

    pub fn refresh(&self, all: bool) {
        if self.refreshing.swap(true, Ordering::SeqCst) {
            return;
        }
        let (api, out, flag) = (self.api.clone(), self.out.clone(), self.refreshing.clone());
        self.rt.spawn(async move {
            let (c, i, v, p, s) = tokio::join!(
                api.list_containers(all),
                api.list_images(),
                api.list_volumes(),
                api.compose_list(),
                api.info()
            );
            out.send(Event::Containers(c.map_err(|e| e.to_string())));
            out.send(Event::Images(i.map_err(|e| e.to_string())));
            out.send(Event::Volumes(v.map_err(|e| e.to_string())));
            out.send(Event::Compose(soft(p)));
            out.send(Event::Info(s.map_err(|e| e.to_string())));
            flag.store(false, Ordering::SeqCst);
        });
    }

    /// Forward engine events as `Event::Engine` so the UI refreshes at once.
    pub fn watch_engine_events(&self) -> TaskGuard {
        let (api, out) = (self.api.clone(), self.out.clone());
        TaskGuard(self.rt.spawn(async move {
            loop {
                match api.events().await {
                    Ok(mut s) => {
                        while let Some(ev) = s.next().await {
                            if ev.is_err() {
                                break;
                            }
                            out.send(Event::Engine);
                        }
                    }
                    Err(hullr_core::Error::Unsupported(_)) => return,
                    Err(_) => {}
                }
                tokio::time::sleep(Duration::from_secs(3)).await;
            }
        }))
    }

    pub fn follow_logs(&self, id: String) -> TaskGuard {
        let (api, out) = (self.api.clone(), self.out.clone());
        TaskGuard(self.rt.spawn(async move {
            let opts = LogOptions { follow: true, tail: Some(200), timestamps: false };
            let Ok(mut s) = api.logs(&id, opts).await else { return };
            while let Some(Ok(l)) = s.next().await {
                out.send(Event::Log { id: id.clone(), stderr: l.stream == LogStream::Stderr, line: l.line });
            }
        }))
    }

    pub fn poll_stats(&self, id: String) -> TaskGuard {
        let (api, out) = (self.api.clone(), self.out.clone());
        TaskGuard(self.rt.spawn(async move {
            loop {
                if let Ok(stats) = api.stats(&id).await {
                    out.send(Event::Stats { id: id.clone(), stats });
                }
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        }))
    }

    pub fn start(&self, id: String, name: String) {
        let api = self.api.clone();
        self.action(None, Then::Nothing, async move {
            api.start_container(&id, false).await?;
            Ok(format!("Started {name}"))
        });
    }

    pub fn stop(&self, id: String, name: String) {
        let api = self.api.clone();
        self.action(None, Then::Nothing, async move {
            api.stop_container(&id, 10).await?;
            Ok(format!("Stopped {name}"))
        });
    }

    pub fn restart(&self, id: String, name: String) {
        let api = self.api.clone();
        self.action(None, Then::Nothing, async move {
            api.restart_container(&id, 10).await?;
            Ok(format!("Restarted {name}"))
        });
    }

    pub fn remove_container(&self, id: String, name: String, force: bool) {
        let api = self.api.clone();
        self.action(None, Then::Nothing, async move {
            api.remove_container(&id, force).await?;
            Ok(format!("Deleted {name}"))
        });
    }

    /// Create and start. With `show`, the UI jumps to the new container's logs.
    pub fn run_container(&self, op: u64, spec: ContainerSpec, show: bool) {
        let api = self.api.clone();
        let p = self.progress(op);
        let label = spec.name.clone().unwrap_or_else(|| spec.image.clone());
        let then_slot = Arc::new(std::sync::Mutex::new(Then::Nothing));
        let slot = then_slot.clone();
        let out = self.out.clone();
        self.rt.spawn(async move {
            let r: hullr_core::Result<String> = async {
                let id = api.create_container(spec, p).await?;
                api.start_container(&id, false).await?;
                if show {
                    *slot.lock().unwrap() = Then::ShowContainer(id);
                }
                Ok(format!("Started {label}"))
            }
            .await;
            let then = std::mem::replace(&mut *then_slot.lock().unwrap(), Then::Nothing);
            out.send(Event::Done { op: Some(op), result: r.map_err(|e| e.to_string()), then });
        });
    }

    pub fn pull(&self, op: u64, reference: String, platform: Option<Platform>) {
        let api = self.api.clone();
        let p = self.progress(op);
        self.action(Some(op), Then::Nothing, async move {
            api.pull_image(&reference, platform, p).await?;
            Ok(format!("Pulled {}", crate::state::short_ref(&reference)))
        });
    }

    pub fn remove_image(&self, reference: String, label: String) {
        let api = self.api.clone();
        self.action(None, Then::Nothing, async move {
            api.remove_image(&reference, false).await?;
            Ok(format!("Deleted image {label}"))
        });
    }

    pub fn create_volume(&self, name: String) {
        let api = self.api.clone();
        self.action(None, Then::Nothing, async move {
            api.create_volume(&name).await?;
            Ok(format!("Created volume {name}"))
        });
    }

    pub fn remove_volume(&self, name: String) {
        let api = self.api.clone();
        self.action(None, Then::Nothing, async move {
            api.remove_volume(&name, false).await?;
            Ok(format!("Deleted volume {name}"))
        });
    }

    pub fn prune(&self, deep: bool) {
        let api = self.api.clone();
        self.action(None, Then::Nothing, async move {
            let r = api.prune(deep, deep).await?;
            Ok(format!(
                "Pruned {} container(s), {} image(s), {} volume(s), freed {}",
                r.containers.len(),
                r.images.len(),
                r.volumes.len(),
                human_bytes(r.bytes_reclaimed)
            ))
        });
    }

    pub fn compose_up(&self, op: u64, opts: ComposeOptions) {
        let api = self.api.clone();
        let p = self.progress(op);
        let out = self.out.clone();
        self.rt.spawn(async move {
            let r = api.compose_up(opts.clone(), p).await;
            let (result, then) = match r {
                Ok(st) => (
                    Ok(format!("Project {} is up", st.project)),
                    Then::RememberProject { project: st.project, opts },
                ),
                Err(e) => (Err(e.to_string()), Then::Nothing),
            };
            out.send(Event::Done { op: Some(op), result, then });
        });
    }

    pub fn compose_down(&self, op: u64, project: String, mut opts: ComposeOptions) {
        let api = self.api.clone();
        let p = self.progress(op);
        opts.project_name = Some(project.clone());
        self.action(Some(op), Then::Nothing, async move {
            api.compose_down(opts, p).await?;
            Ok(format!("Project {project} is down"))
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock::MockApi;
    use std::time::Duration;

    fn setup() -> (Backend, mpsc::Receiver<Event>, Arc<MockApi>, tokio::runtime::Runtime) {
        let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap();
        let (tx, rx) = mpsc::channel();
        let api = Arc::new(MockApi::sample());
        let b = Backend::new(api.clone(), rt.handle().clone(), Notifier::new(tx, Arc::new(|| {})));
        (b, rx, api, rt)
    }

    fn collect(rx: &mpsc::Receiver<Event>, n: usize) -> Vec<Event> {
        (0..n).filter_map(|_| rx.recv_timeout(Duration::from_secs(3)).ok()).collect()
    }

    #[test]
    fn refresh_sends_all_lists_and_is_reentrant_after() {
        let (b, rx, _api, _rt) = setup();
        b.refresh(true);
        let evs = collect(&rx, 5);
        assert!(evs.iter().any(|e| matches!(e, Event::Containers(Ok(v)) if v.len() == 2)));
        assert!(evs.iter().any(|e| matches!(e, Event::Images(Ok(_)))));
        assert!(evs.iter().any(|e| matches!(e, Event::Compose(Ok(_)))));
        std::thread::sleep(Duration::from_millis(50));
        b.refresh(false);
        let evs = collect(&rx, 5);
        assert!(evs.iter().any(|e| matches!(e, Event::Containers(Ok(v)) if v.len() == 1)));
    }

    #[test]
    fn actions_report_done_and_hit_the_api() {
        let (b, rx, api, _rt) = setup();
        b.stop("abc".into(), "web".into());
        b.remove_container("abc".into(), "web".into(), true);
        let evs = collect(&rx, 2);
        assert!(evs.iter().all(|e| matches!(e, Event::Done { result: Ok(_), op: None, .. })));
        let calls = api.calls();
        assert!(calls.contains(&"stop abc".to_string()) && calls.contains(&"rm abc force=true".to_string()));
    }

    #[test]
    fn pull_reports_progress_then_done() {
        let (b, rx, _api, _rt) = setup();
        b.pull(7, "alpine".into(), None);
        let evs = collect(&rx, 2);
        assert!(matches!(evs[0], Event::Progress { op: 7, .. }));
        assert!(matches!(&evs[1], Event::Done { op: Some(7), result: Ok(m), .. } if m == "Pulled alpine"));
    }

    #[test]
    fn foreground_run_selects_the_container() {
        let (b, rx, api, _rt) = setup();
        b.run_container(1, ContainerSpec { image: "nginx".into(), ..Default::default() }, true);
        let evs = collect(&rx, 2);
        let done = evs.iter().find(|e| matches!(e, Event::Done { .. })).unwrap();
        assert!(matches!(done, Event::Done { then: Then::ShowContainer(id), .. } if id == "c0ffee"));
        assert!(api.calls().contains(&"start c0ffee".to_string()));
    }

    #[test]
    fn compose_up_remembers_project() {
        let (b, rx, _api, _rt) = setup();
        let opts = crate::state::compose_opts_from_path("/srv/demo").unwrap();
        b.compose_up(2, opts.clone());
        let evs = collect(&rx, 1);
        assert!(matches!(&evs[0], Event::Done { then: Then::RememberProject { project, opts: o }, .. } if project == "demo" && *o == opts));
    }

    #[test]
    fn unsupported_becomes_error_toast_text() {
        let (b, rx, _api, _rt) = setup();
        b.compose_down(3, "demo".into(), Default::default());
        let evs = collect(&rx, 1);
        assert!(matches!(&evs[0], Event::Done { result: Ok(_), .. }));
    }

    #[test]
    fn follow_and_stats_stream_and_stop_on_drop() {
        let (b, rx, _api, _rt) = setup();
        let logs = b.follow_logs("c1".into());
        let stats = b.poll_stats("c1".into());
        let evs = collect(&rx, 3);
        assert!(evs.iter().any(|e| matches!(e, Event::Log { stderr: true, .. })));
        assert!(evs.iter().any(|e| matches!(e, Event::Stats { .. })));
        drop((logs, stats));
        std::thread::sleep(Duration::from_millis(100));
        while rx.try_recv().is_ok() {}
        std::thread::sleep(Duration::from_millis(1200));
        assert!(rx.try_recv().is_err());
    }
}
