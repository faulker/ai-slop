//! `compose up/down/ps/ls` on top of the engine primitives.

use crate::{health, records, records::Record, volumes, Engine};
use hullr_core::api::{Progress, ProgressFn};
use hullr_core::labels::{CONFIG_HASH, PROJECT, SERVICE};
use hullr_core::*;
use hullr_compose::{DependsCondition, Project};
use std::time::Duration;

fn status(engine: &Engine, msg: String, progress: &ProgressFn) {
    let _ = engine;
    progress(Progress::Status { message: msg });
}

fn project_records(engine: &Engine, project: &str) -> Vec<Record> {
    records::list(&engine.paths)
        .unwrap_or_default()
        .into_iter()
        .filter(|r| r.label(PROJECT) == Some(project))
        .collect()
}

fn service_record<'a>(recs: &'a [Record], service: &str) -> Option<&'a Record> {
    recs.iter().find(|r| r.label(SERVICE) == Some(service))
}

pub async fn up(engine: &Engine, opts: ComposeOptions, progress: ProgressFn) -> Result<ComposeStatus> {
    let project = hullr_compose::load(&opts)?;
    for w in &project.warnings {
        status(engine, format!("warning: {w}"), &progress);
    }
    let order = project.start_order(&opts.services)?;

    // Named volumes declared by the project.
    for (key, def) in &project.volumes {
        let name = def.name.clone().unwrap_or_else(|| format!("{}_{key}", project.name));
        if def.external {
            if !volumes::data_dir(&engine.paths, &name).exists() {
                return Err(Error::NotFound(format!("external volume '{name}' does not exist")));
            }
        } else {
            let mut labels = std::collections::BTreeMap::new();
            labels.insert(PROJECT.to_string(), project.name.clone());
            volumes::ensure(&engine.paths, &name, labels)?;
        }
    }

    // Images: build what needs building, pull what is missing.
    for svc in &order {
        ensure_service_image(engine, &project, svc, opts.build, &progress).await?;
    }

    // All services share one network sandbox so they reach each other by
    // service name (mapped to 127.0.0.1 in /etc/hosts).
    engine.runtime.ensure_sandbox(&project.name, &project.all_ports())?;

    for svc in &order {
        let service = &project.services[svc];
        for dep in &service.depends_on {
            wait_dependency(engine, &project, &dep.service, dep.condition, &progress).await?;
        }
        let hash = project.config_hash(svc);
        let existing = project_records(engine, &project.name);
        if let Some(r) = service_record(&existing, svc) {
            if r.label(CONFIG_HASH) == Some(hash.as_str()) {
                if engine.is_running(&r.id) {
                    status(engine, format!("Container {} Running", r.name), &progress);
                } else {
                    status(engine, format!("Container {} Starting", r.name), &progress);
                    hullr_core::Api::start_container(engine, &r.id, false).await?;
                    status(engine, format!("Container {} Started", r.name), &progress);
                }
                continue;
            }
            status(engine, format!("Container {} Recreate", r.name), &progress);
            hullr_core::Api::stop_container(engine, &r.id, 10).await?;
            hullr_core::Api::remove_container(engine, &r.id, true).await?;
        }
        let mut spec = project.container_spec(svc)?;
        spec.labels.insert(CONFIG_HASH.into(), hash);
        // The image is already resolved above; never re-pull here.
        spec.pull = PullPolicy::Missing;
        let name = spec.name.clone().unwrap_or_default();
        status(engine, format!("Container {name} Creating"), &progress);
        let id = hullr_core::Api::create_container(engine, spec, progress.clone()).await?;
        hullr_core::Api::start_container(engine, &id, false).await?;
        status(engine, format!("Container {name} Started"), &progress);
    }
    ps_project(engine, &project)
}

async fn ensure_service_image(
    engine: &Engine,
    project: &Project,
    svc: &str,
    force_build: bool,
    progress: &ProgressFn,
) -> Result<()> {
    let service = &project.services[svc];
    let image = project.image_name(svc);
    let platform = service.platform.as_deref().map(Platform::parse).transpose()?;
    if let Some(b) = &service.build {
        if force_build || engine.images.find(&image, platform.as_ref())?.is_none() {
            status(engine, format!("Building {svc}"), progress);
            let opts = BuildOptions {
                context: b.context.clone(),
                dockerfile: b.dockerfile.clone(),
                tags: vec![image.clone()],
                build_args: b.args.clone(),
                target: b.target.clone(),
                platform: platform.clone(),
                ..Default::default()
            };
            hullr_core::Api::build_image(engine, opts, progress.clone()).await?;
        }
        return Ok(());
    }
    engine.ensure_image(&image, platform.as_ref(), PullPolicy::Missing, progress.clone()).await?;
    Ok(())
}

