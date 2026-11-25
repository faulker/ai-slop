//! info, prune, volumes, emulation.

use crate::cli::{EmulationCmd, Global, SystemCmd, VolumeCmd};
use crate::output::{self, ago, table};
use anyhow::{bail, Result};
use hullr_core::*;

pub async fn run(api: &dyn Api, s: SystemCmd) -> Result<i32> {
    match s {
        SystemCmd::Info => info(api).await,
        SystemCmd::Prune { all, volumes, force } => {
            if !force {
                let what = match (all, volumes) {
                    (true, true) => "all stopped containers, all unused images and all unused volumes",
                    (true, false) => "all stopped containers and all unused images",
                    (false, true) => "all stopped containers, dangling images and unused volumes",
                    (false, false) => "all stopped containers and dangling images",
                };
                if !output::confirm(&format!("This removes {what}. Continue?")) {
                    return Ok(1);
                }
            }
            let r = api.prune(all, volumes).await?;
            println!(
                "Removed {} containers, {} images, {} volumes. Reclaimed {}.",
                r.containers.len(),
                r.images.len(),
                r.volumes.len(),
                human_bytes(r.bytes_reclaimed)
            );
            Ok(0)
        }
        SystemCmd::Emulation(EmulationCmd::Status) => {
            let i = api.info().await?;
            println!("Native: {}", i.host_platform);
            if i.emulated_platforms.is_empty() {
                println!("Emulated: none");
                #[cfg(target_os = "linux")]
                println!("Enable with: sudo hullr system emulation enable (needs qemu-user-static installed)");
            } else {
                println!("Emulated: {}", i.emulated_platforms.join(", "));
            }
            Ok(0)
        }
        SystemCmd::Emulation(EmulationCmd::Enable) => unreachable!("handled before connecting"),
    }
}

pub async fn info(api: &dyn Api) -> Result<i32> {
    let i = api.info().await?;
    let b = output::bold;
    println!("{} {}", b("hullr"), i.version);
    println!("  Mode:        {}", i.mode);
    println!("  OS:          {} ({})", i.os, i.host_platform);
    println!("  Data root:   {}", i.data_root.display());
    println!("  Rootless:    {}", if i.rootless { "yes" } else { "no" });
    println!(
        "  Emulation:   {}",
        if i.emulated_platforms.is_empty() { "none".to_string() } else { i.emulated_platforms.join(", ") }
    );
    println!("  Containers:  {} ({} running)", i.containers, i.running);
    println!("  Images:      {}", i.images);
    for w in &i.warnings {
        output::warn(w);
    }
    Ok(0)
}

pub async fn volume(api: &dyn Api, v: VolumeCmd) -> Result<i32> {
    match v {
        VolumeCmd::Ls => {
            let list = api.list_volumes().await?;
            let rows: Vec<Vec<String>> = list.iter().map(|v| vec![v.name.clone(), ago(v.created)]).collect();
            table(&["VOLUME NAME", "CREATED"], &rows);
            Ok(0)
        }
        VolumeCmd::Create { name } => {
            println!("{}", api.create_volume(&name).await?.name);
            Ok(0)
        }
        VolumeCmd::Rm { force, names } => {
            let mut code = 0;
            for n in &names {
                match api.remove_volume(n, force).await {
                    Ok(()) => println!("{n}"),
                    Err(e) => {
                        output::error(&format!("{n}: {e}"));
                        code = 1;
                    }
                }
            }
            Ok(code)
        }
        VolumeCmd::Inspect { name } => {
            let list = api.list_volumes().await?;
            let Some(v) = list.into_iter().find(|v| v.name == name) else { bail!("no such volume: {name}") };
            println!("{}", serde_json::to_string_pretty(&v)?);
            Ok(0)
        }
    }
}

pub async fn emulation_enable(_g: &Global) -> Result<i32> {
    #[cfg(target_os = "linux")]
    {
        let arches = hullr_runtime::enable_emulation()?;
        if arches.is_empty() {
            output::warn("no qemu-*-static binaries found. Install qemu-user-static (e.g. `apt install qemu-user-static`) and retry.");
            return Ok(1);
        }
        println!("Emulation enabled for: {}", arches.join(", "));
        println!("Try: hullr run --rm --platform linux/arm64 alpine uname -m");
        Ok(0)
    }
    #[cfg(not(target_os = "linux"))]
    {
        println!("On macOS the hullr VM handles emulation itself (Rosetta for amd64, qemu for others).");
        Ok(0)
    }
}
