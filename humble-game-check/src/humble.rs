//! The Humble Bundle order/key API, authenticated with the `_simpleauth_sess`
//! session cookie copied from a logged-in browser. Fetch and parse are kept
//! separate so the parsing rules are testable offline (see `--dump-dir` in
//! main.rs, which saves the raw fetch output for exactly this reason).

use crate::error::{AppError, Result};
use reqwest::blocking::Client;
use reqwest::header::{HeaderMap, HeaderValue, COOKIE};
use reqwest::{redirect, StatusCode};
use serde::{Deserialize, Serialize};
use std::time::Duration;

pub const BASE: &str = "https://www.humblebundle.com";

/// Humble's `/api/v1/orders` endpoint takes this many `gamekeys` per request
/// before it starts rejecting the query string.
const BATCH: usize = 40;

/// One entry from `GET /api/v1/user/order`, which lists every order the
/// account owns without the per-key detail.
#[derive(Debug, Deserialize)]
struct OrderKey {
    gamekey: String,
}

/// The bundle/product name a Tpk's key belongs to. Unlisted fields are
/// ignored.
#[derive(Debug, Clone, Deserialize)]
pub struct Product {
    pub human_name: String,
}

/// The `tpkd_dict` wrapper around a Tpk list. Subscription orders (Humble
/// Choice, etc.) may have an empty `all_tpks`.
#[derive(Debug, Clone, Deserialize)]
pub struct TpkdDict {
    #[serde(default)]
    pub all_tpks: Vec<Tpk>,
}

/// One order as returned by `/api/v1/orders`.
#[derive(Debug, Clone, Deserialize)]
pub struct Order {
    pub gamekey: String,
    pub product: Product,
    pub tpkd_dict: TpkdDict,
}

/// A single third-party key (Tpk) inside an order. `redeemed_key_val` and
/// `steam_app_id` are kept as raw `serde_json::Value` because their shape
/// varies (string, object, number, numeric string, or absent) depending on
/// key state; `is_revealed`/`app_id` below normalize that.
#[derive(Debug, Clone, Deserialize)]
pub struct Tpk {
    pub machine_name: String,
    #[serde(default)]
    pub human_name: Option<String>,
    pub key_type: String,
    #[serde(default)]
    pub is_expired: bool,
    #[serde(default)]
    pub sold_out: bool,
    #[serde(default)]
    pub redeemed_key_val: Option<serde_json::Value>,
    #[serde(default)]
    pub steam_app_id: Option<serde_json::Value>,
}

impl Tpk {
    /// Whether this key has already been revealed. A present, non-empty
    /// string or an object both count as revealed; absent, null, or an empty
    /// string count as unrevealed.
    pub fn is_revealed(&self) -> bool {
        match &self.redeemed_key_val {
            None => false,
            Some(serde_json::Value::Null) => false,
            Some(serde_json::Value::String(s)) => !s.is_empty(),
            Some(_) => true,
        }
    }

    /// The Steam app id, when known. Accepts a JSON number or a numeric
    /// string; treats 0, non-numeric, null, or missing as unknown.
    pub fn app_id(&self) -> Option<u32> {
        match &self.steam_app_id {
            Some(serde_json::Value::Number(n)) => n
                .as_u64()
                .and_then(|v| u32::try_from(v).ok())
                .filter(|&v| v > 0),
            Some(serde_json::Value::String(s)) => s.parse::<u32>().ok().filter(|&v| v > 0),
            _ => None,
        }
    }

    /// Whether this key is a Steam key still worth redeeming: not revealed,
    /// not expired, not sold out.
    pub fn is_unused_steam(&self) -> bool {
        self.key_type == "steam" && !self.is_revealed() && !self.is_expired && !self.sold_out
    }
}

/// A Humble key worth checking against the Steam library. Built from a Tpk
/// plus its parent order; `redeemed_key_val` is deliberately not carried
/// forward since it must never end up in output.
#[derive(Debug, Clone, Serialize)]
pub struct HumbleKey {
    pub name: String,
    pub machine_name: String,
    pub order_name: String,
    pub gamekey: String,
    pub app_id: Option<u32>,
}

impl HumbleKey {
    /// The deepest link Humble supports to the order page holding this key;
    /// there is no per-key anchor.
    pub fn humble_url(&self) -> String {
        format!(
            "https://www.humblebundle.com/downloads?key={}",
            self.gamekey
        )
    }
}