async fn wait_dependency(
    engine: &Engine,
    project: &Project,
    dep: &str,
    cond: DependsCondition,
    progress: &ProgressFn,
) -> Result<()> {
    let recs = project_records(engine, &project.name);
    let Some(r) = service_record(&recs, dep) else {
        return Err(Error::NotFound(format!("dependency {dep} has no container")));
    };
    match cond {
        DependsCondition::Started => Ok(()),
        DependsCondition::Healthy => {
            status(engine, format!("Container {} Waiting (healthy)", r.name), progress);
            health::wait_healthy(engine, &r.id, Duration::from_secs(300)).await?;
            status(engine, format!("Container {} Healthy", r.name), progress);
            Ok(())
        }
        DependsCondition::CompletedSuccessfully => {
            status(engine, format!("Container {} Waiting (exit)", r.name), progress);
            let code = engine.runtime.wait(&r.id).await?;
            if code != 0 {
                return Err(Error::Runtime(format!("{} exited with code {code}", r.name)));
            }
            Ok(())
        }
    }
}

pub async fn down(engine: &Engine, opts: ComposeOptions, progress: ProgressFn) -> Result<()> {
    // Work even if the compose file is gone: fall back to labels.
    let (name, project) = match hullr_compose::load(&opts) {
        Ok(p) => (p.name.clone(), Some(p)),
        Err(e) => match &opts.project_name {
            Some(n) => (n.clone(), None),
            None => return Err(e),
        },
    };
    let mut recs = project_records(engine, &name);
    // Reverse start order: dependents stop first.
    if let Some(p) = &project {
        if let Ok(order) = p.start_order(&[]) {
            recs.sort_by_key(|r| {
                std::cmp::Reverse(order.iter().position(|s| Some(s.as_str()) == r.label(SERVICE)).unwrap_or(0))
            });
        }
    }
    for r in &recs {
        status(engine, format!("Container {} Stopping", r.name), &progress);
        hullr_core::Api::stop_container(engine, &r.id, 10).await?;
        hullr_core::Api::remove_container(engine, &r.id, true).await?;
        status(engine, format!("Container {} Removed", r.name), &progress);
    }
    let _ = engine.runtime.remove_sandbox(&name);
    if opts.remove_volumes {
        for v in volumes::list(&engine.paths)? {
            if v.labels.get(PROJECT) == Some(&name) {
                volumes::remove(&engine.paths, &v.name)?;
                status(engine, format!("Volume {} Removed", v.name), &progress);
            }
        }
    }
    Ok(())
}

fn ps_project(engine: &Engine, project: &Project) -> Result<ComposeStatus> {
    let recs = project_records(engine, &project.name);
    Ok(ComposeStatus {
        project: project.name.clone(),
        services: project
            .services
            .keys()
            .map(|svc| ComposeService {
                service: svc.clone(),
                container: service_record(&recs, svc).map(|r| engine.to_info(r)),
            })
            .collect(),
    })
}

pub fn ps(engine: &Engine, opts: &ComposeOptions) -> Result<ComposeStatus> {
    match hullr_compose::load(opts) {
        Ok(p) => ps_project(engine, &p),
        Err(e) => {
            let Some(name) = &opts.project_name else { return Err(e) };
            Ok(from_labels(engine, name))
        }
    }
}

fn from_labels(engine: &Engine, project: &str) -> ComposeStatus {
    let recs = project_records(engine, project);
    ComposeStatus {
        project: project.to_string(),
        services: recs
            .iter()
            .map(|r| ComposeService {
                service: r.label(SERVICE).unwrap_or("?").to_string(),
                container: Some(engine.to_info(r)),
            })
            .collect(),
    }
}

pub fn list(engine: &Engine) -> Result<Vec<ComposeStatus>> {
    let mut names: Vec<String> = records::list(&engine.paths)?
        .iter()
        .filter_map(|r| r.label(PROJECT).map(String::from))
        .collect();
    names.sort();
    names.dedup();
    Ok(names.iter().map(|n| from_labels(engine, n)).collect())
}
