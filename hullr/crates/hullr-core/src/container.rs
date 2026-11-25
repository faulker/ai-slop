use crate::{HealthCheck, Platform};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// Everything needed to create a container. Mirrors the useful subset of
/// `docker run` flags.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ContainerSpec {
    #[serde(default)]
    pub name: Option<String>,
    /// Image reference or id.
    pub image: String,
    #[serde(default)]
    pub platform: Option<Platform>,
    #[serde(default)]
    pub entrypoint: Option<Vec<String>>,
    #[serde(default)]
    pub cmd: Option<Vec<String>>,
    /// `KEY=VALUE`, applied over the image env.
    #[serde(default)]
    pub env: Vec<String>,
    #[serde(default)]
    pub working_dir: Option<String>,
    #[serde(default)]
    pub user: Option<String>,
    #[serde(default)]
    pub hostname: Option<String>,
    #[serde(default)]
    pub ports: Vec<PortMapping>,
    #[serde(default)]
    pub mounts: Vec<Mount>,
    #[serde(default)]
    pub network: NetworkMode,
    /// Extra `/etc/hosts` entries: (hostname, ip).
    #[serde(default)]
    pub extra_hosts: Vec<(String, String)>,
    #[serde(default)]
    pub labels: BTreeMap<String, String>,
    #[serde(default)]
    pub tty: bool,
    #[serde(default)]
    pub interactive: bool,
    /// Remove when it exits (`--rm`).
    #[serde(default)]
    pub auto_remove: bool,
    #[serde(default)]
    pub restart: RestartPolicy,
    #[serde(default)]
    pub healthcheck: Option<HealthCheck>,
    #[serde(default)]
    pub pull: PullPolicy,
    /// Memory limit in bytes (best effort, needs cgroup delegation).
    #[serde(default)]
    pub memory_limit: Option<u64>,
    /// CPU limit in cores, e.g. 1.5.
    #[serde(default)]
    pub cpus: Option<f64>,
}


