mod api;
mod error;
mod keychain;
mod local;
mod usage;

use clap::{Parser, ValueEnum};
use error::AppError;
use keychain::get_claude_code_token;
use std::path::PathBuf;
use usage::{format_age, Usage};

#[derive(Parser, Debug)]
#[command(
    name = "claude-usage",
    about = "Fetch Claude Code usage data from the local history file or the OAuth API"
)]
struct Cli {
    /// Where to read usage from. "local" never touches the Keychain.
    #[arg(long, value_enum, default_value_t = SourceArg::Auto)]
    source: SourceArg,

    /// How old a local sample may be, in minutes, before it counts as stale.
    #[arg(long, default_value_t = 30)]
    max_age: u64,

    /// Output format.
    #[arg(long, value_enum, default_value_t = FormatArg::Text)]
    format: FormatArg,

    /// Override the local history file path (testing).
    #[arg(long, hide = true)]
    history_file: Option<PathBuf>,

    /// Override the API base URL (testing).
    #[arg(long, hide = true)]
    api_base: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum SourceArg {
    /// Local file when fresh, otherwise the API.
    Auto,
    /// Claude Desktop's history file only.
    Local,
    /// The OAuth API only.
    Api,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum FormatArg {
    /// Human-readable summary.
    Text,
    /// Normalized JSON, the contract other apps read.
    Json,
    /// The API response body, unmodified.
    Raw,
}

/// What a source resolved to, plus anything the user should know about it.
struct Outcome {
    usage: Usage,
    /// Present only when the API was called, for `--format raw`.
    raw_body: Option<String>,
    warnings: Vec<String>,
}

/// Whether a sample of the given age still counts as fresh.
///
/// Lives at the crate root because both the local source and the CLI need the
/// same answer. `max_age_minutes == 0` makes everything stale, which forces the
/// API to be tried first under `--source auto`.
pub fn is_fresh(age_seconds: i64, max_age_minutes: u64) -> bool {
    age_seconds <= (max_age_minutes as i64) * 60
}

/// Rejects flag combinations that cannot produce output.
fn validate(cli: &Cli) -> error::Result<()> {
    if cli.format == FormatArg::Raw && cli.source == SourceArg::Local {
        return Err(AppError::BadArgs {
            msg: "--format raw returns the API response body, so it cannot be used with --source local".into(),
        });
    }
    Ok(())
}

/// Reads the token, fetches, and normalizes. The token is read first so a
/// Keychain failure becomes an ordinary "the API failed" cause that `--source
/// auto` can fall back from.
fn load_api_usage(cli: &Cli, now_ms: i64) -> error::Result<Outcome> {
    let token = get_claude_code_token()?;
    let base = cli.api_base.as_deref().unwrap_or(api::DEFAULT_API_BASE);
    let body = api::fetch_usage_body(base, &token)?;
    let mut usage = api::parse_api_usage(&body, now_ms)?;

    // The API's own resets_at is authoritative and stays untouched; this just
    // best-effort fills the derived estimate too, so resets_at_estimated_ms
    // isn't only ever populated on the local source. A missing or unreadable
    // history file just leaves it None, same as if local were never checked.
    if let Some(mut five_hour) = usage.five_hour.take() {
        let history_path = local::history_path(cli.history_file.as_deref());
        five_hour.resets_at_estimated_ms =
            local::try_estimate_five_hour_reset(&history_path, now_ms);
        usage.five_hour = Some(five_hour);
    }

    Ok(Outcome {
        usage,
        raw_body: Some(body),
        warnings: Vec::new(),
    })
}

/// Reads the local history file and normalizes it.
fn load_local(cli: &Cli, now_ms: i64) -> error::Result<Usage> {
    let path = local::history_path(cli.history_file.as_deref());
    local::load_local_usage(&path, now_ms, cli.max_age)
}

/// Warning shown whenever stale local data is printed.
fn stale_warning(usage: &Usage) -> String {
    format!(
        "local usage data is {} old; Claude Desktop may not be running",
        format_age(usage.age_seconds)
    )
}

/// Resolves the usage to print, applying the source-selection and fallback
/// rules. `--format raw` needs an API body, so it upgrades `auto` to `api`.
fn select(cli: &Cli, now_ms: i64) -> error::Result<Outcome> {
    let effective_source = if cli.format == FormatArg::Raw {
        SourceArg::Api
    } else {
        cli.source
    };

    match effective_source {
        SourceArg::Api => load_api_usage(cli, now_ms),

        SourceArg::Local => {
            let usage = load_local(cli, now_ms)?;
            // Staleness is never fatal: printing old numbers with a warning beats
            // failing a caller that only wants a prompt-free read.
            let warnings = if usage.stale {
                vec![stale_warning(&usage)]
            } else {
                Vec::new()
            };
            Ok(Outcome {
                usage,
                raw_body: None,
                warnings,
            })
        }

        SourceArg::Auto => {
            let local_result = load_local(cli, now_ms);

            // Fresh local data is the whole point: no Keychain, no network.
            if let Ok(usage) = &local_result {
                if !usage.stale {
                    return Ok(Outcome {
                        usage: usage.clone(),
                        raw_body: None,
                        warnings: Vec::new(),
                    });
                }
            }

            match (load_api_usage(cli, now_ms), local_result) {
                (Ok(mut outcome), local_result) => {
                    if let Err(local_err) = local_result {
                        outcome.warnings.push(format!(
                            "local usage data unavailable ({local_err}); used the API instead"
                        ));
                    }
                    Ok(outcome)
                }
                (Err(api_err), Ok(usage)) => {
                    let warnings = vec![
                        stale_warning(&usage),
                        format!("the API was unreachable ({api_err}); showing local data"),
                    ];
                    Ok(Outcome {
                        usage,
                        raw_body: None,
                        warnings,
                    })
                }
                (Err(api_err), Err(local_err)) => Err(AppError::NoUsageData {
                    local: local_err.to_string(),
                    api: api_err.to_string(),
                }),
            }
        }
    }
}

/// Prints the resolved usage in the requested format.
fn print_output(cli: &Cli, outcome: &Outcome) -> error::Result<()> {
    match cli.format {
        FormatArg::Text => println!("{}", usage::render_text(&outcome.usage)),
        FormatArg::Json => println!("{}", serde_json::to_string(&outcome.usage)?),
        FormatArg::Raw => {
            let body = outcome.raw_body.as_ref().ok_or_else(|| AppError::BadArgs {
                msg: "--format raw needs an API response, but the data came from the local file"
                    .into(),
            })?;
            println!("{body}");
        }
    }
    Ok(())
}

fn run() -> error::Result<()> {
    let cli = Cli::parse();
    validate(&cli)?;

    let outcome = select(&cli, usage::now_ms())?;
    for warning in &outcome.warnings {
        eprintln!("warning: {warning}");
    }
    print_output(&cli, &outcome)
}

fn main() {
    if let Err(e) = run() {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_fresh_at_the_threshold_boundary() {
        assert!(is_fresh(30 * 60, 30), "exactly at the threshold is fresh");
        assert!(is_fresh(30 * 60 - 1, 30));
        assert!(!is_fresh(30 * 60 + 1, 30));
    }

    #[test]
    fn is_fresh_with_zero_max_age_only_accepts_now() {
        assert!(is_fresh(0, 0));
        assert!(!is_fresh(1, 0), "zero max-age forces the API to be tried");
    }

    #[test]
    fn is_fresh_tolerates_negative_ages() {
        assert!(is_fresh(-60, 30), "a clock skew must not read as stale");
    }
}
