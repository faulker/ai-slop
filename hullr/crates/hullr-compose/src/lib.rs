//! Compose file loading: discovery, multi-file merge, interpolation,
//! resolution into typed services, ordering and `ContainerSpec` generation.

use hullr_core::{
    labels, ComposeOptions, ContainerSpec, Error, HealthCheck, Mount, NetworkMode, Platform,
    PortMapping, RestartPolicy, Result,
};
use indexmap::IndexMap;
use serde_yaml::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

pub mod interpolate;
mod merge;
mod resolve;
pub mod shell;
pub mod units;

#[cfg(test)]
mod tests;

#[derive(Debug, Clone, PartialEq)]
pub struct Project {
    pub name: String,
    pub dir: PathBuf,
    pub services: IndexMap<String, Service>,
    /// Declared top-level named volumes (unprefixed name -> external flag).
    pub volumes: IndexMap<String, VolumeDef>,
    /// Non-fatal problems found while loading (unknown keys, undeclared
    /// volumes). Front ends should print these.
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct VolumeDef {
    pub external: bool,
    /// Explicit `name:`; otherwise `<project>_<key>`.
    pub name: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BuildDef {
    pub context: PathBuf,
    /// Absolute path when given in the compose file.
    pub dockerfile: Option<PathBuf>,
    pub args: BTreeMap<String, String>,
    pub target: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DependsCondition {
    Started,
    Healthy,
    CompletedSuccessfully,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dependency {
    pub service: String,
    pub condition: DependsCondition,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Service {
    pub name: String,
    pub image: Option<String>,
    pub build: Option<BuildDef>,
    pub container_name: Option<String>,
    pub command: Option<Vec<String>>,
    pub entrypoint: Option<Vec<String>>,
    /// `KEY=VALUE`, env_file entries already merged in (environment wins).
    pub environment: Vec<String>,
    pub ports: Vec<PortMapping>,
    /// Resolved: bind sources absolute, named volume sources are the full
    /// volume name (`<project>_<name>` unless external / explicit name).
    pub volumes: Vec<Mount>,
    pub depends_on: Vec<Dependency>,
    pub healthcheck: Option<HealthCheck>,
    pub restart: RestartPolicy,
    pub working_dir: Option<String>,
    pub user: Option<String>,
    pub hostname: Option<String>,
    pub platform: Option<String>,
    pub labels: BTreeMap<String, String>,
    pub tty: bool,
    pub stdin_open: bool,
    pub extra_hosts: Vec<(String, String)>,
    pub profiles: Vec<String>,
    pub mem_limit: Option<u64>,
    pub cpus: Option<f64>,
}

const DEFAULT_FILES: &[&str] = &["compose.yaml", "compose.yml", "docker-compose.yaml", "docker-compose.yml"];

/// Compose files to load when none are given: the first default name present
/// plus its matching override file(s).
fn discover_files(dir: &Path) -> Result<Vec<PathBuf>> {
    for base in DEFAULT_FILES {
        let p = dir.join(base);
        if p.is_file() {
            let stem = base.rsplit_once('.').map(|x| x.0).unwrap_or(base);
            let mut files = vec![p];
            for ext in ["yaml", "yml"] {
                let o = dir.join(format!("{stem}.override.{ext}"));
                if o.is_file() {
                    files.push(o);
                }
            }
            return Ok(files);
        }
    }
    Err(Error::NotFound(format!(
        "no compose file found in {} (looked for {})",
        dir.display(),
        DEFAULT_FILES.join(", ")
    )))
}

/// Load, merge (override files) and interpolate per `opts`.
pub fn load(opts: &ComposeOptions) -> Result<Project> {
    let process: BTreeMap<String, String> = std::env::vars().collect();
    load_with_process_env(opts, &process)
}

/// Like [`load`] with an explicit process environment (for tests).
pub fn load_with_process_env(opts: &ComposeOptions, process: &BTreeMap<String, String>) -> Result<Project> {
    let dir = if opts.project_dir.is_absolute() {
        opts.project_dir.clone()
    } else {
        std::env::current_dir().map(|c| c.join(&opts.project_dir)).unwrap_or_else(|_| opts.project_dir.clone())
    };
    let dir = hullr_core::normalize(&dir);
    let files: Vec<PathBuf> = if opts.files.is_empty() {
        discover_files(&dir)?
    } else {
        opts.files.iter().map(|f| if f.is_absolute() { f.clone() } else { dir.join(f) }).collect()
    };

    // precedence: opts.env > process env > .env
    let mut env: BTreeMap<String, String> = BTreeMap::new();
    if let Ok(text) = std::fs::read_to_string(dir.join(".env")) {
        env.extend(interpolate::parse_dotenv(&text));
    }
    env.extend(process.iter().map(|(k, v)| (k.clone(), v.clone())));
    env.extend(opts.env.iter().map(|(k, v)| (k.clone(), v.clone())));
    let lookup = |k: &str| env.get(k).cloned();

    let mut merged: Option<Value> = None;
    for f in &files {
        let text = std::fs::read_to_string(f)
            .map_err(Error::from)
            .map_err(|e| Error::Invalid(format!("cannot read {}: {e}", f.display())))?;
        let mut v: Value = serde_yaml::from_str(&text)
            .map_err(|e| Error::Invalid(format!("{}: {e}", f.display())))?;
        v.apply_merge().map_err(|e| Error::Invalid(format!("{}: {e}", f.display())))?;
        if v.is_null() {
            continue;
        }
        interpolate_value(&mut v, &lookup)
            .map_err(|e| Error::Invalid(format!("{}: {e}", f.display())))?;
        merged = Some(match merged {
            None => v,
            Some(base) => merge::merge(base, v),
        });
    }
    let root = merged.unwrap_or(Value::Null);
    let root = root
        .as_mapping()
        .ok_or_else(|| Error::Invalid("compose file must be a mapping".into()))?;

    let mut warnings = Vec::new();
    for k in root.keys() {
        let k = k.as_str().unwrap_or("?");
        match k {
            "services" | "volumes" | "name" | "version" | "networks" => {}
            k if k.starts_with("x-") => {}
            "secrets" | "configs" => warnings.push(format!("top-level '{k}' is not supported and was ignored")),
            _ => warnings.push(format!("unknown top-level key '{k}' ignored")),
        }
    }

    let name = project_name(
        opts.project_name.as_deref(),
        root.get("name").and_then(resolve::scalar).as_deref(),
        env.get("COMPOSE_PROJECT_NAME").map(String::as_str),
        &dir,
    );

    let mut volumes: IndexMap<String, VolumeDef> = IndexMap::new();
    if let Some(Value::Mapping(vm)) = root.get("volumes") {
        for (k, v) in vm {
            let Some(key) = resolve::scalar(k) else { continue };
            let mut def = VolumeDef::default();
            if let Value::Mapping(m) = v {
                match m.get("external") {
                    Some(Value::Mapping(ext)) => {
                        def.external = true;
                        def.name = ext.get("name").and_then(resolve::scalar);
                    }
                    Some(e) => def.external = matches!(e, Value::Bool(true)) || e.as_str() == Some("true"),
                    None => {}
                }
                if let Some(n) = m.get("name").and_then(resolve::scalar) {
                    def.name = Some(n);
                }
            }
            volumes.insert(key, def);
        }
    }

    let active: BTreeSet<String> = env
        .get("COMPOSE_PROFILES")
        .map(|p| p.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect())
        .unwrap_or_default();

    let mut services: IndexMap<String, Service> = IndexMap::new();
    let mut disabled: BTreeSet<String> = BTreeSet::new();
    let svc_map = match root.get("services") {
        Some(Value::Mapping(m)) => m,
        _ => return Err(Error::Invalid("compose file has no 'services'".into())),
    };
    for (k, v) in svc_map {
        let sname = resolve::scalar(k).ok_or_else(|| Error::Invalid("service names must be strings".into()))?;
        let Value::Mapping(sm) = v else {
            return Err(Error::Invalid(format!("service '{sname}' must be a mapping")));
        };
        let mut ctx = resolve::Ctx { dir: &dir, project: &name, env: &env, volumes: &volumes, warnings: &mut warnings };
        let svc = resolve::resolve_service(&mut ctx, &sname, sm)?;
        let enabled = svc.profiles.is_empty()
            || active.contains("*")
            || svc.profiles.iter().any(|p| active.contains(p))
            || opts.services.contains(&sname);
        if enabled {
            services.insert(sname, svc);
        } else {
            disabled.insert(sname);
        }
    }

    for s in services.values() {
        for d in &s.depends_on {
            if !services.contains_key(&d.service) {
                return Err(if disabled.contains(&d.service) {
                    Error::Invalid(format!(
                        "service '{}' depends on '{}', which is disabled by its profiles",
                        s.name, d.service
                    ))
                } else {
                    Error::Invalid(format!("service '{}' depends on undefined service '{}'", s.name, d.service))
                });
            }
        }
    }

    Ok(Project { name, dir, services, volumes, warnings })
}

fn interpolate_value(v: &mut Value, lookup: &dyn Fn(&str) -> Option<String>) -> Result<()> {
    match v {
        Value::String(s) => *s = interpolate::interpolate(s, lookup)?,
        Value::Sequence(seq) => {
            for x in seq {
                interpolate_value(x, lookup)?;
            }
        }
        Value::Mapping(m) => {
            for (_, x) in m.iter_mut() {
                interpolate_value(x, lookup)?;
            }
        }
        Value::Tagged(t) => interpolate_value(&mut t.value, lookup)?,
        _ => {}
    }
    Ok(())
}

/// Explicit > file `name:` > `COMPOSE_PROJECT_NAME` > directory name.
fn project_name(explicit: Option<&str>, file: Option<&str>, envname: Option<&str>, dir: &Path) -> String {
    let raw = [explicit, file, envname]
        .into_iter()
        .flatten()
        .find(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| {
            dir.canonicalize()
                .ok()
                .as_deref()
                .unwrap_or(dir)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default()
        });
    normalize_project_name(&raw)
}

/// Lowercase and keep only `[a-z0-9_-]`; must start alphanumeric.
pub fn normalize_project_name(raw: &str) -> String {
    let s: String = raw
        .to_lowercase()
        .chars()
        .filter(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '_' || *c == '-')
        .collect();
    let s = s.trim_start_matches(['-', '_']).to_string();
    if s.is_empty() {
        "default".into()
    } else {
        s
    }
}

impl Project {
    /// Topological start order of `only` (all if empty) plus their deps.
    /// Ties are broken by file order.
    pub fn start_order(&self, only: &[String]) -> Result<Vec<String>> {
        let mut include: BTreeSet<&str> = BTreeSet::new();
        if only.is_empty() {
            include.extend(self.services.keys().map(String::as_str));
        } else {
            let mut stack: Vec<&str> = Vec::new();
            for o in only {
                let (k, _) = self
                    .services
                    .get_key_value(o.as_str())
                    .ok_or_else(|| Error::NotFound(format!("no such service '{o}'")))?;
                stack.push(k);
            }
            while let Some(n) = stack.pop() {
                if include.insert(n) {
                    for d in &self.services[n].depends_on {
                        stack.push(&d.service);
                    }
                }
            }
        }
        let mut done: BTreeSet<&str> = BTreeSet::new();
        let mut order: Vec<String> = Vec::new();
        while order.len() < include.len() {
            let next = self.services.keys().map(String::as_str).find(|n| {
                include.contains(n)
                    && !done.contains(n)
                    && self.services[*n].depends_on.iter().all(|d| done.contains(d.service.as_str()))
            });
            match next {
                Some(n) => {
                    done.insert(n);
                    order.push(n.to_string());
                }
                None => {
                    let remaining: Vec<&str> =
                        include.iter().copied().filter(|n| !done.contains(n)).collect();
                    return Err(Error::Invalid(format!(
                        "dependency cycle: {}",
                        self.find_cycle(&remaining).join(" -> ")
                    )));
                }
            }
        }
        Ok(order)
    }

    /// Find one cycle among `candidates` (all of which are stuck), returned
    /// as `a, b, a`.
    fn find_cycle(&self, candidates: &[&str]) -> Vec<String> {
        let mut path: Vec<&str> = Vec::new();
        let mut cur = candidates[0];
        loop {
            if let Some(pos) = path.iter().position(|p| *p == cur) {
                let mut cyc: Vec<String> = path[pos..].iter().map(|s| s.to_string()).collect();
                cyc.push(cur.to_string());
                return cyc;
            }
            path.push(cur);
            let next = self.services[cur]
                .depends_on
                .iter()
                .map(|d| d.service.as_str())
                .find(|d| candidates.contains(d));
            match next {
                Some(n) => cur = n,
                None => return vec![cur.to_string()],
            }
        }
    }

    /// Image name to run/build for a service (`image:` or `<project>-<svc>`).
    pub fn image_name(&self, service: &str) -> String {
        self.services
            .get(service)
            .and_then(|s| s.image.clone())
            .unwrap_or_else(|| format!("{}-{}", self.name, service))
    }

    /// Container name: `container_name` or `<project>-<svc>-1`.
    pub fn container_name(&self, service: &str) -> String {
        self.services
            .get(service)
            .and_then(|s| s.container_name.clone())
            .unwrap_or_else(|| format!("{}-{}-1", self.name, service))
    }

    /// The ContainerSpec for a service: network = Sandbox(project name),
    /// extra_hosts maps every service name to 127.0.0.1, compose labels set.
    pub fn container_spec(&self, service: &str) -> Result<ContainerSpec> {
        let svc = self
            .services
            .get(service)
            .ok_or_else(|| Error::NotFound(format!("no such service '{service}'")))?;
        let mut hosts: Vec<(String, String)> = Vec::new();
        let mut add = |h: &str| {
            if !hosts.iter().any(|(x, _)| x == h) {
                hosts.push((h.to_string(), "127.0.0.1".to_string()));
            }
        };
        for (n, s) in &self.services {
            add(n);
            if let Some(c) = &s.container_name {
                add(c);
            }
        }
        hosts.extend(svc.extra_hosts.iter().cloned());

        let mut lbl = svc.labels.clone();
        lbl.insert(labels::PROJECT.into(), self.name.clone());
        lbl.insert(labels::SERVICE.into(), service.into());
        lbl.insert(labels::CONFIG_HASH.into(), self.config_hash(service));

        Ok(ContainerSpec {
            name: Some(self.container_name(service)),
            image: self.image_name(service),
            platform: svc.platform.as_deref().map(Platform::parse).transpose()?,
            entrypoint: svc.entrypoint.clone(),
            cmd: svc.command.clone(),
            env: svc.environment.clone(),
            working_dir: svc.working_dir.clone(),
            user: svc.user.clone(),
            hostname: Some(svc.hostname.clone().unwrap_or_else(|| service.to_string())),
            ports: svc.ports.clone(),
            mounts: svc.volumes.clone(),
            network: NetworkMode::Sandbox(self.name.clone()),
            extra_hosts: hosts,
            labels: lbl,
            tty: svc.tty,
            interactive: svc.stdin_open,
            restart: svc.restart,
            healthcheck: svc.healthcheck.clone(),
            memory_limit: svc.mem_limit,
            cpus: svc.cpus,
            ..Default::default()
        })
    }

    /// Stable hash of a service's effective config (for recreate detection).
    /// Dependencies and profiles are excluded: they do not change the
    /// container itself.
    pub fn config_hash(&self, service: &str) -> String {
        let Some(s) = self.services.get(service) else { return String::new() };
        let canon = serde_json::json!({
            "image": self.image_name(service),
            "build": s.build.as_ref().map(|b| serde_json::json!({
                "context": b.context,
                "dockerfile": b.dockerfile,
                "args": b.args,
                "target": b.target,
            })),
            "container_name": s.container_name,
            "command": s.command,
            "entrypoint": s.entrypoint,
            "environment": s.environment,
            "ports": s.ports,
            "volumes": s.volumes,
            "healthcheck": s.healthcheck,
            "restart": s.restart,
            "working_dir": s.working_dir,
            "user": s.user,
            "hostname": s.hostname,
            "platform": s.platform,
            "labels": s.labels,
            "tty": s.tty,
            "stdin_open": s.stdin_open,
            "extra_hosts": s.extra_hosts,
            "mem_limit": s.mem_limit,
            "cpus": s.cpus,
        });
        let digest = Sha256::digest(canon.to_string().as_bytes());
        hex::encode(digest)[..16].to_string()
    }

    /// All published ports across services (for the shared sandbox).
    pub fn all_ports(&self) -> Vec<PortMapping> {
        let mut out: Vec<PortMapping> = Vec::new();
        for s in self.services.values() {
            for p in &s.ports {
                if !out.contains(p) {
                    out.push(p.clone());
                }
            }
        }
        out
    }
}
