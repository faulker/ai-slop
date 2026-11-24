//! Resolution of a merged, interpolated YAML service into a [`Service`].

use crate::interpolate::{interpolate, parse_dotenv};
use crate::units::{duration_secs_ceil, parse_bytes};
use crate::{shell, BuildDef, Dependency, DependsCondition, Service, VolumeDef};
use hullr_core::{
    normalize, Error, HealthCheck, Mount, MountKind, PortMapping, Protocol, RestartPolicy, Result,
};
use indexmap::IndexMap;
use serde_yaml::{Mapping, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Keys we understand.
const SUPPORTED: &[&str] = &[
    "image", "build", "container_name", "command", "entrypoint", "environment", "env_file",
    "ports", "volumes", "depends_on", "healthcheck", "restart", "working_dir", "user",
    "hostname", "platform", "labels", "tty", "stdin_open", "extra_hosts", "profiles",
    "mem_limit", "cpus", "tmpfs",
];
/// Keys that are accepted and ignored without a warning (everything shares
/// one network sandbox, so these carry no meaning).
const SILENT: &[&str] = &["networks", "expose", "deploy", "pull_policy", "init"];

pub struct Ctx<'a> {
    pub dir: &'a Path,
    pub project: &'a str,
    pub env: &'a BTreeMap<String, String>,
    pub volumes: &'a IndexMap<String, VolumeDef>,
    pub warnings: &'a mut Vec<String>,
}

fn invalid(svc: &str, msg: impl std::fmt::Display) -> Error {
    Error::Invalid(format!("service '{svc}': {msg}"))
}

