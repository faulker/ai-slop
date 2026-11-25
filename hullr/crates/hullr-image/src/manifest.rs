//! Manifest, index and image-config JSON handling.
//!
//! Understands Docker schema 2 (manifest + manifest list) and OCI (manifest +
//! index), selects a platform from an index, and maps the OCI image config
//! (`Env`, `Cmd`, ...) to and from `hullr_core::ImageConfig`.

use crate::timefmt;
use hullr_core::{
    Error, HealthCheck, ImageConfig, ImageRef, Platform, Result,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;

pub const DOCKER_MANIFEST: &str = "application/vnd.docker.distribution.manifest.v2+json";
pub const DOCKER_LIST: &str = "application/vnd.docker.distribution.manifest.list.v2+json";
pub const OCI_MANIFEST: &str = "application/vnd.oci.image.manifest.v1+json";
pub const OCI_INDEX: &str = "application/vnd.oci.image.index.v1+json";
pub const OCI_CONFIG: &str = "application/vnd.oci.image.config.v1+json";
pub const OCI_LAYER_GZIP: &str = "application/vnd.oci.image.layer.v1.tar+gzip";
pub const OCI_LAYER_ZSTD: &str = "application/vnd.oci.image.layer.v1.tar+zstd";
pub const OCI_LAYER_TAR: &str = "application/vnd.oci.image.layer.v1.tar";

/// `Accept` header value covering every manifest flavour we understand.
pub const ACCEPT: &str = "application/vnd.docker.distribution.manifest.v2+json, \
application/vnd.docker.distribution.manifest.list.v2+json, \
application/vnd.oci.image.manifest.v1+json, \
application/vnd.oci.image.index.v1+json";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlatformJson {
    #[serde(default)]
    pub os: String,
    #[serde(default)]
    pub architecture: String,
    #[serde(default)]
    pub variant: Option<String>,
}

impl PlatformJson {
    pub fn to_platform(&self) -> Platform {
        Platform::new(&self.os, &self.architecture, self.variant.as_deref().filter(|v| !v.is_empty()))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Descriptor {
    #[serde(rename = "mediaType", default)]
    pub media_type: String,
    pub digest: String,
    #[serde(default)]
    pub size: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platform: Option<PlatformJson>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub annotations: Option<BTreeMap<String, String>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageManifest {
    pub config: Descriptor,
    pub layers: Vec<Descriptor>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Manifest {
    Image(ImageManifest),
    Index(Vec<Descriptor>),
}

#[derive(Deserialize)]
struct RawManifest {
    #[serde(rename = "schemaVersion", default)]
    schema_version: u32,
    #[serde(rename = "mediaType", default)]
    media_type: Option<String>,
    #[serde(default)]
    config: Option<Descriptor>,
    #[serde(default)]
    layers: Option<Vec<Descriptor>>,
    #[serde(default)]
    manifests: Option<Vec<Descriptor>>,
}

/// Parse a manifest or index body. `content_type` (from the HTTP response) is
/// only a hint: the document's own shape wins.
pub fn parse_manifest(body: &[u8], content_type: Option<&str>) -> Result<Manifest> {
    let raw: RawManifest = serde_json::from_slice(body)
        .map_err(|e| Error::Registry(format!("invalid manifest json: {e}")))?;
    if raw.schema_version == 1 {
        return Err(Error::Unsupported("docker schema 1 manifests are not supported".into()));
    }
    let mt = raw
        .media_type
        .as_deref()
        .or(content_type)
        .map(|s| s.split(';').next().unwrap_or("").trim().to_string())
        .unwrap_or_default();
    let is_index = mt == DOCKER_LIST || mt == OCI_INDEX || (raw.manifests.is_some() && raw.config.is_none());
    if is_index {
        let m = raw
            .manifests
            .ok_or_else(|| Error::Registry("index without manifests".into()))?;
        return Ok(Manifest::Index(m));
    }
    match (raw.config, raw.layers) {
        (Some(config), Some(layers)) => Ok(Manifest::Image(ImageManifest { config, layers })),
        _ => Err(Error::Registry("manifest has neither config/layers nor manifests".into())),
    }
}

/// Pick the descriptor matching `want` from an index. Attestation manifests
/// (platform `unknown/unknown`) are ignored. With several candidates (for
/// example `linux/arm` without a variant) the highest variant wins.
pub fn select_platform<'a>(entries: &'a [Descriptor], want: &Platform) -> Option<&'a Descriptor> {
    let mut best: Option<&Descriptor> = None;
    for d in entries {
        let Some(p) = &d.platform else { continue };
        if p.os == "unknown" || p.architecture == "unknown" {
            continue;
        }
        if !want.matches(&p.to_platform()) {
            continue;
        }
        let better = match best {
            None => true,
            Some(b) => {
                let bv = b.platform.as_ref().and_then(|p| p.variant.clone()).unwrap_or_default();
                p.variant.clone().unwrap_or_default() > bv
            }
        };
        if better {
            best = Some(d);
        }
    }
    best
}

/// `linux/amd64, linux/arm64` style summary for error messages.
pub fn available_platforms(entries: &[Descriptor]) -> String {
    let v: Vec<String> = entries
        .iter()
        .filter_map(|d| d.platform.as_ref())
        .filter(|p| p.os != "unknown")
        .map(|p| p.to_platform().to_string())
        .collect();
    v.join(", ")
}

// ---------------------------------------------------------------------------
// Image config
// ---------------------------------------------------------------------------

/// A parsed image config blob.
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedConfig {
    pub config: ImageConfig,
    pub diff_ids: Vec<String>,
    pub platform: Platform,
    pub created: i64,
    pub history: Vec<Value>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct RawHealth {
    #[serde(rename = "Test")]
    test: Option<Vec<String>>,
    #[serde(rename = "Interval")]
    interval: Option<i64>,
    #[serde(rename = "Timeout")]
    timeout: Option<i64>,
    #[serde(rename = "StartPeriod")]
    start_period: Option<i64>,
    #[serde(rename = "Retries")]
    retries: Option<u32>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct RawContainerConfig {
    #[serde(rename = "User")]
    user: Option<String>,
    #[serde(rename = "Env")]
    env: Option<Vec<String>>,
    #[serde(rename = "Entrypoint")]
    entrypoint: Option<Vec<String>>,
    #[serde(rename = "Cmd")]
    cmd: Option<Vec<String>>,
    #[serde(rename = "WorkingDir")]
    working_dir: Option<String>,
    #[serde(rename = "ExposedPorts")]
    exposed_ports: Option<BTreeMap<String, Value>>,
    #[serde(rename = "Volumes")]
    volumes: Option<BTreeMap<String, Value>>,
    #[serde(rename = "Labels")]
    labels: Option<BTreeMap<String, String>>,
    #[serde(rename = "StopSignal")]
    stop_signal: Option<String>,
    #[serde(rename = "Healthcheck")]
    healthcheck: Option<RawHealth>,
    #[serde(rename = "Shell")]
    shell: Option<Vec<String>>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct RawRootfs {
    diff_ids: Vec<String>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct RawConfig {
    architecture: String,
    os: String,
    variant: Option<String>,
    created: Option<String>,
    config: Option<RawContainerConfig>,
    rootfs: RawRootfs,
    history: Vec<Value>,
}

fn ns_to_secs(ns: Option<i64>, default: u64) -> u64 {
    match ns {
        Some(n) if n > 0 => (n as u64).div_ceil(1_000_000_000),
        _ => default,
    }
}

fn nonempty(s: Option<String>) -> Option<String> {
    s.filter(|v| !v.is_empty())
}

/// Parse an OCI/Docker image config blob into runtime types.
pub fn parse_config(blob: &[u8]) -> Result<ParsedConfig> {
    let raw: RawConfig = serde_json::from_slice(blob)
        .map_err(|e| Error::Registry(format!("invalid image config json: {e}")))?;
    let c = raw.config.unwrap_or_default();
    let healthcheck = c.healthcheck.and_then(|h| {
        let test = h.test.unwrap_or_default();
        if test.is_empty() {
            return None;
        }
        Some(HealthCheck {
            test,
            interval_secs: ns_to_secs(h.interval, 30),
            timeout_secs: ns_to_secs(h.timeout, 30),
            retries: h.retries.filter(|r| *r > 0).unwrap_or(3),
            start_period_secs: ns_to_secs(h.start_period, 0),
        })
    });
    let config = ImageConfig {
        user: nonempty(c.user),
        env: c.env.unwrap_or_default(),
        entrypoint: c.entrypoint,
        cmd: c.cmd,
        working_dir: nonempty(c.working_dir),
        exposed_ports: c.exposed_ports.map(|m| m.into_keys().collect()).unwrap_or_default(),
        volumes: c.volumes.map(|m| m.into_keys().collect()).unwrap_or_default(),
        labels: c.labels.unwrap_or_default(),
        stop_signal: nonempty(c.stop_signal),
        healthcheck,
        shell: c.shell,
    };
    let os = if raw.os.is_empty() { "linux".to_string() } else { raw.os };
    let arch = if raw.architecture.is_empty() { Platform::host().architecture } else { raw.architecture };
    let platform = Platform::new(&os, &arch, raw.variant.as_deref().filter(|v| !v.is_empty()));
    let created = raw.created.as_deref().and_then(timefmt::parse_rfc3339).unwrap_or(0);
    Ok(ParsedConfig { config, diff_ids: raw.rootfs.diff_ids, platform, created, history: raw.history })
}

fn keys_to_map(keys: &[String]) -> Value {
    let mut m = serde_json::Map::new();
    for k in keys {
        m.insert(k.clone(), json!({}));
    }
    Value::Object(m)
}

/// Build the `config` section of an OCI image config from an `ImageConfig`.
pub fn config_to_json(c: &ImageConfig) -> Value {
    let mut m = serde_json::Map::new();
    if let Some(u) = &c.user {
        m.insert("User".into(), json!(u));
    }
    if !c.env.is_empty() {
        m.insert("Env".into(), json!(c.env));
    }
    if let Some(v) = &c.entrypoint {
        m.insert("Entrypoint".into(), json!(v));
    }
    if let Some(v) = &c.cmd {
        m.insert("Cmd".into(), json!(v));
    }
    if let Some(w) = &c.working_dir {
        m.insert("WorkingDir".into(), json!(w));
    }
    if !c.exposed_ports.is_empty() {
        m.insert("ExposedPorts".into(), keys_to_map(&c.exposed_ports));
    }
    if !c.volumes.is_empty() {
        m.insert("Volumes".into(), keys_to_map(&c.volumes));
    }
    if !c.labels.is_empty() {
        m.insert("Labels".into(), json!(c.labels));
    }
    if let Some(s) = &c.stop_signal {
        m.insert("StopSignal".into(), json!(s));
    }
    if let Some(h) = &c.healthcheck {
        let ns = 1_000_000_000u64;
        m.insert(
            "Healthcheck".into(),
            json!({
                "Test": h.test,
                "Interval": h.interval_secs * ns,
                "Timeout": h.timeout_secs * ns,
                "StartPeriod": h.start_period_secs * ns,
                "Retries": h.retries,
            }),
        );
    }
    if let Some(s) = &c.shell {
        m.insert("Shell".into(), json!(s));
    }
    Value::Object(m)
}

/// Serialize a complete OCI image config.
pub fn build_config_blob(
    config: &ImageConfig,
    platform: &Platform,
    created: i64,
    diff_ids: &[String],
    history: &[Value],
) -> Vec<u8> {
    let mut v = json!({
        "architecture": platform.architecture,
        "os": platform.os,
        "created": timefmt::format_rfc3339(created),
        "config": config_to_json(config),
        "rootfs": { "type": "layers", "diff_ids": diff_ids },
        "history": history,
    });
    if let Some(var) = &platform.variant {
        v["variant"] = json!(var);
    }
    serde_json::to_vec(&v).expect("config serializes")
}

/// Build an OCI image manifest document.
pub fn build_manifest(config: &Descriptor, layers: &[Descriptor]) -> Vec<u8> {
    let strip = |d: &Descriptor| json!({"mediaType": d.media_type, "digest": d.digest, "size": d.size});
    let v = json!({
        "schemaVersion": 2,
        "mediaType": OCI_MANIFEST,
        "config": strip(config),
        "layers": layers.iter().map(strip).collect::<Vec<_>>(),
    });
    serde_json::to_vec(&v).expect("manifest serializes")
}

/// Normalize a layer media type to its OCI equivalent.
pub fn oci_layer_type(mt: &str) -> &'static str {
    if mt.contains("zstd") {
        OCI_LAYER_ZSTD
    } else if mt.contains("gzip") {
        OCI_LAYER_GZIP
    } else {
        OCI_LAYER_TAR
    }
}

/// `sha256:<hex>` of some bytes.
pub fn sha256_digest(data: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!("sha256:{}", hex::encode(Sha256::digest(data)))
}

/// Hex part of a `sha256:` digest, validated to be safe as a file name.
pub fn digest_hex(digest: &str) -> Result<&str> {
    let h = digest
        .strip_prefix("sha256:")
        .ok_or_else(|| Error::Unsupported(format!("unsupported digest algorithm in '{digest}'")))?;
    if h.len() != 64 || !h.bytes().all(|c| c.is_ascii_hexdigit()) {
        return Err(Error::Invalid(format!("malformed digest '{digest}'")));
    }
    Ok(h)
}

/// Repository path shown in progress messages.
pub fn repo_display(r: &ImageRef) -> String {
    if r.registry == hullr_core::reference::DOCKER_HUB {
        r.repository.clone()
    } else {
        format!("{}/{}", r.registry, r.repository)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn desc(os: &str, arch: &str, variant: Option<&str>, digest: &str) -> Descriptor {
        Descriptor {
            media_type: OCI_MANIFEST.into(),
            digest: digest.into(),
            size: 1,
            platform: Some(PlatformJson { os: os.into(), architecture: arch.into(), variant: variant.map(Into::into) }),
            annotations: None,
        }
    }

    #[test]
    fn parses_docker_manifest() {
        let body = br#"{"schemaVersion":2,"mediaType":"application/vnd.docker.distribution.manifest.v2+json",
          "config":{"mediaType":"application/vnd.docker.container.image.v1+json","size":10,"digest":"sha256:aa"},
          "layers":[{"mediaType":"application/vnd.docker.image.rootfs.diff.tar.gzip","size":5,"digest":"sha256:bb"}]}"#;
        match parse_manifest(body, None).unwrap() {
            Manifest::Image(m) => {
                assert_eq!(m.config.digest, "sha256:aa");
                assert_eq!(m.layers.len(), 1);
            }
            _ => panic!("expected image manifest"),
        }
    }

    #[test]
    fn parses_oci_index_without_media_type() {
        let body = br#"{"schemaVersion":2,"manifests":[
          {"mediaType":"application/vnd.oci.image.manifest.v1+json","digest":"sha256:a","size":1,
           "platform":{"architecture":"amd64","os":"linux"}}]}"#;
        match parse_manifest(body, Some("application/vnd.oci.image.index.v1+json; charset=utf-8")).unwrap() {
            Manifest::Index(m) => assert_eq!(m.len(), 1),
            _ => panic!("expected index"),
        }
    }

    #[test]
    fn rejects_schema1() {
        let body = br#"{"schemaVersion":1,"name":"x","fsLayers":[]}"#;
        assert!(matches!(parse_manifest(body, None), Err(Error::Unsupported(_))));
    }

    #[test]
    fn selects_platform_and_skips_attestations() {
        let entries = vec![
            desc("unknown", "unknown", None, "sha256:att"),
            desc("linux", "amd64", None, "sha256:amd"),
            desc("linux", "arm64", Some("v8"), "sha256:arm"),
            desc("linux", "arm", Some("v6"), "sha256:v6"),
            desc("linux", "arm", Some("v7"), "sha256:v7"),
        ];
        let pick = |s: &str| select_platform(&entries, &Platform::parse(s).unwrap()).map(|d| d.digest.clone());
        assert_eq!(pick("linux/amd64").as_deref(), Some("sha256:amd"));
        assert_eq!(pick("linux/arm64").as_deref(), Some("sha256:arm"));
        assert_eq!(pick("linux/arm").as_deref(), Some("sha256:v7"));
        assert_eq!(pick("linux/arm/v6").as_deref(), Some("sha256:v6"));
        assert_eq!(pick("linux/riscv64"), None);
        assert!(available_platforms(&entries).contains("linux/arm64/v8"));
    }

    #[test]
    fn maps_config() {
        let blob = br#"{"architecture":"arm64","os":"linux","variant":"v8","created":"2024-05-30T23:53:14Z",
          "config":{"User":"1000:1000","Env":["A=1"],"Cmd":["sh"],"Entrypoint":null,"WorkingDir":"/app",
            "ExposedPorts":{"80/tcp":{},"53/udp":{}},"Volumes":{"/data":{}},"Labels":{"a":"b"},
            "StopSignal":"SIGQUIT","Shell":["/bin/bash","-c"],
            "Healthcheck":{"Test":["CMD-SHELL","true"],"Interval":5000000000,"Timeout":0,"Retries":0,"StartPeriod":2000000000}},
          "rootfs":{"type":"layers","diff_ids":["sha256:1","sha256:2"]},"history":[{"created_by":"x"}]}"#;
        let p = parse_config(blob).unwrap();
        assert_eq!(p.platform, Platform::new("linux", "arm64", Some("v8")));
        assert_eq!(p.created, 1717113194);
        assert_eq!(p.diff_ids.len(), 2);
        assert_eq!(p.config.user.as_deref(), Some("1000:1000"));
        assert_eq!(p.config.entrypoint, None);
        assert_eq!(p.config.cmd, Some(vec!["sh".to_string()]));
        assert_eq!(p.config.exposed_ports, vec!["53/udp", "80/tcp"]);
        assert_eq!(p.config.volumes, vec!["/data"]);
        let h = p.config.healthcheck.unwrap();
        assert_eq!((h.interval_secs, h.timeout_secs, h.retries, h.start_period_secs), (5, 30, 3, 2));
        assert_eq!(p.history.len(), 1);
    }

    #[test]
    fn config_round_trip() {
        let mut cfg = ImageConfig::default();
        cfg.env = vec!["PATH=/bin".into()];
        cfg.cmd = Some(vec!["sh".into()]);
        cfg.exposed_ports = vec!["8080/tcp".into()];
        cfg.labels.insert("k".into(), "v".into());
        cfg.healthcheck = Some(HealthCheck {
            test: vec!["CMD".into(), "true".into()],
            interval_secs: 10,
            timeout_secs: 3,
            retries: 5,
            start_period_secs: 1,
        });
        let plat = Platform::new("linux", "amd64", None);
        let blob = build_config_blob(&cfg, &plat, 1717113194, &["sha256:x".to_string()], &[]);
        let back = parse_config(&blob).unwrap();
        assert_eq!(back.config, cfg);
        assert_eq!(back.platform, plat);
        assert_eq!(back.created, 1717113194);
        assert_eq!(back.diff_ids, vec!["sha256:x"]);
    }

    #[test]
    fn digest_validation() {
        assert!(digest_hex("sha256:../../etc").is_err());
        assert!(digest_hex(&format!("sha256:{}", "a".repeat(64))).is_ok());
        assert!(digest_hex("sha512:abc").is_err());
    }
}
