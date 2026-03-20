use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("keychain error: {msg}")]
    Keychain { msg: String },

    #[error("json parse error: {msg}")]
    JsonParse { msg: String },

    #[error("http error: {msg}")]
    Http { msg: String },
}

impl From<reqwest::Error> for AppError {
    fn from(e: reqwest::Error) -> Self {
        AppError::Http {
            msg: e.to_string(),
        }
    }
}

impl From<serde_json::Error> for AppError {
    fn from(e: serde_json::Error) -> Self {
        AppError::JsonParse {
            msg: e.to_string(),
        }
    }
}

pub type Result<T> = std::result::Result<T, AppError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_keychain_error() {
        let e = AppError::Keychain {
            msg: "not found".into(),
        };
        assert!(e.to_string().contains("keychain"));
        assert!(e.to_string().contains("not found"));
    }

    #[test]
    fn display_json_parse_error() {
        let e = AppError::JsonParse {
            msg: "unexpected token".into(),
        };
        assert!(e.to_string().contains("json parse"));
        assert!(e.to_string().contains("unexpected token"));
    }

    #[test]
    fn display_http_error() {
        let e = AppError::Http {
            msg: "connection refused".into(),
        };
        assert!(e.to_string().contains("http error"));
        assert!(e.to_string().contains("connection refused"));
    }

    #[test]
    fn from_reqwest_error() {
        let err = reqwest::blocking::get("http://[::invalid]").unwrap_err();
        let app_err: AppError = err.into();
        assert!(matches!(app_err, AppError::Http { .. }));
    }

    #[test]
    fn from_serde_json_error() {
        let err: serde_json::Error = serde_json::from_str::<serde_json::Value>("not json").unwrap_err();
        let app_err: AppError = err.into();
        assert!(matches!(app_err, AppError::JsonParse { .. }));
    }
}