pub fn scalar(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

fn as_bool(v: &Value) -> Option<bool> {
    match v {
        Value::Bool(b) => Some(*b),
        Value::String(s) => match s.to_ascii_lowercase().as_str() {
            "true" | "yes" | "1" | "on" => Some(true),
            "false" | "no" | "0" | "off" | "" => Some(false),
            _ => None,
        },
        Value::Number(n) => n.as_i64().map(|i| i != 0),
        _ => None,
    }
}

fn as_f64(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

fn string_list(svc: &str, key: &str, v: &Value) -> Result<Vec<String>> {
    match v {
        Value::Sequence(s) => s
            .iter()
            .map(|x| scalar(x).ok_or_else(|| invalid(svc, format!("'{key}' entries must be scalars"))))
            .collect(),
        other => scalar(other)
            .map(|s| vec![s])
            .ok_or_else(|| invalid(svc, format!("'{key}' must be a string or list"))),
    }
}

/// `KEY=VALUE` list or `KEY: VALUE` map into ordered pairs; a missing value
/// is `None`.
fn kv_pairs(svc: &str, key: &str, v: &Value) -> Result<Vec<(String, Option<String>)>> {
    match v {
        Value::Mapping(m) => m
            .iter()
            .map(|(k, v)| {
                let k = scalar(k).ok_or_else(|| invalid(svc, format!("bad key in '{key}'")))?;
                Ok((k, if v.is_null() { None } else { scalar(v) }))
            })
            .collect(),
        Value::Sequence(s) => s
            .iter()
            .map(|x| {
                let s = scalar(x).ok_or_else(|| invalid(svc, format!("bad entry in '{key}'")))?;
                Ok(match s.split_once('=') {
                    Some((k, v)) => (k.to_string(), Some(v.to_string())),
                    None => (s, None),
                })
            })
            .collect(),
        _ => Err(invalid(svc, format!("'{key}' must be a list or map"))),
    }
}

fn command_list(svc: &str, key: &str, v: &Value) -> Result<Vec<String>> {
    match v {
        Value::String(s) => shell::split(s),
        Value::Sequence(_) => string_list(svc, key, v),
        _ => Err(invalid(svc, format!("'{key}' must be a string or list"))),
    }
}

pub fn resolve_service(ctx: &mut Ctx, name: &str, m: &Mapping) -> Result<Service> {
    for k in m.keys() {
        let k = k.as_str().unwrap_or("?");
        if !SUPPORTED.contains(&k) && !SILENT.contains(&k) && !k.starts_with("x-") {
            ctx.warnings.push(format!("service '{name}': unsupported key '{k}' ignored"));
        }
    }
    let get = |k: &str| m.get(k).filter(|v| !v.is_null());
    let text = |k: &str| -> Result<Option<String>> {
        match get(k) {
            None => Ok(None),
            Some(v) => scalar(v).map(Some).ok_or_else(|| invalid(name, format!("'{k}' must be a string"))),
        }
    };

    let image = text("image")?;
    let build = get("build").map(|v| resolve_build(ctx, name, v)).transpose()?;
    if image.is_none() && build.is_none() {
        return Err(invalid(name, "needs either 'image' or 'build'"));
    }

    let mut volumes = Vec::new();
    if let Some(v) = get("volumes") {
        let Value::Sequence(items) = v else {
            return Err(invalid(name, "'volumes' must be a list"));
        };
        for it in items {
            volumes.push(resolve_volume(ctx, name, it)?);
        }
    }
    if let Some(v) = get("tmpfs") {
        for t in string_list(name, "tmpfs", v)? {
            let target = t.split(':').next().unwrap_or("").to_string();
            volumes.push(Mount { kind: MountKind::Tmpfs, source: String::new(), target, read_only: false });
        }
    }

    let mut ports = Vec::new();
    if let Some(v) = get("ports") {
        let Value::Sequence(items) = v else {
            return Err(invalid(name, "'ports' must be a list"));
        };
        for it in items {
            for p in resolve_port(it).map_err(|e| invalid(name, e))? {
                if !ports.contains(&p) {
                    ports.push(p);
                }
            }
        }
    }

    let restart = match text("restart")? {
        Some(r) => RestartPolicy::parse(&r).map_err(|e| invalid(name, e))?,
        None => RestartPolicy::No,
    };

    let mut mem_limit = match text("mem_limit")? {
        Some(s) => Some(parse_bytes(&s).map_err(|e| invalid(name, e))?),
        None => None,
    };
    let mut cpus = get("cpus").and_then(as_f64);
    if let Some(limits) = m
        .get("deploy")
        .and_then(|d| d.get("resources"))
        .and_then(|r| r.get("limits"))
    {
        if mem_limit.is_none() {
            if let Some(s) = limits.get("memory").and_then(scalar) {
                mem_limit = Some(parse_bytes(&s).map_err(|e| invalid(name, e))?);
            }
        }
        if cpus.is_none() {
            cpus = limits.get("cpus").and_then(as_f64);
        }
    }

    Ok(Service {
        name: name.to_string(),
        image,
        build,
        container_name: text("container_name")?,
        command: get("command").map(|v| command_list(name, "command", v)).transpose()?,
        entrypoint: get("entrypoint").map(|v| command_list(name, "entrypoint", v)).transpose()?,
        environment: resolve_environment(ctx, name, m)?,
        ports,
        volumes,
        depends_on: resolve_depends(name, get("depends_on"))?,
        healthcheck: get("healthcheck").map(|v| resolve_healthcheck(name, v)).transpose()?,
        restart,
        working_dir: text("working_dir")?,
        user: text("user")?,
        hostname: text("hostname")?,
        platform: text("platform")?,
        labels: resolve_labels(name, get("labels"))?,
        tty: get("tty").and_then(as_bool).unwrap_or(false),
        stdin_open: get("stdin_open").and_then(as_bool).unwrap_or(false),
        extra_hosts: resolve_extra_hosts(name, get("extra_hosts"))?,
        profiles: get("profiles").map(|v| string_list(name, "profiles", v)).transpose()?.unwrap_or_default(),
        mem_limit,
        cpus,
    })
}

fn abs(dir: &Path, p: &str) -> PathBuf {
    let path = Path::new(p);
    if path.is_absolute() {
        normalize(path)
    } else if let Some(rest) = p.strip_prefix("~/") {
        dirs_home().join(rest)
    } else {
        normalize(&dir.join(path))
    }
}

fn dirs_home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default()
}

fn resolve_build(ctx: &mut Ctx, svc: &str, v: &Value) -> Result<BuildDef> {
    let (context, dockerfile, args, target) = match v {
        Value::String(s) => (s.clone(), None, Vec::new(), None),
        Value::Mapping(m) => {
            let get = |k: &str| m.get(k).filter(|v| !v.is_null()).and_then(scalar);
            let args = match m.get("args").filter(|v| !v.is_null()) {
                Some(a) => kv_pairs(svc, "build.args", a)?,
                None => Vec::new(),
            };
            (get("context").unwrap_or_else(|| ".".into()), get("dockerfile"), args, get("target"))
        }
        _ => return Err(invalid(svc, "'build' must be a string or map")),
    };
    let context = abs(ctx.dir, &context);
    let dockerfile = dockerfile.map(|d| abs(&context, &d));
    let mut out = BTreeMap::new();
    for (k, v) in args {
        if let Some(v) = v.or_else(|| ctx.env.get(&k).cloned()) {
            out.insert(k, v);
        }
    }
    Ok(BuildDef { context, dockerfile, args: out, target })
}

fn volume_name(ctx: &mut Ctx, key: &str) -> String {
    match ctx.volumes.get(key) {
        Some(def) => match (&def.name, def.external) {
            (Some(n), _) => n.clone(),
            (None, true) => key.to_string(),
            (None, false) => format!("{}_{}", ctx.project, key),
        },
        None => {
            ctx.warnings.push(format!("volume '{key}' is not declared in top-level 'volumes'"));
            format!("{}_{}", ctx.project, key)
        }
    }
}

fn resolve_volume(ctx: &mut Ctx, svc: &str, v: &Value) -> Result<Mount> {
    match v {
        Value::String(s) => {
            let mut m = Mount::parse(s, ctx.dir).map_err(|e| invalid(svc, e))?;
            if m.kind == MountKind::Volume && !m.source.is_empty() {
                m.source = volume_name(ctx, &m.source.clone());
            }
            Ok(m)
        }
        Value::Mapping(map) => {
            let get = |k: &str| map.get(k).filter(|v| !v.is_null()).and_then(scalar);
            let target = get("target").ok_or_else(|| invalid(svc, "volume needs 'target'"))?;
            let source = get("source").unwrap_or_default();
            let read_only = map.get("read_only").and_then(as_bool).unwrap_or(false);
            let kind = match get("type").as_deref() {
                Some("bind") => MountKind::Bind,
                Some("volume") => MountKind::Volume,
                Some("tmpfs") => MountKind::Tmpfs,
                Some(o) => return Err(invalid(svc, format!("unsupported volume type '{o}'"))),
                None if source.is_empty() => MountKind::Volume,
                None if source.starts_with(['/', '.', '~']) => MountKind::Bind,
                None => MountKind::Volume,
            };
            let source = match kind {
                MountKind::Bind => abs(ctx.dir, &source).to_string_lossy().into_owned(),
                MountKind::Volume if !source.is_empty() => volume_name(ctx, &source),
                MountKind::Volume => String::new(),
                MountKind::Tmpfs => String::new(),
            };
            Ok(Mount { kind, source, target, read_only })
        }
        _ => Err(invalid(svc, "volume entries must be strings or maps")),
    }
}

fn port_range(s: &str) -> std::result::Result<(u16, u16), String> {
    let p = |x: &str| x.trim().parse::<u16>().map_err(|_| format!("bad port '{x}'"));
    match s.split_once('-') {
        Some((a, b)) => {
            let (a, b) = (p(a)?, p(b)?);
            if b < a {
                return Err(format!("bad port range '{s}'"));
            }
            Ok((a, b))
        }
        None => p(s).map(|x| (x, x)),
    }
}

/// Parse one `ports:` entry (short string, number or long map) into one
/// mapping per port (ranges expand).
pub fn resolve_port(v: &Value) -> std::result::Result<Vec<PortMapping>, String> {
    match v {
        Value::Mapping(m) => {
            let get = |k: &str| m.get(k).filter(|v| !v.is_null()).and_then(scalar);
            let target = get("target").ok_or("port needs 'target'")?;
            let mut s = String::new();
            if let Some(ip) = get("host_ip") {
                s.push_str(&if ip.contains(':') { format!("[{ip}]:") } else { format!("{ip}:") });
                s.push_str(&get("published").unwrap_or_default());
                s.push(':');
            } else if let Some(p) = get("published") {
                s.push_str(&format!("{p}:"));
            }
            s.push_str(&target);
            if let Some(proto) = get("protocol") {
                s.push_str(&format!("/{proto}"));
            }
            parse_port_spec(&s)
        }
        other => parse_port_spec(&scalar(other).ok_or("ports entries must be scalars or maps")?),
    }
}

/// Parse `[ip:][host:]container[/proto]`, each port possibly a range.
pub fn parse_port_spec(s: &str) -> std::result::Result<Vec<PortMapping>, String> {
    let (rest, protocol) = match s.rsplit_once('/') {
        Some((r, "tcp")) => (r, Protocol::Tcp),
        Some((r, "udp")) => (r, Protocol::Udp),
        Some((_, o)) => return Err(format!("unsupported protocol '{o}' in port '{s}'")),
        None => (s, Protocol::Tcp),
    };
    let (ip, rest) = if let Some(r) = rest.strip_prefix('[') {
        let (ip, after) = r.split_once(']').ok_or_else(|| format!("bad port '{s}'"))?;
        (Some(ip.to_string()), after.strip_prefix(':').unwrap_or(after))
    } else {
        (None, rest)
    };
    let parts: Vec<&str> = rest.split(':').collect();
    let (ip, host, ctr) = match (ip, parts.as_slice()) {
        (ip, [c]) => (ip, "", *c),
        (ip, [h, c]) => (ip, *h, *c),
        (None, [ip, h, c]) => (Some(ip.to_string()), *h, *c),
        _ => return Err(format!("bad port '{s}'")),
    };
    let (c0, c1) = port_range(ctr)?;
    let n = (c1 - c0) as usize + 1;
    let (h0, h1) = if host.is_empty() { (c0, c1) } else { port_range(host)? };
    let hn = (h1 - h0) as usize + 1;
    if hn != n && !(hn == 1 && n == 1) {
        return Err(format!("port ranges differ in length in '{s}'"));
    }
    Ok((0..n as u16)
        .map(|i| PortMapping {
            host_ip: ip.clone(),
            host_port: h0 + i,
            container_port: c0 + i,
            protocol,
        })
        .collect())
}

fn resolve_environment(ctx: &mut Ctx, svc: &str, m: &Mapping) -> Result<Vec<String>> {
    let mut vars: IndexMap<String, String> = IndexMap::new();
    if let Some(ef) = m.get("env_file").filter(|v| !v.is_null()) {
        let entries: Vec<&Value> = match ef {
            Value::Sequence(s) => s.iter().collect(),
            one => vec![one],
        };
        for e in entries {
            let (path, required) = match e {
                Value::Mapping(mm) => (
                    mm.get("path").and_then(scalar).ok_or_else(|| invalid(svc, "env_file needs 'path'"))?,
                    mm.get("required").and_then(as_bool).unwrap_or(true),
                ),
                other => (scalar(other).ok_or_else(|| invalid(svc, "bad env_file entry"))?, true),
            };
            let full = abs(ctx.dir, &path);
            match std::fs::read_to_string(&full) {
                Ok(text) => {
                    for (k, v) in parse_dotenv(&text) {
                        let env = ctx.env;
                        let lookup = |n: &str| env.get(n).cloned();
                        vars.insert(k, interpolate(&v, &lookup).map_err(|e| invalid(svc, e))?);
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound && !required => {}
                Err(e) => {
                    return Err(invalid(svc, format!("cannot read env_file {}: {e}", full.display())))
                }
            }
        }
    }
    if let Some(env) = m.get("environment").filter(|v| !v.is_null()) {
        for (k, v) in kv_pairs(svc, "environment", env)? {
            if let Some(v) = v.or_else(|| ctx.env.get(&k).cloned()) {
                vars.insert(k, v);
            }
        }
    }
    Ok(vars.into_iter().map(|(k, v)| format!("{k}={v}")).collect())
}

fn resolve_depends(svc: &str, v: Option<&Value>) -> Result<Vec<Dependency>> {
    let Some(v) = v else { return Ok(Vec::new()) };
    match v {
        Value::Sequence(s) => s
            .iter()
            .map(|x| {
                let service = scalar(x).ok_or_else(|| invalid(svc, "bad depends_on entry"))?;
                Ok(Dependency { service, condition: DependsCondition::Started })
            })
            .collect(),
        Value::Mapping(m) => m
            .iter()
            .map(|(k, v)| {
                let service = scalar(k).ok_or_else(|| invalid(svc, "bad depends_on key"))?;
                let condition = match v.get("condition").and_then(Value::as_str) {
                    None | Some("service_started") => DependsCondition::Started,
                    Some("service_healthy") => DependsCondition::Healthy,
                    Some("service_completed_successfully") => DependsCondition::CompletedSuccessfully,
                    Some(o) => return Err(invalid(svc, format!("unknown depends_on condition '{o}'"))),
                };
                Ok(Dependency { service, condition })
            })
            .collect(),
        _ => Err(invalid(svc, "'depends_on' must be a list or map")),
    }
}

fn resolve_healthcheck(svc: &str, v: &Value) -> Result<HealthCheck> {
    let Value::Mapping(m) = v else {
        return Err(invalid(svc, "'healthcheck' must be a map"));
    };
    let secs = |k: &str, default: u64| -> Result<u64> {
        match m.get(k).filter(|v| !v.is_null()).and_then(scalar) {
            Some(s) => duration_secs_ceil(&s).map_err(|e| invalid(svc, e)),
            None => Ok(default),
        }
    };
    let mut hc = HealthCheck {
        test: vec!["NONE".into()],
        interval_secs: secs("interval", 30)?,
        timeout_secs: secs("timeout", 30)?,
        retries: m.get("retries").and_then(as_f64).map(|r| r as u32).unwrap_or(3),
        start_period_secs: secs("start_period", 0)?,
    };
    if m.get("disable").and_then(as_bool).unwrap_or(false) {
        return Ok(hc);
    }
    hc.test = match m.get("test").filter(|v| !v.is_null()) {
        Some(Value::String(s)) => vec!["CMD-SHELL".into(), s.clone()],
        Some(Value::Sequence(_)) => {
            let t = string_list(svc, "healthcheck.test", m.get("test").unwrap())?;
            match t.first().map(String::as_str) {
                Some("CMD" | "CMD-SHELL" | "NONE") => t,
                _ => return Err(invalid(svc, "healthcheck test must start with CMD, CMD-SHELL or NONE")),
            }
        }
        Some(_) => return Err(invalid(svc, "bad healthcheck test")),
        None => return Err(invalid(svc, "healthcheck needs 'test' (or 'disable: true')")),
    };
    Ok(hc)
}

fn resolve_labels(svc: &str, v: Option<&Value>) -> Result<BTreeMap<String, String>> {
    let Some(v) = v else { return Ok(BTreeMap::new()) };
    Ok(kv_pairs(svc, "labels", v)?
        .into_iter()
        .map(|(k, v)| (k, v.unwrap_or_default()))
        .collect())
}

fn resolve_extra_hosts(svc: &str, v: Option<&Value>) -> Result<Vec<(String, String)>> {
    let Some(v) = v else { return Ok(Vec::new()) };
    match v {
        Value::Mapping(m) => m
            .iter()
            .map(|(k, v)| {
                Ok((
                    scalar(k).ok_or_else(|| invalid(svc, "bad extra_hosts key"))?,
                    scalar(v).ok_or_else(|| invalid(svc, "bad extra_hosts value"))?,
                ))
            })
            .collect(),
        Value::Sequence(s) => s
            .iter()
            .map(|x| {
                let s = scalar(x).ok_or_else(|| invalid(svc, "bad extra_hosts entry"))?;
                let (h, ip) = s
                    .split_once(':')
                    .or_else(|| s.split_once('='))
                    .ok_or_else(|| invalid(svc, format!("extra_hosts entry '{s}' needs host:ip")))?;
                Ok((h.trim().to_string(), ip.trim().to_string()))
            })
            .collect(),
        _ => Err(invalid(svc, "'extra_hosts' must be a list or map")),
    }
}
