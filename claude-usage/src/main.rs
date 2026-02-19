mod cookies;
mod crypto;
mod error;
mod keychain;

use clap::Parser;
use cookies::{default_db_path, extract_claude_cookies, Cookie};
use error::AppError;
use keychain::get_brave_password;

#[derive(Parser, Debug)]
#[command(name = "claude-usage", about = "Fetch Claude.ai usage data using Brave browser cookies")]
struct Cli {
    /// Custom Cookies database path
    #[arg(short, long)]
    db: Option<std::path::PathBuf>,
}

fn run() -> error::Result<()> {
    let cli = Cli::parse();

    let db_path = cli.db.unwrap_or_else(default_db_path);
    let password = get_brave_password()?;
    let cookies = extract_claude_cookies(&db_path, &password)?;

    if !cookies.iter().any(|c| c.name == "sessionKey") {
        return Err(AppError::NoCookies);
    }

    let last_active_org = cookies
        .iter()
        .find(|c| c.name == "lastActiveOrg")
        .ok_or(AppError::NoCookies)?;

    let url = format!(
        "https://claude.ai/api/organizations/{}/usage",
        last_active_org.value
    );

    // Send all claude.ai cookies so Cloudflare cf_clearance is included
    let cookie_header = build_cookie_header(&cookies);

    let client = reqwest::blocking::Client::new();
    let resp = client
        .get(&url)
        .header("Cookie", cookie_header)
        .header(
            "User-Agent",
            "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Brave/131 Chrome/131.0.0.0 Safari/537.36",
        )
        .header("Accept", "application/json")
        .send()?;

    let body = resp.text()?;
    println!("{body}");

    Ok(())
}

fn build_cookie_header(cookies: &[Cookie]) -> String {
    cookies
        .iter()
        .map(|c| format!("{}={}", c.name, c.value))
        .collect::<Vec<_>>()
        .join("; ")
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

    fn cookie(name: &str, value: &str) -> Cookie {
        Cookie {
            name: name.to_string(),
            value: value.to_string(),
        }
    }

    #[test]
    fn build_cookie_header_joins_all_cookies() {
        let cookies = vec![
            cookie("sessionKey", "sk-ant-123"),
            cookie("cf_clearance", "abc"),
            cookie("lastActiveOrg", "org-uuid"),
        ];
        let header = build_cookie_header(&cookies);
        assert_eq!(
            header,
            "sessionKey=sk-ant-123; cf_clearance=abc; lastActiveOrg=org-uuid"
        );
    }

    #[test]
    fn build_cookie_header_single_cookie() {
        let cookies = vec![cookie("sessionKey", "val")];
        assert_eq!(build_cookie_header(&cookies), "sessionKey=val");
    }

    #[test]
    fn build_cookie_header_empty() {
        let cookies: Vec<Cookie> = vec![];
        assert_eq!(build_cookie_header(&cookies), "");
    }
}
