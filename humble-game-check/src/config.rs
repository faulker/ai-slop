//! Saved credentials: a JSON file that fills in missing flags/env vars on
//! later runs. Flags and environment variables still win over anything stored.

use crate::error::{AppError, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Humble session, Steam API key, and Steam account identity. Used both as
/// the on-disk config shape and as the in-memory bag the wizard fills.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub humble_session: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub steam_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub steam_id: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub steam_vanity: Option<String>,
}

impl StoredConfig {
    /// Turns empty strings into `None` so `HUMBLE_SESSION=` is treated as unset.
    pub fn normalize_blanks(&mut self) {
        if is_blank(&self.humble_session) {
            self.humble_session = None;
        }
        if is_blank(&self.steam_key) {
            self.steam_key = None;
        }
        if is_blank(&self.steam_vanity) {
            self.steam_vanity = None;
        }
    }

    /// Copies stored values into empty fields of `target`. Steam account
    /// fields are only copied when `target` has neither a steam id nor a
    /// vanity name, so `STEAM_VANITY` is not overridden by a stored steam id.
    pub fn fill_into(&self, target: &mut StoredConfig) {
        if target.humble_session.is_none() {
            target.humble_session = self.humble_session.clone();
        }
        if target.steam_key.is_none() {
            target.steam_key = self.steam_key.clone();
        }
        if target.steam_id.is_none() && target.steam_vanity.is_none() {
            target.steam_id = self.steam_id;
            target.steam_vanity = self.steam_vanity.clone();
        }
    }

    /// Overlay newly entered wizard values onto an existing stored config.
    pub fn merge_entered(&self, entered: &StoredConfig) -> StoredConfig {
        StoredConfig {
            humble_session: entered
                .humble_session
                .clone()
                .or_else(|| self.humble_session.clone()),
            steam_key: entered.steam_key.clone().or_else(|| self.steam_key.clone()),
            steam_id: entered.steam_id.or(self.steam_id),
            steam_vanity: entered
                .steam_vanity
                .clone()
                .or_else(|| self.steam_vanity.clone()),
        }
    }

    pub fn missing_humble_session(&self) -> bool {
        self.humble_session.is_none()
    }

    pub fn missing_steam_key(&self) -> bool {
        self.steam_key.is_none()
    }

    pub fn missing_steam_account(&self) -> bool {
        self.steam_id.is_none() && self.steam_vanity.is_none()
    }

    pub fn has_missing(&self) -> bool {
        self.missing_humble_session() || self.missing_steam_key() || self.missing_steam_account()
    }
}

fn is_blank(value: &Option<String>) -> bool {
    value.as_ref().map(|s| s.trim().is_empty()).unwrap_or(true)
}

/// Resolves the config path from an env override or the platform config dir.
/// Empty env values are ignored so `HUMBLE_GAME_CHECK_CONFIG=` falls through.
pub fn path_from(env: Option<&str>, config_dir: Option<PathBuf>) -> Option<PathBuf> {
    if let Some(p) = env {
        if !p.is_empty() {
            return Some(PathBuf::from(p));
        }
    }
    let mut dir = config_dir?;
    dir.push("humble-game-check");
    dir.push("config.json");
    Some(dir)
}

/// Config file path: `HUMBLE_GAME_CHECK_CONFIG` if set, else
/// `$config_dir/humble-game-check/config.json`.
pub fn path() -> Option<PathBuf> {
    path_from(
        std::env::var("HUMBLE_GAME_CHECK_CONFIG").ok().as_deref(),
        dirs::config_dir(),
    )
}

/// Reads a stored config, or returns defaults if the file does not exist.
pub fn load(path: &Path) -> Result<StoredConfig> {
    if !path.exists() {
        return Ok(StoredConfig::default());
    }
    let text = std::fs::read_to_string(path).map_err(|e| AppError::Config {
        path: path.display().to_string(),
        msg: e.to_string(),
    })?;
    let mut cfg: StoredConfig = serde_json::from_str(&text).map_err(|e| AppError::Config {
        path: path.display().to_string(),
        msg: e.to_string(),
    })?;
    cfg.normalize_blanks();
    Ok(cfg)
}

/// Writes `cfg` as pretty JSON, creating parent directories and setting
/// the file mode to 0600 so the session cookie and API key are not world-readable.
pub fn save(path: &Path, cfg: &StoredConfig) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| AppError::Config {
            path: parent.display().to_string(),
            msg: e.to_string(),
        })?;
    }
    let json = serde_json::to_string_pretty(cfg).map_err(|e| AppError::Config {
        path: path.display().to_string(),
        msg: e.to_string(),
    })?;
    std::fs::write(path, json).map_err(|e| AppError::Config {
        path: path.display().to_string(),
        msg: e.to_string(),
    })?;
    set_private(path)?;
    Ok(())
}

#[cfg(unix)]
fn set_private(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).map_err(|e| {
        AppError::Config {
            path: path.display().to_string(),
            msg: e.to_string(),
        }
    })
}

