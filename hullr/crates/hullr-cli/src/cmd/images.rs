//! pull, push, images, rmi, build, login.

use crate::cli::BuildArgs;
use crate::output::{self, ago, table};
use anyhow::{bail, Context, Result};
use hullr_core::*;
use std::io::{IsTerminal, Read, Write};

pub async fn pull(api: &dyn Api, image: &str, platform: Option<String>) -> Result<i32> {
    let platform = platform.as_deref().map(Platform::parse).transpose()?;
    let info = api.pull_image(image, platform, output::progress_printer()).await?;
    let name = ImageRef::parse(image).map(|r| r.short()).unwrap_or_else(|_| image.to_string());
    eprintln!("{} {name} ({}, {})", output::green("Pulled"), info.platform, human_bytes(info.size));
    Ok(0)
}

pub async fn push(api: &dyn Api, image: &str) -> Result<i32> {
    api.push_image(image, output::progress_printer()).await?;
    Ok(0)
}

pub async fn list(api: &dyn Api, quiet: bool) -> Result<i32> {
    let images = api.list_images().await?;
    if quiet {
        for i in &images {
            println!("{}", i.short_id());
        }
        return Ok(0);
    }
    if images.is_empty() {
        eprintln!("No images yet. Try: hullr pull alpine");
        return Ok(0);
    }
    let mut rows = Vec::new();
    for i in &images {
        let tags: Vec<(String, String)> = if i.repo_tags.is_empty() {
            vec![("<none>".into(), "<none>".into())]
        } else {
            i.repo_tags
                .iter()
                .map(|t| match ImageRef::parse(t) {
                    Ok(r) => {
                        let short = ImageRef { tag: None, digest: None, ..r.clone() }.short();
                        (short, r.tag.unwrap_or_else(|| "<none>".into()))
                    }
                    Err(_) => (t.clone(), "<none>".into()),
                })
                .collect()
        };
        for (repo, tag) in tags {
            rows.push(vec![repo, tag, i.short_id().to_string(), ago(i.created), human_bytes(i.size), i.platform.to_string()]);
        }
    }
    table(&["REPOSITORY", "TAG", "IMAGE ID", "CREATED", "SIZE", "PLATFORM"], &rows);
    Ok(0)
}

pub async fn rmi(api: &dyn Api, images: &[String], force: bool) -> Result<i32> {
    let mut code = 0;
    for i in images {
        match api.remove_image(i, force).await {
            Ok(removed) => {
                for r in removed {
                    println!("Deleted: {r}");
                }
            }
            Err(e) => {
                output::error(&format!("{i}: {e}"));
                code = 1;
            }
        }
    }
    Ok(code)
}

fn kv(list: &[String]) -> Result<std::collections::BTreeMap<String, String>> {
    let mut m = std::collections::BTreeMap::new();
    for s in list {
        match s.split_once('=') {
            Some((k, v)) => {
                m.insert(k.to_string(), v.to_string());
            }
            // `--build-arg FOO` takes FOO from the environment, like Docker.
            None => {
                if let Ok(v) = std::env::var(s) {
                    m.insert(s.clone(), v);
                }
            }
        }
    }
    Ok(m)
}

pub async fn build(api: &dyn Api, b: BuildArgs) -> Result<i32> {
    let context = std::fs::canonicalize(&b.context).with_context(|| format!("build context {}", b.context.display()))?;
    if !context.is_dir() {
        bail!("build context {} is not a directory", context.display());
    }
    let dockerfile = b.file.as_ref().map(|f| if f.is_absolute() { f.clone() } else { std::env::current_dir().unwrap_or_default().join(f) });
    let opts = BuildOptions {
        context,
        dockerfile,
        tags: b.tag.clone(),
        build_args: kv(&b.build_arg)?,
        target: b.target.clone(),
        platform: b.platform.as_deref().map(Platform::parse).transpose()?,
        no_cache: b.no_cache,
        pull: b.pull,
        labels: kv(&b.label)?,
    };
    let started = std::time::Instant::now();
    let info = api.build_image(opts, output::progress_printer()).await?;
    eprintln!(
        "{} {} in {:.1}s",
        output::green("Built"),
        if b.tag.is_empty() { info.short_id().to_string() } else { b.tag.join(", ") },
        started.elapsed().as_secs_f64()
    );
    println!("{}", info.id);
    Ok(0)
}

pub async fn login(
    api: &dyn Api,
    registry: Option<String>,
    username: Option<String>,
    password: Option<String>,
    password_stdin: bool,
) -> Result<i32> {
    let registry = registry.unwrap_or_else(|| "docker.io".into());
    let username = match username {
        Some(u) => u,
        None => {
            eprint!("Username: ");
            let _ = std::io::stderr().flush();
            let mut s = String::new();
            std::io::stdin().read_line(&mut s)?;
            s.trim().to_string()
        }
    };
    let password = if password_stdin {
        let mut s = String::new();
        std::io::stdin().read_to_string(&mut s)?;
        s.trim_end_matches(['\n', '\r']).to_string()
    } else if let Some(p) = password {
        output::warn("passing a password on the command line is insecure; prefer --password-stdin");
        p
    } else {
        read_secret("Password: ")?
    };
    if username.is_empty() || password.is_empty() {
        bail!("username and password are required");
    }
    api.login(&registry, &username, &password).await?;
    eprintln!("{} to {registry}", output::green("Login succeeded"));
    Ok(0)
}

/// Read a line with echo off when stdin is a terminal.
fn read_secret(prompt: &str) -> Result<String> {
    eprint!("{prompt}");
    let _ = std::io::stderr().flush();
    let tty = std::io::stdin().is_terminal();
    // SAFETY: termios get/set on stdin; restored before returning.
    let mut old: libc::termios = unsafe { std::mem::zeroed() };
    if tty && unsafe { libc::tcgetattr(0, &mut old) } == 0 {
        let mut new = old;
        new.c_lflag &= !libc::ECHO;
        unsafe { libc::tcsetattr(0, libc::TCSANOW, &new) };
    }
    let mut s = String::new();
    let r = std::io::stdin().read_line(&mut s);
    if tty {
        unsafe { libc::tcsetattr(0, libc::TCSANOW, &old) };
        eprintln!();
    }
    r?;
    Ok(s.trim_end_matches(['\n', '\r']).to_string())
}
