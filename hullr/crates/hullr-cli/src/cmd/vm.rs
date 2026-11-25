//! `hullr vm ...` (macOS).

use crate::cli::VmCmd;
use anyhow::Result;
use hullr_core::paths::Paths;

pub async fn run(c: VmCmd) -> Result<i32> {
    let paths = Paths::from_env();
    match c {
        VmCmd::Start => {
            let sock = hullr_vm::ensure_running(&paths, &crate::connect::exe()).await?;
            println!("VM running; API at {}", sock.display());
        }
        VmCmd::Stop => {
            hullr_vm::stop(&paths).await?;
            println!("VM stopped");
        }
        VmCmd::Status => match hullr_vm::status(&paths) {
            hullr_vm::VmStatus::Stopped => println!("stopped"),
            hullr_vm::VmStatus::Starting => println!("starting"),
            hullr_vm::VmStatus::Running { pid } => println!("running (pid {pid})"),
        },
        VmCmd::Config { cpus, memory, disk, rosetta } => {
            let mut cfg = hullr_vm::load_config(&paths);
            let changed = cpus.is_some() || memory.is_some() || disk.is_some() || rosetta.is_some();
            if let Some(v) = cpus {
                cfg.cpus = v;
            }
            if let Some(v) = memory {
                cfg.memory_mib = v;
            }
            if let Some(v) = disk {
                cfg.disk_gib = v;
            }
            if let Some(v) = rosetta {
                cfg.rosetta = v;
            }
            if changed {
                hullr_vm::save_config(&paths, &cfg)?;
                eprintln!("Saved. Restart the VM to apply: hullr vm stop && hullr vm start");
            }
            println!("{}", serde_json::to_string_pretty(&cfg)?);
        }
    }
    Ok(0)
}
