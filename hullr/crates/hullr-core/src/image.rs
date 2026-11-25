use crate::Platform;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// The runtime-relevant part of an OCI image config (`config` object).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImageConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    #[serde(default)]
    pub env: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entrypoint: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cmd: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub working_dir: Option<String>,
    /// e.g. `80/tcp`
    #[serde(default)]
    pub exposed_ports: Vec<String>,
    #[serde(default)]
    pub volumes: Vec<String>,
    #[serde(default)]
    pub labels: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop_signal: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub healthcheck: Option<HealthCheck>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shell: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HealthCheck {
    /// `["CMD", ...]`, `["CMD-SHELL", "..."]` or `["NONE"]`, as in Docker.
    pub test: Vec<String>,
    #[serde(default = "default_interval")]
    pub interval_secs: u64,
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
    #[serde(default = "default_retries")]
    pub retries: u32,
    #[serde(default)]
    pub start_period_secs: u64,
}

fn default_interval() -> u64 {
    30
}
fn default_timeout() -> u64 {
    30
}
fn default_retries() -> u32 {
    3
}

/// Summary of a stored image, for `images` listings and UIs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImageInfo {
    /// `sha256:<hex>` of the image config blob.
    pub id: String,
    /// Normalized refs, e.g. `docker.io/library/nginx:latest`.
    pub repo_tags: Vec<String>,
    /// Bytes on disk (unpacked).
    pub size: u64,
    /// Unix seconds.
    pub created: i64,
    pub platform: Platform,
    #[serde(default)]
    pub config: ImageConfig,
}

impl ImageInfo {
    pub fn short_id(&self) -> &str {
        let hex = self.id.strip_prefix("sha256:").unwrap_or(&self.id);
        &hex[..hex.len().min(12)]
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuildOptions {
    /// Build context directory (absolute).
    pub context: PathBuf,
    /// Dockerfile path; defaults to `<context>/Dockerfile`.
    #[serde(default)]
    pub dockerfile: Option<PathBuf>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub build_args: BTreeMap<String, String>,
    /// Stop at this stage name.
    #[serde(default)]
    pub target: Option<String>,
    #[serde(default)]
    pub platform: Option<Platform>,
    #[serde(default)]
    pub no_cache: bool,
    /// Always re-pull base images.
    #[serde(default)]
    pub pull: bool,
    #[serde(default)]
    pub labels: BTreeMap<String, String>,
}
