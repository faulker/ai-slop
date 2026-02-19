use crate::crypto::{decrypt_cookie, derive_key};
use crate::error::{AppError, Result};
use rusqlite::Connection;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct Cookie {
    pub name: String,
    pub value: String,
}

pub fn default_db_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/Users".into());
    PathBuf::from(home)
        .join("Library/Application Support/BraveSoftware/Brave-Browser/Default/Cookies")
}

pub fn extract_claude_cookies(db_path: &std::path::Path, password: &str) -> Result<Vec<Cookie>> {
    if !db_path.exists() {
        return Err(AppError::DbNotFound {
            path: db_path.display().to_string(),
        });
    }

    // Copy the DB to a temp file to avoid lock contention with Brave
    let tmp = tempfile::NamedTempFile::new()?;
    std::fs::copy(db_path, tmp.path())?;

    // Also copy the WAL and SHM files if they exist, for consistency
    let db_dir = db_path.parent().unwrap_or(std::path::Path::new("."));
    let db_name = db_path.file_name().unwrap().to_str().unwrap_or("Cookies");
    let tmp_dir = tmp.path().parent().unwrap();
    let tmp_name = tmp.path().file_name().unwrap().to_str().unwrap();

    for ext in &["-wal", "-shm"] {
        let src = db_dir.join(format!("{db_name}{ext}"));
        if src.exists() {
            let dst = tmp_dir.join(format!("{tmp_name}{ext}"));
            let _ = std::fs::copy(&src, &dst);
        }
    }

    let conn = Connection::open(tmp.path())?;

    // Get DB version (meta table stores value as TEXT, so read and parse)
    let db_version: i32 = conn
        .query_row("SELECT value FROM meta WHERE key = 'version'", [], |row| {
            let val: String = row.get(0)?;
            val.parse::<i32>().map_err(|e| {
                rusqlite::Error::FromSqlConversionFailure(
                    0,
                    rusqlite::types::Type::Text,
                    Box::new(e),
                )
            })
        })
        .unwrap_or(0);

    let key = derive_key(password);

    let mut stmt = conn.prepare(
        "SELECT host_key, name, encrypted_value FROM cookies
         WHERE host_key IN ('.claude.ai', 'claude.ai')
         ORDER BY name",
    )?;

    let cookies: Vec<Cookie> = stmt
        .query_map([], |row| {
            let host_key: String = row.get(0)?;
            let name: String = row.get(1)?;
            let encrypted_value: Vec<u8> = row.get(2)?;
            Ok((host_key, name, encrypted_value))
        })?
        .filter_map(|row| {
            let (host_key, name, encrypted_value) = row.ok()?;
            if encrypted_value.is_empty() {
                return None;
            }
            match decrypt_cookie(&encrypted_value, &host_key, db_version, &key) {
                Ok(value) if !value.is_empty() => Some(Cookie {
                    name,
                    value,
                }),
                Ok(_) => None,
                Err(e) => {
                    eprintln!("warning: failed to decrypt cookie '{}': {}", name, e);
                    None
                }
            }
        })
        .collect();

    if cookies.is_empty() {
        return Err(AppError::NoCookies);
    }

    Ok(cookies)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_path_contains_brave() {
        let p = default_db_path();
        assert!(
            p.to_str().unwrap().contains("BraveSoftware"),
            "default path should reference BraveSoftware"
        );
    }

    #[test]
    fn missing_db_returns_not_found() {
        let result = extract_claude_cookies(
            std::path::Path::new("/nonexistent/path/Cookies"),
            "password",
        );
        assert!(result.is_err());
        match result.unwrap_err() {
            AppError::DbNotFound { path } => {
                assert!(path.contains("nonexistent"));
            }
            other => panic!("expected DbNotFound, got: {other}"),
        }
    }
}