#[cfg(not(unix))]
fn set_private(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_path(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "humble-game-check-config-{label}-{}-{nanos}.json",
            std::process::id()
        ))
    }

    #[test]
    fn path_from_env_override() {
        let p = path_from(Some("/tmp/custom.json"), Some(PathBuf::from("/unused")));
        assert_eq!(p, Some(PathBuf::from("/tmp/custom.json")));
    }

    #[test]
    fn path_from_empty_env_falls_through() {
        let p = path_from(Some(""), Some(PathBuf::from("/cfg")));
        assert_eq!(p, Some(PathBuf::from("/cfg/humble-game-check/config.json")));
    }

    #[test]
    fn path_from_platform_config_dir() {
        let p = path_from(None, Some(PathBuf::from("/cfg")));
        assert_eq!(p, Some(PathBuf::from("/cfg/humble-game-check/config.json")));
    }

    #[test]
    fn path_from_none_without_config_dir() {
        assert!(path_from(None, None).is_none());
    }

    #[test]
    fn load_missing_file_is_default() {
        let path = PathBuf::from("/tmp/does-not-exist-humble-game-check-config.json");
        let cfg = load(&path).unwrap();
        assert_eq!(cfg, StoredConfig::default());
    }

    #[test]
    fn save_load_roundtrip() {
        let path = temp_path("roundtrip");
        let cfg = StoredConfig {
            humble_session: Some("sess".into()),
            steam_key: Some("key".into()),
            steam_id: Some(7656119_000000000),
            steam_vanity: None,
        };
        save(&path, &cfg).unwrap();
        let loaded = load(&path).unwrap();
        assert_eq!(loaded, cfg);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn save_creates_parent_dirs() {
        let dir = std::env::temp_dir().join(format!(
            "humble-game-check-cfg-dir-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let path = dir.join("nested").join("config.json");
        let cfg = StoredConfig {
            steam_key: Some("k".into()),
            ..StoredConfig::default()
        };
        save(&path, &cfg).unwrap();
        assert!(path.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_treats_empty_strings_as_unset() {
        let path = temp_path("empty");
        std::fs::write(&path, r#"{"humble_session":"","steam_key":"  "}"#).unwrap();
        let cfg = load(&path).unwrap();
        assert!(cfg.humble_session.is_none());
        assert!(cfg.steam_key.is_none());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn load_invalid_json_is_config_error() {
        let path = temp_path("badjson");
        std::fs::write(&path, "not json").unwrap();
        let err = load(&path).unwrap_err();
        assert!(matches!(err, AppError::Config { .. }));
        let _ = std::fs::remove_file(&path);
    }

    #[cfg(unix)]
    #[test]
    fn save_sets_mode_600() {
        use std::os::unix::fs::PermissionsExt;
        let path = temp_path("perms");
        save(
            &path,
            &StoredConfig {
                steam_key: Some("secret".into()),
                ..StoredConfig::default()
            },
        )
        .unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn normalize_blanks_clears_whitespace() {
        let mut cfg = StoredConfig {
            humble_session: Some("   ".into()),
            steam_key: Some("ok".into()),
            steam_id: None,
            steam_vanity: Some("".into()),
        };
        cfg.normalize_blanks();
        assert!(cfg.humble_session.is_none());
        assert_eq!(cfg.steam_key.as_deref(), Some("ok"));
        assert!(cfg.steam_vanity.is_none());
    }

    #[test]
    fn fill_into_only_fills_gaps() {
        let stored = StoredConfig {
            humble_session: Some("stored-sess".into()),
            steam_key: Some("stored-key".into()),
            steam_id: Some(1),
            steam_vanity: Some("stored-vanity".into()),
        };
        let mut target = StoredConfig {
            humble_session: Some("cli-sess".into()),
            steam_key: None,
            steam_id: None,
            steam_vanity: None,
        };
        stored.fill_into(&mut target);
        assert_eq!(target.humble_session.as_deref(), Some("cli-sess"));
        assert_eq!(target.steam_key.as_deref(), Some("stored-key"));
        assert_eq!(target.steam_id, Some(1));
        assert_eq!(target.steam_vanity.as_deref(), Some("stored-vanity"));
    }

    #[test]
    fn fill_into_does_not_override_explicit_vanity_with_stored_id() {
        let stored = StoredConfig {
            steam_id: Some(1),
            steam_vanity: Some("stored".into()),
            ..StoredConfig::default()
        };
        let mut target = StoredConfig {
            steam_vanity: Some("from-env".into()),
            ..StoredConfig::default()
        };
        stored.fill_into(&mut target);
        assert!(target.steam_id.is_none());
        assert_eq!(target.steam_vanity.as_deref(), Some("from-env"));
    }

    #[test]
    fn merge_entered_overlays_new_values() {
        let stored = StoredConfig {
            humble_session: Some("old-sess".into()),
            steam_key: Some("old-key".into()),
            steam_id: None,
            steam_vanity: Some("old-vanity".into()),
        };
        let entered = StoredConfig {
            steam_key: Some("new-key".into()),
            steam_id: Some(9),
            ..StoredConfig::default()
        };
        let merged = stored.merge_entered(&entered);
        assert_eq!(merged.humble_session.as_deref(), Some("old-sess"));
        assert_eq!(merged.steam_key.as_deref(), Some("new-key"));
        assert_eq!(merged.steam_id, Some(9));
        assert_eq!(merged.steam_vanity.as_deref(), Some("old-vanity"));
    }

    #[test]
    fn has_missing_detects_each_gap() {
        let mut cfg = StoredConfig::default();
        assert!(cfg.has_missing());
        cfg.humble_session = Some("s".into());
        cfg.steam_key = Some("k".into());
        assert!(cfg.missing_steam_account());
        cfg.steam_id = Some(1);
        assert!(!cfg.has_missing());
    }

    #[test]
    fn skip_serializing_none_fields() {
        let cfg = StoredConfig {
            steam_key: Some("k".into()),
            ..StoredConfig::default()
        };
        let json = serde_json::to_string(&cfg).unwrap();
        assert!(json.contains("steam_key"));
        assert!(!json.contains("humble_session"));
        assert!(!json.contains("steam_id"));
        assert!(!json.contains("steam_vanity"));
    }
}
