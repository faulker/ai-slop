//! The official Steam Web API: the account's owned-games library and vanity
//! URL resolution. Both fetch functions are split from their parsers so the
//! parsing rules are testable offline.

use crate::error::{AppError, Result};
use reqwest::StatusCode;
use serde::Deserialize;
use std::collections::{HashMap, HashSet};

pub const BASE: &str = "https://api.steampowered.com";

/// Top-level `GetOwnedGames` response shape.
#[derive(Debug, Deserialize)]
struct OwnedGames {
    response: OwnedResponse,
}

#[derive(Debug, Default, Deserialize)]
struct OwnedResponse {
    #[serde(default)]
    games: Option<Vec<OwnedGame>>,
}

/// One owned game. `name` is missing for delisted apps.
#[derive(Debug, Clone, Deserialize)]
pub struct OwnedGame {
    pub appid: u32,
    #[serde(default)]
    pub name: Option<String>,
}

/// Top-level `ResolveVanityURL` response shape.
#[derive(Debug, Deserialize)]
struct VanityResponse {
    response: VanityInner,
}

#[derive(Debug, Deserialize)]
struct VanityInner {
    success: i32,
    #[serde(default)]
    steamid: Option<String>,
    #[serde(default)]
    message: Option<String>,
}

/// Performs the vanity-URL lookup and returns the raw response body.
fn fetch_vanity(base: &str, key: &str, name: &str) -> Result<serde_json::Value> {
    let url = format!("{base}/ISteamUser/ResolveVanityURL/v1/?key={key}&vanityurl={name}");
    let resp = reqwest::blocking::get(&url)?;
    let body = resp.text()?;
    Ok(serde_json::from_str(&body)?)
}

/// Reads a vanity-lookup response. `success != 1` means the name is unknown;
/// the API's `message` explains why (e.g. "No match").
pub fn parse_vanity(raw: &serde_json::Value, name: &str) -> Result<u64> {
    let parsed: VanityResponse = serde_json::from_value(raw.clone())?;
    if parsed.response.success != 1 {
        let msg = parsed.response.message.unwrap_or_else(|| "no match".into());
        return Err(AppError::VanityNotFound {
            name: name.to_string(),
            msg,
        });
    }
    let steamid_str = parsed.response.steamid.ok_or_else(|| AppError::JsonParse {
        msg: "vanity response missing steamid".into(),
    })?;
    steamid_str.parse::<u64>().map_err(|e| AppError::JsonParse {
        msg: format!("invalid steamid \"{steamid_str}\": {e}"),
    })
}

/// Resolves a Steam profile vanity name (from a profile URL like
/// `steamcommunity.com/id/<name>`) to a SteamID64.
pub fn resolve_vanity(base: &str, key: &str, name: &str) -> Result<u64> {
    let raw = fetch_vanity(base, key, name)?;
    parse_vanity(&raw, name)
}

/// Fetches the account's owned-games library and returns the raw JSON body,
/// unparsed, so `--dump-dir` can save it untouched.
pub fn fetch_owned_games(base: &str, key: &str, steamid: u64) -> Result<serde_json::Value> {
    let url = format!(
        "{base}/IPlayerService/GetOwnedGames/v1/?key={key}&steamid={steamid}&include_appinfo=1&include_played_free_games=1&skip_unvetted_apps=0&format=json"
    );
    let resp = reqwest::blocking::get(&url)?;
    let status = resp.status();
    if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
        return Err(AppError::SteamAuth {
            status: status.as_u16(),
        });
    }
    let body = resp.text()?;
    Ok(serde_json::from_str(&body)?)
}

/// Parses a raw `GetOwnedGames` body. A missing or empty `games` list means
/// the library isn't visible to this key (private profile, wrong account).
pub fn parse_owned_games(raw: &serde_json::Value, steamid: u64) -> Result<Vec<OwnedGame>> {
    let parsed: OwnedGames = serde_json::from_value(raw.clone())?;
    let games = parsed.response.games.unwrap_or_default();
    if games.is_empty() {
        return Err(AppError::SteamEmpty { steamid });
    }
    Ok(games)
}

