//! Choosing how to reach the engine: daemon socket, the macOS VM, or an
//! in-process engine (Linux, no daemon needed).

use crate::cli::Global;
use anyhow::{Context, Result};
use hullr_core::paths::Paths;
use hullr_core::Api;
use std::sync::Arc;

pub fn exe() -> std::path::PathBuf {
    std::env::current_exe().unwrap_or_else(|_| "hullr".into())
}

pub async fn connect(g: &Global) -> Result<Arc<dyn Api>> {
    let paths = Paths::from_env();
    let sock = g.socket.clone().unwrap_or_else(|| paths.socket());
    if !g.no_daemon {
        let c = hullr_api::Client::new(&sock);
        if c.ping().await {
            return Ok(Arc::new(c));
        }
    }
    local(paths, g).await
}

#[cfg(target_os = "macos")]
async fn local(paths: Paths, _g: &Global) -> Result<Arc<dyn Api>> {
    // No Linux kernel here: containers live in the hullr VM, started on demand.
    let sock = hullr_vm::ensure_running(&paths, &exe())
        .await
        .context("starting the hullr VM (see `hullr vm status`)")?;
    Ok(Arc::new(hullr_api::Client::new(sock)))
}

#[cfg(not(target_os = "macos"))]
async fn local(paths: Paths, _g: &Global) -> Result<Arc<dyn Api>> {
    let engine = hullr_engine::Engine::open(paths, exe(), "in-process").context("opening the hullr data root")?;
    Ok(engine)
}
