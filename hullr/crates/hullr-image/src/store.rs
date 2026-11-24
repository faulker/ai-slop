//! Content-addressed on-disk image store.
//!
//! ```text
//! <root>/                        (= Paths::images())
//!   lock                         flock(2) target serializing all writers
//!   auth.json                    registry credentials (docker config.json format)
//!   tags.json                    { "docker.io/library/alpine:3.20": "sha256:<image id>" }
//!   cache.json                   build cache: { "<key>": "sha256:<image id>" }
//!   records/<id hex>.json        ImageRecord: config, diff_ids, platform, created, size
//!   blobs/sha256/<hex>           raw blobs by digest: image configs and
//!                                compressed layers (kept so `push` can reuse them)
//!   layers/<diff hex>/           layer extracted verbatim (whiteouts kept as files)
//!   layers/<diff hex>.json       LayerMeta: compressed digest/size/media type, unpacked size
//!   rootfs/<id hex>/             flattened rootfs, built lazily by `rootfs()`
//!   tmp/                         scratch space; renamed into place when complete
//! ```
//!
//! Everything is published by atomic rename, so concurrent readers never see
//! partial data and need no lock. Writers (tag/record/GC changes) hold the
//! exclusive `lock` file. A layer is "ready" once its `layers/<diff>.json`
//! exists (written last). Orphaned layers and blobs younger than the GC grace
//! period are left alone so an in-flight pull or build is not undermined.

use crate::fsutil;
use crate::layer;
use crate::manifest;
use hullr_core::{new_id, now_unix, Error, ImageConfig, ImageInfo, ImageRef, Platform, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::io::{self, Write};
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// Persistent description of one image (the config blob lives in `blobs/`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImageRecord {
    /// `sha256:<hex>` of the config blob.
    pub id: String,
    pub config: ImageConfig,
    pub diff_ids: Vec<String>,
    pub platform: Platform,
    pub created: i64,
    /// Unpacked size in bytes.
    pub size: u64,
}

/// Metadata for an extracted layer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LayerMeta {
    pub diff_id: String,
    /// Digest of the compressed blob in `blobs/`.
    pub digest: String,
    /// Size of the compressed blob.
    pub size: u64,
    pub media_type: String,
    /// Unpacked size in bytes.
    pub unpacked: u64,
}

/// Holds the store's exclusive lock until dropped.
pub struct LockGuard(fs::File);

impl Drop for LockGuard {
    fn drop(&mut self) {
        // SAFETY: fd is valid for the lifetime of the File.
        unsafe {
            libc::flock(self.0.as_raw_fd(), libc::LOCK_UN);
        }
    }
}

pub struct Store {
    pub root: PathBuf,
    /// Orphans younger than this are not garbage collected.
    pub gc_grace: Duration,
}

struct HashingWriter<W> {
    inner: W,
    hasher: Sha256,
    n: u64,
}

impl<W: Write> Write for HashingWriter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let n = self.inner.write(buf)?;
        self.hasher.update(&buf[..n]);
        self.n += n as u64;
        Ok(n)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

