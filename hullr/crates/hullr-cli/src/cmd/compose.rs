//! `hullr compose ...`

use crate::cli::{ComposeArgs, ComposeCmd};
use crate::output::{self, table};
use anyhow::{Context, Result};
use futures::StreamExt;
use hullr_core::*;
use std::path::PathBuf;

fn options(c: &ComposeArgs, services: Vec<String>) -> Result<ComposeOptions> {
    let cwd = std::env::current_dir()?;
    let abs = |p: &PathBuf| if p.is_absolute() { p.clone() } else { cwd.join(p) };
    let files: Vec<PathBuf> = c.file.iter().map(abs).collect();
    let project_dir = match (&c.project_directory, files.first()) {
        (Some(d), _) => abs(d),
        (None, Some(f)) => f.parent().map(PathBuf::from).unwrap_or(cwd.clone()),
        (None, None) => cwd.clone(),
    };
    Ok(ComposeOptions {
        files,
        project_dir,
        project_name: c.project_name.clone(),
        services,
        build: false,
        remove_volumes: false,
        // The daemon's own environment is not the user's: send ours along.
        env: std::env::vars().collect(),
    })
}

pub async fn run(api: &dyn Api, c: ComposeArgs) -> Result<i32> {
    match &c.command {
        ComposeCmd::Up { detach, build, services } => {
            let mut opts = options(&c, services.clone())?;
            opts.build = *build;
            let status = api.compose_up(opts.clone(), output::progress_printer()).await?;
            if *detach {
                return Ok(0);
            }
            eprintln!("{}", output::dim("Attaching to logs. Press Ctrl-C to stop the services."));
            follow_logs(api, &status, true, None).await?;
            eprintln!("Stopping...");
            for s in &status.services {
                if let Some(ct) = &s.container {
                    let _ = api.stop_container(&ct.id, 10).await;
                }
            }
            Ok(0)
        }
        ComposeCmd::Down { volumes } => {
            let mut opts = options(&c, vec![])?;
            opts.remove_volumes = *volumes;
            api.compose_down(opts, output::progress_printer()).await?;
            Ok(0)
        }
        ComposeCmd::Ps => {
            let st = api.compose_ps(options(&c, vec![])?).await?;
            let now = now_unix();
            let rows: Vec<Vec<String>> = st
                .services
                .iter()
                .map(|s| match &s.container {
                    Some(ct) => vec![
                        ct.name.clone(),
                        s.service.clone(),
                        ct.image.clone(),
                        ct.status_text(now),
                        ct.spec.ports.iter().map(|p| p.to_string()).collect::<Vec<_>>().join(", "),
                    ],
                    None => vec!["-".into(), s.service.clone(), "-".into(), "not created".into(), String::new()],
                })
                .collect();
            table(&["NAME", "SERVICE", "IMAGE", "STATUS", "PORTS"], &rows);
            Ok(0)
        }
        ComposeCmd::Logs { follow, tail, services } => {
            let st = api.compose_ps(options(&c, vec![])?).await?;
            let st = ComposeStatus {
                project: st.project,
                services: st.services.into_iter().filter(|s| services.is_empty() || services.contains(&s.service)).collect(),
            };
            follow_logs(api, &st, *follow, *tail).await?;
            Ok(0)
        }
        ComposeCmd::Build { services } => {
            let opts = options(&c, services.clone())?;
            let project = hullr_compose::load(&opts)?;
            for (name, svc) in &project.services {
                if !services.is_empty() && !services.contains(name) {
                    continue;
                }
                let Some(b) = &svc.build else { continue };
                eprintln!("{} {name}", output::bold("Building"));
                let bo = BuildOptions {
                    context: b.context.clone(),
                    dockerfile: b.dockerfile.clone(),
                    tags: vec![project.image_name(name)],
                    build_args: b.args.clone(),
                    target: b.target.clone(),
                    platform: svc.platform.as_deref().map(Platform::parse).transpose()?,
                    ..Default::default()
                };
                api.build_image(bo, output::progress_printer()).await?;
            }
            Ok(0)
        }
        ComposeCmd::Pull { services } => {
            let opts = options(&c, services.clone())?;
            let project = hullr_compose::load(&opts)?;
            for (name, svc) in &project.services {
                if (!services.is_empty() && !services.contains(name)) || svc.image.is_none() {
                    continue;
                }
                let img = project.image_name(name);
                let platform = svc.platform.as_deref().map(Platform::parse).transpose()?;
                api.pull_image(&img, platform, output::progress_printer()).await.with_context(|| format!("pulling {img}"))?;
            }
            Ok(0)
        }
        ComposeCmd::Stop { services } | ComposeCmd::Start { services } | ComposeCmd::Restart { services } => {
            let st = api.compose_ps(options(&c, vec![])?).await?;
            let mut code = 0;
            for s in st.services.iter().filter(|s| services.is_empty() || services.contains(&s.service)) {
                let Some(ct) = &s.container else { continue };
                let r = match &c.command {
                    ComposeCmd::Stop { .. } => api.stop_container(&ct.id, 10).await,
                    ComposeCmd::Start { .. } => api.start_container(&ct.id, false).await,
                    _ => api.restart_container(&ct.id, 10).await,
                };
                match r {
                    Ok(()) => eprintln!("Container {} {}", ct.name, output::green("done")),
                    Err(e) => {
                        output::error(&format!("{}: {e}", ct.name));
                        code = 1;
                    }
                }
            }
            Ok(code)
        }
        ComposeCmd::Ls => {
            let list = api.compose_list().await?;
            if list.is_empty() {
                eprintln!("No compose projects running.");
                return Ok(0);
            }
            let rows: Vec<Vec<String>> = list
                .iter()
                .map(|p| {
                    let running = p
                        .services
                        .iter()
                        .filter(|s| s.container.as_ref().map(|c| c.state == ContainerState::Running).unwrap_or(false))
                        .count();
                    vec![p.project.clone(), format!("running({running}/{})", p.services.len())]
                })
                .collect();
            table(&["NAME", "STATUS"], &rows);
            Ok(0)
        }
    }
}

/// Interleave service logs with colored `name |` prefixes until Ctrl-C
/// (or the end, without follow).
async fn follow_logs(api: &dyn Api, st: &ComposeStatus, follow: bool, tail: Option<usize>) -> Result<()> {
    let width = st.services.iter().filter_map(|s| s.container.as_ref()).map(|c| c.name.len()).max().unwrap_or(4);
    let mut streams = Vec::new();
    for (i, s) in st.services.iter().enumerate() {
        let Some(ct) = &s.container else { continue };
        let prefix = output::service_color(i, &format!("{:width$} |", ct.name));
        let opts = LogOptions { follow, tail: tail.or(if follow { Some(20) } else { None }), timestamps: false };
        let s = api.logs(&ct.id, opts).await?;
        streams.push(s.map(move |l| (prefix.clone(), l)).boxed());
    }
    let mut all = futures::stream::select_all(streams);
    loop {
        tokio::select! {
            item = all.next() => match item {
                Some((p, Ok(l))) => println!("{p} {}", l.line),
                Some((_, Err(e))) => output::warn(&e.to_string()),
                None => break,
            },
            _ = tokio::signal::ctrl_c() => break,
        }
    }
    Ok(())
}
