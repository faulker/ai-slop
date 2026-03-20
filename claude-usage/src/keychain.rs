use crate::error::{AppError, Result};
use security_framework::passwords::get_generic_password;

pub fn get_claude_code_token() -> Result<String> {
    let username = std::env::var("USER").unwrap_or_else(|_| "unknown".into());
    let cred_bytes = get_generic_password("Claude Code-credentials", &username)
        .map_err(|e| AppError::Keychain {
            msg: e.to_string(),
        })?;

    let cred_str = String::from_utf8(cred_bytes.to_vec()).map_err(|e| AppError::Keychain {
        msg: format!("credentials not valid UTF-8: {e}"),
    })?;

    let cred: serde_json::Value = serde_json::from_str(&cred_str).map_err(|e| AppError::JsonParse {
        msg: format!("failed to parse keychain JSON: {e}"),
    })?;

    cred["claudeAiOauth"]["accessToken"]
        .as_str()
        .map(|s| s.to_string())
        .ok_or_else(|| AppError::JsonParse {
            msg: "missing claudeAiOauth.accessToken in keychain credentials".into(),
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore] // requires Claude Code logged in + Keychain access
    fn keychain_returns_token() {
        let token = get_claude_code_token().expect("should read keychain");
        assert!(!token.is_empty(), "token should not be empty");
        assert!(token.starts_with("sk-ant-oat"), "token should be an OAuth token");
    }
}
