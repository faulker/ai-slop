//! OCI registry client + content-addressed local image store.
//!
//! Module map:
//! - `registry`: Docker Registry v2 / OCI distribution client (token and basic auth,
//!   blob redirects, digest verification, uploads).
//! - `auth`: credential storage (`auth.json`, `~/.docker/config.json`) and challenge parsing.
//! - `manifest`: manifest/index parsing, platform selection, image config mapping.
//! - `layer`: safe, rootless-friendly tar extraction (gzip, zstd, plain).
//! - `rootfs`: flattening extracted layers with OCI whiteout handling.
//! - `store`: the on-disk layout (documented there), tags, GC and locking.
//! - `pull` / `push`: the async transfer flows.
//!
//! `ImageStore` is the public facade used by hullr-engine and hullr-build.

mod auth;
mod fsutil;
mod layer;
pub mod manifest;
mod pull;
mod push;
pub mod registry;
pub mod rootfs;
pub mod store;
mod timefmt;

use hullr_core::{api::ProgressFn, Error, ImageConfig, ImageInfo, ImageRef, Platform, Result};
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub use auth::Credentials;

/// A locally stored image.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredImage {
    pub info: ImageInfo,
    /// Uncompressed layer digests (`sha256:..`), base layer first.
    pub diff_ids: Vec<String>,
}

#[derive(Clone)]
pub struct ImageStore {
    inner: Arc<store::Store>,
}

impl ImageStore {
    /// `root` is `Paths::images()`. Creates the layout if missing.
    pub fn open(root: impl Into<PathBuf>) -> Result<Self> {
        Ok(Self { inner: Arc::new(store::Store::open(root)?) })
    }

    /// The store directory.
    pub fn root(&self) -> &Path {
        &self.inner.root
    }

    fn stored(&self, id: &str) -> Result<StoredImage> {
        let rec = self
            .inner
            .load_record(id)?
            .ok_or_else(|| Error::NotFound(format!("no such image: {id}")))?;
        let tags = self.inner.load_tags()?;
        Ok(StoredImage { info: self.inner.info_for(&rec, &tags), diff_ids: rec.diff_ids })
    }

    /// Pull `r` for `platform` (selecting from a manifest list/OCI index),
    /// verify digests, unpack layers, tag it. Docker Hub + GHCR anonymous
    /// token auth, plus credentials from `login`/`~/.docker/config.json`.
    pub async fn pull(&self, r: &ImageRef, platform: &Platform, progress: ProgressFn) -> Result<StoredImage> {
        pull::pull(&self.inner, r, platform, progress).await
    }

    /// Push a local image (by ref) to its registry.
    pub async fn push(&self, r: &ImageRef, progress: ProgressFn) -> Result<()> {
        let img = self.get(&r.to_string())?;
        push::push(&self.inner, r, &img, progress).await
    }

    /// Look up by ref (`nginx`, `ghcr.io/a/b:1`), full id, or unique id
    /// prefix (with or without `sha256:`). `Error::NotFound` if absent.
    /// A ref that matches a tag wins over an id prefix. An ambiguous prefix
    /// is `Error::Invalid`.
    pub fn get(&self, reference: &str) -> Result<StoredImage> {
        match self.inner.resolve(reference)? {
            Some(id) => self.stored(&id),
            None => Err(Error::NotFound(format!("no such image: {reference}"))),
        }
    }

    /// Like `get` but `Ok(None)` when missing, and also `None` if a platform
    /// is given and the stored image's platform does not match.
    pub fn find(&self, reference: &str, platform: Option<&Platform>) -> Result<Option<StoredImage>> {
        let img = match self.get(reference) {
            Ok(i) => i,
            Err(Error::NotFound(_)) => return Ok(None),
            Err(e) => return Err(e),
        };
        if let Some(p) = platform {
            if !p.matches(&img.info.platform) {
                return Ok(None);
            }
        }
        Ok(Some(img))
    }

