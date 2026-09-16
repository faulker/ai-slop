mod config;
mod dump;
mod error;
mod humble;
mod matching;
mod report;
mod steam;
mod wizard;

use clap::{Parser, ValueEnum};
use config::StoredConfig;
use error::{AppError, Result};
use std::path::PathBuf;

/// Command-line arguments. Credential flags mirror their env vars so a value
/// can live in the shell profile instead of the command line; `hide_env_values`
/// keeps `--help` from ever echoing one back.
#[derive(Parser, Debug)]
#[command(
    name = "humble-game-check",
    about = "List unrevealed Humble Bundle Steam keys for games not in your Steam library"
)]
struct Cli {
    /// The _simpleauth_sess session cookie from a logged-in humblebundle.com browser tab.
    #[arg(long, env = "HUMBLE_SESSION", hide_env_values = true)]
    humble_session: Option<String>,

    /// A Steam Web API key, from https://steamcommunity.com/dev/apikey.
    #[arg(long, env = "STEAM_API_KEY", hide_env_values = true)]
    steam_key: Option<String>,

    /// The account's SteamID64. Alternative to --steam-vanity.
    #[arg(long, env = "STEAM_ID")]
    steam_id: Option<u64>,

    /// The account's custom profile name (steamcommunity.com/id/<name>), resolved to a SteamID64.
    #[arg(long, env = "STEAM_VANITY")]
    steam_vanity: Option<String>,

    /// Output format.
    #[arg(long, value_enum, default_value_t = FormatArg::Text)]
    format: FormatArg,

    /// Also list keys already owned on Steam.
    #[arg(long)]
    show_owned: bool,

    /// Save the raw Humble/Steam API responses here for later offline runs.
    #[arg(long)]
    dump_dir: Option<PathBuf>,

    /// Skip the network entirely and classify from a directory saved by --dump-dir.
    #[arg(long)]
    from_dump: Option<PathBuf>,

    /// Override the Humble Bundle base URL (testing).
    #[arg(long, hide = true)]
    humble_base: Option<String>,

    /// Override the Steam Web API base URL (testing).
    #[arg(long, hide = true)]
    steam_base: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum FormatArg {
    /// Plain-text report.
    Text,
    /// Structured JSON.
    Json,
}

/// Returns `value`, or a `MissingCredential` naming the env var, the flag
/// that overrides it, and a hint for where to find the value.
fn require(value: Option<String>, name: &str, flag: &str, hint: &str) -> Result<String> {
    value.ok_or_else(|| AppError::MissingCredential {
        name: name.into(),
        flag: flag.into(),
        hint: hint.into(),
    })
}

/// Resolves the Steam account id to query: an explicit `--steam-id` first,
/// else a `--steam-vanity` name resolved through the Steam API, else
/// `MissingSteamId`.
fn resolve_steam_id(cli: &Cli, steam_key: &str, steam_base: &str) -> Result<u64> {
    if let Some(id) = cli.steam_id {
        return Ok(id);
    }
    if let Some(vanity) = &cli.steam_vanity {
        return steam::resolve_vanity(steam_base, steam_key, vanity);
    }
    Err(AppError::MissingSteamId)
}

/// Pulls flags/env into a config bag, fills gaps from the saved file, and
/// if anything is still missing, runs the setup wizard when stdin is a TTY
/// (or `HUMBLE_GAME_CHECK_WIZARD=1`). Writes collected values back onto `cli`.
fn resolve_credentials(cli: &mut Cli) -> Result<()> {
    let mut creds = StoredConfig {
        humble_session: cli.humble_session.take(),
        steam_key: cli.steam_key.take(),
        steam_id: cli.steam_id.take(),
        steam_vanity: cli.steam_vanity.take(),
    };
    creds.normalize_blanks();

    let config_path = config::path();
    let stored = match &config_path {
        Some(path) => config::load(path)?,
        None => StoredConfig::default(),
    };
    stored.fill_into(&mut creds);

    if creds.has_missing() && wizard::should_run() {
        let missing = wizard::Missing::from_config(&creds);
        let stdin = std::io::stdin();
        let mut input = stdin.lock();
        let mut stderr = std::io::stderr();
        let (entered, save) = wizard::run(&mut input, &mut stderr, missing)?;
        entered.fill_into(&mut creds);
        if save {
            match &config_path {
                Some(path) => {
                    config::save(path, &stored.merge_entered(&entered))?;
                    eprintln!("Saved credentials to {}", path.display());
                }
                None => {
                    eprintln!(
                        "Could not determine a config directory; credentials were not saved."
                    );
                }
            }
        }
    }

    cli.humble_session = creds.humble_session;
    cli.steam_key = creds.steam_key;
    cli.steam_id = creds.steam_id;
    cli.steam_vanity = creds.steam_vanity;
    Ok(())
}

/// Runs the full fetch/classify/render pipeline (or loads a saved dump
/// instead of hitting the network) and returns the rendered report.
fn run(mut cli: Cli) -> Result<String> {
    let (raw_orders, raw_steam, steamid_for_empty) = if let Some(dump_dir) = &cli.from_dump {
        let (orders, steam) = dump::load_dump(dump_dir)?;
        (orders, steam, 0)
    } else {
        resolve_credentials(&mut cli)?;
        let humble_session = require(
            cli.humble_session.clone(),
            "HUMBLE_SESSION",
            "humble-session",
            "the _simpleauth_sess cookie from humblebundle.com",
        )?;
        let steam_key = require(
            cli.steam_key.clone(),
            "STEAM_API_KEY",
            "steam-key",
            "from https://steamcommunity.com/dev/apikey",
        )?;

        let steam_base = cli.steam_base.as_deref().unwrap_or(steam::BASE);
        let steamid = resolve_steam_id(&cli, &steam_key, steam_base)?;

        let humble_base = cli.humble_base.as_deref().unwrap_or(humble::BASE);
        let client = humble::client(&humble_session)?;
        let order_keys = humble::fetch_order_keys(&client, humble_base)?;
        let raw_orders = humble::fetch_orders(&client, humble_base, &order_keys)?;
        let raw_steam = steam::fetch_owned_games(steam_base, &steam_key, steamid)?;

        if let Some(dump_dir) = &cli.dump_dir {
            dump::save_dump(dump_dir, &raw_orders, &raw_steam)?;
        }

        (raw_orders, raw_steam, steamid)
    };

    let orders = humble::parse_orders(&raw_orders)?;
    let keys = humble::unused_steam_keys(&orders);

    let games = steam::parse_owned_games(&raw_steam, steamid_for_empty)?;
    let index = steam::SteamIndex::build(&games);

    let rows: Vec<report::Row> = keys
        .into_iter()
        .map(|key| {
            let m = matching::classify(&key, &index);
            report::Row { key, m }
        })
        .collect();

    match cli.format {
        FormatArg::Text => Ok(report::render_text(&rows, cli.show_owned)),
        FormatArg::Json => report::render_json(&rows, cli.show_owned),
    }
}

fn main() {
    match run(Cli::parse()) {
        Ok(output) => println!("{output}"),
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}