fn read_json<T: serde::de::DeserializeOwned>(p: &Path) -> Result<Option<T>> {
    match fs::read(p) {
        Ok(d) => Ok(Some(serde_json::from_slice(&d)?)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

fn write_json<T: Serialize>(p: &Path, v: &T) -> Result<()> {
    hullr_core::paths::write_atomic(p, &serde_json::to_vec(v)?)?;
    Ok(())
}

fn is_hex(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}

impl Store {
    pub fn open(root: impl Into<PathBuf>) -> Result<Self> {
        let s = Self { root: root.into(), gc_grace: Duration::from_secs(3600) };
        for d in ["records", "blobs/sha256", "layers", "rootfs", "tmp"] {
            fs::create_dir_all(s.root.join(d))?;
        }
        Ok(s)
    }

    // ---- paths ----

    pub fn blob_path(&self, digest: &str) -> Result<PathBuf> {
        Ok(self.root.join("blobs/sha256").join(manifest::digest_hex(digest)?))
    }
    pub fn layer_dir(&self, diff_id: &str) -> Result<PathBuf> {
        Ok(self.root.join("layers").join(manifest::digest_hex(diff_id)?))
    }
    fn layer_meta_path(&self, diff_id: &str) -> Result<PathBuf> {
        Ok(self.root.join("layers").join(format!("{}.json", manifest::digest_hex(diff_id)?)))
    }
    fn record_path(&self, id: &str) -> Result<PathBuf> {
        Ok(self.root.join("records").join(format!("{}.json", manifest::digest_hex(id)?)))
    }
    pub fn rootfs_path(&self, id: &str) -> Result<PathBuf> {
        Ok(self.root.join("rootfs").join(manifest::digest_hex(id)?))
    }
    pub fn tmp_path(&self, prefix: &str) -> PathBuf {
        self.root.join("tmp").join(format!("{prefix}-{}", &new_id()[..24]))
    }

    // ---- locking ----

    pub fn lock(&self) -> Result<LockGuard> {
        let f = fs::OpenOptions::new().create(true).truncate(false).write(true).open(self.root.join("lock"))?;
        loop {
            // SAFETY: fd is valid for the lifetime of f.
            let r = unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX) };
            if r == 0 {
                return Ok(LockGuard(f));
            }
            let e = io::Error::last_os_error();
            if e.kind() != io::ErrorKind::Interrupted {
                return Err(e.into());
            }
        }
    }

    // ---- blobs ----

    pub fn has_blob(&self, digest: &str) -> bool {
        self.blob_path(digest).map(|p| p.is_file()).unwrap_or(false)
    }

    /// Store `data` as a blob and return its digest.
    pub fn put_blob(&self, data: &[u8]) -> Result<String> {
        let digest = manifest::sha256_digest(data);
        let dest = self.blob_path(&digest)?;
        if dest.is_file() {
            return Ok(digest);
        }
        let tmp = self.tmp_path("blob");
        fs::write(&tmp, data)?;
        fs::rename(&tmp, &dest)?;
        Ok(digest)
    }

    pub fn read_blob(&self, digest: &str) -> Result<Vec<u8>> {
        Ok(fs::read(self.blob_path(digest)?)?)
    }

    /// Move a verified temp file into the blob store.
    pub fn install_blob(&self, tmp: &Path, digest: &str) -> Result<()> {
        fs::rename(tmp, self.blob_path(digest)?)?;
        Ok(())
    }

    // ---- layers ----

    pub fn layer_meta(&self, diff_id: &str) -> Option<LayerMeta> {
        read_json(&self.layer_meta_path(diff_id).ok()?).ok().flatten()
    }

    /// True when the layer is fully extracted and its compressed blob exists.
    pub fn layer_ready(&self, diff_id: &str) -> bool {
        match (self.layer_meta(diff_id), self.layer_dir(diff_id)) {
            (Some(m), Ok(d)) => d.is_dir() && self.has_blob(&m.digest),
            _ => false,
        }
    }

    /// Mark a layer as recently used so GC leaves it alone.
    pub fn touch_layer(&self, diff_id: &str) {
        if let Ok(p) = self.layer_meta_path(diff_id) {
            let t = filetime::FileTime::from_system_time(SystemTime::now());
            let _ = filetime::set_file_mtime(p, t);
        }
    }

    /// Publish an extracted layer directory (from `tmp`) plus its metadata.
    pub fn install_layer(&self, tmp_dir: &Path, meta: &LayerMeta) -> Result<()> {
        let _g = self.lock()?;
        if self.layer_ready(&meta.diff_id) {
            fsutil::remove_tree(tmp_dir)?;
            self.touch_layer(&meta.diff_id);
            return Ok(());
        }
        let dest = self.layer_dir(&meta.diff_id)?;
        fsutil::remove_tree(&dest)?; // stale half-finished attempt
        fs::rename(tmp_dir, &dest)?;
        write_json(&self.layer_meta_path(&meta.diff_id)?, meta)?;
        Ok(())
    }

    /// Import a layer tar (plain, gzip or zstd), producing a gzip blob and an
    /// extracted directory. Returns the diff_id.
    pub fn import_layer_tar(&self, tar_path: &Path) -> Result<String> {
        let tmp_dir = self.tmp_path("layer");
        let tmp_blob = self.tmp_path("blob");
        let res = (|| -> Result<(layer::Extracted, String, u64)> {
            fs::create_dir(&tmp_dir)?;
            let hw = HashingWriter { inner: fs::File::create(&tmp_blob)?, hasher: Sha256::new(), n: 0 };
            let mut enc = flate2::write::GzEncoder::new(hw, flate2::Compression::default());
            let ex = layer::extract_layer(tar_path, "", &tmp_dir, Some(&mut enc))?;
            let hw = enc.finish()?;
            let digest = format!("sha256:{}", hex::encode(hw.hasher.finalize()));
            Ok((ex, digest, hw.n))
        })();
        let (ex, digest, size) = match res {
            Ok(v) => v,
            Err(e) => {
                let _ = fsutil::remove_tree(&tmp_dir);
                let _ = fs::remove_file(&tmp_blob);
                return Err(e);
            }
        };
        self.install_blob(&tmp_blob, &digest)?;
        let meta = LayerMeta {
            diff_id: ex.diff_id.clone(),
            digest,
            size,
            media_type: manifest::OCI_LAYER_GZIP.to_string(),
            unpacked: ex.unpacked,
        };
        self.install_layer(&tmp_dir, &meta)?;
        Ok(ex.diff_id)
    }

    // ---- records, tags, cache ----

    pub fn load_tags(&self) -> Result<BTreeMap<String, String>> {
        Ok(read_json(&self.root.join("tags.json"))?.unwrap_or_default())
    }
    fn save_tags(&self, t: &BTreeMap<String, String>) -> Result<()> {
        write_json(&self.root.join("tags.json"), t)
    }

    pub fn load_record(&self, id: &str) -> Result<Option<ImageRecord>> {
        read_json(&self.record_path(id)?)
    }

    pub fn record_ids(&self) -> Result<Vec<String>> {
        let mut out = Vec::new();
        for e in fs::read_dir(self.root.join("records"))? {
            let name = e?.file_name().to_string_lossy().to_string();
            if let Some(h) = name.strip_suffix(".json") {
                if h.len() == 64 && is_hex(h) {
                    out.push(format!("sha256:{h}"));
                }
            }
        }
        out.sort();
        Ok(out)
    }

    pub fn all_records(&self) -> Result<Vec<ImageRecord>> {
        let mut v = Vec::new();
        for id in self.record_ids()? {
            if let Some(r) = self.load_record(&id)? {
                v.push(r);
            }
        }
        Ok(v)
    }

    /// Resolve a reference (normalized tag, full id, or id prefix) to an id.
    pub fn resolve(&self, reference: &str) -> Result<Option<String>> {
        let reference = reference.trim();
        let tags = self.load_tags()?;
        if let Ok(r) = ImageRef::parse(reference) {
            if let Some(id) = tags.get(&r.to_string()) {
                return Ok(Some(id.clone()));
            }
        }
        let hex = reference.strip_prefix("sha256:").unwrap_or(reference);
        if is_hex(hex) && hex.len() <= 64 {
            let matches: Vec<String> =
                self.record_ids()?.into_iter().filter(|id| id["sha256:".len()..].starts_with(hex)).collect();
            return match matches.len() {
                0 => Ok(None),
                1 => Ok(Some(matches.into_iter().next().unwrap())),
                _ => Err(Error::Invalid(format!("ambiguous image id prefix '{reference}'"))),
            };
        }
        Ok(None)
    }

    pub fn info_for(&self, rec: &ImageRecord, tags: &BTreeMap<String, String>) -> ImageInfo {
        let mut repo_tags: Vec<String> = tags.iter().filter(|(_, v)| **v == rec.id).map(|(k, _)| k.clone()).collect();
        repo_tags.sort();
        ImageInfo {
            id: rec.id.clone(),
            repo_tags,
            size: rec.size,
            created: rec.created,
            platform: rec.platform.clone(),
            config: rec.config.clone(),
        }
    }

    /// Record a freshly built or pulled image and (optionally) tag it.
    /// Returns true when the record is new.
    pub fn register_image(&self, rec: &ImageRecord, tags: &[String]) -> Result<bool> {
        let _g = self.lock()?;
        for d in &rec.diff_ids {
            if !self.layer_ready(d) {
                return Err(Error::Other(format!("layer {d} disappeared during the operation; retry")));
            }
            self.touch_layer(d);
        }
        let path = self.record_path(&rec.id)?;
        let is_new = !path.exists();
        if is_new {
            write_json(&path, rec)?;
        }
        if !tags.is_empty() {
            let mut t = self.load_tags()?;
            for k in tags {
                t.insert(k.clone(), rec.id.clone());
            }
            self.save_tags(&t)?;
        }
        Ok(is_new)
    }

    pub fn tag(&self, id_or_ref: &str, r: &ImageRef) -> Result<()> {
        let _g = self.lock()?;
        let id = self.resolve(id_or_ref)?.ok_or_else(|| Error::NotFound(format!("no such image: {id_or_ref}")))?;
        let mut t = self.load_tags()?;
        t.insert(r.to_string(), id);
        self.save_tags(&t)
    }

    pub fn cache_get(&self, key: &str) -> Option<String> {
        let m: BTreeMap<String, String> = read_json(&self.root.join("cache.json")).ok().flatten()?;
        let id = m.get(key)?.clone();
        // A cache entry is only valid while its image still exists.
        self.record_path(&id).ok().filter(|p| p.is_file()).map(|_| id)
    }

    pub fn cache_put(&self, key: &str, image_id: &str) -> Result<()> {
        let _g = self.lock()?;
        let p = self.root.join("cache.json");
        let mut m: BTreeMap<String, String> = read_json(&p)?.unwrap_or_default();
        m.insert(key.to_string(), image_id.to_string());
        write_json(&p, &m)
    }

    // ---- commit ----

    /// Write config blob + record for a new image made of existing layers.
    pub fn commit(
        &self,
        parent: Option<(&ImageRecord, Vec<serde_json::Value>)>,
        new_layers: &[String],
        config: ImageConfig,
        platform: Platform,
        tags: &[ImageRef],
    ) -> Result<ImageRecord> {
        let mut diff_ids: Vec<String> = parent.as_ref().map(|(p, _)| p.diff_ids.clone()).unwrap_or_default();
        let mut history: Vec<serde_json::Value> = parent.map(|(_, h)| h).unwrap_or_default();
        let created = now_unix();
        for l in new_layers {
            if !self.layer_ready(l) {
                return Err(Error::NotFound(format!("layer {l} was not imported")));
            }
            diff_ids.push(l.clone());
            history.push(serde_json::json!({
                "created": crate::timefmt::format_rfc3339(created),
                "created_by": "hullr build",
            }));
        }
        let blob = manifest::build_config_blob(&config, &platform, created, &diff_ids, &history);
        let id = self.put_blob(&blob)?;
        let size = diff_ids.iter().filter_map(|d| self.layer_meta(d)).map(|m| m.unpacked).sum();
        let rec = ImageRecord { id, config, diff_ids, platform, created, size };
        let keys: Vec<String> = tags.iter().map(|t| t.to_string()).collect();
        self.register_image(&rec, &keys)?;
        Ok(rec)
    }

    // ---- removal and GC ----

    fn drop_image_locked(&self, id: &str, freed: &mut u64) -> Result<()> {
        let Some(rec) = self.load_record(id)? else { return Ok(()) };
        fs::remove_file(self.record_path(id)?)?;
        let _ = fs::remove_file(self.blob_path(id)?);
        let rootfs = self.rootfs_path(id)?;
        if rootfs.exists() {
            fsutil::remove_tree(&rootfs)?;
        }
        // Forget build-cache entries pointing at this image.
        let cp = self.root.join("cache.json");
        if let Some(mut m) = read_json::<BTreeMap<String, String>>(&cp)? {
            let before = m.len();
            m.retain(|_, v| v != id);
            if m.len() != before {
                write_json(&cp, &m)?;
            }
        }
        self.gc_layers_locked(&rec.diff_ids, freed)
    }

    fn referenced_layers(&self) -> Result<HashSet<String>> {
        Ok(self.all_records()?.into_iter().flat_map(|r| r.diff_ids).collect())
    }

    fn gc_layers_locked(&self, candidates: &[String], freed: &mut u64) -> Result<()> {
        let live = self.referenced_layers()?;
        for d in candidates {
            if live.contains(d) {
                continue;
            }
            self.remove_layer_locked(d, freed)?;
        }
        Ok(())
    }

    fn remove_layer_locked(&self, diff_id: &str, freed: &mut u64) -> Result<()> {
        if let Some(m) = self.layer_meta(diff_id) {
            *freed += m.unpacked + m.size;
            if let Ok(b) = self.blob_path(&m.digest) {
                let _ = fs::remove_file(b);
            }
        }
        fsutil::remove_tree(&self.layer_dir(diff_id)?)?;
        let _ = fs::remove_file(self.layer_meta_path(diff_id)?);
        Ok(())
    }

    fn older_than_grace(&self, p: &Path) -> bool {
        fs::metadata(p)
            .and_then(|m| m.modified())
            .map(|t| t.elapsed().map(|e| e >= self.gc_grace).unwrap_or(false))
            .unwrap_or(true)
    }

    /// Remove orphaned layers, blobs and scratch files older than the grace period.
    fn gc_orphans_locked(&self, freed: &mut u64) -> Result<()> {
        let recs = self.all_records()?;
        let live_layers: HashSet<String> = recs.iter().flat_map(|r| r.diff_ids.clone()).collect();
        let mut seen: HashSet<String> = HashSet::new();
        for e in fs::read_dir(self.root.join("layers"))? {
            let name = e?.file_name().to_string_lossy().to_string();
            let h = name.strip_suffix(".json").unwrap_or(&name);
            if h.len() == 64 && is_hex(h) {
                seen.insert(format!("sha256:{h}"));
            }
        }
        for diff in seen {
            if live_layers.contains(&diff) {
                continue;
            }
            let meta_path = self.layer_meta_path(&diff)?;
            let probe = if meta_path.exists() { meta_path } else { self.layer_dir(&diff)? };
            if self.older_than_grace(&probe) {
                self.remove_layer_locked(&diff, freed)?;
            }
        }
        let mut live_blobs = self.referenced_blob_digests()?;
        live_blobs.extend(recs.iter().map(|r| r.id["sha256:".len()..].to_string()));
        for e in fs::read_dir(self.root.join("blobs/sha256"))? {
            let path = e?.path();
            let name = path.file_name().unwrap().to_string_lossy().to_string();
            if !live_blobs.contains(&name) && self.older_than_grace(&path) {
                *freed += fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                let _ = fs::remove_file(&path);
            }
        }
        for e in fs::read_dir(self.root.join("tmp"))? {
            let path = e?.path();
            if self.older_than_grace(&path) {
                let _ = fsutil::remove_tree(&path);
            }
        }
        Ok(())
    }

    fn referenced_blob_digests(&self) -> Result<HashSet<String>> {
        let mut out = HashSet::new();
        for e in fs::read_dir(self.root.join("layers"))? {
            let path = e?.path();
            if path.extension().and_then(|s| s.to_str()) == Some("json") {
                if let Ok(Some(m)) = read_json::<LayerMeta>(&path) {
                    out.insert(m.digest["sha256:".len()..].to_string());
                }
            }
        }
        Ok(out)
    }

    /// Remove by tag, id or id prefix. See `ImageStore::remove`.
    pub fn remove(&self, reference: &str, force: bool, in_use: &dyn Fn(&str) -> bool) -> Result<Vec<String>> {
        let _g = self.lock()?;
        let mut tags = self.load_tags()?;
        let mut out = Vec::new();
        let mut freed = 0u64;

        let tag_key = ImageRef::parse(reference).ok().map(|r| r.to_string()).filter(|k| tags.contains_key(k));
        if let Some(key) = tag_key {
            let id = tags[&key].clone();
            let others = tags.iter().filter(|(k, v)| **v == id && **k != key).count();
            if others == 0 && in_use(&id) && !force {
                return Err(Error::Conflict(format!("image {reference} is in use by a container")));
            }
            tags.remove(&key);
            self.save_tags(&tags)?;
            out.push(key);
            if others == 0 {
                self.drop_image_locked(&id, &mut freed)?;
                out.push(id);
            }
            return Ok(out);
        }

        let id = self
            .resolve(reference)?
            .ok_or_else(|| Error::NotFound(format!("no such image: {reference}")))?;
        if in_use(&id) && !force {
            return Err(Error::Conflict(format!("image {reference} is in use by a container")));
        }
        let mine: Vec<String> = tags.iter().filter(|(_, v)| **v == id).map(|(k, _)| k.clone()).collect();
        if mine.len() > 1 && !force {
            return Err(Error::Conflict(format!(
                "image {} is referenced in multiple repositories ({}); use force",
                &id[..19.min(id.len())],
                mine.join(", ")
            )));
        }
        for k in mine {
            tags.remove(&k);
            out.push(k);
        }
        self.save_tags(&tags)?;
        self.drop_image_locked(&id, &mut freed)?;
        out.push(id);
        Ok(out)
    }

    /// See `ImageStore::prune`.
    pub fn prune(&self, all: bool, in_use: &dyn Fn(&str) -> bool) -> Result<(Vec<String>, u64)> {
        let _g = self.lock()?;
        let mut tags = self.load_tags()?;
        let mut removed = Vec::new();
        let mut freed = 0u64;
        for rec in self.all_records()? {
            let tagged = tags.values().any(|v| *v == rec.id);
            let candidate = if all { true } else { !tagged };
            if !candidate || in_use(&rec.id) {
                continue;
            }
            tags.retain(|_, v| *v != rec.id);
            self.drop_image_locked(&rec.id, &mut freed)?;
            removed.push(rec.id);
        }
        self.save_tags(&tags)?;
        self.gc_orphans_locked(&mut freed)?;
        Ok((removed, freed))
    }

    // ---- rootfs ----

    /// Flattened rootfs for an image, built on first use.
    pub fn rootfs(&self, id: &str, diff_ids: &[String]) -> Result<PathBuf> {
        let dest = self.rootfs_path(id)?;
        if dest.is_dir() {
            return Ok(dest);
        }
        let mut dirs = Vec::new();
        for d in diff_ids {
            if !self.layer_ready(d) {
                return Err(Error::NotFound(format!("layer {d} of image {id} is missing")));
            }
            dirs.push(self.layer_dir(d)?);
        }
        let tmp = self.tmp_path("rootfs");
        fs::create_dir(&tmp)?;
        fsutil::chmod(&tmp, 0o755)?;
        if let Err(e) = crate::rootfs::flatten(&dirs, &tmp) {
            let _ = fsutil::remove_tree(&tmp);
            return Err(e);
        }
        match fs::rename(&tmp, &dest) {
            Ok(()) => Ok(dest),
            Err(_) if dest.is_dir() => {
                // Another process won the race.
                let _ = fsutil::remove_tree(&tmp);
                Ok(dest)
            }
            Err(e) => {
                let _ = fsutil::remove_tree(&tmp);
                Err(e.into())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer::testutil::*;

    fn store() -> (tempfile::TempDir, Store) {
        let d = tempfile::tempdir().unwrap();
        let mut s = Store::open(d.path().join("images")).unwrap();
        s.gc_grace = Duration::ZERO;
        (d, s)
    }

    fn import(s: &Store, entries: &[E]) -> String {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("l.tar");
        fs::write(&p, build_tar(entries)).unwrap();
        s.import_layer_tar(&p).unwrap()
    }

    fn mk(s: &Store, layers: &[String], tag: &str) -> ImageRecord {
        let cfg = ImageConfig { cmd: Some(vec!["sh".into()]), ..Default::default() };
        let tags: Vec<ImageRef> = if tag.is_empty() { vec![] } else { vec![ImageRef::parse(tag).unwrap()] };
        s.commit(None, layers, cfg, Platform::new("linux", "amd64", None), &tags).unwrap()
    }

    const NOT_IN_USE: &dyn Fn(&str) -> bool = &|_| false;

    #[test]
    fn import_commit_and_resolve() {
        let (_d, s) = store();
        let l = import(&s, &[E::File("a", b"hello", 0o644)]);
        assert!(s.layer_ready(&l));
        let rec = mk(&s, std::slice::from_ref(&l), "myapp:1");
        assert_eq!(rec.size, 5);
        // by tag, by id, by prefix with and without sha256:
        assert_eq!(s.resolve("myapp:1").unwrap().as_deref(), Some(rec.id.as_str()));
        assert_eq!(s.resolve("docker.io/library/myapp:1").unwrap().as_deref(), Some(rec.id.as_str()));
        assert_eq!(s.resolve(&rec.id).unwrap().as_deref(), Some(rec.id.as_str()));
        assert_eq!(s.resolve(&rec.id[7..15]).unwrap().as_deref(), Some(rec.id.as_str()));
        assert_eq!(s.resolve(&rec.id[..15]).unwrap().as_deref(), Some(rec.id.as_str()));
        assert_eq!(s.resolve("nope").unwrap(), None);
        // the config blob is a valid OCI config
        let blob = s.read_blob(&rec.id).unwrap();
        let p = manifest::parse_config(&blob).unwrap();
        assert_eq!(p.diff_ids, vec![l]);
        assert_eq!(manifest::sha256_digest(&blob), rec.id);
    }

    #[test]
    fn ambiguous_prefix_is_invalid() {
        let (_d, s) = store();
        // Fabricate two records sharing a prefix.
        let l = import(&s, &[E::File("a", b"1", 0o644)]);
        let mut r = mk(&s, &[l], "");
        r.id = format!("sha256:ab{}", "0".repeat(62));
        write_json(&s.record_path(&r.id).unwrap(), &r).unwrap();
        r.id = format!("sha256:ab{}", "1".repeat(62));
        write_json(&s.record_path(&r.id).unwrap(), &r).unwrap();
        assert!(matches!(s.resolve("ab"), Err(Error::Invalid(_))));
        assert!(s.resolve("ab0").unwrap().is_some());
    }

    #[test]
    fn tag_moves_and_remove_by_tag_keeps_image_with_other_tags() {
        let (_d, s) = store();
        let l = import(&s, &[E::File("a", b"hello", 0o644)]);
        let rec = mk(&s, std::slice::from_ref(&l), "app:1");
        s.tag(&rec.id, &ImageRef::parse("app:latest").unwrap()).unwrap();
        let out = s.remove("app:1", false, NOT_IN_USE).unwrap();
        assert_eq!(out, vec!["docker.io/library/app:1"]);
        assert!(s.load_record(&rec.id).unwrap().is_some());
        // removing the last tag deletes image and layer
        let out = s.remove("app:latest", false, NOT_IN_USE).unwrap();
        assert_eq!(out, vec!["docker.io/library/app:latest".to_string(), rec.id.clone()]);
        assert!(s.load_record(&rec.id).unwrap().is_none());
        assert!(!s.layer_ready(&l));
        assert!(!s.layer_dir(&l).unwrap().exists());
    }

    #[test]
    fn shared_layers_survive_until_last_image_goes() {
        let (_d, s) = store();
        let base = import(&s, &[E::File("a", b"base", 0o644)]);
        let top = import(&s, &[E::File("b", b"top", 0o644)]);
        let one = mk(&s, std::slice::from_ref(&base), "one");
        let two = mk(&s, &[base.clone(), top.clone()], "two");
        s.remove("one", false, NOT_IN_USE).unwrap();
        assert!(s.layer_ready(&base));
        s.remove(&two.id, false, NOT_IN_USE).unwrap();
        assert!(!s.layer_ready(&base) && !s.layer_ready(&top));
        assert!(s.load_record(&one.id).unwrap().is_none());
    }

    #[test]
    fn remove_refuses_in_use_unless_forced_and_multi_tag_by_id() {
        let (_d, s) = store();
        let l = import(&s, &[E::File("a", b"x", 0o644)]);
        let rec = mk(&s, &[l], "a");
        s.tag(&rec.id, &ImageRef::parse("b").unwrap()).unwrap();
        let busy: &dyn Fn(&str) -> bool = &|_| true;
        // two tags: removing one by tag does not need the image
        s.remove("a", false, busy).unwrap();
        // last tag while in use is refused
        assert!(matches!(s.remove("b", false, busy), Err(Error::Conflict(_))));
        // forced works
        s.remove("b", true, busy).unwrap();
        assert!(matches!(s.remove("b", false, NOT_IN_USE), Err(Error::NotFound(_))));
    }

    #[test]
    fn remove_by_id_with_multiple_tags_needs_force() {
        let (_d, s) = store();
        let l = import(&s, &[E::File("a", b"x", 0o644)]);
        let rec = mk(&s, &[l], "a");
        s.tag(&rec.id, &ImageRef::parse("b").unwrap()).unwrap();
        assert!(matches!(s.remove(&rec.id, false, NOT_IN_USE), Err(Error::Conflict(_))));
        let out = s.remove(&rec.id, true, NOT_IN_USE).unwrap();
        assert_eq!(out.len(), 3);
        assert!(s.load_tags().unwrap().is_empty());
    }

    #[test]
    fn prune_dangling_and_all() {
        let (_d, s) = store();
        let l1 = import(&s, &[E::File("a", b"1", 0o644)]);
        let l2 = import(&s, &[E::File("b", b"2", 0o644)]);
        let tagged = mk(&s, std::slice::from_ref(&l1), "keep");
        let dangling = mk(&s, std::slice::from_ref(&l2), "");
        let (removed, freed) = s.prune(false, NOT_IN_USE).unwrap();
        assert_eq!(removed, vec![dangling.id.clone()]);
        assert!(freed > 0);
        assert!(s.layer_ready(&l1) && !s.layer_ready(&l2));
        // in-use images are skipped even with all
        let busy: &dyn Fn(&str) -> bool = &|id| id == tagged.id;
        assert!(s.prune(true, busy).unwrap().0.is_empty());
        let (removed, _) = s.prune(true, NOT_IN_USE).unwrap();
        assert_eq!(removed, vec![tagged.id]);
        assert!(s.load_tags().unwrap().is_empty());
    }

    #[test]
    fn prune_collects_orphan_layers_after_grace() {
        let (_d, s) = store();
        let l = import(&s, &[E::File("a", b"1", 0o644)]);
        assert!(s.layer_ready(&l));
        s.prune(false, NOT_IN_USE).unwrap();
        assert!(!s.layer_ready(&l));
        assert!(fs::read_dir(s.root.join("blobs/sha256")).unwrap().next().is_none());
    }

    #[test]
    fn grace_period_protects_fresh_orphans() {
        let (_d, mut s) = store();
        s.gc_grace = Duration::from_secs(3600);
        let l = import(&s, &[E::File("a", b"1", 0o644)]);
        s.prune(false, NOT_IN_USE).unwrap();
        assert!(s.layer_ready(&l));
    }

    #[test]
    fn cache_entries_follow_image_lifetime() {
        let (_d, s) = store();
        let l = import(&s, &[E::File("a", b"1", 0o644)]);
        let rec = mk(&s, &[l], "x");
        s.cache_put("k", &rec.id).unwrap();
        assert_eq!(s.cache_get("k").as_deref(), Some(rec.id.as_str()));
        s.remove("x", false, NOT_IN_USE).unwrap();
        assert_eq!(s.cache_get("k"), None);
    }

    #[test]
    fn commit_extends_parent_history_and_layers() {
        let (_d, s) = store();
        let l1 = import(&s, &[E::File("a", b"1", 0o644)]);
        let l2 = import(&s, &[E::File("b", b"2", 0o644)]);
        let parent = mk(&s, std::slice::from_ref(&l1), "p");
        let hist = manifest::parse_config(&s.read_blob(&parent.id).unwrap()).unwrap().history;
        let child = s
            .commit(Some((&parent, hist)), std::slice::from_ref(&l2), ImageConfig::default(), Platform::new("linux", "amd64", None), &[])
            .unwrap();
        assert_eq!(child.diff_ids, vec![l1, l2]);
        let parsed = manifest::parse_config(&s.read_blob(&child.id).unwrap()).unwrap();
        assert_eq!(parsed.history.len(), 2);
        assert_eq!(child.size, 2);
    }

    #[test]
    fn commit_rejects_unknown_layer() {
        let (_d, s) = store();
        let bogus = format!("sha256:{}", "e".repeat(64));
        let r = s.commit(None, &[bogus], ImageConfig::default(), Platform::host(), &[]);
        assert!(matches!(r, Err(Error::NotFound(_))));
    }

    #[test]
    fn rootfs_is_cached_and_whiteout_free() {
        let (_d, s) = store();
        let l1 = import(&s, &[E::Dir("etc/", 0o755), E::File("etc/a", b"1", 0o644), E::File("etc/b", b"2", 0o644)]);
        let l2 = import(&s, &[E::Dir("etc/", 0o755), E::File("etc/.wh.a", b"", 0o644)]);
        let rec = mk(&s, &[l1, l2], "w");
        let p = s.rootfs(&rec.id, &rec.diff_ids).unwrap();
        assert!(p.join("etc/b").exists() && !p.join("etc/a").exists());
        assert!(!p.join("etc/.wh.a").exists());
        assert_eq!(s.rootfs(&rec.id, &rec.diff_ids).unwrap(), p);
        s.remove("w", false, NOT_IN_USE).unwrap();
        assert!(!p.exists());
    }

    #[test]
    fn concurrent_writers_do_not_lose_tags() {
        let (_d, s) = store();
        let s = std::sync::Arc::new(s);
        let l = import(&s, &[E::File("a", b"1", 0o644)]);
        let rec = mk(&s, &[l], "base");
        let mut hs = Vec::new();
        for i in 0..8 {
            let s = s.clone();
            let id = rec.id.clone();
            hs.push(std::thread::spawn(move || {
                s.tag(&id, &ImageRef::parse(&format!("t{i}")).unwrap()).unwrap();
            }));
        }
        for h in hs {
            h.join().unwrap();
        }
        assert_eq!(s.load_tags().unwrap().len(), 9);
    }
}
