//! Named volumes: `<root>/volumes/<name>/{data/, meta.json}`.

use hullr_core::paths::{write_atomic, Paths};
use hullr_core::*;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

fn valid(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && name.chars().all(|c| c.is_ascii_alphanumeric() || "_.-".contains(c))
        && !name.starts_with('.')
}

pub fn data_dir(paths: &Paths, name: &str) -> PathBuf {
    paths.volumes().join(name).join("data")
}

/// Create if missing. Returns the volume and whether it was just created.
pub fn ensure(paths: &Paths, name: &str, labels: BTreeMap<String, String>) -> Result<(VolumeInfo, bool)> {
    if !valid(name) {
        return Err(Error::Invalid(format!("bad volume name '{name}'")));
    }
    let dir = paths.volumes().join(name);
    let meta = dir.join("meta.json");
    if let Ok(b) = std::fs::read(&meta) {
        if let Ok(v) = serde_json::from_slice::<VolumeInfo>(&b) {
            return Ok((v, false));
        }
    }
    std::fs::create_dir_all(dir.join("data"))?;
    let v = VolumeInfo { name: name.to_string(), path: dir.join("data"), created: now_unix(), labels };
    write_atomic(&meta, &serde_json::to_vec_pretty(&v)?)?;
    Ok((v, true))
}

pub fn list(paths: &Paths) -> Result<Vec<VolumeInfo>> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(paths.volumes()) else { return Ok(out) };
    for e in rd.flatten() {
        if let Ok(b) = std::fs::read(e.path().join("meta.json")) {
            if let Ok(v) = serde_json::from_slice::<VolumeInfo>(&b) {
                out.push(v);
            }
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

pub fn remove(paths: &Paths, name: &str) -> Result<()> {
    if !valid(name) {
        return Err(Error::Invalid(format!("bad volume name '{name}'")));
    }
    let dir = paths.volumes().join(name);
    if !dir.exists() {
        return Err(Error::NotFound(format!("no such volume: {name}")));
    }
    std::fs::remove_dir_all(&dir).context(format!("removing volume {name}"))
}

/// Bytes used under `p` (best effort).
pub fn size(p: &Path) -> u64 {
    let mut total = 0;
    let mut stack = vec![p.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let Ok(md) = e.path().symlink_metadata() else { continue };
            if md.is_dir() {
                stack.push(e.path());
            } else {
                total += md.len();
            }
        }
    }
    total
}

/// Docker copies image content at the mount target into a fresh volume.
pub fn copy_up(image_rootfs: &Path, target: &str, volume_data: &Path) {
    let src = image_rootfs.join(target.trim_start_matches('/'));
    if !src.is_dir() {
        return;
    }
    let empty = std::fs::read_dir(volume_data).map(|mut d| d.next().is_none()).unwrap_or(false);
    if !empty {
        return;
    }
    let _ = std::process::Command::new("cp")
        .arg("-a")
        .arg(format!("{}/.", src.display()))
        .arg(volume_data)
        .status();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lifecycle() {
        let t = tempfile::tempdir().unwrap();
        let p = Paths::new(t.path());
        let (v, created) = ensure(&p, "pgdata", Default::default()).unwrap();
        assert!(created && v.path.is_dir());
        assert!(!ensure(&p, "pgdata", Default::default()).unwrap().1);
        assert_eq!(list(&p).unwrap().len(), 1);
        assert!(ensure(&p, "../x", Default::default()).is_err());
        remove(&p, "pgdata").unwrap();
        assert!(matches!(remove(&p, "pgdata"), Err(Error::NotFound(_))));
    }
}