#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PullPolicy {
    /// Pull only if not present locally.
    #[default]
    Missing,
    Always,
    Never,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Protocol {
    #[default]
    Tcp,
    Udp,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PortMapping {
    #[serde(default)]
    pub host_ip: Option<String>,
    pub host_port: u16,
    pub container_port: u16,
    #[serde(default)]
    pub protocol: Protocol,
}

impl PortMapping {
    /// Parse `-p` syntax: `80`, `8080:80`, `127.0.0.1:8080:80`, `53:53/udp`.
    pub fn parse(s: &str) -> crate::Result<Self> {
        let bad = || crate::Error::Invalid(format!("bad port mapping '{s}'"));
        let (rest, protocol) = match s.rsplit_once('/') {
            Some((r, "tcp")) => (r, Protocol::Tcp),
            Some((r, "udp")) => (r, Protocol::Udp),
            Some(_) => return Err(bad()),
            None => (s, Protocol::Tcp),
        };
        let port = |p: &str| p.parse::<u16>().map_err(|_| bad());
        let parts: Vec<&str> = rest.split(':').collect();
        let (host_ip, host_port, container_port) = match parts.as_slice() {
            [c] => (None, port(c)?, port(c)?),
            [h, c] => (None, port(h)?, port(c)?),
            [ip, h, c] => (Some(ip.to_string()), port(h)?, port(c)?),
            _ => return Err(bad()),
        };
        Ok(Self { host_ip, host_port, container_port, protocol })
    }
}

impl std::fmt::Display for PortMapping {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let proto = match self.protocol {
            Protocol::Tcp => "tcp",
            Protocol::Udp => "udp",
        };
        write!(
            f,
            "{}:{}->{}/{}",
            self.host_ip.as_deref().unwrap_or("0.0.0.0"),
            self.host_port,
            self.container_port,
            proto
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MountKind {
    /// Host path.
    Bind,
    /// Named volume managed by hullr (source is the volume name).
    Volume,
    /// In-memory tmpfs (source ignored).
    Tmpfs,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mount {
    pub kind: MountKind,
    pub source: String,
    pub target: String,
    #[serde(default)]
    pub read_only: bool,
}

impl Mount {
    /// Parse `-v` syntax: `/host:/ctr[:ro]`, `./rel:/ctr`, `name:/ctr`, `/ctr`
    /// (anonymous volume). Relative host paths resolve against `base`.
    pub fn parse(s: &str, base: &std::path::Path) -> crate::Result<Self> {
        let parts: Vec<&str> = s.split(':').collect();
        let (src, dst, opts) = match parts.as_slice() {
            [d] => ("", *d, ""),
            [s, d] => (*s, *d, ""),
            [s, d, o] => (*s, *d, *o),
            _ => return Err(crate::Error::Invalid(format!("bad volume '{s}'"))),
        };
        if !dst.starts_with('/') {
            return Err(crate::Error::Invalid(format!(
                "volume target must be absolute in '{s}'"
            )));
        }
        let read_only = opts.split(',').any(|o| o == "ro");
        let (kind, source) = if src.is_empty() {
            (MountKind::Volume, String::new())
        } else if src.starts_with('/') {
            (MountKind::Bind, src.to_string())
        } else if src.starts_with('.') || src.starts_with('~') {
            let p = if let Some(rest) = src.strip_prefix("~/") {
                dirs::home_dir().unwrap_or_default().join(rest)
            } else {
                base.join(src)
            };
            (MountKind::Bind, normalize(&p).to_string_lossy().into_owned())
        } else {
            (MountKind::Volume, src.to_string())
        };
        Ok(Self { kind, source, target: dst.to_string(), read_only })
    }
}

/// Lexically clean a path (`a/./b/../c` -> `a/c`) without touching disk.
pub fn normalize(p: &std::path::Path) -> PathBuf {
    use std::path::Component;
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "mode", content = "name")]
pub enum NetworkMode {
    /// Own network namespace with outbound access (via pasta/slirp4netns),
    /// published ports forwarded from the host.
    #[default]
    Private,
    /// Share the host network stack.
    Host,
    /// Loopback only.
    None,
    /// Join a named shared network sandbox (all members share one network
    /// namespace and reach each other via localhost; compose uses this).
    Sandbox(String),
    /// Join another container's network namespace.
    Container(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "policy", content = "max_retries")]
pub enum RestartPolicy {
    #[default]
    No,
    Always,
    UnlessStopped,
    OnFailure(u32),
}

impl RestartPolicy {
    pub fn parse(s: &str) -> crate::Result<Self> {
        match s {
            "no" | "" => Ok(Self::No),
            "always" => Ok(Self::Always),
            "unless-stopped" => Ok(Self::UnlessStopped),
            "on-failure" => Ok(Self::OnFailure(0)),
            s if s.starts_with("on-failure:") => s[11..]
                .parse()
                .map(Self::OnFailure)
                .map_err(|_| crate::Error::Invalid(format!("bad restart policy '{s}'"))),
            _ => Err(crate::Error::Invalid(format!("bad restart policy '{s}'"))),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContainerState {
    Created,
    Running,
    Paused,
    Restarting,
    Exited,
    Dead,
}

impl std::fmt::Display for ContainerState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::Created => "created",
            Self::Running => "running",
            Self::Paused => "paused",
            Self::Restarting => "restarting",
            Self::Exited => "exited",
            Self::Dead => "dead",
        };
        f.write_str(s)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Health {
    Starting,
    Healthy,
    Unhealthy,
}

/// A container as shown by `ps`, `inspect` and the UIs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContainerInfo {
    pub id: String,
    pub name: String,
    /// The reference the user asked for.
    pub image: String,
    pub image_id: String,
    /// Final argv run in the container.
    pub command: Vec<String>,
    pub created: i64,
    pub state: ContainerState,
    #[serde(default)]
    pub exit_code: Option<i32>,
    #[serde(default)]
    pub pid: Option<u32>,
    #[serde(default)]
    pub started_at: Option<i64>,
    #[serde(default)]
    pub finished_at: Option<i64>,
    #[serde(default)]
    pub health: Option<Health>,
    pub platform: Platform,
    pub spec: ContainerSpec,
}

impl ContainerInfo {
    pub fn short_id(&self) -> &str {
        &self.id[..self.id.len().min(12)]
    }

    /// Docker-style status text: `Up 5 minutes`, `Exited (0) 2 hours ago`.
    pub fn status_text(&self, now: i64) -> String {
        match self.state {
            ContainerState::Running => {
                let up = format!("Up {}", human_duration(now - self.started_at.unwrap_or(now)));
                match self.health {
                    Some(Health::Healthy) => format!("{up} (healthy)"),
                    Some(Health::Unhealthy) => format!("{up} (unhealthy)"),
                    Some(Health::Starting) => format!("{up} (health: starting)"),
                    None => up,
                }
            }
            ContainerState::Exited => format!(
                "Exited ({}) {} ago",
                self.exit_code.unwrap_or(0),
                human_duration(now - self.finished_at.unwrap_or(now))
            ),
            other => {
                let mut s = other.to_string();
                if let Some(c) = s.get_mut(0..1) {
                    c.make_ascii_uppercase();
                }
                s
            }
        }
    }
}

pub fn human_duration(secs: i64) -> String {
    let secs = secs.max(0);
    match secs {
        0..=1 => "1 second".into(),
        2..=59 => format!("{secs} seconds"),
        60..=119 => "1 minute".into(),
        120..=3599 => format!("{} minutes", secs / 60),
        3600..=7199 => "1 hour".into(),
        7200..=86399 => format!("{} hours", secs / 3600),
        86400..=172799 => "1 day".into(),
        _ => format!("{} days", secs / 86400),
    }
}

/// Human byte size: `12.3MB`.
pub fn human_bytes(b: u64) -> String {
    const UNITS: [&str; 5] = ["B", "kB", "MB", "GB", "TB"];
    let mut v = b as f64;
    let mut i = 0;
    while v >= 1000.0 && i < UNITS.len() - 1 {
        v /= 1000.0;
        i += 1;
    }
    if i == 0 {
        format!("{b}B")
    } else {
        format!("{v:.1}{}", UNITS[i])
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Stats {
    pub cpu_percent: f64,
    pub memory_bytes: u64,
    #[serde(default)]
    pub memory_limit: Option<u64>,
    pub pids: u32,
    #[serde(default)]
    pub net_rx_bytes: u64,
    #[serde(default)]
    pub net_tx_bytes: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecSpec {
    pub cmd: Vec<String>,
    #[serde(default)]
    pub env: Vec<String>,
    #[serde(default)]
    pub working_dir: Option<String>,
    #[serde(default)]
    pub user: Option<String>,
    #[serde(default)]
    pub tty: bool,
    #[serde(default)]
    pub interactive: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogOptions {
    #[serde(default)]
    pub follow: bool,
    /// Only the last N lines.
    #[serde(default)]
    pub tail: Option<usize>,
    #[serde(default)]
    pub timestamps: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogStream {
    Stdout,
    Stderr,
}

/// One line of container output, as stored in the container's log file
/// (JSON lines: `{"ts":..,"stream":"stdout","line":".."}`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogLine {
    /// Unix milliseconds.
    pub ts: i64,
    pub stream: LogStream,
    pub line: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VolumeInfo {
    pub name: String,
    pub path: PathBuf,
    pub created: i64,
    #[serde(default)]
    pub labels: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComposeOptions {
    /// Compose files; empty means look for compose.yaml etc. in project_dir.
    #[serde(default)]
    pub files: Vec<PathBuf>,
    pub project_dir: PathBuf,
    #[serde(default)]
    pub project_name: Option<String>,
    /// Restrict to these services (plus their dependencies for `up`).
    #[serde(default)]
    pub services: Vec<String>,
    /// `up --build`
    #[serde(default)]
    pub build: bool,
    /// `down -v`
    #[serde(default)]
    pub remove_volumes: bool,
    /// Extra env used for interpolation (process env is applied by the CLI).
    #[serde(default)]
    pub env: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ComposeService {
    pub service: String,
    pub container: Option<ContainerInfo>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ComposeStatus {
    pub project: String,
    pub services: Vec<ComposeService>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PruneReport {
    pub containers: Vec<String>,
    pub images: Vec<String>,
    pub volumes: Vec<String>,
    pub bytes_reclaimed: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SystemInfo {
    pub version: String,
    pub os: String,
    pub host_platform: crate::Platform,
    /// "in-process", "daemon" or "vm".
    pub mode: String,
    pub data_root: PathBuf,
    pub rootless: bool,
    /// Foreign architectures that can run via emulation.
    pub emulated_platforms: Vec<String>,
    pub containers: usize,
    pub running: usize,
    pub images: usize,
    /// Notes about degraded features (no pasta, no cgroups, ...).
    #[serde(default)]
    pub warnings: Vec<String>,
}

/// Labels hullr puts on compose-managed containers.
pub mod labels {
    pub const PROJECT: &str = "dev.hullr.compose.project";
    pub const SERVICE: &str = "dev.hullr.compose.service";
    pub const CONFIG_HASH: &str = "dev.hullr.compose.config-hash";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ports() {
        let p = PortMapping::parse("8080:80").unwrap();
        assert_eq!((p.host_port, p.container_port), (8080, 80));
        let p = PortMapping::parse("127.0.0.1:53:53/udp").unwrap();
        assert_eq!(p.host_ip.as_deref(), Some("127.0.0.1"));
        assert_eq!(p.protocol, Protocol::Udp);
        assert!(PortMapping::parse("x:80").is_err());
        assert_eq!(PortMapping::parse("80").unwrap().host_port, 80);
    }

    #[test]
    fn mounts() {
        let base = std::path::Path::new("/proj");
        let m = Mount::parse("./data:/data:ro", base).unwrap();
        assert_eq!(m.kind, MountKind::Bind);
        assert_eq!(m.source, "/proj/data");
        assert!(m.read_only);
        let m = Mount::parse("pgdata:/var/lib/postgresql/data", base).unwrap();
        assert_eq!(m.kind, MountKind::Volume);
        assert!(Mount::parse("data:rel", base).is_err());
    }

    #[test]
    fn restart() {
        assert_eq!(RestartPolicy::parse("on-failure:3").unwrap(), RestartPolicy::OnFailure(3));
        assert!(RestartPolicy::parse("sometimes").is_err());
    }

    #[test]
    fn human() {
        assert_eq!(human_bytes(512), "512B");
        assert_eq!(human_bytes(12_300_000), "12.3MB");
        assert_eq!(human_duration(300), "5 minutes");
    }
}