/// Builds the HTTP client used for every Humble request: the session cookie
/// as a default header, redirects disabled (a redirect back to the login
/// page is how an expired session shows up), and a 30s timeout.
pub fn client(session: &str) -> Result<Client> {
    let mut headers = HeaderMap::new();
    let cookie = HeaderValue::from_str(&format!("_simpleauth_sess={session}"))
        .map_err(|e| AppError::Http { msg: e.to_string() })?;
    headers.insert(COOKIE, cookie);

    Client::builder()
        .default_headers(headers)
        .redirect(redirect::Policy::none())
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(AppError::from)
}

/// Reads the response status/body and turns an expired or missing session
/// into `AppError::HumbleAuth`. Redirects (disabled at the client, so they
/// arrive here as 3xx) and 401/403 are auth failures; so is a 200 whose body
/// isn't JSON, which is what Humble's login page looks like to this client.
fn check_auth(resp: reqwest::blocking::Response) -> Result<String> {
    let status = resp.status();
    if status.is_redirection()
        || status == StatusCode::UNAUTHORIZED
        || status == StatusCode::FORBIDDEN
    {
        return Err(AppError::HumbleAuth {
            status: status.as_u16(),
        });
    }
    let body = resp.text()?;
    if serde_json::from_str::<serde_json::Value>(&body).is_err() {
        return Err(AppError::HumbleAuth {
            status: status.as_u16(),
        });
    }
    Ok(body)
}

/// Fetches every gamekey the account owns.
pub fn fetch_order_keys(client: &Client, base: &str) -> Result<Vec<String>> {
    let url = format!("{base}/api/v1/user/order");
    let resp = client.get(&url).send()?;
    let body = check_auth(resp)?;
    let keys: Vec<OrderKey> = serde_json::from_str(&body)?;
    Ok(keys.into_iter().map(|k| k.gamekey).collect())
}

/// Fetches order detail (including all keys) for the given gamekeys, in
/// batches of `BATCH`. The response is a JSON object keyed by gamekey; each
/// value is pushed as-is so the caller can keep the raw payload for
/// `--dump-dir` before parsing it.
pub fn fetch_orders(
    client: &Client,
    base: &str,
    keys: &[String],
) -> Result<Vec<serde_json::Value>> {
    let mut orders = Vec::new();
    for chunk in keys.chunks(BATCH) {
        let mut url = format!("{base}/api/v1/orders?all_tpkds=true");
        for key in chunk {
            url.push_str("&gamekeys=");
            url.push_str(key);
        }
        let resp = client.get(&url).send()?;
        let body = check_auth(resp)?;
        let value: serde_json::Value = serde_json::from_str(&body)?;
        let map = value.as_object().ok_or_else(|| AppError::JsonParse {
            msg: "expected /api/v1/orders response to be a JSON object keyed by gamekey".into(),
        })?;
        orders.extend(map.values().cloned());
    }
    Ok(orders)
}

/// Parses raw order objects (as returned by `fetch_orders`) into `Order`s.
pub fn parse_orders(raw: &[serde_json::Value]) -> Result<Vec<Order>> {
    raw.iter()
        .map(|v| serde_json::from_value(v.clone()).map_err(AppError::from))
        .collect()
}

