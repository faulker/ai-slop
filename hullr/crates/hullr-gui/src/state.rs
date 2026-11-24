//! All GUI state and the logic that does not need a window: toasts, the run
//! dialog conversion, progress aggregation, event application. Everything in
//! here is plain data and plain functions so it can be unit tested.

use hullr_core::api::Progress;
use hullr_core::{
    ComposeOptions, ComposeStatus, ContainerInfo, ContainerSpec, ContainerState, ImageInfo, Mount,
    Platform, PortMapping, Protocol, RestartPolicy, Stats, SystemInfo, VolumeInfo,
};
use std::collections::{BTreeMap, VecDeque};
use std::path::{Path, PathBuf};

pub const MAX_LOG_LINES: usize = 10_000;
pub const MAX_HISTORY: usize = 120;
pub const MAX_TOASTS: usize = 5;

// ---------------------------------------------------------------- toasts

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Info,
    Success,
    Error,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Toast {
    pub id: u64,
    pub text: String,
    pub level: Level,
    /// Seconds (egui clock) at which it was created or last refreshed.
    pub created: f64,
    pub ttl: f64,
}

#[derive(Debug, Default)]
pub struct Toasts {
    pub items: Vec<Toast>,
    next_id: u64,
}

impl Toasts {
    /// Add a toast. An identical live toast is refreshed instead of
    /// duplicated, so a failing 2 s refresh does not flood the screen.
    pub fn push(&mut self, level: Level, text: impl Into<String>, now: f64) {
        let text = text.into();
        let ttl = if level == Level::Error { 9.0 } else { 4.0 };
        if let Some(t) = self.items.iter_mut().find(|t| t.text == text && t.level == level) {
            t.created = now;
            return;
        }
        self.next_id += 1;
        self.items.push(Toast { id: self.next_id, text, level, created: now, ttl });
        while self.items.len() > MAX_TOASTS {
            self.items.remove(0);
        }
    }

    pub fn expire(&mut self, now: f64) {
        self.items.retain(|t| now - t.created < t.ttl);
    }

    pub fn dismiss(&mut self, id: u64) {
        self.items.retain(|t| t.id != id);
    }
}

// ---------------------------------------------------------------- progress

#[derive(Debug, Clone, PartialEq)]
pub struct LayerProgress {
    pub id: String,
    pub current: u64,
    pub total: u64,
    pub done: bool,
}

impl LayerProgress {
    pub fn fraction(&self) -> f32 {
        if self.done {
            1.0
        } else if self.total == 0 {
            0.0
        } else {
            (self.current as f64 / self.total as f64).clamp(0.0, 1.0) as f32
        }
    }
}

/// Aggregated progress of a long operation.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ProgressState {
    pub layers: Vec<LayerProgress>,
    pub lines: VecDeque<String>,
    /// `(step, total)` of the build or compose step in flight.
    pub step: Option<(usize, usize)>,
}

impl ProgressState {
    pub fn apply(&mut self, p: Progress) {
        match p {
            Progress::Status { message } => self.push_line(message),
            Progress::Layer { id, current, total, done } => {
                match self.layers.iter_mut().find(|l| l.id == id) {
                    Some(l) => {
                        l.current = current;
                        l.total = total;
                        l.done = done;
                    }
                    None => self.layers.push(LayerProgress { id, current, total, done }),
                }
            }
            Progress::Step { step, total, text, cached } => {
                self.step = Some((step, total));
                let c = if cached { " (cached)" } else { "" };
                self.push_line(format!("[{step}/{total}] {text}{c}"));
            }
            Progress::Output { source, line } => self.push_line(format!("{source} | {line}")),
        }
    }

    fn push_line(&mut self, s: String) {
        self.lines.push_back(s);
        while self.lines.len() > 300 {
            self.lines.pop_front();
        }
    }

    /// Overall download fraction over layers with a known size.
    pub fn fraction(&self) -> Option<f32> {
        let (cur, total) = self.bytes()?;
        Some((cur as f64 / total.max(1) as f64).clamp(0.0, 1.0) as f32)
    }

    /// `(downloaded, total)` over layers with a known size.
    pub fn bytes(&self) -> Option<(u64, u64)> {
        let known: Vec<&LayerProgress> = self.layers.iter().filter(|l| l.total > 0).collect();
        if known.is_empty() {
            return None;
        }
        let total = known.iter().map(|l| l.total).sum();
        let cur = known.iter().map(|l| if l.done { l.total } else { l.current.min(l.total) }).sum();
        Some((cur, total))
    }

