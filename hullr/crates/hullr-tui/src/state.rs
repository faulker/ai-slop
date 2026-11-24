//! UI state and the reducer. `App::update` is a pure function from a message
//! to state changes plus a list of [`Cmd`] side effects for the runtime to
//! execute. Nothing in here touches the terminal or the API.

use crate::format::{parse_run_line, sanitize_line};
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use hullr_core::api::Progress;
use hullr_core::{
    ComposeStatus, ContainerInfo, ContainerState, ImageInfo, Stats, SystemInfo, VolumeInfo,
};
use std::collections::VecDeque;
use std::path::PathBuf;

/// Ticks arrive every 250 ms; refresh every 8 of them (2 s).
pub const TICKS_PER_REFRESH: u32 = 8;
pub const MAX_LOG_LINES: usize = 5000;
pub const MAX_HISTORY: usize = 120;
const LOG_PAGE: usize = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Containers,
    Images,
    Volumes,
    Compose,
    System,
}

impl Tab {
    pub const ALL: [Tab; 5] = [Tab::Containers, Tab::Images, Tab::Volumes, Tab::Compose, Tab::System];

    pub fn title(self) -> &'static str {
        match self {
            Tab::Containers => "Containers",
            Tab::Images => "Images",
            Tab::Volumes => "Volumes",
            Tab::Compose => "Compose",
            Tab::System => "System",
        }
    }

    pub fn index(self) -> usize {
        Tab::ALL.iter().position(|t| *t == self).unwrap_or(0)
    }

    pub fn from_index(i: usize) -> Tab {
        Tab::ALL[i % Tab::ALL.len()]
    }

    /// Index into per-list selection arrays; the System tab has no list.
    fn list_slot(self) -> Option<usize> {
        match self {
            Tab::System => None,
            t => Some(t.index()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Info,
    Success,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Status {
    pub text: String,
    pub level: Level,
    ticks_left: u32,
}

/// Side effects requested by the reducer.
#[derive(Debug, Clone, PartialEq)]
pub enum Cmd {
    Refresh { all: bool },
    /// Follow logs and stats for a container; `None` stops watching.
    Watch(Option<WatchTarget>),
    Start(String),
    Stop(String),
    Restart(String),
    RemoveContainer { id: String, force: bool },
    Shell(String),
    Pull(String),
    Run(Box<hullr_core::ContainerSpec>),
    RemoveImage(String),
    CreateVolume(String),
    RemoveVolume(String),
    Prune { all_images: bool, volumes: bool },
    ComposeUp { dir: PathBuf },
    ComposeDown { project: String, dir: PathBuf },
    Quit,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatchTarget {
    pub id: String,
    /// Stats are only polled for running containers.
    pub running: bool,
}

/// Everything that can happen to the UI.
#[derive(Debug)]
pub enum Msg {
    Key(KeyEvent),
    Tick { now: i64 },
    Containers(Result<Vec<ContainerInfo>, String>),
    Images(Result<Vec<ImageInfo>, String>),
    Volumes(Result<Vec<VolumeInfo>, String>),
    Compose(Result<Vec<ComposeStatus>, String>),
    Info(Result<SystemInfo, String>),
    Stats { id: String, stats: Stats },
    Log { id: String, stderr: bool, line: String },
    Progress(Progress),
    /// An action finished. `Ok` carries a success message.
    OpDone(Result<String, String>),
    /// Something changed on the engine (from the event stream).
    EngineEvent,
    /// The terminal was resized or otherwise needs a redraw.
    Redraw,
    /// The compose project's directory, remembered for `down`.
    ComposeDir { project: String, dir: PathBuf },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputKind {
    Pull,
    Run,
    Volume,
    ComposeUp,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayerProgress {
    pub id: String,
    pub current: u64,
    pub total: u64,
    pub done: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProgressView {
    pub title: String,
    pub lines: VecDeque<String>,
    pub layers: Vec<LayerProgress>,
    pub done: Option<Result<String, String>>,
    pub auto_close: bool,
}

impl ProgressView {
    pub fn new(title: impl Into<String>, auto_close: bool) -> Self {
        Self { title: title.into(), lines: VecDeque::new(), layers: Vec::new(), done: None, auto_close }
    }

    fn push_line(&mut self, s: String) {
        self.lines.push_back(sanitize_line(&s));
        while self.lines.len() > 200 {
            self.lines.pop_front();
        }
    }

    pub fn apply(&mut self, p: Progress) {
        match p {
            Progress::Status { message } => self.push_line(message),
            Progress::Layer { id, current, total, done } => {
                if let Some(l) = self.layers.iter_mut().find(|l| l.id == id) {
                    l.current = current;
                    l.total = total;
                    l.done = done;
                } else {
                    self.layers.push(LayerProgress { id, current, total, done });
                }
            }
            Progress::Step { step, total, text, cached } => {
                let c = if cached { " (cached)" } else { "" };
                self.push_line(format!("[{step}/{total}] {text}{c}"));
            }
            Progress::Output { source, line } => self.push_line(format!("{source} | {line}")),
        }
    }

    /// Combined bytes across layers with a known size, as `(current, total)`.
    pub fn overall(&self) -> Option<(u64, u64)> {
        let known: Vec<&LayerProgress> = self.layers.iter().filter(|l| l.total > 0).collect();
        if known.is_empty() {
            return None;
        }
        let total = known.iter().map(|l| l.total).sum();
        let cur = known.iter().map(|l| if l.done { l.total } else { l.current.min(l.total) }).sum();
        Some((cur, total))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Modal {
    None,
    Help,
    Confirm { prompt: String, yes: Cmd },
    Input { kind: InputKind, title: String, hint: String, buf: String },
    Progress(ProgressView),
    /// Fullscreen log viewer for the watched container.
    Logs,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogEntry {
    pub stderr: bool,
    pub text: String,
}

/// Live data for the selected container.
#[derive(Debug, Default)]
pub struct Detail {
    pub stats: Option<Stats>,
    pub cpu_hist: VecDeque<u64>,
    pub mem_hist: VecDeque<u64>,
    pub logs: VecDeque<LogEntry>,
    /// Lines scrolled up from the bottom; 0 means following.
    pub log_scroll: usize,
}

fn push_capped<T>(q: &mut VecDeque<T>, v: T, cap: usize) {
    q.push_back(v);
    while q.len() > cap {
        q.pop_front();
    }
}

pub struct App {
    pub tab: Tab,
    pub now: i64,
    pub cwd: PathBuf,
    pub quit: bool,

    pub containers: Vec<ContainerInfo>,
    pub images: Vec<ImageInfo>,
    pub volumes: Vec<VolumeInfo>,
    pub projects: Vec<ComposeStatus>,
    pub info: Option<SystemInfo>,
    /// Which lists have loaded at least once.
    pub loaded: [bool; 4],
    /// Last refresh error, shown in the empty states.
    pub conn_error: Option<String>,

    pub show_all: bool,
    pub filter: String,
    pub filter_edit: bool,
    pub sel: [usize; 4],
    sel_key: [Option<String>; 4],

    pub modal: Modal,
    pub status: Option<Status>,

    pub detail: Detail,
    watching: Option<String>,
    watch_key: Option<String>,
    compose_dirs: Vec<(String, PathBuf)>,
    ticks: u32,
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

impl App {
    pub fn new() -> Self {
        Self {
            tab: Tab::Containers,
            now: hullr_core::now_unix(),
            cwd: std::env::current_dir().unwrap_or_default(),
            quit: false,
            containers: vec![],
            images: vec![],
            volumes: vec![],
            projects: vec![],
            info: None,
            loaded: [false; 4],
            conn_error: None,
            show_all: true,
            filter: String::new(),
            filter_edit: false,
            sel: [0; 4],
            sel_key: Default::default(),
            modal: Modal::None,
            status: None,
            detail: Detail::default(),
            watching: None,
            watch_key: None,
            compose_dirs: vec![],
            ticks: 0,
        }
    }

    // ---- visible lists ------------------------------------------------

    fn matches(&self, hay: &[&str]) -> bool {
        if self.filter.is_empty() {
            return true;
        }
        let q = self.filter.to_lowercase();
        hay.iter().any(|h| h.to_lowercase().contains(&q))
    }

    pub fn visible_containers(&self) -> Vec<&ContainerInfo> {
        self.containers
            .iter()
            .filter(|c| self.matches(&[&c.name, &c.image, c.short_id()]))
            .collect()
    }

    pub fn visible_images(&self) -> Vec<&ImageInfo> {
        self.images
            .iter()
            .filter(|i| {
                let mut hay: Vec<&str> = i.repo_tags.iter().map(String::as_str).collect();
                hay.push(i.short_id());
                self.matches(&hay)
            })
            .collect()
    }

    pub fn visible_volumes(&self) -> Vec<&VolumeInfo> {
        self.volumes.iter().filter(|v| self.matches(&[&v.name])).collect()
    }

    pub fn visible_projects(&self) -> Vec<&ComposeStatus> {
        self.projects.iter().filter(|p| self.matches(&[&p.project])).collect()
    }

    fn visible_len(&self, tab: Tab) -> usize {
        match tab {
            Tab::Containers => self.visible_containers().len(),
            Tab::Images => self.visible_images().len(),
            Tab::Volumes => self.visible_volumes().len(),
            Tab::Compose => self.visible_projects().len(),
            Tab::System => 0,
        }
    }

    fn key_at(&self, tab: Tab, i: usize) -> Option<String> {
        match tab {
            Tab::Containers => self.visible_containers().get(i).map(|c| c.id.clone()),
            Tab::Images => self.visible_images().get(i).map(|c| c.id.clone()),
            Tab::Volumes => self.visible_volumes().get(i).map(|c| c.name.clone()),
            Tab::Compose => self.visible_projects().get(i).map(|c| c.project.clone()),
            Tab::System => None,
        }
    }

    pub fn selected_container(&self) -> Option<&ContainerInfo> {
        self.visible_containers().get(self.sel[0]).copied()
    }

    pub fn selected_image(&self) -> Option<&ImageInfo> {
        self.visible_images().get(self.sel[1]).copied()
    }

    pub fn selected_volume(&self) -> Option<&VolumeInfo> {
        self.visible_volumes().get(self.sel[2]).copied()
    }

    pub fn selected_project(&self) -> Option<&ComposeStatus> {
        self.visible_projects().get(self.sel[3]).copied()
    }

    /// After the data or the filter changed: keep the same item selected if it
    /// still exists, otherwise clamp.
    fn reselect(&mut self, tab: Tab) {
        let Some(slot) = tab.list_slot() else { return };
        let len = self.visible_len(tab);
        if let Some(key) = self.sel_key[slot].clone() {
            for i in 0..len {
                if self.key_at(tab, i).as_deref() == Some(key.as_str()) {
                    self.sel[slot] = i;
                    return;
                }
            }
        }
        self.sel[slot] = self.sel[slot].min(len.saturating_sub(1));
        self.sel_key[slot] = self.key_at(tab, self.sel[slot]);
    }

    fn move_sel(&mut self, delta: isize) {
        let Some(slot) = self.tab.list_slot() else { return };
        let len = self.visible_len(self.tab);
        if len == 0 {
            return;
        }
        let next = (self.sel[slot] as isize + delta).clamp(0, len as isize - 1) as usize;
        self.set_sel(next);
    }

    fn set_sel(&mut self, i: usize) {
        let Some(slot) = self.tab.list_slot() else { return };
        let len = self.visible_len(self.tab);
        self.sel[slot] = i.min(len.saturating_sub(1));
        self.sel_key[slot] = self.key_at(self.tab, self.sel[slot]);
    }

    // ---- status ---------------------------------------------------------

    pub fn set_status(&mut self, level: Level, text: impl Into<String>) {
        let ticks_left = if level == Level::Error { 24 } else { 14 };
        self.status = Some(Status { text: text.into(), level, ticks_left });
    }

    // ---- reducer --------------------------------------------------------

    pub fn update(&mut self, msg: Msg) -> Vec<Cmd> {
        let mut cmds = Vec::new();
        match msg {
            Msg::Key(k) => self.on_key(k, &mut cmds),
            Msg::Tick { now } => {
                self.now = now;
                self.ticks += 1;
                #[allow(clippy::manual_is_multiple_of)]
                if self.ticks % TICKS_PER_REFRESH == 0 {
                    cmds.push(Cmd::Refresh { all: self.show_all });
                }
                if let Some(s) = &mut self.status {
                    s.ticks_left = s.ticks_left.saturating_sub(1);
                    if s.ticks_left == 0 {
                        self.status = None;
                    }
                }
            }
            Msg::Containers(r) => match r {
                Ok(mut v) => {
                    v.sort_by(|a, b| a.name.cmp(&b.name));
                    self.containers = v;
                    self.loaded[0] = true;
                    self.conn_error = None;
                    self.reselect(Tab::Containers);
                }
                Err(e) => self.conn_error = Some(e),
            },
            Msg::Images(r) => match r {
                Ok(mut v) => {
                    v.sort_by_key(|i| std::cmp::Reverse(i.created));
                    self.images = v;
                    self.loaded[1] = true;
                    self.reselect(Tab::Images);
                }
                Err(e) => self.conn_error = Some(e),
            },
            Msg::Volumes(r) => match r {
                Ok(mut v) => {
                    v.sort_by(|a, b| a.name.cmp(&b.name));
                    self.volumes = v;
                    self.loaded[2] = true;
                    self.reselect(Tab::Volumes);
                }
                Err(e) => self.conn_error = Some(e),
            },
            Msg::Compose(r) => match r {
                Ok(mut v) => {
                    v.sort_by(|a, b| a.project.cmp(&b.project));
                    self.projects = v;
                    self.loaded[3] = true;
                    self.reselect(Tab::Compose);
                }
                Err(e) => self.conn_error = Some(e),
            },
            Msg::Info(r) => match r {
                Ok(i) => self.info = Some(i),
                Err(e) => self.conn_error = Some(e),
            },
            Msg::Stats { id, stats } => {
                if self.watching.as_deref() == Some(&id) {
                    push_capped(&mut self.detail.cpu_hist, (stats.cpu_percent * 10.0).max(0.0) as u64, MAX_HISTORY);
                    push_capped(&mut self.detail.mem_hist, stats.memory_bytes, MAX_HISTORY);
                    self.detail.stats = Some(stats);
                }
            }
            Msg::Log { id, stderr, line } => {
                if self.watching.as_deref() == Some(&id) {
                    push_capped(&mut self.detail.logs, LogEntry { stderr, text: sanitize_line(&line) }, MAX_LOG_LINES);
                    if self.detail.log_scroll > 0 {
                        // Keep the viewport still while new lines arrive.
                        self.detail.log_scroll = (self.detail.log_scroll + 1).min(self.detail.logs.len().saturating_sub(1));
                    }
                }
            }
            Msg::Progress(p) => {
                if let Modal::Progress(v) = &mut self.modal {
                    v.apply(p);
                }
            }
            Msg::OpDone(r) => {
                if let Modal::Progress(v) = &mut self.modal {
                    v.done = Some(r.clone());
                    if v.auto_close && r.is_ok() {
                        self.modal = Modal::None;
                    }
                }
                match &r {
                    Ok(m) => self.set_status(Level::Success, m.clone()),
                    Err(e) => self.set_status(Level::Error, e.clone()),
                }
                cmds.push(Cmd::Refresh { all: self.show_all });
            }
            Msg::Redraw => {}
            Msg::EngineEvent => cmds.push(Cmd::Refresh { all: self.show_all }),
            Msg::ComposeDir { project, dir } => {
                self.compose_dirs.retain(|(p, _)| *p != project);
                self.compose_dirs.push((project, dir));
            }
        }
        self.sync_watch(&mut cmds);
        cmds
    }

    /// Emit a `Watch` command when the container that should be followed
    /// changed (selection, tab, or its running state).
    fn sync_watch(&mut self, cmds: &mut Vec<Cmd>) {
        let want = if self.tab == Tab::Containers {
            self.selected_container().map(|c| (c.id.clone(), c.state == ContainerState::Running, c.started_at))
        } else {
            None
        };
        let key = want.as_ref().map(|(id, run, st)| format!("{id}:{run}:{st:?}"));
        if key != self.watch_key {
            let id_changed = want.as_ref().map(|w| &w.0) != self.watching.as_ref();
            if id_changed || key.is_none() {
                self.detail = Detail::default();
            } else {
                // Same container restarted: logs will be re-fetched.
                self.detail.logs.clear();
                self.detail.log_scroll = 0;
            }
            self.watching = want.as_ref().map(|w| w.0.clone());
            self.watch_key = key;
            cmds.push(Cmd::Watch(want.map(|(id, running, _)| WatchTarget { id, running })));
        }
    }

    // ---- keys -------------------------------------------------------------

    fn on_key(&mut self, k: KeyEvent, cmds: &mut Vec<Cmd>) {
        if k.kind == KeyEventKind::Release {
            return;
        }
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        if ctrl && k.code == KeyCode::Char('c') {
            self.quit = true;
            cmds.push(Cmd::Quit);
            return;
        }
        let modal = std::mem::replace(&mut self.modal, Modal::None);
        match modal {
            Modal::None => {
                if self.filter_edit {
                    self.on_filter_key(k);
                } else {
                    self.on_main_key(k, cmds);
                }
            }
            Modal::Help => {
                // Any key closes help.
            }
            Modal::Confirm { prompt, yes } => match k.code {
                KeyCode::Char('y') | KeyCode::Char('Y') => self.run_cmd(yes, cmds),
                KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc | KeyCode::Char('q') => {}
                _ => self.modal = Modal::Confirm { prompt, yes },
            },
            Modal::Input { kind, title, hint, mut buf } => match k.code {
                KeyCode::Esc => {}
                KeyCode::Enter => self.submit_input(kind, title, hint, buf, cmds),
                KeyCode::Backspace => {
                    buf.pop();
                    self.modal = Modal::Input { kind, title, hint, buf };
                }
                KeyCode::Char('u') if ctrl => {
                    buf.clear();
                    self.modal = Modal::Input { kind, title, hint, buf };
                }
                KeyCode::Char('w') if ctrl => {
                    let trimmed = buf.trim_end().len();
                    let cut = buf[..trimmed].rfind(' ').map(|i| i + 1).unwrap_or(0);
                    buf.truncate(cut);
                    self.modal = Modal::Input { kind, title, hint, buf };
                }
                KeyCode::Char(c) if !ctrl => {
                    buf.push(c);
                    self.modal = Modal::Input { kind, title, hint, buf };
                }
                _ => self.modal = Modal::Input { kind, title, hint, buf },
            },
            Modal::Progress(v) => {
                if v.done.is_some() && matches!(k.code, KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q')) {
                    // closed
                } else if v.done.is_none() && k.code == KeyCode::Esc {
                    self.set_status(Level::Info, "Still running in the background");
                } else {
                    self.modal = Modal::Progress(v);
                }
            }
            Modal::Logs => match k.code {
                KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('l') => {}
                _ => {
                    self.modal = Modal::Logs;
                    self.log_key(k);
                }
            },
        }
    }

    fn log_key(&mut self, k: KeyEvent) {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        let max = self.detail.logs.len().saturating_sub(1);
        let d = &mut self.detail;
        match k.code {
            KeyCode::Char('k') | KeyCode::Up => d.log_scroll = (d.log_scroll + 1).min(max),
            KeyCode::Char('j') | KeyCode::Down => d.log_scroll = d.log_scroll.saturating_sub(1),
            KeyCode::PageUp => d.log_scroll = (d.log_scroll + LOG_PAGE).min(max),
            KeyCode::PageDown => d.log_scroll = d.log_scroll.saturating_sub(LOG_PAGE),
            KeyCode::Char('u') if ctrl => d.log_scroll = (d.log_scroll + LOG_PAGE).min(max),
            KeyCode::Char('d') if ctrl => d.log_scroll = d.log_scroll.saturating_sub(LOG_PAGE),
            KeyCode::Char('g') | KeyCode::Home => d.log_scroll = max,
            KeyCode::Char('G') | KeyCode::End | KeyCode::Char('f') => d.log_scroll = 0,
            _ => {}
        }
    }

    fn on_filter_key(&mut self, k: KeyEvent) {
        match k.code {
            KeyCode::Esc => {
                self.filter.clear();
                self.filter_edit = false;
            }
            KeyCode::Enter => self.filter_edit = false,
            KeyCode::Backspace => {
                self.filter.pop();
            }
            KeyCode::Char(c) if !k.modifiers.contains(KeyModifiers::CONTROL) => self.filter.push(c),
            _ => {}
        }
        for t in [Tab::Containers, Tab::Images, Tab::Volumes, Tab::Compose] {
            if let Some(slot) = t.list_slot() {
                self.sel_key[slot] = None;
                self.sel[slot] = 0;
            }
            self.reselect(t);
        }
    }

    fn switch_tab(&mut self, t: Tab) {
        self.tab = t;
    }

    fn on_main_key(&mut self, k: KeyEvent, cmds: &mut Vec<Cmd>) {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        match k.code {
            KeyCode::Char('q') => {
                self.quit = true;
                cmds.push(Cmd::Quit);
            }
            KeyCode::Char('?') | KeyCode::F(1) => self.modal = Modal::Help,
            KeyCode::Tab => self.switch_tab(Tab::from_index(self.tab.index() + 1)),
            KeyCode::BackTab => self.switch_tab(Tab::from_index(self.tab.index() + Tab::ALL.len() - 1)),
            KeyCode::Right => self.switch_tab(Tab::from_index(self.tab.index() + 1)),
            KeyCode::Left => self.switch_tab(Tab::from_index(self.tab.index() + Tab::ALL.len() - 1)),
            KeyCode::Char(c @ '1'..='5') => self.switch_tab(Tab::from_index(c as usize - '1' as usize)),
            KeyCode::Char('/') => self.filter_edit = true,
            KeyCode::Esc => {
                if !self.filter.is_empty() {
                    self.filter.clear();
                    for t in [Tab::Containers, Tab::Images, Tab::Volumes, Tab::Compose] {
                        self.reselect(t);
                    }
                }
            }
            KeyCode::Char('j') | KeyCode::Down => self.move_sel(1),
            KeyCode::Char('k') | KeyCode::Up => self.move_sel(-1),
            KeyCode::Char('g') | KeyCode::Home => self.set_sel(0),
            KeyCode::Char('G') | KeyCode::End => self.set_sel(usize::MAX),
            KeyCode::PageUp => self.scroll_logs(LOG_PAGE as isize),
            KeyCode::PageDown => self.scroll_logs(-(LOG_PAGE as isize)),
            KeyCode::Char('u') if ctrl => self.scroll_logs(LOG_PAGE as isize),
            KeyCode::Char('d') if ctrl => self.scroll_logs(-(LOG_PAGE as isize)),
            KeyCode::Char('r') if ctrl => cmds.push(Cmd::Refresh { all: self.show_all }),
            KeyCode::F(5) => cmds.push(Cmd::Refresh { all: self.show_all }),
            KeyCode::Char('P') => {
                self.modal = Modal::Confirm {
                    prompt: "Prune stopped containers and unused images? Volumes are kept.".into(),
                    yes: Cmd::Prune { all_images: false, volumes: false },
                }
            }
            KeyCode::Char('A') => {
                self.modal = Modal::Confirm {
                    prompt: "Deep prune: remove ALL unused images and unused VOLUMES (their data is lost)?".into(),
                    yes: Cmd::Prune { all_images: true, volumes: true },
                }
            }
            _ => match self.tab {
                Tab::Containers => self.container_key(k, cmds),
                Tab::Images => self.image_key(k),
                Tab::Volumes => self.volume_key(k),
                Tab::Compose => self.compose_key(k),
                Tab::System => {}
            },
        }
    }

    fn scroll_logs(&mut self, delta: isize) {
        if self.tab != Tab::Containers {
            return;
        }
        let max = self.detail.logs.len().saturating_sub(1) as isize;
        self.detail.log_scroll = (self.detail.log_scroll as isize + delta).clamp(0, max) as usize;
    }

    fn open_input(&mut self, kind: InputKind, title: &str, hint: &str, buf: String) {
        self.modal = Modal::Input { kind, title: title.into(), hint: hint.into(), buf };
    }

    fn container_key(&mut self, k: KeyEvent, cmds: &mut Vec<Cmd>) {
        let sel = self.selected_container().map(|c| (c.id.clone(), c.name.clone(), c.state));
        match (k.code, sel) {
            (KeyCode::Char('r'), _) => self.open_input(
                InputKind::Run,
                "Run a container",
                "image [-p 8080:80] [-e KEY=VAL] [-v src:dst] [--name n] [--restart always] [command...]",
                String::new(),
            ),
            (KeyCode::Char('a'), _) => {
                self.show_all = !self.show_all;
                let msg = if self.show_all { "Showing all containers" } else { "Showing running containers only" };
                self.set_status(Level::Info, msg);
                cmds.push(Cmd::Refresh { all: self.show_all });
            }
            (KeyCode::Char('s'), Some((id, name, _))) => {
                self.set_status(Level::Info, format!("Starting {name}..."));
                cmds.push(Cmd::Start(id));
            }
            (KeyCode::Char('x'), Some((id, name, _))) => {
                self.set_status(Level::Info, format!("Stopping {name}..."));
                cmds.push(Cmd::Stop(id));
            }
            (KeyCode::Char('R'), Some((id, name, _))) => {
                self.set_status(Level::Info, format!("Restarting {name}..."));
                cmds.push(Cmd::Restart(id));
            }
            (KeyCode::Char('d') | KeyCode::Delete, Some((id, name, state))) => {
                let force = matches!(state, ContainerState::Running | ContainerState::Paused | ContainerState::Restarting);
                let prompt = if force {
                    format!("Stop and remove running container '{name}'? Its filesystem is deleted.")
                } else {
                    format!("Remove container '{name}'? Its filesystem is deleted.")
                };
                self.modal = Modal::Confirm { prompt, yes: Cmd::RemoveContainer { id, force } };
            }
            (KeyCode::Char('l') | KeyCode::Enter, Some(_)) => self.modal = Modal::Logs,
            (KeyCode::Char('e'), Some((id, _, state))) => {
                if state == ContainerState::Running {
                    cmds.push(Cmd::Shell(id));
                } else {
                    self.set_status(Level::Error, "Start the container first (s), then open a shell");
                }
            }
            (KeyCode::Char('s' | 'x' | 'R' | 'd' | 'l' | 'e') | KeyCode::Enter | KeyCode::Delete, None) => {
                self.set_status(Level::Info, "No container selected. Press r to run one");
            }
            _ => {}
        }
    }

    fn image_key(&mut self, k: KeyEvent) {
        match k.code {
            KeyCode::Char('p') => self.open_input(
                InputKind::Pull,
                "Pull an image",
                "name[:tag], e.g. nginx:latest or ghcr.io/org/app:1.2",
                String::new(),
            ),
            KeyCode::Char('r') | KeyCode::Enter => {
                if let Some(img) = self.selected_image() {
                    let r = img.repo_tags.first().cloned().unwrap_or_else(|| img.id.clone());
                    let r = crate::format::short_ref(&r).to_string();
                    self.open_input(
                        InputKind::Run,
                        "Run a container",
                        "image [-p 8080:80] [-e KEY=VAL] [-v src:dst] [--name n] [--restart always] [command...]",
                        format!("{r} "),
                    );
                }
            }
            KeyCode::Char('d') | KeyCode::Delete => {
                if let Some(img) = self.selected_image() {
                    let r = img.repo_tags.first().cloned().unwrap_or_else(|| img.id.clone());
                    let shown = crate::format::short_ref(&r).to_string();
                    self.modal = Modal::Confirm {
                        prompt: format!("Remove image '{shown}'?"),
                        yes: Cmd::RemoveImage(r),
                    };
                }
            }
            _ => {}
        }
    }

    fn volume_key(&mut self, k: KeyEvent) {
        match k.code {
            KeyCode::Char('n') => self.open_input(InputKind::Volume, "New volume", "volume name", String::new()),
            KeyCode::Char('d') | KeyCode::Delete => {
                if let Some(v) = self.selected_volume() {
                    self.modal = Modal::Confirm {
                        prompt: format!("Remove volume '{}'? Its data is lost.", v.name),
                        yes: Cmd::RemoveVolume(v.name.clone()),
                    };
                }
            }
            _ => {}
        }
    }

    fn compose_key(&mut self, k: KeyEvent) {
        match k.code {
            KeyCode::Char('u') => {
                let dir = self.cwd.display().to_string();
                self.open_input(
                    InputKind::ComposeUp,
                    "Compose up",
                    "directory containing compose.yaml",
                    dir,
                );
            }
            KeyCode::Char('x') | KeyCode::Char('d') | KeyCode::Delete => {
                if let Some(p) = self.selected_project() {
                    let project = p.project.clone();
                    let dir = self
                        .compose_dirs
                        .iter()
                        .find(|(n, _)| *n == project)
                        .map(|(_, d)| d.clone())
                        .unwrap_or_else(|| self.cwd.clone());
                    self.modal = Modal::Confirm {
                        prompt: format!("Bring down project '{project}'? Containers are stopped and removed."),
                        yes: Cmd::ComposeDown { project, dir },
                    };
                }
            }
            _ => {}
        }
    }

    fn submit_input(&mut self, kind: InputKind, title: String, hint: String, buf: String, cmds: &mut Vec<Cmd>) {
        let text = buf.trim().to_string();
        if text.is_empty() {
            self.set_status(Level::Error, "Nothing entered");
            self.modal = Modal::Input { kind, title, hint, buf };
            return;
        }
        match kind {
            InputKind::Pull => self.run_cmd(Cmd::Pull(text), cmds),
            InputKind::Volume => self.run_cmd(Cmd::CreateVolume(text), cmds),
            InputKind::ComposeUp => self.run_cmd(Cmd::ComposeUp { dir: PathBuf::from(text) }, cmds),
            InputKind::Run => match parse_run_line(&text, &self.cwd) {
                Ok(spec) => self.run_cmd(Cmd::Run(Box::new(spec)), cmds),
                Err(e) => {
                    self.set_status(Level::Error, e);
                    self.modal = Modal::Input { kind, title, hint, buf };
                }
            },
        }
    }

    /// Queue a command, opening a progress popup for long operations.
    fn run_cmd(&mut self, cmd: Cmd, cmds: &mut Vec<Cmd>) {
        match &cmd {
            Cmd::Pull(r) => self.modal = Modal::Progress(ProgressView::new(format!("Pulling {r}"), true)),
            Cmd::Run(s) => self.modal = Modal::Progress(ProgressView::new(format!("Running {}", s.image), true)),
            Cmd::ComposeUp { dir } => {
                self.modal = Modal::Progress(ProgressView::new(format!("Compose up in {}", dir.display()), false))
            }
            Cmd::ComposeDown { project, .. } => {
                self.modal = Modal::Progress(ProgressView::new(format!("Compose down {project}"), false))
            }
            Cmd::Prune { .. } => self.set_status(Level::Info, "Pruning..."),
            Cmd::RemoveContainer { .. } => self.set_status(Level::Info, "Removing..."),
            Cmd::RemoveImage(_) => self.set_status(Level::Info, "Removing image..."),
            Cmd::RemoveVolume(_) | Cmd::CreateVolume(_) => self.set_status(Level::Info, "Working..."),
            _ => {}
        }
        cmds.push(cmd);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::*;

    fn key(c: char) -> Msg {
        Msg::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE))
    }

    fn code(c: KeyCode) -> Msg {
        Msg::Key(KeyEvent::new(c, KeyModifiers::NONE))
    }

    fn loaded() -> App {
        let mut app = App::new();
        app.update(Msg::Containers(Ok(vec![
            container("web", ContainerState::Running),
            container("db", ContainerState::Exited),
            container("cache", ContainerState::Running),
        ])));
        app
    }

    #[test]
    fn containers_sorted_and_first_selected_triggers_watch() {
        let mut app = App::new();
        let cmds = app.update(Msg::Containers(Ok(vec![
            container("web", ContainerState::Running),
            container("db", ContainerState::Exited),
        ])));
        assert_eq!(app.containers[0].name, "db");
        assert_eq!(app.selected_container().unwrap().name, "db");
        assert!(matches!(&cmds[..], [Cmd::Watch(Some(t))] if !t.running));
    }

    #[test]
    fn navigation_clamps_and_rewatches() {
        let mut app = loaded();
        app.update(key('k'));
        assert_eq!(app.sel[0], 0);
        let cmds = app.update(key('j'));
        assert_eq!(app.sel[0], 1);
        assert!(matches!(&cmds[..], [Cmd::Watch(Some(t))] if !t.running)); // db is exited
        app.update(key('G'));
        assert_eq!(app.sel[0], 2);
        app.update(key('j'));
        assert_eq!(app.sel[0], 2);
        app.update(key('g'));
        assert_eq!(app.sel[0], 0);
    }

    #[test]
    fn selection_follows_item_across_refresh() {
        let mut app = loaded();
        app.update(key('G')); // web
        assert_eq!(app.selected_container().unwrap().name, "web");
        app.update(Msg::Containers(Ok(vec![
            container("aaa", ContainerState::Running),
            container("db", ContainerState::Exited),
            container("cache", ContainerState::Running),
            container("web", ContainerState::Running),
        ])));
        assert_eq!(app.selected_container().unwrap().name, "web");
        assert_eq!(app.sel[0], 3);
    }

    #[test]
    fn tabs_switch_with_keys() {
        let mut app = loaded();
        app.update(key('2'));
        assert_eq!(app.tab, Tab::Images);
        app.update(code(KeyCode::Tab));
        assert_eq!(app.tab, Tab::Volumes);
        app.update(code(KeyCode::BackTab));
        assert_eq!(app.tab, Tab::Images);
        app.update(key('5'));
        assert_eq!(app.tab, Tab::System);
        let cmds = app.update(key('1'));
        assert_eq!(app.tab, Tab::Containers);
        assert!(matches!(&cmds[..], [Cmd::Watch(Some(_))]));
    }

    #[test]
    fn leaving_containers_stops_watching() {
        let mut app = loaded();
        let cmds = app.update(key('2'));
        assert_eq!(cmds, vec![Cmd::Watch(None)]);
    }

    #[test]
    fn filter_narrows_and_esc_clears() {
        let mut app = loaded();
        app.update(key('/'));
        assert!(app.filter_edit);
        app.update(key('w'));
        app.update(key('e'));
        assert_eq!(app.visible_containers().len(), 1);
        app.update(code(KeyCode::Enter));
        assert!(!app.filter_edit);
        assert_eq!(app.selected_container().unwrap().name, "web");
        app.update(code(KeyCode::Esc));
        assert_eq!(app.visible_containers().len(), 3);
    }

    #[test]
    fn filter_esc_in_edit_mode_clears() {
        let mut app = loaded();
        app.update(key('/'));
        app.update(key('z'));
        assert_eq!(app.visible_containers().len(), 0);
        app.update(code(KeyCode::Esc));
        assert!(app.filter.is_empty() && !app.filter_edit);
        assert_eq!(app.visible_containers().len(), 3);
    }

    #[test]
    fn remove_needs_confirmation() {
        let mut app = loaded();
        app.update(key('G')); // running web
        let cmds = app.update(key('d'));
        assert!(!cmds.iter().any(|c| matches!(c, Cmd::RemoveContainer { .. })));
        assert!(matches!(&app.modal, Modal::Confirm { prompt, .. } if prompt.contains("Stop and remove")));
        let cmds = app.update(key('y'));
        assert!(cmds.iter().any(|c| matches!(c, Cmd::RemoveContainer { force: true, .. })));
        assert_eq!(app.modal, Modal::None);
    }

    #[test]
    fn confirm_cancel() {
        let mut app = loaded();
        app.update(key('d'));
        let cmds = app.update(key('n'));
        assert!(cmds.iter().all(|c| !matches!(c, Cmd::RemoveContainer { .. })));
        assert_eq!(app.modal, Modal::None);
    }

    #[test]
    fn start_stop_restart_emit_commands() {
        let mut app = loaded();
        let id = app.selected_container().unwrap().id.clone();
        assert!(app.update(key('s')).contains(&Cmd::Start(id.clone())));
        assert!(app.update(key('x')).contains(&Cmd::Stop(id.clone())));
        assert!(app.update(key('R')).contains(&Cmd::Restart(id)));
        assert_eq!(app.status.as_ref().unwrap().level, Level::Info);
    }

    #[test]
    fn shell_requires_running() {
        let mut app = loaded();
        // "cache" sorts first and is running.
        assert_eq!(app.selected_container().unwrap().name, "cache");
        assert!(app.update(key('e')).iter().any(|c| matches!(c, Cmd::Shell(_))));
        app.update(key('j')); // db, exited
        let cmds = app.update(key('e'));
        assert!(!cmds.iter().any(|c| matches!(c, Cmd::Shell(_))));
        assert_eq!(app.status.as_ref().unwrap().level, Level::Error);
    }

    #[test]
    fn toggle_all_refreshes() {
        let mut app = loaded();
        let cmds = app.update(key('a'));
        assert!(!app.show_all);
        assert!(cmds.contains(&Cmd::Refresh { all: false }));
    }

    #[test]
    fn tick_refreshes_every_two_seconds_and_expires_status() {
        let mut app = App::new();
        app.set_status(Level::Info, "hi");
        let mut refreshes = 0;
        for i in 0..16 {
            let cmds = app.update(Msg::Tick { now: 100 + i });
            refreshes += cmds.iter().filter(|c| matches!(c, Cmd::Refresh { .. })).count();
        }
        assert_eq!(refreshes, 2);
        assert!(app.status.is_none());
        assert_eq!(app.now, 115);
    }

    #[test]
    fn engine_event_refreshes() {
        let mut app = App::new();
        assert_eq!(app.update(Msg::EngineEvent), vec![Cmd::Refresh { all: true }]);
    }

    #[test]
    fn logs_and_stats_only_for_watched() {
        let mut app = loaded();
        let id = app.selected_container().unwrap().id.clone();
        app.update(Msg::Log { id: "other".into(), stderr: false, line: "x".into() });
        assert!(app.detail.logs.is_empty());
        app.update(Msg::Log { id: id.clone(), stderr: true, line: "\u{1b}[31mboom\u{1b}[0m".into() });
        assert_eq!(app.detail.logs[0], LogEntry { stderr: true, text: "boom".into() });
        app.update(Msg::Stats { id: id.clone(), stats: Stats { cpu_percent: 12.34, memory_bytes: 1000, ..Default::default() } });
        assert_eq!(app.detail.cpu_hist.back(), Some(&123));
        assert_eq!(app.detail.mem_hist.back(), Some(&1000));
        app.update(Msg::Stats { id: "other".into(), stats: Stats::default() });
        assert_eq!(app.detail.cpu_hist.len(), 1);
    }

    #[test]
    fn log_scroll_holds_position_when_scrolled() {
        let mut app = loaded();
        let id = app.selected_container().unwrap().id.clone();
        for i in 0..30 {
            app.update(Msg::Log { id: id.clone(), stderr: false, line: format!("l{i}") });
        }
        app.update(code(KeyCode::PageUp));
        assert_eq!(app.detail.log_scroll, 10);
        app.update(Msg::Log { id, stderr: false, line: "new".into() });
        assert_eq!(app.detail.log_scroll, 11);
        app.update(Msg::Key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL)));
        assert_eq!(app.detail.log_scroll, 1);
    }

    #[test]
    fn fullscreen_logs_keys() {
        let mut app = loaded();
        let id = app.selected_container().unwrap().id.clone();
        for i in 0..50 {
            app.update(Msg::Log { id: id.clone(), stderr: false, line: format!("l{i}") });
        }
        app.update(key('l'));
        assert_eq!(app.modal, Modal::Logs);
        app.update(key('g'));
        assert_eq!(app.detail.log_scroll, 49);
        app.update(key('G'));
        assert_eq!(app.detail.log_scroll, 0);
        app.update(code(KeyCode::Esc));
        assert_eq!(app.modal, Modal::None);
    }

    #[test]
    fn pull_prompt_to_progress_flow() {
        let mut app = loaded();
        app.update(key('2'));
        app.update(key('p'));
        assert!(matches!(app.modal, Modal::Input { kind: InputKind::Pull, .. }));
        for c in "alpine".chars() {
            app.update(key(c));
        }
        let cmds = app.update(code(KeyCode::Enter));
        assert!(cmds.contains(&Cmd::Pull("alpine".into())));
        assert!(matches!(app.modal, Modal::Progress(_)));
        app.update(Msg::Progress(Progress::Layer { id: "l1".into(), current: 5, total: 10, done: false }));
        app.update(Msg::Progress(Progress::Layer { id: "l1".into(), current: 10, total: 10, done: true }));
        app.update(Msg::Progress(Progress::Status { message: "done".into() }));
        if let Modal::Progress(v) = &app.modal {
            assert_eq!(v.layers.len(), 1);
            assert_eq!(v.overall(), Some((10, 10)));
            assert_eq!(v.lines.len(), 1);
        }
        let cmds = app.update(Msg::OpDone(Ok("Pulled alpine".into())));
        assert_eq!(app.modal, Modal::None);
        assert!(cmds.contains(&Cmd::Refresh { all: true }));
        assert_eq!(app.status.as_ref().unwrap().level, Level::Success);
    }

    #[test]
    fn failed_op_keeps_popup_open() {
        let mut app = App::new();
        app.modal = Modal::Progress(ProgressView::new("Pulling x", true));
        app.update(Msg::OpDone(Err("registry error: nope".into())));
        assert!(matches!(&app.modal, Modal::Progress(v) if v.done == Some(Err("registry error: nope".into()))));
        app.update(code(KeyCode::Esc));
        assert_eq!(app.modal, Modal::None);
        assert_eq!(app.status.as_ref().unwrap().level, Level::Error);
    }

    #[test]
    fn empty_input_rejected_and_run_line_parsed() {
        let mut app = App::new();
        app.update(key('r'));
        let cmds = app.update(code(KeyCode::Enter));
        assert!(cmds.is_empty());
        assert!(matches!(app.modal, Modal::Input { .. }));
        for c in "-p 8080:80 nginx".chars() {
            app.update(key(c));
        }
        let cmds = app.update(code(KeyCode::Enter));
        match &cmds[0] {
            Cmd::Run(s) => {
                assert_eq!(s.image, "nginx");
                assert_eq!(s.ports[0].container_port, 80);
            }
            c => panic!("unexpected {c:?}"),
        }
    }

    #[test]
    fn bad_run_line_stays_in_prompt() {
        let mut app = App::new();
        app.update(key('r'));
        for c in "-p bad nginx".chars() {
            app.update(key(c));
        }
        let cmds = app.update(code(KeyCode::Enter));
        assert!(cmds.is_empty());
        assert!(matches!(app.modal, Modal::Input { .. }));
        assert_eq!(app.status.as_ref().unwrap().level, Level::Error);
    }

    #[test]
    fn input_editing_keys() {
        let mut app = App::new();
        app.update(key('2'));
        app.update(key('p'));
        for c in "foo bar".chars() {
            app.update(key(c));
        }
        app.update(Msg::Key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL)));
        assert!(matches!(&app.modal, Modal::Input { buf, .. } if buf == "foo "));
        app.update(code(KeyCode::Backspace));
        assert!(matches!(&app.modal, Modal::Input { buf, .. } if buf == "foo"));
        app.update(Msg::Key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL)));
        assert!(matches!(&app.modal, Modal::Input { buf, .. } if buf.is_empty()));
        app.update(code(KeyCode::Esc));
        assert_eq!(app.modal, Modal::None);
    }

    #[test]
    fn image_remove_and_run_shortcut() {
        let mut app = App::new();
        app.update(Msg::Images(Ok(vec![image("docker.io/library/nginx:latest")])));
        app.update(key('2'));
        app.update(key('r'));
        assert!(matches!(&app.modal, Modal::Input { kind: InputKind::Run, buf, .. } if buf == "nginx:latest "));
        app.update(code(KeyCode::Esc));
        app.update(key('d'));
        let cmds = app.update(key('y'));
        assert!(cmds.contains(&Cmd::RemoveImage("docker.io/library/nginx:latest".into())));
    }

    #[test]
    fn prune_needs_confirmation() {
        let mut app = App::new();
        let cmds = app.update(key('P'));
        assert!(cmds.is_empty());
        let cmds = app.update(key('y'));
        assert!(cmds.contains(&Cmd::Prune { all_images: false, volumes: false }));
        app.update(key('A'));
        let cmds = app.update(key('y'));
        assert!(cmds.contains(&Cmd::Prune { all_images: true, volumes: true }));
    }

    #[test]
    fn volumes_create_and_delete() {
        let mut app = App::new();
        app.update(Msg::Volumes(Ok(vec![volume("data")])));
        app.update(key('3'));
        app.update(key('n'));
        for c in "cache".chars() {
            app.update(key(c));
        }
        assert!(app.update(code(KeyCode::Enter)).contains(&Cmd::CreateVolume("cache".into())));
        app.update(key('d'));
        assert!(app.update(key('y')).contains(&Cmd::RemoveVolume("data".into())));
    }

    #[test]
    fn compose_down_uses_remembered_dir() {
        let mut app = App::new();
        app.update(Msg::Compose(Ok(vec![ComposeStatus { project: "shop".into(), services: vec![] }])));
        app.update(Msg::ComposeDir { project: "shop".into(), dir: "/srv/shop".into() });
        app.update(key('4'));
        app.update(key('x'));
        let cmds = app.update(key('y'));
        assert!(cmds.contains(&Cmd::ComposeDown { project: "shop".into(), dir: "/srv/shop".into() }));
        assert!(matches!(app.modal, Modal::Progress(_)));
    }

    #[test]
    fn help_toggles_and_any_key_closes() {
        let mut app = App::new();
        app.update(key('?'));
        assert_eq!(app.modal, Modal::Help);
        app.update(key('x'));
        assert_eq!(app.modal, Modal::None);
    }

    #[test]
    fn quit_keys() {
        let mut app = App::new();
        assert_eq!(app.update(key('q')), vec![Cmd::Quit]);
        assert!(app.quit);
        let mut app = App::new();
        app.update(Msg::Key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)));
        assert!(app.quit);
    }

    #[test]
    fn q_in_prompt_types_instead_of_quitting() {
        let mut app = App::new();
        app.update(key('r'));
        app.update(key('q'));
        assert!(!app.quit);
        assert!(matches!(&app.modal, Modal::Input { buf, .. } if buf == "q"));
    }

    #[test]
    fn errors_populate_conn_error_and_clear() {
        let mut app = App::new();
        app.update(Msg::Containers(Err("engine unreachable".into())));
        assert_eq!(app.conn_error.as_deref(), Some("engine unreachable"));
        app.update(Msg::Containers(Ok(vec![])));
        assert!(app.conn_error.is_none() && app.loaded[0]);
    }
}
