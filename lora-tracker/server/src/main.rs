//! `lora-tracker-server`: receives sealed reports from boards over HTTP,
//! stores them in SQLite, and serves a live map. See `server.example.toml`.

mod bootstrap;
mod config;
mod db;
mod live;
mod routes;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use clap::Parser;

/// Command-line options.
#[derive(Parser)]
#[command(version, about = "lora-tracker server: report ingest, storage, and map")]
struct Args {
    /// Path to server.toml. Created with defaults (and keys from tracker/gateway
    /// configs in the same folder) if it doesn't exist.
    #[arg(short, long, default_value = "server.toml")]
    config: PathBuf,
}

/// Loads (or first creates) the config, opens the database, and serves until Ctrl-C.
#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    if !args.config.exists() {
        bootstrap::write_default(&args.config)?;
    }
    let cfg = config::load(&args.config)?;
    let conn = db::open(&cfg.database)?;

    if cfg.devices.is_empty() {
        eprintln!("warning: no [[device]] entries; every report will be rejected (run `lora-tracker pair`)");
    }

    let listener = tokio::net::TcpListener::bind(cfg.listen)
        .await
        .with_context(|| format!("binding {}", cfg.listen))?;
    println!(
        "listening on http://{} ({} device(s), database {})",
        cfg.listen,
        cfg.devices.len(),
        cfg.database.display()
    );

    println!("live log: http://{}/log (board activity is also printed below)", cfg.listen);

    let state = routes::AppState { cfg: Arc::new(cfg), db: Arc::new(Mutex::new(conn)), live: Arc::new(live::Live::new(true)) };
    axum::serve(listener, routes::router(state).into_make_service_with_connect_info::<SocketAddr>())
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await
        .context("server error")
}