    pub fn layers_done(&self) -> usize {
        self.layers.iter().filter(|l| l.done).count()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum OpStatus {
    Running,
    Done(String),
    Failed(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Operation {
    pub id: u64,
    pub title: String,
    pub progress: ProgressState,
    pub status: OpStatus,
}

// ---------------------------------------------------------------- run dialog

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PlatformChoice {
    #[default]
    Native,
    Amd64,
    Arm64,
}

impl PlatformChoice {
    pub const ALL: [PlatformChoice; 3] = [PlatformChoice::Native, PlatformChoice::Amd64, PlatformChoice::Arm64];

    pub fn label(self) -> &'static str {
        match self {
            PlatformChoice::Native => "native",
            PlatformChoice::Amd64 => "linux/amd64",
            PlatformChoice::Arm64 => "linux/arm64",
        }
    }

    pub fn platform(self) -> Option<Platform> {
        match self {
            PlatformChoice::Native => None,
            PlatformChoice::Amd64 => Platform::parse("linux/amd64").ok(),
            PlatformChoice::Arm64 => Platform::parse("linux/arm64").ok(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RestartChoice {
    #[default]
    No,
    Always,
    UnlessStopped,
    OnFailure,
}

impl RestartChoice {
    pub const ALL: [RestartChoice; 4] =
        [RestartChoice::No, RestartChoice::Always, RestartChoice::UnlessStopped, RestartChoice::OnFailure];

    pub fn label(self) -> &'static str {
        match self {
            RestartChoice::No => "no",
            RestartChoice::Always => "always",
            RestartChoice::UnlessStopped => "unless-stopped",
            RestartChoice::OnFailure => "on-failure",
        }
    }

    pub fn policy(self) -> RestartPolicy {
        match self {
            RestartChoice::No => RestartPolicy::No,
            RestartChoice::Always => RestartPolicy::Always,
            RestartChoice::UnlessStopped => RestartPolicy::UnlessStopped,
            RestartChoice::OnFailure => RestartPolicy::OnFailure(0),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct PortRow {
    pub host: String,
    pub container: String,
    pub udp: bool,
}

impl PortRow {
    pub fn is_blank(&self) -> bool {
        self.host.trim().is_empty() && self.container.trim().is_empty()
    }

    /// `None` for a blank row. A lone container port publishes on the same
    /// host port. A single `8080:80/udp` style entry in the host box is
    /// accepted too, so pasting a docker-style mapping just works.
    pub fn to_mapping(&self) -> Result<Option<PortMapping>, String> {
        if self.is_blank() {
            return Ok(None);
        }
        let host = self.host.trim();
        let ctr = self.container.trim();
        if ctr.is_empty() && (host.contains(':') || host.contains('/')) {
            return PortMapping::parse(host).map(Some).map_err(|e| e.to_string());
        }
        let port = |label: &str, s: &str| -> Result<u16, String> {
            match s.parse::<u16>() {
                Ok(p) if p > 0 => Ok(p),
                _ => Err(format!("{label} port '{s}' must be a number from 1 to 65535")),
            }
        };
        let (h, c) = match (host.is_empty(), ctr.is_empty()) {
            (false, false) => (port("Host", host)?, port("Container", ctr)?),
            (true, false) => {
                let c = port("Container", ctr)?;
                (c, c)
            }
            (false, true) => {
                let h = port("Host", host)?;
                (h, h)
            }
            (true, true) => unreachable!(),
        };
        Ok(Some(PortMapping {
            host_ip: None,
            host_port: h,
            container_port: c,
            protocol: if self.udp { Protocol::Udp } else { Protocol::Tcp },
        }))
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct EnvRow {
    pub key: String,
    pub value: String,
}

impl EnvRow {
    /// `KEY=VALUE`, or `None` for a blank row. Pasting `KEY=VALUE` into the
    /// key box also works.
    pub fn to_env(&self) -> Result<Option<String>, String> {
        let key = self.key.trim();
        if key.is_empty() && self.value.is_empty() {
            return Ok(None);
        }
        if self.value.is_empty() {
            if let Some((k, v)) = key.split_once('=') {
                return EnvRow { key: k.into(), value: v.into() }.to_env();
            }
        }
        if key.is_empty() {
            return Err("An environment variable needs a name".into());
        }
        if key.contains('=') || key.chars().any(char::is_whitespace) {
            return Err(format!("Environment variable name '{key}' must not contain spaces or '='"));
        }
        Ok(Some(format!("{key}={}", self.value)))
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct VolumeRow {
    /// Host path or volume name; empty for an anonymous volume.
    pub source: String,
    pub target: String,
    pub read_only: bool,
}

impl VolumeRow {
    pub fn to_mount(&self, base: &Path) -> Result<Option<Mount>, String> {
        let (src, dst) = (self.source.trim(), self.target.trim());
        if src.is_empty() && dst.is_empty() {
            return Ok(None);
        }
        if dst.is_empty() {
            return Err(format!("Volume '{src}' needs a path inside the container"));
        }
        let spec = if src.is_empty() {
            dst.to_string()
        } else {
            format!("{src}:{dst}{}", if self.read_only { ":ro" } else { "" })
        };
        Mount::parse(&spec, base).map(Some).map_err(|e| e.to_string())
    }
}

/// Split a command line into words, honouring quotes.
pub fn split_words(line: &str) -> Result<Vec<String>, String> {
    let mut words = Vec::new();
    let mut cur = String::new();
    let mut in_word = false;
    let mut quote: Option<char> = None;
    for c in line.chars() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), c) => cur.push(c),
            (None, '"') | (None, '\'') => {
                quote = Some(c);
                in_word = true;
            }
            (None, c) if c.is_whitespace() => {
                if in_word {
                    words.push(std::mem::take(&mut cur));
                    in_word = false;
                }
            }
            (None, c) => {
                cur.push(c);
                in_word = true;
            }
        }
    }
    if quote.is_some() {
        return Err("The command has an unclosed quote".into());
    }
    if in_word {
        words.push(cur);
    }
    Ok(words)
}

#[derive(Debug, Clone, PartialEq)]
pub struct RunDialog {
    pub image: String,
    pub name: String,
    pub command: String,
    pub ports: Vec<PortRow>,
    pub env: Vec<EnvRow>,
    pub volumes: Vec<VolumeRow>,
    pub platform: PlatformChoice,
    pub restart: RestartChoice,
    /// Run in the background. When off, the logs open after the start.
    pub detach: bool,
    pub auto_remove: bool,
    pub error: Option<String>,
}

impl Default for RunDialog {
    fn default() -> Self {
        Self {
            image: String::new(),
            name: String::new(),
            command: String::new(),
            ports: vec![PortRow::default()],
            env: vec![EnvRow::default()],
            volumes: vec![VolumeRow::default()],
            platform: PlatformChoice::Native,
            restart: RestartChoice::No,
            detach: true,
            auto_remove: false,
            error: None,
        }
    }
}

impl RunDialog {
    pub fn for_image(image: &str) -> Self {
        Self { image: image.to_string(), ..Default::default() }
    }

    /// Validate the form and build the spec handed to `create_container`.
    pub fn to_spec(&self, base: &Path) -> Result<ContainerSpec, String> {
        let image = self.image.trim();
        if image.is_empty() {
            return Err("Enter an image to run, for example nginx:latest".into());
        }
        let mut spec = ContainerSpec { image: image.to_string(), ..Default::default() };
        let name = self.name.trim();
        if !name.is_empty() {
            if name.chars().any(|c| c.is_whitespace() || c == '/') {
                return Err("The container name must not contain spaces or '/'".into());
            }
            spec.name = Some(name.to_string());
        }
        let words = split_words(&self.command)?;
        if !words.is_empty() {
            spec.cmd = Some(words);
        }
        for r in &self.ports {
            if let Some(p) = r.to_mapping()? {
                if spec.ports.iter().any(|q| q.host_port == p.host_port && q.protocol == p.protocol) {
                    return Err(format!("Host port {} is used twice", p.host_port));
                }
                spec.ports.push(p);
            }
        }
        for r in &self.env {
            if let Some(e) = r.to_env()? {
                spec.env.push(e);
            }
        }
        for r in &self.volumes {
            if let Some(m) = r.to_mount(base)? {
                spec.mounts.push(m);
            }
        }
        spec.platform = self.platform.platform();
        spec.restart = self.restart.policy();
        spec.auto_remove = self.auto_remove;
        Ok(spec)
    }
}

// ---------------------------------------------------------------- compose

/// Turn what the user typed into compose options. A `.yml` / `.yaml` path is a
/// compose file (its folder is the project dir); anything else is a folder.
pub fn compose_opts_from_path(input: &str) -> Result<ComposeOptions, String> {
    let s = input.trim();
    if s.is_empty() {
        return Err("Enter the path of a compose file or of a folder containing one".into());
    }
    let s = match s.strip_prefix("~/") {
        Some(rest) => dirs::home_dir().unwrap_or_default().join(rest),
        None => PathBuf::from(s),
    };
    let ext = s.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase);
    if matches!(ext.as_deref(), Some("yml") | Some("yaml")) {
        let dir = s.parent().filter(|p| !p.as_os_str().is_empty()).map(Path::to_path_buf).unwrap_or_else(|| PathBuf::from("."));
        Ok(ComposeOptions { files: vec![s], project_dir: dir, ..Default::default() })
    } else {
        Ok(ComposeOptions { project_dir: s, ..Default::default() })
    }
}

/// Remembers where each compose project came from so Down and Open folder
/// work later. Stored as JSON in the user's config directory.
#[derive(Debug, Default, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct KnownProjects {
    pub projects: BTreeMap<String, ComposeOptions>,
}

impl KnownProjects {
    pub fn default_path() -> Option<PathBuf> {
        dirs::config_dir().map(|d| d.join("hullr").join("gui-projects.json"))
    }

    pub fn load(path: &Path) -> Self {
        std::fs::read_to_string(path).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, serde_json::to_string_pretty(self)?)
    }
}

// ---------------------------------------------------------------- helpers

/// Links to offer for a container's published TCP ports.
pub fn port_links(ports: &[PortMapping]) -> Vec<(u16, String)> {
    ports
        .iter()
        .filter(|p| p.protocol == Protocol::Tcp)
        .map(|p| {
            let host = match p.host_ip.as_deref() {
                None | Some("") | Some("0.0.0.0") | Some("::") => "localhost",
                Some(ip) => ip,
            };
            (p.host_port, format!("http://{host}:{}", p.host_port))
        })
        .collect()
}

/// Case-insensitive substring filter over log lines.
pub fn filter_logs<'a>(logs: &'a VecDeque<LogEntry>, query: &str) -> Vec<&'a LogEntry> {
    let q = query.trim().to_lowercase();
    logs.iter().filter(|l| q.is_empty() || l.text.to_lowercase().contains(&q)).collect()
}

/// Remove ANSI escapes and control characters from log text.
pub fn sanitize_line(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        match c {
            '\u{1b}' => {
                if it.peek() == Some(&'[') {
                    it.next();
                    for n in it.by_ref() {
                        if ('\u{40}'..='\u{7e}').contains(&n) {
                            break;
                        }
                    }
                } else {
                    it.next();
                }
            }
            '\t' => out.push_str("    "),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out
}

pub fn fmt_datetime(unix: i64) -> String {
    let days = unix.div_euclid(86400);
    let secs = unix.rem_euclid(86400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02} {:02}:{:02} UTC", secs / 3600, (secs % 3600) / 60)
}

pub fn short_ref(r: &str) -> &str {
    r.strip_prefix("docker.io/library/").or_else(|| r.strip_prefix("docker.io/")).unwrap_or(r)
}

// ---------------------------------------------------------------- app state

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Containers,
    Images,
    Volumes,
    Compose,
    System,
}

impl Page {
    pub const ALL: [Page; 5] = [Page::Containers, Page::Images, Page::Volumes, Page::Compose, Page::System];

    pub fn title(self) -> &'static str {
        match self {
            Page::Containers => "Containers",
            Page::Images => "Images",
            Page::Volumes => "Volumes",
            Page::Compose => "Compose",
            Page::System => "System",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetailTab {
    Overview,
    Logs,
    Stats,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ThemeChoice {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogEntry {
    pub stderr: bool,
    pub text: String,
}

#[derive(Debug)]
pub struct LogView {
    pub id: Option<String>,
    pub lines: VecDeque<LogEntry>,
    pub search: String,
    pub autoscroll: bool,
}

impl Default for LogView {
    fn default() -> Self {
        Self { id: None, lines: VecDeque::new(), search: String::new(), autoscroll: true }
    }
}

#[derive(Debug, Default)]
pub struct StatsHistory {
    pub id: Option<String>,
    pub cpu: VecDeque<f32>,
    pub mem: VecDeque<f32>,
    pub last: Option<Stats>,
}

/// Something awaiting the user's yes.
#[derive(Debug, Clone, PartialEq)]
pub enum Confirm {
    RemoveContainer { id: String, name: String, running: bool },
    RemoveImage { reference: String, label: String },
    RemoveVolume { name: String },
    Prune { deep: bool },
    ComposeDown { project: String },
}

impl Confirm {
    pub fn message(&self) -> String {
        match self {
            Confirm::RemoveContainer { name, running: true, .. } => {
                format!("Stop and delete the running container '{name}'? Its filesystem is removed.")
            }
            Confirm::RemoveContainer { name, .. } => format!("Delete container '{name}'? Its filesystem is removed."),
            Confirm::RemoveImage { label, .. } => format!("Delete image '{label}'?"),
            Confirm::RemoveVolume { name } => format!("Delete volume '{name}'? Its data is lost for good."),
            Confirm::Prune { deep: false } => "Remove stopped containers and unused images? Volumes are kept.".into(),
            Confirm::Prune { deep: true } => {
                "Deep prune: remove ALL unused images and unused volumes? Volume data is lost for good.".into()
            }
            Confirm::ComposeDown { project } => {
                format!("Bring down '{project}'? Its containers are stopped and removed.")
            }
        }
    }

    pub fn button(&self) -> &'static str {
        match self {
            Confirm::Prune { .. } => "Prune",
            Confirm::ComposeDown { .. } => "Bring down",
            _ => "Delete",
        }
    }
}

/// Follow-up after an operation finishes.
#[derive(Debug, Clone, PartialEq)]
pub enum Then {
    Nothing,
    /// Select the container and show its logs (a foreground run).
    ShowContainer(String),
    /// Remember the compose project's options.
    RememberProject { project: String, opts: ComposeOptions },
}

/// Everything the backend can tell the UI.
#[derive(Debug)]
pub enum Event {
    Containers(Result<Vec<ContainerInfo>, String>),
    Images(Result<Vec<ImageInfo>, String>),
    Volumes(Result<Vec<VolumeInfo>, String>),
    Compose(Result<Vec<ComposeStatus>, String>),
    Info(Result<SystemInfo, String>),
    Stats { id: String, stats: Stats },
    Log { id: String, stderr: bool, line: String },
    Progress { op: u64, progress: Progress },
    /// A background action finished (`op` is set for ones with an activity row).
    Done { op: Option<u64>, result: Result<String, String>, then: Then },
    /// The engine reported a change; refresh soon.
    Engine,
}

#[derive(Debug, Default)]
pub struct Data {
    pub containers: Vec<ContainerInfo>,
    pub images: Vec<ImageInfo>,
    pub volumes: Vec<VolumeInfo>,
    pub projects: Vec<ComposeStatus>,
    pub info: Option<SystemInfo>,
    pub loaded: [bool; 4],
    pub error: Option<String>,
}

pub struct AppState {
    pub page: Page,
    pub detail_tab: DetailTab,
    pub theme: ThemeChoice,
    pub data: Data,
    pub show_all: bool,
    pub filter: String,

    pub selected: Option<String>,
    pub logs: LogView,
    pub stats: StatsHistory,

    pub toasts: Toasts,
    pub ops: Vec<Operation>,
    next_op: u64,

    pub run: Option<RunDialog>,
    pub pull: Option<PullDialog>,
    pub new_volume: Option<String>,
    pub compose_path: Option<String>,
    pub confirm: Option<Confirm>,

    pub known: KnownProjects,
    pub known_path: Option<PathBuf>,
    pub cwd: PathBuf,
    /// Current egui clock in seconds, set every frame.
    pub now: f64,
    /// Set when an engine event arrived: refresh at the next frame.
    pub refresh_now: bool,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct PullDialog {
    pub reference: String,
    pub platform: PlatformChoice,
    pub error: Option<String>,
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

impl AppState {
    pub fn new() -> Self {
        Self {
            page: Page::Containers,
            detail_tab: DetailTab::Overview,
            theme: ThemeChoice::System,
            data: Data::default(),
            show_all: true,
            filter: String::new(),
            selected: None,
            logs: LogView::default(),
            stats: StatsHistory::default(),
            toasts: Toasts::default(),
            ops: Vec::new(),
            next_op: 0,
            run: None,
            pull: None,
            new_volume: None,
            compose_path: None,
            confirm: None,
            known: KnownProjects::default(),
            known_path: None,
            cwd: std::env::current_dir().unwrap_or_default(),
            now: 0.0,
            refresh_now: false,
        }
    }

    pub fn start_op(&mut self, title: impl Into<String>) -> u64 {
        self.next_op += 1;
        self.ops.push(Operation {
            id: self.next_op,
            title: title.into(),
            progress: ProgressState::default(),
            status: OpStatus::Running,
        });
        self.next_op
    }

    pub fn dismiss_op(&mut self, id: u64) {
        self.ops.retain(|o| o.id != id);
    }

    pub fn selected_container(&self) -> Option<&ContainerInfo> {
        let id = self.selected.as_deref()?;
        self.data.containers.iter().find(|c| c.id == id)
    }

    pub fn visible_containers(&self) -> Vec<&ContainerInfo> {
        let q = self.filter.trim().to_lowercase();
        self.data
            .containers
            .iter()
            .filter(|c| self.show_all || c.state == ContainerState::Running)
            .filter(|c| {
                q.is_empty()
                    || c.name.to_lowercase().contains(&q)
                    || c.image.to_lowercase().contains(&q)
                    || c.short_id().contains(&q)
            })
            .collect()
    }

    /// One row per image tag (untagged images get a single row by id).
    pub fn image_rows(&self) -> Vec<ImageRow> {
        let q = self.filter.trim().to_lowercase();
        let mut rows = Vec::new();
        for i in &self.data.images {
            if i.repo_tags.is_empty() {
                rows.push(ImageRow { reference: i.id.clone(), label: "<untagged>".into(), info: i.clone() });
            }
            for t in &i.repo_tags {
                rows.push(ImageRow { reference: t.clone(), label: short_ref(t).to_string(), info: i.clone() });
            }
        }
        rows.retain(|r| q.is_empty() || r.label.to_lowercase().contains(&q) || r.info.short_id().contains(&q));
        rows
    }

    /// Apply a backend event. `now` is the egui clock in seconds.
    pub fn apply(&mut self, ev: Event, now: f64) {
        match ev {
            Event::Containers(r) => match r {
                Ok(mut v) => {
                    v.sort_by(|a, b| a.name.cmp(&b.name));
                    self.data.containers = v;
                    self.data.loaded[0] = true;
                    self.data.error = None;
                    if let Some(id) = &self.selected {
                        if !self.data.containers.iter().any(|c| &c.id == id) {
                            self.selected = None;
                        }
                    }
                }
                Err(e) => self.engine_error(e, now),
            },
            Event::Images(r) => match r {
                Ok(mut v) => {
                    v.sort_by_key(|i| std::cmp::Reverse(i.created));
                    self.data.images = v;
                    self.data.loaded[1] = true;
                }
                Err(e) => self.engine_error(e, now),
            },
            Event::Volumes(r) => match r {
                Ok(mut v) => {
                    v.sort_by(|a, b| a.name.cmp(&b.name));
                    self.data.volumes = v;
                    self.data.loaded[2] = true;
                }
                Err(e) => self.engine_error(e, now),
            },
            Event::Compose(r) => match r {
                Ok(mut v) => {
                    v.sort_by(|a, b| a.project.cmp(&b.project));
                    self.data.projects = v;
                    self.data.loaded[3] = true;
                }
                Err(e) => self.engine_error(e, now),
            },
            Event::Info(r) => match r {
                Ok(i) => self.data.info = Some(i),
                Err(e) => self.engine_error(e, now),
            },
            Event::Stats { id, stats } => {
                if self.stats.id.as_deref() == Some(&id) {
                    push_capped(&mut self.stats.cpu, stats.cpu_percent as f32, MAX_HISTORY);
                    push_capped(&mut self.stats.mem, stats.memory_bytes as f32, MAX_HISTORY);
                    self.stats.last = Some(stats);
                }
            }
            Event::Log { id, stderr, line } => {
                if self.logs.id.as_deref() == Some(&id) {
                    push_capped(&mut self.logs.lines, LogEntry { stderr, text: sanitize_line(&line) }, MAX_LOG_LINES);
                }
            }
            Event::Progress { op, progress } => {
                if let Some(o) = self.ops.iter_mut().find(|o| o.id == op) {
                    o.progress.apply(progress);
                }
            }
            Event::Done { op, result, then } => {
                if let Some(id) = op {
                    if let Some(o) = self.ops.iter_mut().find(|o| o.id == id) {
                        o.status = match &result {
                            Ok(m) => OpStatus::Done(m.clone()),
                            Err(e) => OpStatus::Failed(e.clone()),
                        };
                    }
                }
                match (&result, then) {
                    (Ok(m), then) => {
                        self.toasts.push(Level::Success, m.clone(), now);
                        match then {
                            Then::Nothing => {}
                            Then::ShowContainer(id) => {
                                self.page = Page::Containers;
                                self.selected = Some(id);
                                self.detail_tab = DetailTab::Logs;
                            }
                            Then::RememberProject { project, opts } => {
                                self.known.projects.insert(project, opts);
                                if let Some(p) = &self.known_path {
                                    let _ = self.known.save(p);
                                }
                            }
                        }
                    }
                    (Err(e), _) => self.toasts.push(Level::Error, e.clone(), now),
                }
                self.refresh_now = true;
            }
            Event::Engine => self.refresh_now = true,
        }
    }

    fn engine_error(&mut self, e: String, now: f64) {
        self.toasts.push(Level::Error, format!("Can't reach the engine: {e}"), now);
        self.data.error = Some(e);
    }
}

#[derive(Debug, Clone)]
pub struct ImageRow {
    pub reference: String,
    pub label: String,
    pub info: ImageInfo,
}

fn push_capped<T>(q: &mut VecDeque<T>, v: T, cap: usize) {
    q.push_back(v);
    while q.len() > cap {
        q.pop_front();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::*;

    #[test]
    fn toasts_push_expire_dismiss() {
        let mut t = Toasts::default();
        t.push(Level::Info, "a", 0.0);
        t.push(Level::Error, "b", 0.0);
        assert_eq!(t.items.len(), 2);
        t.expire(5.0);
        assert_eq!(t.items.len(), 1, "info (4s) expired, error (9s) kept");
        let id = t.items[0].id;
        t.dismiss(id);
        assert!(t.items.is_empty());
    }

    #[test]
    fn toasts_dedupe_refreshes_lifetime() {
        let mut t = Toasts::default();
        t.push(Level::Error, "boom", 0.0);
        t.push(Level::Error, "boom", 8.0);
        assert_eq!(t.items.len(), 1);
        t.expire(12.0);
        assert_eq!(t.items.len(), 1, "refreshed at t=8, lives until 17");
        t.expire(18.0);
        assert!(t.items.is_empty());
    }

    #[test]
    fn toasts_are_capped() {
        let mut t = Toasts::default();
        for i in 0..9 {
            t.push(Level::Info, format!("m{i}"), 0.0);
        }
        assert_eq!(t.items.len(), MAX_TOASTS);
        assert_eq!(t.items[0].text, "m4");
    }

    #[test]
    fn port_rows() {
        let p = |h: &str, c: &str, udp| PortRow { host: h.into(), container: c.into(), udp }.to_mapping();
        assert_eq!(p("", "", false), Ok(None));
        let m = p("8080", "80", false).unwrap().unwrap();
        assert_eq!((m.host_port, m.container_port, m.protocol), (8080, 80, Protocol::Tcp));
        let m = p("", "443", false).unwrap().unwrap();
        assert_eq!((m.host_port, m.container_port), (443, 443));
        let m = p("53", "53", true).unwrap().unwrap();
        assert_eq!(m.protocol, Protocol::Udp);
        let m = p("127.0.0.1:9000:90", "", false).unwrap().unwrap();
        assert_eq!(m.host_ip.as_deref(), Some("127.0.0.1"));
        assert!(p("abc", "80", false).is_err());
        assert!(p("0", "80", false).is_err());
        assert!(p("70000", "80", false).is_err());
    }

    #[test]
    fn env_rows() {
        let e = |k: &str, v: &str| EnvRow { key: k.into(), value: v.into() }.to_env();
        assert_eq!(e("", ""), Ok(None));
        assert_eq!(e("A", "1"), Ok(Some("A=1".into())));
        assert_eq!(e("A", ""), Ok(Some("A=".into())));
        assert_eq!(e("PASTED=x=y", ""), Ok(Some("PASTED=x=y".into())));
        assert!(e("", "v").is_err());
        assert!(e("A B", "1").is_err());
        assert!(e("A=B", "1").is_err());
    }

    #[test]
    fn volume_rows() {
        let base = Path::new("/proj");
        let v = |s: &str, t: &str, ro| VolumeRow { source: s.into(), target: t.into(), read_only: ro }.to_mount(base);
        assert_eq!(v("", "", false), Ok(None));
        let m = v("./data", "/data", true).unwrap().unwrap();
        assert_eq!(m.source, "/proj/data");
        assert!(m.read_only);
        let m = v("pgdata", "/var/lib/pg", false).unwrap().unwrap();
        assert_eq!(m.kind, hullr_core::MountKind::Volume);
        let m = v("", "/scratch", false).unwrap().unwrap();
        assert_eq!(m.target, "/scratch");
        assert!(v("/h", "", false).is_err());
        assert!(v("/h", "relative", false).is_err());
    }

    #[test]
    fn run_dialog_full_conversion() {
        let d = RunDialog {
            image: " nginx:latest ".into(),
            name: "web".into(),
            command: "nginx -g 'daemon off;'".into(),
            ports: vec![
                PortRow { host: "8080".into(), container: "80".into(), udp: false },
                PortRow::default(),
            ],
            env: vec![EnvRow { key: "MODE".into(), value: "prod".into() }, EnvRow::default()],
            volumes: vec![VolumeRow { source: "data".into(), target: "/data".into(), read_only: false }],
            platform: PlatformChoice::Amd64,
            restart: RestartChoice::UnlessStopped,
            detach: true,
            auto_remove: true,
            error: None,
        };
        let s = d.to_spec(Path::new("/")).unwrap();
        assert_eq!(s.image, "nginx:latest");
        assert_eq!(s.name.as_deref(), Some("web"));
        assert_eq!(s.cmd.unwrap(), vec!["nginx", "-g", "daemon off;"]);
        assert_eq!(s.ports.len(), 1);
        assert_eq!(s.env, vec!["MODE=prod"]);
        assert_eq!(s.mounts.len(), 1);
        assert_eq!(s.platform.unwrap().to_string(), "linux/amd64");
        assert_eq!(s.restart, RestartPolicy::UnlessStopped);
        assert!(s.auto_remove);
    }

    #[test]
    fn run_dialog_defaults_and_errors() {
        let base = Path::new("/");
        assert!(RunDialog::default().to_spec(base).unwrap_err().contains("Enter an image"));
        let s = RunDialog::for_image("alpine").to_spec(base).unwrap();
        assert_eq!(s.image, "alpine");
        assert!(s.name.is_none() && s.cmd.is_none() && s.ports.is_empty() && s.platform.is_none());
        assert_eq!(s.restart, RestartPolicy::No);

        let mut d = RunDialog::for_image("alpine");
        d.name = "bad name".into();
        assert!(d.to_spec(base).is_err());
        d.name.clear();
        d.command = "echo 'oops".into();
        assert!(d.to_spec(base).unwrap_err().contains("quote"));
        d.command.clear();
        d.ports = vec![
            PortRow { host: "80".into(), container: "80".into(), udp: false },
            PortRow { host: "80".into(), container: "8080".into(), udp: false },
        ];
        assert!(d.to_spec(base).unwrap_err().contains("used twice"));
        d.ports.clear();
        d.env = vec![EnvRow { key: "".into(), value: "x".into() }];
        assert!(d.to_spec(base).is_err());
    }

    #[test]
    fn restart_and_platform_choices() {
        assert_eq!(RestartChoice::OnFailure.policy(), RestartPolicy::OnFailure(0));
        assert_eq!(PlatformChoice::Native.platform(), None);
        assert_eq!(PlatformChoice::Arm64.platform().unwrap().architecture, "arm64");
        assert_eq!(PlatformChoice::ALL.len(), 3);
    }

    #[test]
    fn progress_aggregation() {
        let mut p = ProgressState::default();
        assert_eq!(p.fraction(), None);
        p.apply(Progress::Layer { id: "a".into(), current: 50, total: 100, done: false });
        p.apply(Progress::Layer { id: "b".into(), current: 0, total: 100, done: false });
        p.apply(Progress::Layer { id: "c".into(), current: 0, total: 0, done: false });
        assert_eq!(p.bytes(), Some((50, 200)));
        assert_eq!(p.fraction(), Some(0.25));
        p.apply(Progress::Layer { id: "b".into(), current: 100, total: 100, done: true });
        assert_eq!(p.bytes(), Some((150, 200)));
        assert_eq!(p.layers.len(), 3);
        assert_eq!(p.layers_done(), 1);
        p.apply(Progress::Status { message: "hi".into() });
        p.apply(Progress::Step { step: 2, total: 5, text: "RUN x".into(), cached: true });
        p.apply(Progress::Output { source: "web".into(), line: "up".into() });
        assert_eq!(p.step, Some((2, 5)));
        assert_eq!(p.lines.len(), 3);
        assert!(p.lines[1].contains("(cached)"));
        assert_eq!(p.layers[1].fraction(), 1.0);
    }

    #[test]
    fn done_layers_count_as_full_even_if_current_lags() {
        let mut p = ProgressState::default();
        p.apply(Progress::Layer { id: "a".into(), current: 3, total: 10, done: true });
        assert_eq!(p.bytes(), Some((10, 10)));
    }

    #[test]
    fn log_filter_is_case_insensitive() {
        let mut logs = VecDeque::new();
        for t in ["Started server", "ERROR: disk full", "ok"] {
            logs.push_back(LogEntry { stderr: false, text: t.into() });
        }
        assert_eq!(filter_logs(&logs, "").len(), 3);
        assert_eq!(filter_logs(&logs, "error").len(), 1);
        assert_eq!(filter_logs(&logs, "  SERVER ").len(), 1);
        assert!(filter_logs(&logs, "zzz").is_empty());
    }

    #[test]
    fn sanitize() {
        assert_eq!(sanitize_line("\u{1b}[1;31mfail\u{1b}[0m\r"), "fail");
        assert_eq!(sanitize_line("a\tb"), "a    b");
    }

    #[test]
    fn links_for_tcp_ports_only() {
        let ports = vec![
            PortMapping::parse("8080:80").unwrap(),
            PortMapping::parse("53:53/udp").unwrap(),
            PortMapping::parse("127.0.0.1:9000:90").unwrap(),
        ];
        let l = port_links(&ports);
        assert_eq!(l.len(), 2);
        assert_eq!(l[0], (8080, "http://localhost:8080".into()));
        assert_eq!(l[1].1, "http://127.0.0.1:9000");
    }

    #[test]
    fn compose_paths() {
        let o = compose_opts_from_path("/srv/app/compose.yaml").unwrap();
        assert_eq!(o.files, vec![PathBuf::from("/srv/app/compose.yaml")]);
        assert_eq!(o.project_dir, PathBuf::from("/srv/app"));
        let o = compose_opts_from_path(" /srv/app ").unwrap();
        assert!(o.files.is_empty());
        assert_eq!(o.project_dir, PathBuf::from("/srv/app"));
        let o = compose_opts_from_path("docker-compose.YML").unwrap();
        assert_eq!(o.project_dir, PathBuf::from("."));
        assert!(compose_opts_from_path("  ").is_err());
    }

    #[test]
    fn known_projects_roundtrip() {
        let dir = std::env::temp_dir().join(format!("hullr-gui-test-{}", std::process::id()));
        let path = dir.join("p.json");
        let mut k = KnownProjects::default();
        k.projects.insert("shop".into(), compose_opts_from_path("/srv/shop/compose.yaml").unwrap());
        k.save(&path).unwrap();
        assert_eq!(KnownProjects::load(&path), k);
        assert_eq!(KnownProjects::load(&dir.join("missing.json")), KnownProjects::default());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn containers_event_sorts_and_clears_stale_selection() {
        let mut s = AppState::new();
        s.apply(Event::Containers(Ok(vec![container("web", ContainerState::Running), container("db", ContainerState::Exited)])), 0.0);
        assert_eq!(s.data.containers[0].name, "db");
        let id = s.data.containers[1].id.clone();
        s.selected = Some(id.clone());
        assert_eq!(s.selected_container().unwrap().name, "web");
        s.apply(Event::Containers(Ok(vec![container("db", ContainerState::Exited)])), 1.0);
        assert!(s.selected.is_none());
    }

    #[test]
    fn visible_containers_filters() {
        let mut s = AppState::new();
        s.apply(Event::Containers(Ok(vec![container("web", ContainerState::Running), container("db", ContainerState::Exited)])), 0.0);
        assert_eq!(s.visible_containers().len(), 2);
        s.show_all = false;
        assert_eq!(s.visible_containers().len(), 1);
        s.show_all = true;
        s.filter = "DB".into();
        assert_eq!(s.visible_containers().len(), 1);
        s.filter = "nginx".into();
        assert_eq!(s.visible_containers().len(), 2, "matches the image");
    }

    #[test]
    fn engine_errors_toast_once() {
        let mut s = AppState::new();
        for t in 0..5 {
            s.apply(Event::Containers(Err("refused".into())), t as f64);
            s.apply(Event::Images(Err("refused".into())), t as f64);
        }
        assert_eq!(s.toasts.items.len(), 1);
        assert!(s.toasts.items[0].text.contains("refused"));
        assert!(s.data.error.is_some());
        s.apply(Event::Containers(Ok(vec![])), 6.0);
        assert!(s.data.error.is_none());
    }

    #[test]
    fn op_lifecycle() {
        let mut s = AppState::new();
        let op = s.start_op("Pulling alpine");
        s.apply(Event::Progress { op, progress: Progress::Layer { id: "l".into(), current: 1, total: 2, done: false } }, 0.0);
        assert_eq!(s.ops[0].progress.fraction(), Some(0.5));
        assert_eq!(s.ops[0].status, OpStatus::Running);
        s.apply(Event::Done { op: Some(op), result: Ok("Pulled alpine".into()), then: Then::Nothing }, 1.0);
        assert_eq!(s.ops[0].status, OpStatus::Done("Pulled alpine".into()));
        assert_eq!(s.toasts.items[0].level, Level::Success);
        assert!(s.refresh_now);
        s.dismiss_op(op);
        assert!(s.ops.is_empty());
    }

    #[test]
    fn failed_op_toasts_error() {
        let mut s = AppState::new();
        let op = s.start_op("Pulling x");
        s.apply(Event::Done { op: Some(op), result: Err("registry error: denied".into()), then: Then::ShowContainer("zzz".into()) }, 0.0);
        assert!(matches!(&s.ops[0].status, OpStatus::Failed(m) if m.contains("denied")));
        assert_eq!(s.toasts.items[0].level, Level::Error);
        assert!(s.selected.is_none(), "follow-up only runs on success");
    }

    #[test]
    fn foreground_run_opens_logs() {
        let mut s = AppState::new();
        s.page = Page::Images;
        s.apply(Event::Done { op: None, result: Ok("Started web".into()), then: Then::ShowContainer("abc".into()) }, 0.0);
        assert_eq!(s.page, Page::Containers);
        assert_eq!(s.selected.as_deref(), Some("abc"));
        assert_eq!(s.detail_tab, DetailTab::Logs);
    }

    #[test]
    fn remember_project_persists() {
        let dir = std::env::temp_dir().join(format!("hullr-gui-test2-{}", std::process::id()));
        let path = dir.join("p.json");
        let mut s = AppState::new();
        s.known_path = Some(path.clone());
        let opts = compose_opts_from_path("/srv/x").unwrap();
        s.apply(Event::Done { op: None, result: Ok("up".into()), then: Then::RememberProject { project: "x".into(), opts: opts.clone() } }, 0.0);
        assert_eq!(KnownProjects::load(&path).projects.get("x"), Some(&opts));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn logs_and_stats_only_for_followed_container() {
        let mut s = AppState::new();
        s.logs.id = Some("a".into());
        s.stats.id = Some("a".into());
        s.apply(Event::Log { id: "b".into(), stderr: false, line: "no".into() }, 0.0);
        s.apply(Event::Log { id: "a".into(), stderr: true, line: "\u{1b}[31myes\u{1b}[0m".into() }, 0.0);
        assert_eq!(s.logs.lines.len(), 1);
        assert_eq!(s.logs.lines[0], LogEntry { stderr: true, text: "yes".into() });
        s.apply(Event::Stats { id: "b".into(), stats: Stats::default() }, 0.0);
        assert!(s.stats.cpu.is_empty());
        s.apply(Event::Stats { id: "a".into(), stats: Stats { cpu_percent: 5.0, memory_bytes: 10, ..Default::default() } }, 0.0);
        assert_eq!(s.stats.cpu.back(), Some(&5.0));
        assert_eq!(s.stats.mem.back(), Some(&10.0));
    }

    #[test]
    fn history_is_capped() {
        let mut s = AppState::new();
        s.stats.id = Some("a".into());
        for i in 0..(MAX_HISTORY + 20) {
            s.apply(Event::Stats { id: "a".into(), stats: Stats { cpu_percent: i as f64, ..Default::default() } }, 0.0);
        }
        assert_eq!(s.stats.cpu.len(), MAX_HISTORY);
    }

    #[test]
    fn image_rows_per_tag() {
        let mut s = AppState::new();
        let mut img = image("docker.io/library/nginx:latest");
        img.repo_tags.push("docker.io/library/nginx:1.27".into());
        let mut untagged = image("x");
        untagged.repo_tags.clear();
        s.apply(Event::Images(Ok(vec![img, untagged])), 0.0);
        let rows = s.image_rows();
        assert_eq!(rows.len(), 3);
        assert!(rows.iter().any(|r| r.label == "nginx:1.27"));
        assert!(rows.iter().any(|r| r.label == "<untagged>"));
        s.filter = "1.27".into();
        assert_eq!(s.image_rows().len(), 1);
    }

    #[test]
    fn confirm_messages_warn_about_data() {
        assert!(Confirm::RemoveVolume { name: "d".into() }.message().contains("lost"));
        assert!(Confirm::RemoveContainer { id: "i".into(), name: "web".into(), running: true }.message().contains("Stop and delete"));
        assert_eq!(Confirm::Prune { deep: true }.button(), "Prune");
        assert!(Confirm::Prune { deep: true }.message().contains("Deep prune"));
    }

    #[test]
    fn datetime() {
        assert_eq!(fmt_datetime(1_700_000_000), "2023-11-14 22:13 UTC");
    }
}
