//! Registry credentials and `WWW-Authenticate` challenge parsing.
//!
//! Credentials live in `<root>/auth.json` (Docker `config.json` format:
//! `{"auths": {"<registry>": {"auth": "<base64 user:pass>"}}}`). As a fallback
//! `$DOCKER_CONFIG/config.json` or `~/.docker/config.json` is read; entries
//! that only name a credential helper are skipped.

use base64::Engine;
use hullr_core::{paths::write_atomic, Context, Error, Result};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const HUB_KEYS: &[&str] = &[
    "docker.io",
    "index.docker.io",
    "registry-1.docker.io",
    "https://index.docker.io/v1/",
    "https://index.docker.io/v2/",
    "https://docker.io",
];

/// Username and password for a registry.
#[derive(Clone, PartialEq, Eq)]
pub struct Credentials {
    pub username: String,
    pub password: String,
}

impl std::fmt::Debug for Credentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Credentials({}, ***)", self.username)
    }
}

impl Credentials {
    pub fn basic_header(&self) -> String {
        let raw = format!("{}:{}", self.username, self.password);
        format!("Basic {}", base64::engine::general_purpose::STANDARD.encode(raw))
    }
}

/// Reduce user input (`https://index.docker.io/v1/`, `ghcr.io/`, `registry-1.docker.io`)
/// to a bare registry name where Docker Hub is always `docker.io`.
pub fn normalize_registry(s: &str) -> String {
    let s = s.trim();
    let s = s.strip_prefix("https://").or_else(|| s.strip_prefix("http://")).unwrap_or(s);
    let host = s.split('/').next().unwrap_or(s);
    match host {
        "index.docker.io" | "registry-1.docker.io" | "registry.hub.docker.com" => "docker.io".to_string(),
        h => h.to_string(),
    }
}

fn candidate_keys(registry: &str) -> Vec<String> {
    let reg = normalize_registry(registry);
    if reg == "docker.io" {
        HUB_KEYS.iter().map(|s| s.to_string()).collect()
    } else {
        vec![reg.clone(), format!("https://{reg}"), format!("http://{reg}"), format!("https://{reg}/")]
    }
}

fn entry_credentials(entry: &Value) -> Option<Credentials> {
    if let Some(auth) = entry.get("auth").and_then(Value::as_str) {
        let raw = base64::engine::general_purpose::STANDARD.decode(auth.trim()).ok()?;
        let s = String::from_utf8(raw).ok()?;
        let (u, p) = s.split_once(':')?;
        return Some(Credentials { username: u.to_string(), password: p.to_string() });
    }
    let u = entry.get("username").and_then(Value::as_str)?;
    let p = entry.get("password").and_then(Value::as_str)?;
    if u.is_empty() {
        return None;
    }
    Some(Credentials { username: u.to_string(), password: p.to_string() })
}

/// Look up credentials for `registry` inside one config file.
pub fn lookup_in_file(path: &Path, registry: &str) -> Option<Credentials> {
    let data = std::fs::read(path).ok()?;
    let v: Value = serde_json::from_slice(&data).ok()?;
    let auths = v.get("auths")?.as_object()?;
    for key in candidate_keys(registry) {
        if let Some(c) = auths.get(&key).and_then(entry_credentials) {
            return Some(c);
        }
    }
    None
}

fn docker_config_path() -> Option<PathBuf> {
    if let Some(d) = std::env::var_os("DOCKER_CONFIG") {
        return Some(PathBuf::from(d).join("config.json"));
    }
    dirs::home_dir().map(|h| h.join(".docker").join("config.json"))
}

/// Credentials from our own `auth.json` first, then the Docker config.
pub fn lookup(root: &Path, registry: &str) -> Option<Credentials> {
    lookup_in_file(&root.join("auth.json"), registry)
        .or_else(|| docker_config_path().and_then(|p| lookup_in_file(&p, registry)))
}

/// Store credentials in `<root>/auth.json` (mode 0600), keeping other entries.
/// The caller serializes concurrent writers.
pub fn save(root: &Path, registry: &str, creds: &Credentials) -> Result<()> {
    let path = root.join("auth.json");
    let mut doc: Value = std::fs::read(&path)
        .ok()
        .and_then(|d| serde_json::from_slice(&d).ok())
        .unwrap_or_else(|| json!({}));
    if !doc.is_object() {
        doc = json!({});
    }
    let reg = normalize_registry(registry);
    let key = if reg == "docker.io" { "https://index.docker.io/v1/".to_string() } else { reg };
    let raw = format!("{}:{}", creds.username, creds.password);
    let auth = base64::engine::general_purpose::STANDARD.encode(raw);
    let auths = doc
        .as_object_mut()
        .expect("object")
        .entry("auths")
        .or_insert_with(|| json!({}));
    if !auths.is_object() {
        *auths = json!({});
    }
    // Drop any alias of the same registry so lookups are unambiguous.
    let aliases = candidate_keys(&key);
    if let Some(m) = auths.as_object_mut() {
        m.retain(|k, _| !aliases.contains(k));
        m.insert(key, json!({ "auth": auth }));
    }
    write_atomic(&path, &serde_json::to_vec_pretty(&doc)?).context("write auth.json")?;
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    Ok(())
}

