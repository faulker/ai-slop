//! Turning a user's ContainerSpec + image config into a runtime RunSpec.

use crate::{lock::FileLock, names, records, records::Record, volumes, Engine};
use hullr_core::api::EventKind;
use hullr_core::*;
use hullr_runtime::{ResolvedMount, RunSpec};

pub async fn create(engine: &Engine, spec: ContainerSpec, progress: ProgressFn) -> Result<String> {
    if let Some(n) = &spec.name {
        if !names::valid(n) {
            return Err(Error::Invalid(format!("invalid container name '{n}'")));
        }
    }
    let img = engine
        .ensure_image(&spec.image, spec.platform.as_ref(), spec.pull, progress)
        .await?;
    let cfg = &img.info.config;
    let args = command(&spec, cfg)?;
    let env = merge_env(&cfg.env, &spec.env);
    let platform = spec.platform.clone().unwrap_or_else(|| img.info.platform.clone());
    let lower = engine.images.rootfs(&img)?;

    let id = new_id();
    let mut mounts = Vec::new();
    let mut vol_names = Vec::new();
    for m in &spec.mounts {
        mounts.push(match m.kind {
            MountKind::Bind => {
                let src = std::path::PathBuf::from(&m.source);
                if !src.exists() {
                    // Docker creates missing bind sources as directories.
                    std::fs::create_dir_all(&src).context(format!("creating bind source {}", m.source))?;
                }
                ResolvedMount { kind: MountKind::Bind, source: src, target: m.target.clone(), read_only: m.read_only }
            }
            MountKind::Volume => {
                let name = if m.source.is_empty() { id[..32].to_string() } else { m.source.clone() };
                let (v, created) = volumes::ensure(&engine.paths, &name, Default::default())?;
                if created {
                    volumes::copy_up(&lower, &m.target, &v.path);
                    engine.emit(EventKind::VolumeCreated, &name, &name);
                }
                vol_names.push(name);
                ResolvedMount { kind: MountKind::Volume, source: v.path, target: m.target.clone(), read_only: m.read_only }
            }
            MountKind::Tmpfs => ResolvedMount {
                kind: MountKind::Tmpfs,
                source: Default::default(),
                target: m.target.clone(),
                read_only: m.read_only,
            },
        });
    }
    // Image VOLUMEs without a user mount get anonymous volumes, like Docker.
    for target in &cfg.volumes {
        if mounts.iter().any(|m| &m.target == target) {
            continue;
        }
        let name = format!("{}{}", &id[..24], vol_names.len());
        let (v, _) = volumes::ensure(&engine.paths, &name, Default::default())?;
        volumes::copy_up(&lower, target, &v.path);
        vol_names.push(name);
        mounts.push(ResolvedMount { kind: MountKind::Volume, source: v.path, target: target.clone(), read_only: false });
    }

    let network = match &spec.network {
        NetworkMode::Container(other) => NetworkMode::Container(engine.find_record(other)?.id),
        n => n.clone(),
    };

    let _lock = FileLock::acquire(&engine.paths.engine().join("create.lock"))?;
    let existing = records::list(&engine.paths)?;
    let name = match &spec.name {
        Some(n) => {
            if existing.iter().any(|r| &r.name == n) {
                return Err(Error::Conflict(format!(
                    "container name '{n}' is already in use (remove it or pick another name)"
                )));
            }
            n.clone()
        }
        None => {
            let mut n = names::generate(&id);
            let mut i = 2;
            while existing.iter().any(|r| r.name == n) {
                n = format!("{}_{i}", names::generate(&id));
                i += 1;
            }
            n
        }
    };

    let run = RunSpec {
        id: id.clone(),
        hostname: spec.hostname.clone().unwrap_or_else(|| id[..12].to_string()),
        lower,
        args: args.clone(),
        env,
        cwd: spec.working_dir.clone().or_else(|| cfg.working_dir.clone()).filter(|w| !w.is_empty()).unwrap_or_else(|| "/".into()),
        user: spec.user.clone().or_else(|| cfg.user.clone()).filter(|u| !u.is_empty()),
        mounts,
        ports: spec.ports.clone(),
        network,
        extra_hosts: spec.extra_hosts.clone(),
        tty: spec.tty,
        interactive: spec.interactive,
        platform: platform.clone(),
        restart: spec.restart,
        stop_signal: cfg.stop_signal.clone(),
        memory_limit: spec.memory_limit,
        cpus: spec.cpus,
    };
    engine.runtime.create(&run)?;
    let healthcheck = spec
        .healthcheck
        .clone()
        .or_else(|| cfg.healthcheck.clone())
        .filter(|h| h.test.first().map(String::as_str) != Some("NONE"));
    let rec = Record {
        id: id.clone(),
        name: name.clone(),
        image: spec.image.clone(),
        image_id: img.info.id.clone(),
        command: args,
        created: now_unix(),
        platform,
        spec,
        healthcheck,
        health: None,
        volumes: vol_names,
    };
    records::save(&engine.paths, &rec)?;
    engine.emit(EventKind::ContainerCreated, &id, &name);
    Ok(id)
}

/// Docker semantics: a user entrypoint drops the image CMD unless a cmd is
/// also given.
pub fn command(spec: &ContainerSpec, cfg: &ImageConfig) -> Result<Vec<String>> {
    let entrypoint = spec.entrypoint.clone().or_else(|| cfg.entrypoint.clone()).unwrap_or_default();
    let cmd = match (&spec.cmd, &spec.entrypoint) {
        (Some(c), _) => c.clone(),
        (None, Some(_)) => vec![],
        (None, None) => cfg.cmd.clone().unwrap_or_default(),
    };
    let args: Vec<String> = entrypoint.into_iter().chain(cmd).collect();
    if args.is_empty() {
        return Err(Error::Invalid("no command specified (the image has no CMD or ENTRYPOINT)".into()));
    }
    Ok(args)
}

/// Image env, then overrides by key, keeping order.
pub fn merge_env(base: &[String], over: &[String]) -> Vec<String> {
    let key = |s: &str| s.split('=').next().unwrap_or("").to_string();
    let mut out: Vec<String> = base.to_vec();
    for e in over {
        let k = key(e);
        // `-e FOO` without a value passes the caller's value through.
        let e = if e.contains('=') {
            e.clone()
        } else {
            match std::env::var(&k) {
                Ok(v) => format!("{k}={v}"),
                Err(_) => continue,
            }
        };
        match out.iter_mut().find(|x| key(x) == k) {
            Some(slot) => *slot = e,
            None => out.push(e),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn command_rules() {
        let cfg = ImageConfig { entrypoint: Some(s(&["/ep"])), cmd: Some(s(&["a"])), ..Default::default() };
        let mut spec = ContainerSpec::default();
        assert_eq!(command(&spec, &cfg).unwrap(), s(&["/ep", "a"]));
        spec.cmd = Some(s(&["b"]));
        assert_eq!(command(&spec, &cfg).unwrap(), s(&["/ep", "b"]));
        spec.cmd = None;
        spec.entrypoint = Some(s(&["/other"]));
        assert_eq!(command(&spec, &cfg).unwrap(), s(&["/other"]));
        assert!(command(&ContainerSpec::default(), &ImageConfig::default()).is_err());
    }

    #[test]
    fn env_merge() {
        let out = merge_env(&s(&["PATH=/bin", "A=1"]), &s(&["A=2", "B=3"]));
        assert_eq!(out, s(&["PATH=/bin", "A=2", "B=3"]));
    }
}
