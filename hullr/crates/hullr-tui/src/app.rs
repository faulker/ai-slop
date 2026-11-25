//! The runtime: owns the terminal, turns [`Cmd`]s into async API calls and
//! their results into [`Msg`]s. All API work happens on spawned tasks, so the
//! draw loop never waits on the engine.

use crate::state::{App, Cmd, Msg, WatchTarget};
use crate::term;
use crate::ui;
use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyEventKind},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen},
};
use futures::StreamExt;
use hullr_core::api::{Api, Progress, ProgressFn};
use hullr_core::container::human_bytes;
use hullr_core::{ComposeOptions, ExecSpec, LogOptions, LogStream};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc::UnboundedSender;
use tokio::task::JoinHandle;

type Tx = UnboundedSender<Msg>;

/// Run the terminal UI until the user quits.
pub async fn run(api: Arc<dyn Api>) -> Result<()> {
    let guard = TermGuard::enter()?;
    let mut terminal = Terminal::new(CrosstermBackend::new(std::io::stdout()))?;
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Msg>();

    let gate = InputGate::spawn(tx.clone());
    let ticker = {
        let tx = tx.clone();
        tokio::spawn(async move {
            let mut iv = tokio::time::interval(Duration::from_millis(250));
            loop {
                iv.tick().await;
                if tx.send(Msg::Tick { now: hullr_core::now_unix() }).is_err() {
                    break;
                }
            }
        })
    };
    let events = spawn_event_watcher(api.clone(), tx.clone());

    let mut rt = Runtime::new(api.clone(), tx.clone());
    let mut app = App::new();
    rt.exec(Cmd::Refresh { all: app.show_all });

    let result: Result<()> = async {
        loop {
            terminal.draw(|f| ui::draw(f, &app))?;
            let Some(first) = rx.recv().await else { break };
            let mut batch = vec![first];
            while batch.len() < 256 {
                match rx.try_recv() {
                    Ok(m) => batch.push(m),
                    Err(_) => break,
                }
            }
            for msg in batch {
                for cmd in app.update(msg) {
                    match cmd {
                        Cmd::Quit => {}
                        Cmd::Shell(id) => {
                            if let Some(note) = run_shell(&api, &gate, &mut terminal, &id).await {
                                app.set_status(note.0, note.1);
                            }
                        }
                        other => rt.exec(other),
                    }
                }
            }
            if app.quit {
                break;
            }
        }
        Ok(())
    }
    .await;

    rt.shutdown();
    ticker.abort();
    events.abort();
    gate.stop();
    drop(guard);
    result
}

/// Suspend the TUI, run an interactive shell in the container, then restore.
async fn run_shell(
    api: &Arc<dyn Api>,
    gate: &InputGate,
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    id: &str,
) -> Option<(crate::state::Level, String)> {
    use crate::state::Level;
    let paused = gate.pause();
    TermGuard::suspend();
    let spec = ExecSpec { cmd: vec!["/bin/sh".into()], tty: true, interactive: true, ..Default::default() };
    let note = match api.exec(id, spec).await {
        Ok(stream) => match term::pump(stream, true).await {
            Ok(0) | Ok(term::DETACHED) => None,
            Ok(code) => Some((Level::Info, format!("Shell exited with code {code}"))),
            Err(e) => Some((Level::Error, format!("Shell failed: {e}"))),
        },
        Err(e) => Some((Level::Error, format!("Could not open a shell: {e}"))),
    };
    if let Some((_, msg)) = &note {
        // The alternate screen is gone, so this stays visible until we return.
        println!("{msg}");
        tokio::time::sleep(Duration::from_millis(900)).await;
    }
    TermGuard::resume();
    let _ = terminal.clear();
    drop(paused);
    note
}

