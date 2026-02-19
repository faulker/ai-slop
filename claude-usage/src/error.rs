use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("keychain error: {msg}")]
    Keychain { msg: String },

    #[error("sqlite error: {msg}")]
    Sqlite { msg: String },

    #[error("decryption error: {msg}")]
    Decrypt { msg: String },

    #[error("io error: {msg}")]
    Io { msg: String },

    #[error("cookies database not found at: {path}")]
    DbNotFound { path: String },

    #[error("no claude.ai cookies found")]
    NoCookies,

    #[error("http error: {msg}")]
    Http { msg: String },
}

impl From<rusqlite::Error> for AppError {
    fn from(e: rusqlite::Error) -> Self {
        AppError::Sqlite {
            msg: e.to_string(),
        }
    }
}

impl From<reqwest::Error> for AppError {
    fn from(e: reqwest::Error) -> Self {
        AppError::Http {
            msg: e.to_string(),
        }
    }
}

impl From<std::io::Error> for AppError {
    fn from(e: std::io::Error) -> Self {
        AppError::Io {
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
    fn display_db_not_found() {
        let e = AppError::DbNotFound {
            path: "/some/path".into(),
        };
        assert!(e.to_string().contains("/some/path"));
    }

    #[test]
    fn from_io_error() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "gone");
        let app_err: AppError = io_err.into();
        assert!(matches!(app_err, AppError::Io { .. }));
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
        // Build an invalid URL to produce a reqwest::Error
        let err = reqwest::blocking::get("http://[::invalid]").unwrap_err();
        let app_err: AppError = err.into();
        assert!(matches!(app_err, AppError::Http { .. }));
    }

    #[test]
    fn from_rusqlite_error() {
        let sql_err = rusqlite::Error::QueryReturnedNoRows;
        let app_err: AppError = sql_err.into();
        assert!(matches!(app_err, AppError::Sqlite { .. }));
    }
}
