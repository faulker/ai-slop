//! Container records: what the user asked for, persisted per container.

use hullr_core::paths::{write_atomic, Paths};
use hullr_core::*;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Record {
    pub id: String,
    pub name: String,
    pub image: String,
    pub image_id: String,
    pub command: Vec<String>,
    pub created: i64,
    pub platform: Platform,
    pub spec: ContainerSpec,
    /// Effective healthcheck (spec or image), if any.
    #[serde(default)]
    pub healthcheck: Option<HealthCheck>,
    #[serde(default)]
    pub health: Option<Health>,
    /// Volume names this container mounts (named + anonymous).
    #[serde(default)]
    pub volumes: Vec<String>,
}

impl Record {
    pub fn volume_names(&self) -> Vec<String> {
        self.volumes.clone()
    }
    pub fn label(&self, k: &str) -> Option<&str> {
        self.spec.labels.get(k).map(String::as_str)
    }
}

pub fn dir(paths: &Paths) -> PathBuf {
    paths.engine().join("containers")
}

fn file(paths: &Paths, id: &str) -> PathBuf {
    dir(paths).join(format!("{id}.json"))
}

pub fn save(paths: &Paths, r: &Record) -> Result<()> {
    let data = serde_json::to_vec_pretty(r)?;
    write_atomic(&file(paths, &r.id), &data).context("saving container record")
}

pub fn delete(paths: &Paths, id: &str) -> Result<()> {
    match std::fs::remove_file(file(paths, id)) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}

pub fn list(paths: &Paths) -> Result<Vec<Record>> {
    let mut out = Vec::new();
    let rd = match std::fs::read_dir(dir(paths)) {
        Ok(rd) => rd,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(out),
        Err(e) => return Err(e.into()),
    };
    for ent in rd.flatten() {
        let p = ent.path();
        if p.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        // A record being written or corrupt is skipped, never fatal.
        if let Ok(b) = std::fs::read(&p) {
            if let Ok(r) = serde_json::from_slice::<Record>(&b) {
                out.push(r);
            }
        }
    }
    Ok(out)
}

/// Exact id, exact name, then unique id prefix.
pub fn find(paths: &Paths, key: &str) -> Result<Record> {
    let key = key.trim().trim_start_matches('/');
    if key.is_empty() {
        return Err(Error::Invalid("empty container reference".into()));
    }
    let all = list(paths)?;
    if let Some(r) = all.iter().find(|r| r.id == key || r.name == key) {
        return Ok(r.clone());
    }
    let matches: Vec<&Record> = all.iter().filter(|r| r.id.starts_with(key)).collect();
    match matches.len() {
        1 => Ok(matches[0].clone()),
        0 => Err(Error::NotFound(format!("no such container: {key}"))),
        _ => Err(Error::Invalid(format!("container reference '{key}' is ambiguous"))),
    }
}

pub fn set_health(paths: &Paths, id: &str, health: Option<Health>) -> Result<()> {
    let mut r = find(paths, id)?;
    if r.healthcheck.is_none() {
        return Ok(());
    }
    if r.health != health {
        r.health = health;
        save(paths, &r)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(id: &str, name: &str) -> Record {
        Record {
            id: id.into(),
            name: name.into(),
            image: "alpine".into(),
            image_id: "sha256:x".into(),
            command: vec!["sh".into()],
            created: 1,
            platform: Platform::host(),
            spec: ContainerSpec { image: "alpine".into(), ..Default::default() },
            healthcheck: None,
            health: None,
            volumes: vec![],
        }
    }

    #[test]
    fn find_rules() {
        let t = tempfile::tempdir().unwrap();
        let p = Paths::new(t.path());
        std::fs::create_dir_all(dir(&p)).unwrap();
        save(&p, &rec("abc123", "web")).unwrap();
        save(&p, &rec("abd456", "db")).unwrap();
        assert_eq!(find(&p, "web").unwrap().id, "abc123");
        assert_eq!(find(&p, "abd").unwrap().name, "db");
        assert!(matches!(find(&p, "ab"), Err(Error::Invalid(_))));
        assert!(matches!(find(&p, "zzz"), Err(Error::NotFound(_))));
        delete(&p, "abc123").unwrap();
        assert_eq!(list(&p).unwrap().len(), 1);
    }
}