/// Puts the terminal in TUI mode and restores it on drop, including on
/// panics (a hook restores it before the panic message prints).
type PanicHook = Box<dyn Fn(&std::panic::PanicHookInfo<'_>) + Sync + Send + 'static>;

pub struct TermGuard {
    prev_hook: Arc<PanicHook>,
}

impl TermGuard {
    pub fn enter() -> Result<Self> {
        Self::resume();
        let prev = Arc::new(std::panic::take_hook());
        let p2 = prev.clone();
        std::panic::set_hook(Box::new(move |info| {
            Self::suspend();
            p2(info)
        }));
        Ok(Self { prev_hook: prev })
    }

    fn resume() {
        let _ = crossterm::terminal::enable_raw_mode();
        let _ = execute!(std::io::stdout(), EnterAlternateScreen, crossterm::cursor::Hide);
    }

    fn suspend() {
        let _ = execute!(std::io::stdout(), crossterm::cursor::Show, LeaveAlternateScreen);
        let _ = crossterm::terminal::disable_raw_mode();
    }
}

impl Drop for TermGuard {
    fn drop(&mut self) {
        Self::suspend();
        let prev = self.prev_hook.clone();
        std::panic::set_hook(Box::new(move |info| prev(info)));
    }
}

/// Reads terminal events on its own thread and can be paused so a child
/// process (the shell) gets the keyboard to itself.
pub struct InputGate {
    paused: Arc<AtomicBool>,
    idle: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
}

/// Dropping this resumes terminal input.
pub struct Paused {
    paused: Arc<AtomicBool>,
}

impl Drop for Paused {
    fn drop(&mut self) {
        self.paused.store(false, Ordering::SeqCst);
    }
}

impl InputGate {
    pub fn spawn(tx: Tx) -> Self {
        let paused = Arc::new(AtomicBool::new(false));
        let idle = Arc::new(AtomicBool::new(false));
        let stop = Arc::new(AtomicBool::new(false));
        let (p, i, s) = (paused.clone(), idle.clone(), stop.clone());
        std::thread::spawn(move || {
            while !s.load(Ordering::Relaxed) {
                if p.load(Ordering::SeqCst) {
                    i.store(true, Ordering::SeqCst);
                    std::thread::sleep(Duration::from_millis(20));
                    continue;
                }
                i.store(false, Ordering::SeqCst);
                if event::poll(Duration::from_millis(50)).unwrap_or(false) {
                    let msg = match event::read() {
                        Ok(Event::Key(k)) if k.kind != KeyEventKind::Release => Msg::Key(k),
                        Ok(Event::Resize(..)) => Msg::Redraw,
                        _ => continue,
                    };
                    if tx.send(msg).is_err() {
                        break;
                    }
                }
            }
        });
        Self { paused, idle, stop }
    }

    /// Stop reading input until the returned guard drops. Waits (at most
    /// half a second) for an in-flight poll so the reader is truly idle.
    pub fn pause(&self) -> Paused {
        self.idle.store(false, Ordering::SeqCst);
        self.paused.store(true, Ordering::SeqCst);
        for _ in 0..100 {
            if self.idle.load(Ordering::SeqCst) {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        Paused { paused: self.paused.clone() }
    }

    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

fn spawn_event_watcher(api: Arc<dyn Api>, tx: Tx) -> JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            match api.events().await {
                Ok(mut s) => {
                    while let Some(ev) = s.next().await {
                        if ev.is_err() || tx.send(Msg::EngineEvent).is_err() {
                            break;
                        }
                    }
                }
                // This engine has no event stream; polling covers it.
                Err(hullr_core::Error::Unsupported(_)) => return,
                Err(_) => {}
            }
            tokio::time::sleep(Duration::from_secs(3)).await;
        }
    })
}

/// Executes [`Cmd`]s against the API on background tasks.
pub struct Runtime {
    api: Arc<dyn Api>,
    tx: Tx,
    refreshing: Arc<AtomicBool>,
    watchers: Vec<JoinHandle<()>>,
}