/// One normalized, name-bearing Steam library entry, kept for fuzzy matching
/// in `matching.rs`.
#[derive(Debug, Clone)]
pub struct IndexEntry {
    pub appid: u32,
    pub name: String,
    pub norm: String,
    pub tokens: Vec<String>,
}

/// A lookup structure over the Steam library, built once and queried per
/// Humble key: exact app id, exact normalized name, and the full entry list
/// for fuzzy scoring.
pub struct SteamIndex {
    pub appids: HashSet<u32>,
    pub by_norm: HashMap<String, (u32, String)>,
    pub entries: Vec<IndexEntry>,
}

impl SteamIndex {
    /// Builds the index from the owned-games list. Games without a `name`
    /// (delisted apps) only go into `appids`, since there is nothing to
    /// normalize or match by name.
    pub fn build(games: &[OwnedGame]) -> Self {
        let mut appids = HashSet::new();
        let mut by_norm = HashMap::new();
        let mut entries = Vec::new();

        for game in games {
            appids.insert(game.appid);
            let Some(name) = &game.name else { continue };
            let norm = crate::matching::normalize(name);
            let tokens = crate::matching::tokens(&norm);
            by_norm
                .entry(norm.clone())
                .or_insert((game.appid, name.clone()));
            entries.push(IndexEntry {
                appid: game.appid,
                name: name.clone(),
                norm,
                tokens,
            });
        }

        SteamIndex {
            appids,
            by_norm,
            entries,
        }
    }

    /// Whether the given app id is in the library.
    pub fn has_appid(&self, appid: u32) -> bool {
        self.appids.contains(&appid)
    }

    /// The `(appid, name)` of the library entry with this exact normalized
    /// name, if any.
    pub fn by_norm(&self, norm: &str) -> Option<&(u32, String)> {
        self.by_norm.get(norm)
    }

    /// All named library entries, for fuzzy scoring.
    pub fn entries(&self) -> &[IndexEntry] {
        &self.entries
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn owned_game_without_name_parses() {
        let raw = json!({"response": {"game_count": 1, "games": [{"appid": 570}]}});
        let games = parse_owned_games(&raw, 7656119_0000000001).unwrap();
        assert_eq!(games.len(), 1);
        assert_eq!(games[0].appid, 570);
        assert_eq!(games[0].name, None);
    }

    #[test]
    fn empty_response_object_is_steam_empty() {
        let raw = json!({"response": {}});
        let err = parse_owned_games(&raw, 7656119_0000000001).unwrap_err();
        assert!(matches!(err, AppError::SteamEmpty { .. }));
    }

    #[test]
    fn empty_games_list_is_steam_empty() {
        let raw = json!({"response": {"game_count": 0, "games": []}});
        let err = parse_owned_games(&raw, 7656119_0000000001).unwrap_err();
        assert!(matches!(err, AppError::SteamEmpty { .. }));
    }

    #[test]
    fn steam_index_build_puts_nameless_games_only_in_appid_set() {
        let games = vec![
            OwnedGame {
                appid: 570,
                name: Some("Awesome Game".into()),
            },
            OwnedGame {
                appid: 999,
                name: None,
            },
        ];
        let index = SteamIndex::build(&games);
        assert!(index.has_appid(570));
        assert!(index.has_appid(999));
        assert_eq!(index.entries().len(), 1);
        assert_eq!(index.entries()[0].appid, 570);
        assert!(index.by_norm("awesome game").is_some());
    }

    #[test]
    fn resolve_vanity_success_parses_steamid() {
        let raw = json!({"response": {"success": 1, "steamid": "76561197960287930"}});
        let id = parse_vanity(&raw, "yourvanityname").unwrap();
        assert_eq!(id, 76561197960287930);
    }

    #[test]
    fn resolve_vanity_failure_is_vanity_not_found() {
        let raw = json!({"response": {"success": 42, "message": "No match"}});
        let err = parse_vanity(&raw, "nosuchname").unwrap_err();
        match err {
            AppError::VanityNotFound { name, msg } => {
                assert_eq!(name, "nosuchname");
                assert_eq!(msg, "No match");
            }
            other => panic!("expected VanityNotFound, got {other:?}"),
        }
    }
}
