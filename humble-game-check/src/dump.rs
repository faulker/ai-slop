//! Saves and loads the raw Humble/Steam API payloads to disk, so a run
//! without network access (`--from-dump`) can reclassify the exact same data
//! a live run saved (`--dump-dir`), without hitting either API again.

use crate::error::{AppError, Result};
use std::path::Path;

/// Writes the raw Humble orders (as returned by `fetch_orders`) and the raw
/// Steam owned-games response to `dir/humble_orders.json` and
/// `dir/steam_owned.json`, creating `dir` if it doesn't exist.
pub fn save_dump(
    dir: &Path,
    humble: &[serde_json::Value],
    steam: &serde_json::Value,
) -> Result<()> {
    std::fs::create_dir_all(dir).map_err(|e| AppError::Dump {
        path: dir.display().to_string(),
        msg: e.to_string(),
    })?;

    let humble_path = dir.join("humble_orders.json");
    let humble_json = serde_json::to_string_pretty(humble).map_err(|e| AppError::Dump {
        path: humble_path.display().to_string(),
        msg: e.to_string(),
    })?;
    std::fs::write(&humble_path, humble_json).map_err(|e| AppError::Dump {
        path: humble_path.display().to_string(),
        msg: e.to_string(),
    })?;

    let steam_path = dir.join("steam_owned.json");
    let steam_json = serde_json::to_string_pretty(steam).map_err(|e| AppError::Dump {
        path: steam_path.display().to_string(),
        msg: e.to_string(),
    })?;
    std::fs::write(&steam_path, steam_json).map_err(|e| AppError::Dump {
        path: steam_path.display().to_string(),
        msg: e.to_string(),
    })?;

    Ok(())
}

/// Reads back a dump written by `save_dump` (or a hand-written fixture with
/// the same two files).
pub fn load_dump(dir: &Path) -> Result<(Vec<serde_json::Value>, serde_json::Value)> {
    let humble_path = dir.join("humble_orders.json");
    let humble_text = std::fs::read_to_string(&humble_path).map_err(|e| AppError::Dump {
        path: humble_path.display().to_string(),
        msg: e.to_string(),
    })?;
    let humble: Vec<serde_json::Value> =
        serde_json::from_str(&humble_text).map_err(|e| AppError::Dump {
            path: humble_path.display().to_string(),
            msg: e.to_string(),
        })?;

    let steam_path = dir.join("steam_owned.json");
    let steam_text = std::fs::read_to_string(&steam_path).map_err(|e| AppError::Dump {
        path: steam_path.display().to_string(),
        msg: e.to_string(),
    })?;
    let steam: serde_json::Value =
        serde_json::from_str(&steam_text).map_err(|e| AppError::Dump {
            path: steam_path.display().to_string(),
            msg: e.to_string(),
        })?;

    Ok((humble, steam))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn round_trips_through_a_temp_dir() {
        let dir = std::env::temp_dir().join(format!(
            "humble-game-check-dump-test-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);

        let humble = vec![json!({"gamekey": "k1", "product": {"human_name": "Test Bundle"}})];
        let steam =
            json!({"response": {"game_count": 1, "games": [{"appid": 1, "name": "Test Game"}]}});

        save_dump(&dir, &humble, &steam).unwrap();
        assert!(dir.join("humble_orders.json").exists());
        assert!(dir.join("steam_owned.json").exists());

        let (loaded_humble, loaded_steam) = load_dump(&dir).unwrap();
        assert_eq!(loaded_humble, humble);
        assert_eq!(loaded_steam, steam);

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn load_missing_dir_is_a_dump_error() {
        let dir = std::env::temp_dir().join("humble-game-check-dump-test-nonexistent");
        let err = load_dump(&dir).unwrap_err();
        assert!(matches!(err, AppError::Dump { .. }));
    }
}
