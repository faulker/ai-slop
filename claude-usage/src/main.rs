mod error;
mod keychain;

use clap::Parser;
use error::AppError;
use keychain::get_claude_code_token;

#[derive(Parser, Debug)]
#[command(name = "claude-usage", about = "Fetch Claude Code usage data via OAuth")]
struct Cli {}

fn run() -> error::Result<()> {
    let _cli = Cli::parse();

    let token = get_claude_code_token()?;

    let client = reqwest::blocking::Client::new();
    let resp = client
        .get("https://api.anthropic.com/api/oauth/usage")
        .header("Authorization", format!("Bearer {token}"))
        .header("anthropic-beta", "oauth-2025-04-20")
        .header("Content-Type", "application/json")
        .header("User-Agent", "claude-code/2.1.62")
        .send()?;

    let status = resp.status();
    let body = resp.text()?;

    if !status.is_success() {
        return Err(AppError::Http {
            msg: format!("API returned {status}: {body}"),
        });
    }

    // Validate it's valid JSON before printing
    let _: serde_json::Value = serde_json::from_str(&body)?;
    println!("{body}");

    Ok(())
}

fn main() {
    if let Err(e) = run() {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}