fn progress_fn(tx: &Tx) -> ProgressFn {
    let tx = tx.clone();
    Arc::new(move |p: Progress| {
        let _ = tx.send(Msg::Progress(p));
    })
}

fn soft<T: Default>(r: hullr_core::Result<T>) -> Result<T, String> {
    match r {
        Err(hullr_core::Error::Unsupported(_)) => Ok(T::default()),
        r => r.map_err(|e| e.to_string()),
    }
}

impl Runtime {
    pub fn new(api: Arc<dyn Api>, tx: Tx) -> Self {
        Self { api, tx, refreshing: Arc::new(AtomicBool::new(false)), watchers: Vec::new() }
    }

    pub fn shutdown(&mut self) {
        for h in self.watchers.drain(..) {
            h.abort();
        }
    }

    /// Spawn an action whose result becomes an `OpDone` message.
    fn action<F>(&self, fut: F)
    where
        F: std::future::Future<Output = hullr_core::Result<String>> + Send + 'static,
    {
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let r = fut.await.map_err(|e| e.to_string());
            let _ = tx.send(Msg::OpDone(r));
        });
    }

    pub fn exec(&mut self, cmd: Cmd) {
        let api = self.api.clone();
        let tx = self.tx.clone();
        match cmd {
            Cmd::Quit | Cmd::Shell(_) => {}
            Cmd::Refresh { all } => {
                if self.refreshing.swap(true, Ordering::SeqCst) {
                    return;
                }
                let flag = self.refreshing.clone();
                tokio::spawn(async move {
                    let (c, i, v, p, s) = tokio::join!(
                        api.list_containers(all),
                        api.list_images(),
                        api.list_volumes(),
                        api.compose_list(),
                        api.info()
                    );
                    let _ = tx.send(Msg::Containers(c.map_err(|e| e.to_string())));
                    let _ = tx.send(Msg::Images(i.map_err(|e| e.to_string())));
                    let _ = tx.send(Msg::Volumes(v.map_err(|e| e.to_string())));
                    let _ = tx.send(Msg::Compose(soft(p)));
                    let _ = tx.send(Msg::Info(s.map_err(|e| e.to_string())));
                    flag.store(false, Ordering::SeqCst);
                });
            }
            Cmd::Watch(target) => {
                self.shutdown();
                if let Some(t) = target {
                    self.watchers.push(spawn_logs(api.clone(), tx.clone(), t.id.clone()));
                    if t.running {
                        self.watchers.push(spawn_stats(api, tx, t));
                    }
                }
            }
            Cmd::Start(id) => self.action(async move {
                api.start_container(&id, false).await?;
                Ok(format!("Started {}", &id[..id.len().min(12)]))
            }),
            Cmd::Stop(id) => self.action(async move {
                api.stop_container(&id, 10).await?;
                Ok(format!("Stopped {}", &id[..id.len().min(12)]))
            }),
            Cmd::Restart(id) => self.action(async move {
                api.restart_container(&id, 10).await?;
                Ok(format!("Restarted {}", &id[..id.len().min(12)]))
            }),
            Cmd::RemoveContainer { id, force } => self.action(async move {
                api.remove_container(&id, force).await?;
                Ok(format!("Removed {}", &id[..id.len().min(12)]))
            }),
            Cmd::Pull(r) => {
                let p = progress_fn(&tx);
                self.action(async move {
                    api.pull_image(&r, None, p).await?;
                    Ok(format!("Pulled {r}"))
                })
            }
            Cmd::Run(spec) => {
                let p = progress_fn(&tx);
                self.action(async move {
                    let label = spec.name.clone().unwrap_or_else(|| spec.image.clone());
                    let id = api.create_container(*spec, p).await?;
                    api.start_container(&id, false).await?;
                    Ok(format!("Started {label}"))
                })
            }
            Cmd::RemoveImage(r) => self.action(async move {
                api.remove_image(&r, false).await?;
                Ok(format!("Removed image {}", crate::format::short_ref(&r)))
            }),
            Cmd::CreateVolume(n) => self.action(async move {
                api.create_volume(&n).await?;
                Ok(format!("Created volume {n}"))
            }),
            Cmd::RemoveVolume(n) => self.action(async move {
                api.remove_volume(&n, false).await?;
                Ok(format!("Removed volume {n}"))
            }),
            Cmd::Prune { all_images, volumes } => self.action(async move {
                let r = api.prune(all_images, volumes).await?;
                Ok(format!(
                    "Pruned {} container(s), {} image(s), {} volume(s), freed {}",
                    r.containers.len(),
                    r.images.len(),
                    r.volumes.len(),
                    human_bytes(r.bytes_reclaimed)
                ))
            }),
            Cmd::ComposeUp { dir } => {
                let p = progress_fn(&tx);
                let tx2 = tx.clone();
                self.action(async move {
                    let opts = ComposeOptions { project_dir: dir.clone(), ..Default::default() };
                    let st = api.compose_up(opts, p).await?;
                    let _ = tx2.send(Msg::ComposeDir { project: st.project.clone(), dir });
                    Ok(format!("Project {} is up", st.project))
                })
            }
            Cmd::ComposeDown { project, dir } => {
                let p = progress_fn(&tx);
                self.action(async move {
                    let opts = ComposeOptions { project_dir: dir, project_name: Some(project.clone()), ..Default::default() };
                    api.compose_down(opts, p).await?;
                    Ok(format!("Project {project} is down"))
                })
            }
        }
    }
}

