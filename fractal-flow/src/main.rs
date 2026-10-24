//! fractal-flow entry point: parse args, load settings, then either render a
//! snapshot or run the windowed visualizer.

use clap::Parser;
use fractal_flow::settings::{Cli, Settings};

/// Program entry.
fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("fractal_flow=info,warn")).init();
    let cli = Cli::parse();
    let config_path = cli.config.clone().or_else(Settings::default_path);
    let (mut settings, warning) = match &config_path {
        Some(p) => Settings::load(p),
        None => (Settings::default(), Some("No config directory available; settings will not be saved.".to_string())),
    };
    if let Some(w) = &warning {
        log::warn!("{w}");
    }
    settings.apply_cli(&cli);

    if let Some(path) = &cli.snapshot {
        let seed = settings.scenes.seed;
        fractal_flow::snapshot::render_png(&settings, cli.snapshot_scene, seed, cli.snapshot_time, cli.snapshot_size, path)?;
        println!("wrote {}", path.display());
        return Ok(());
    }
    fractal_flow::app::run(settings, config_path, warning)
}