/// Every unused Steam key across the given orders, tagged with its order
/// name for display.
pub fn unused_steam_keys(orders: &[Order]) -> Vec<HumbleKey> {
    orders
        .iter()
        .flat_map(|order| {
            order
                .tpkd_dict
                .all_tpks
                .iter()
                .filter(|tpk| tpk.is_unused_steam())
                .map(move |tpk| HumbleKey {
                    name: tpk
                        .human_name
                        .clone()
                        .unwrap_or_else(|| tpk.machine_name.clone()),
                    machine_name: tpk.machine_name.clone(),
                    order_name: order.product.human_name.clone(),
                    gamekey: order.gamekey.clone(),
                    app_id: tpk.app_id(),
                })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn tpk(extra: serde_json::Value) -> Tpk {
        let mut base = json!({
            "machine_name": "awesomegame3_steam",
            "human_name": "Awesome Game 3",
            "gamekey": "abc123XYZ",
            "key_type": "steam",
        });
        for (k, v) in extra.as_object().unwrap() {
            base[k] = v.clone();
        }
        serde_json::from_value(base).unwrap()
    }

    #[test]
    fn redeemed_key_val_as_string_is_revealed() {
        assert!(tpk(json!({"redeemed_key_val": "AAAA-BBBB-CCCC"})).is_revealed());
    }

    #[test]
    fn redeemed_key_val_as_object_is_revealed() {
        assert!(tpk(json!({"redeemed_key_val": {"key": "AAAA-BBBB-CCCC"}})).is_revealed());
    }

    #[test]
    fn redeemed_key_val_absent_is_unrevealed() {
        assert!(!tpk(json!({})).is_revealed());
    }

    #[test]
    fn redeemed_key_val_empty_string_is_unrevealed() {
        assert!(!tpk(json!({"redeemed_key_val": ""})).is_revealed());
    }

    #[test]
    fn steam_app_id_null_is_none() {
        assert_eq!(tpk(json!({"steam_app_id": null})).app_id(), None);
    }

    #[test]
    fn steam_app_id_missing_is_none() {
        assert_eq!(tpk(json!({})).app_id(), None);
    }

    #[test]
    fn steam_app_id_zero_is_none() {
        assert_eq!(tpk(json!({"steam_app_id": 0})).app_id(), None);
    }

    #[test]
    fn steam_app_id_number_is_some() {
        assert_eq!(tpk(json!({"steam_app_id": 12345})).app_id(), Some(12345));
    }

    #[test]
    fn steam_app_id_numeric_string_is_some() {
        assert_eq!(tpk(json!({"steam_app_id": "12345"})).app_id(), Some(12345));
    }

    #[test]
    fn subscription_order_with_empty_tpks_parses() {
        let order: Order = serde_json::from_value(json!({
            "gamekey": "sub123",
            "product": {"human_name": "Humble Choice September", "unknown_field": "ignored"},
            "tpkd_dict": {"all_tpks": []},
            "unrelated_top_level_field": 42,
        }))
        .unwrap();
        assert!(order.tpkd_dict.all_tpks.is_empty());
        assert_eq!(order.product.human_name, "Humble Choice September");
    }

    #[test]
    fn unused_steam_keys_filters_and_names_from_product() {
        let order: Order = serde_json::from_value(json!({
            "gamekey": "bundle1",
            "product": {"human_name": "Awesome Bundle"},
            "tpkd_dict": {"all_tpks": [
                {"machine_name": "awesomegame3_steam", "human_name": "Awesome Game 3",
                 "gamekey": "abc123XYZ", "key_type": "steam"},
                {"machine_name": "gogkey_gog", "human_name": "GOG Game",
                 "gamekey": "gog111", "key_type": "gog"},
                {"machine_name": "expired_steam", "human_name": "Expired Game",
                 "gamekey": "exp111", "key_type": "steam", "is_expired": true},
                {"machine_name": "soldout_steam", "human_name": "Sold Out Game",
                 "gamekey": "sold111", "key_type": "steam", "sold_out": true},
                {"machine_name": "revealed_steam", "human_name": "Revealed Game",
                 "gamekey": "rev111", "key_type": "steam", "redeemed_key_val": "AAAA-BBBB"},
            ]},
        }))
        .unwrap();

        let keys = unused_steam_keys(&[order]);
        assert_eq!(keys.len(), 1);
        assert_eq!(keys[0].name, "Awesome Game 3");
        assert_eq!(keys[0].order_name, "Awesome Bundle");
        assert_eq!(keys[0].gamekey, "bundle1");
    }

    #[test]
    fn parse_orders_over_a_batch_object_keyed_by_gamekey() {
        let raw = vec![
            json!({"gamekey": "k1", "product": {"human_name": "Game One"}, "tpkd_dict": {"all_tpks": []}}),
            json!({"gamekey": "k2", "product": {"human_name": "Game Two"}, "tpkd_dict": {"all_tpks": []}}),
        ];
        let orders = parse_orders(&raw).unwrap();
        assert_eq!(orders.len(), 2);
        assert_eq!(orders[0].gamekey, "k1");
        assert_eq!(orders[1].product.human_name, "Game Two");
    }

    #[test]
    fn humble_url_format() {
        let key = HumbleKey {
            name: "Awesome Game 3".into(),
            machine_name: "awesomegame3_steam".into(),
            order_name: "Awesome Bundle".into(),
            gamekey: "abc123XYZ".into(),
            app_id: None,
        };
        assert_eq!(
            key.humble_url(),
            "https://www.humblebundle.com/downloads?key=abc123XYZ"
        );
    }
}
