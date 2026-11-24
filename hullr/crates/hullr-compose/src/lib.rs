//! Compose file loading. INTERFACE STUB.

use hullr_core::{ComposeOptions, ContainerSpec, HealthCheck, Mount, PortMapping, RestartPolicy, Result};
use indexmap::IndexMap;
use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq)]
pub struct Project {
    pub name: String,
    pub dir: PathBuf,
    pub services: IndexMap<String, Service>,
    /// Declared top-level named volumes (unprefixed name -> external flag).
    pub volumes: IndexMap<String, VolumeDef>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct VolumeDef {
    pub external: bool,
    /// Explicit `name:`; otherwise `<project>_<key>`.
    pub name: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BuildDef {
    pub context: PathBuf,
    pub dockerfile: Option<PathBuf>,
    pub args: BTreeMap<String, String>,
    pub target: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DependsCondition {
    Started,
    Healthy,
    CompletedSuccessfully,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dependency {
    pub service: String,
    pub condition: DependsCondition,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Service {
    pub name: String,
    pub image: Option<String>,
    pub build: Option<BuildDef>,
    pub container_name: Option<String>,
    pub command: Option<Vec<String>>,
    pub entrypoint: Option<Vec<String>>,
    /// `KEY=VALUE`, env_file entries already merged in (environment wins).
    pub environment: Vec<String>,
    pub ports: Vec<PortMapping>,
    /// Resolved: bind sources absolute, named volume sources are the full
    /// volume name (`<project>_<name>` unless external / explicit name).
    pub volumes: Vec<Mount>,
    pub depends_on: Vec<Dependency>,
    pub healthcheck: Option<HealthCheck>,
    pub restart: RestartPolicy,
    pub working_dir: Option<String>,
    pub user: Option<String>,
    pub hostname: Option<String>,
    pub platform: Option<String>,
    pub labels: BTreeMap<String, String>,
    pub tty: bool,
    pub stdin_open: bool,
    pub extra_hosts: Vec<(String, String)>,
    pub profiles: Vec<String>,
    pub mem_limit: Option<u64>,
    pub cpus: Option<f64>,
}

/// Load, merge (override files) and interpolate per `opts`.
pub fn load(_opts: &ComposeOptions) -> Result<Project> {
    todo!()
}

impl Project {
    /// Topological start order of `only` (all if empty) plus their deps.
    pub fn start_order(&self, _only: &[String]) -> Result<Vec<String>> {
        todo!()
    }
    /// Image name to run/build for a service (`image:` or `<project>-<svc>`).
    pub fn image_name(&self, _service: &str) -> String {
        todo!()
    }
    /// Container name: `container_name` or `<project>-<svc>-1`.
    pub fn container_name(&self, _service: &str) -> String {
        todo!()
    }
    /// The ContainerSpec for a service: network = Sandbox(project name),
    /// extra_hosts maps every service name to 127.0.0.1, compose labels set.
    pub fn container_spec(&self, _service: &str) -> Result<ContainerSpec> {
        todo!()
    }
    /// Stable hash of a service's effective config (for recreate detection).
    pub fn config_hash(&self, _service: &str) -> String {
        todo!()
    }
    /// All published ports across services (for the shared sandbox).
    pub fn all_ports(&self) -> Vec<PortMapping> {
        todo!()
    }
}