/// A parsed `WWW-Authenticate` challenge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Challenge {
    Bearer { realm: String, service: Option<String>, scope: Option<String> },
    Basic,
}

/// Parse a `WWW-Authenticate` header value such as
/// `Bearer realm="https://auth.docker.io/token",service="registry.docker.io",scope="repository:library/alpine:pull"`.
pub fn parse_challenge(header: &str) -> Result<Challenge> {
    let header = header.trim();
    let (scheme, rest) = header.split_once(char::is_whitespace).unwrap_or((header, ""));
    if scheme.eq_ignore_ascii_case("basic") {
        return Ok(Challenge::Basic);
    }
    if !scheme.eq_ignore_ascii_case("bearer") {
        return Err(Error::Registry(format!("unsupported auth scheme '{scheme}'")));
    }
    let params = parse_params(rest);
    let realm = params
        .get("realm")
        .cloned()
        .ok_or_else(|| Error::Registry("bearer challenge without realm".into()))?;
    Ok(Challenge::Bearer { realm, service: params.get("service").cloned(), scope: params.get("scope").cloned() })
}

fn parse_params(s: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        while i < b.len() && (b[i] == b' ' || b[i] == b',' || b[i] == b'\t') {
            i += 1;
        }
        let ks = i;
        while i < b.len() && b[i] != b'=' && b[i] != b',' {
            i += 1;
        }
        if i >= b.len() || b[i] != b'=' {
            continue;
        }
        let key = s[ks..i].trim().to_ascii_lowercase();
        i += 1;
        let val = if i < b.len() && b[i] == b'"' {
            i += 1;
            let mut v = String::new();
            while i < b.len() && b[i] != b'"' {
                if b[i] == b'\\' && i + 1 < b.len() {
                    i += 1;
                }
                v.push(b[i] as char);
                i += 1;
            }
            i += 1;
            v
        } else {
            let vs = i;
            while i < b.len() && b[i] != b',' {
                i += 1;
            }
            s[vs..i].trim().to_string()
        };
        out.insert(key, val);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_bearer_challenge() {
        let c = parse_challenge(
            r#"Bearer realm="https://auth.docker.io/token",service="registry.docker.io",scope="repository:library/alpine:pull""#,
        )
        .unwrap();
        assert_eq!(
            c,
            Challenge::Bearer {
                realm: "https://auth.docker.io/token".into(),
                service: Some("registry.docker.io".into()),
                scope: Some("repository:library/alpine:pull".into())
            }
        );
        assert_eq!(parse_challenge("Basic realm=\"x\"").unwrap(), Challenge::Basic);
        assert!(parse_challenge("Negotiate").is_err());
    }

    #[test]
    fn ghcr_style_challenge() {
        let c = parse_challenge(r#"Bearer realm="https://ghcr.io/token",service="ghcr.io",scope="repository:a/b:pull""#).unwrap();
        assert!(matches!(c, Challenge::Bearer { service: Some(s), .. } if s == "ghcr.io"));
    }

    #[test]
    fn save_and_lookup_round_trip() {
        let d = tempfile::tempdir().unwrap();
        let c = Credentials { username: "bob".into(), password: "p:w".into() };
        save(d.path(), "docker.io", &c).unwrap();
        save(d.path(), "ghcr.io", &Credentials { username: "x".into(), password: "y".into() }).unwrap();
        assert_eq!(lookup_in_file(&d.path().join("auth.json"), "docker.io"), Some(c.clone()));
        assert_eq!(lookup_in_file(&d.path().join("auth.json"), "https://index.docker.io/v1/"), Some(c));
        assert_eq!(lookup_in_file(&d.path().join("auth.json"), "ghcr.io").unwrap().username, "x");
        assert!(lookup_in_file(&d.path().join("auth.json"), "quay.io").is_none());
    }

    #[test]
    fn docker_config_hub_key_and_helpers() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("config.json");
        let auth = base64::engine::general_purpose::STANDARD.encode("u:p");
        std::fs::write(
            &p,
            format!(r#"{{"auths":{{"https://index.docker.io/v1/":{{"auth":"{auth}"}},"ghcr.io":{{}}}},"credsStore":"desktop"}}"#),
        )
        .unwrap();
        assert_eq!(lookup_in_file(&p, "docker.io").unwrap().username, "u");
        assert!(lookup_in_file(&p, "ghcr.io").is_none());
    }

    #[test]
    fn registry_normalization() {
        assert_eq!(normalize_registry("https://index.docker.io/v1/"), "docker.io");
        assert_eq!(normalize_registry("ghcr.io/"), "ghcr.io");
        assert_eq!(normalize_registry("localhost:5000"), "localhost:5000");
    }
}