    /// One entry per image (not per tag), newest first.
    pub fn list(&self) -> Result<Vec<ImageInfo>> {
        let tags = self.inner.load_tags()?;
        let mut v: Vec<ImageInfo> = self.inner.all_records()?.iter().map(|r| self.inner.info_for(r, &tags)).collect();
        v.sort_by(|a, b| b.created.cmp(&a.created).then_with(|| a.id.cmp(&b.id)));
        Ok(v)
    }

    /// Point `r` at image `id` (moving the tag if it existed). `id` may be
    /// anything `get` accepts.
    pub fn tag(&self, id: &str, r: &ImageRef) -> Result<()> {
        self.inner.tag(id, r)
    }

    /// Remove a tag, or the image if referenced by id / last tag.
    /// `in_use(image_id)` says whether a container uses it (refuse unless force).
    /// Returns removed tags (normalized refs) and deleted image ids
    /// (`sha256:..`) in the order they were removed. Removing by id while the
    /// image carries several tags needs `force`.
    pub fn remove(&self, reference: &str, force: bool, in_use: &dyn Fn(&str) -> bool) -> Result<Vec<String>> {
        self.inner.remove(reference, force, in_use)
    }

    /// Path to the image's flattened root filesystem (all layers applied,
    /// whiteouts resolved). Cached; containers must treat it as read-only.
    /// Files are hardlinked from the extracted layers where possible.
    pub fn rootfs(&self, img: &StoredImage) -> Result<PathBuf> {
        self.inner.rootfs(&img.info.id, &img.diff_ids)
    }

    /// Import an uncompressed layer tar (OCI whiteouts allowed) produced by
    /// the builder. Returns its diff_id.
    pub fn add_layer_from_tar(&self, tar_path: &Path) -> Result<String> {
        self.inner.import_layer_tar(tar_path)
    }

    /// Create a new image from `parent`'s layers + `new_layers` with `config`.
    pub fn commit(
        &self,
        parent: Option<&StoredImage>,
        new_layers: &[String],
        config: ImageConfig,
        platform: Platform,
        tags: &[ImageRef],
    ) -> Result<StoredImage> {
        let parent_part = match parent {
            Some(p) => {
                let rec = self
                    .inner
                    .load_record(&p.info.id)?
                    .ok_or_else(|| Error::NotFound(format!("parent image {} is gone", p.info.id)))?;
                let history = self
                    .inner
                    .read_blob(&rec.id)
                    .ok()
                    .and_then(|b| manifest::parse_config(&b).ok())
                    .map(|c| c.history)
                    .unwrap_or_default();
                Some((rec, history))
            }
            None => None,
        };
        let rec = self.inner.commit(
            parent_part.as_ref().map(|(r, h)| (r, h.clone())),
            new_layers,
            config,
            platform,
            tags,
        )?;
        self.stored(&rec.id)
    }

    /// Build cache: key -> image id. Entries whose image was removed read as `None`.
    pub fn cache_get(&self, key: &str) -> Option<String> {
        self.inner.cache_get(key)
    }
    pub fn cache_put(&self, key: &str, image_id: &str) -> Result<()> {
        self.inner.cache_put(key, image_id)
    }

    /// Delete dangling (untagged) images, or all unused with `all`.
    /// Returns removed ids and bytes freed.
    pub fn prune(&self, all: bool, in_use: &dyn Fn(&str) -> bool) -> Result<(Vec<String>, u64)> {
        self.inner.prune(all, in_use)
    }

    /// Verify credentials against the registry, then save them.
    pub async fn login(&self, registry: &str, username: &str, password: &str) -> Result<()> {
        if username.is_empty() || password.is_empty() {
            return Err(Error::Invalid("username and password are required".into()));
        }
        let creds = auth::Credentials { username: username.to_string(), password: password.to_string() };
        registry::Registry::for_login(registry, Some(creds.clone()))?.check_login().await?;
        let inner = self.inner.clone();
        let registry = registry.to_string();
        tokio::task::spawn_blocking(move || {
            let _g = inner.lock()?;
            auth::save(&inner.root, &registry, &creds)
        })
        .await
        .map_err(|e| Error::Other(format!("join: {e}")))?
    }
}

#[cfg(test)]
mod tests;