fn spawn_logs(api: Arc<dyn Api>, tx: Tx, id: String) -> JoinHandle<()> {
    tokio::spawn(async move {
        let opts = LogOptions { follow: true, tail: Some(200), timestamps: false };
        let Ok(mut s) = api.logs(&id, opts).await else { return };
        while let Some(Ok(l)) = s.next().await {
            let msg = Msg::Log { id: id.clone(), stderr: l.stream == LogStream::Stderr, line: l.line };
            if tx.send(msg).is_err() {
                break;
            }
        }
    })
}

fn spawn_stats(api: Arc<dyn Api>, tx: Tx, t: WatchTarget) -> JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            if let Ok(stats) = api.stats(&t.id).await {
                if tx.send(Msg::Stats { id: t.id.clone(), stats }).is_err() {
                    break;
                }
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::MockApi;
    use tokio::sync::mpsc::unbounded_channel;

    async fn collect(rx: &mut tokio::sync::mpsc::UnboundedReceiver<Msg>, n: usize) -> Vec<Msg> {
        let mut out = Vec::new();
        while out.len() < n {
            match tokio::time::timeout(Duration::from_secs(3), rx.recv()).await {
                Ok(Some(m)) => out.push(m),
                _ => break,
            }
        }
        out
    }

    #[tokio::test]
    async fn refresh_delivers_all_lists() {
        let (tx, mut rx) = unbounded_channel();
        let mut rt = Runtime::new(Arc::new(MockApi::sample()), tx);
        rt.exec(Cmd::Refresh { all: true });
        let msgs = collect(&mut rx, 5).await;
        assert_eq!(msgs.len(), 5);
        assert!(msgs.iter().any(|m| matches!(m, Msg::Containers(Ok(v)) if v.len() == 2)));
        assert!(msgs.iter().any(|m| matches!(m, Msg::Images(Ok(v)) if v.len() == 1)));
        assert!(msgs.iter().any(|m| matches!(m, Msg::Info(Ok(_)))));
        // The in-flight guard released: a second refresh works.
        rt.exec(Cmd::Refresh { all: false });
        let msgs = collect(&mut rx, 5).await;
        assert!(msgs.iter().any(|m| matches!(m, Msg::Containers(Ok(v)) if v.len() == 1)));
    }

    #[tokio::test]
    async fn actions_call_api_and_report() {
        let (tx, mut rx) = unbounded_channel();
        let api = Arc::new(MockApi::sample());
        let mut rt = Runtime::new(api.clone(), tx);
        rt.exec(Cmd::Stop("abc".into()));
        rt.exec(Cmd::RemoveContainer { id: "abc".into(), force: true });
        rt.exec(Cmd::Prune { all_images: false, volumes: false });
        let msgs = collect(&mut rx, 3).await;
        assert!(msgs.iter().all(|m| matches!(m, Msg::OpDone(Ok(_)))));
        let calls = api.calls();
        assert!(calls.contains(&"stop abc".to_string()));
        assert!(calls.contains(&"rm abc force=true".to_string()));
        assert!(calls.contains(&"prune false false".to_string()));
    }

    #[tokio::test]
    async fn failures_become_error_messages() {
        let (tx, mut rx) = unbounded_channel();
        let mut rt = Runtime::new(Arc::new(MockApi::sample()), tx);
        rt.exec(Cmd::RemoveImage("x".into()));
        // MockApi supports remove_image; use an unsupported op instead.
        rt.exec(Cmd::ComposeDown { project: "p".into(), dir: ".".into() });
        let msgs = collect(&mut rx, 2).await;
        assert_eq!(msgs.len(), 2);
    }

    #[tokio::test]
    async fn pull_streams_progress_then_done() {
        let (tx, mut rx) = unbounded_channel();
        let mut rt = Runtime::new(Arc::new(MockApi::sample()), tx);
        rt.exec(Cmd::Pull("alpine".into()));
        let msgs = collect(&mut rx, 2).await;
        assert!(matches!(msgs[0], Msg::Progress(Progress::Layer { .. })));
        assert!(matches!(&msgs[1], Msg::OpDone(Ok(m)) if m == "Pulled alpine"));
    }

    #[tokio::test]
    async fn compose_up_reports_dir() {
        let (tx, mut rx) = unbounded_channel();
        let mut rt = Runtime::new(Arc::new(MockApi::sample()), tx);
        rt.exec(Cmd::ComposeUp { dir: "/srv/demo".into() });
        let msgs = collect(&mut rx, 2).await;
        assert!(msgs.iter().any(|m| matches!(m, Msg::ComposeDir { project, .. } if project == "demo")));
        assert!(msgs.iter().any(|m| matches!(m, Msg::OpDone(Ok(_)))));
    }

    #[tokio::test]
    async fn watch_streams_logs_and_stats_and_can_stop() {
        let (tx, mut rx) = unbounded_channel();
        let mut rt = Runtime::new(Arc::new(MockApi::sample()), tx);
        rt.exec(Cmd::Watch(Some(WatchTarget { id: "c1".into(), running: true })));
        let msgs = collect(&mut rx, 3).await;
        assert!(msgs.iter().any(|m| matches!(m, Msg::Log { stderr: true, .. })));
        assert!(msgs.iter().any(|m| matches!(m, Msg::Stats { .. })));
        rt.exec(Cmd::Watch(None));
        while rx.try_recv().is_ok() {}
        tokio::time::sleep(Duration::from_millis(1300)).await;
        assert!(rx.try_recv().is_err(), "watchers should be aborted");
    }

    #[tokio::test]
    async fn end_to_end_reducer_and_runtime() {
        let (tx, mut rx) = unbounded_channel();
        let mut rt = Runtime::new(Arc::new(MockApi::sample()), tx);
        let mut app = App::new();
        rt.exec(Cmd::Refresh { all: true });
        for m in collect(&mut rx, 5).await {
            for c in app.update(m) {
                rt.exec(c);
            }
        }
        assert_eq!(app.containers.len(), 2);
        assert!(app.selected_container().is_some());
        // Selection triggered a watch, so logs arrive.
        for m in collect(&mut rx, 2).await {
            app.update(m);
        }
        assert!(!app.detail.logs.is_empty());
        assert!(crate::testutil::render(&app, 120, 32).contains("listening on :80"));
    }
}
