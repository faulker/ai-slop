//! OCI registry client + content-addressed local image store.
//!
//! INTERFACE STUB: signatures are the contract used by hullr-engine and
//! hullr-build. Bodies are to be implemented.

use hullr_core::{api::ProgressFn, ImageConfig, ImageInfo, ImageRef, Platform, Result};
use std::path::{Path, PathBuf};

/// A locally stored image.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredImage {
    pub info: ImageInfo,
    /// Uncompressed layer digests (`sha256:..`), base layer first.
    pub diff_ids: Vec<String>,
}

pub struct ImageStore {
    #[allow(dead_code)]
    root: PathBuf,
}

impl ImageStore {
    /// `root` is `Paths::images()`. Creates the layout if missing.
    pub fn open(root: impl Into<PathBuf>) -> Result<Self> {
        Ok(Self { root: root.into() })
    }

    /// Pull `r` for `platform` (selecting from a manifest list/OCI index),
    /// verify digests, unpack layers, tag it. Docker Hub + GHCR anonymous
    /// token auth, plus credentials from `login`/`~/.docker/config.json`.
    pub async fn pull(&self, _r: &ImageRef, _platform: &Platform, _progress: ProgressFn) -> Result<StoredImage> {
        todo!()
    }

    /// Push a local image (by ref) to its registry.
    pub async fn push(&self, _r: &ImageRef, _progress: ProgressFn) -> Result<()> {
        todo!()
    }

    /// Look up by ref (`nginx`, `ghcr.io/a/b:1`), full id, or unique id
    /// prefix (with or without `sha256:`). `Error::NotFound` if absent.
    pub fn get(&self, _reference: &str) -> Result<StoredImage> {
        todo!()
    }

    /// Like `get` but `Ok(None)` when missing, and also `None` if a platform
    /// is given and the stored image's platform does not match.
    pub fn find(&self, _reference: &str, _platform: Option<&Platform>) -> Result<Option<StoredImage>> {
        todo!()
    }

    pub fn list(&self) -> Result<Vec<ImageInfo>> {
        todo!()
    }

    /// Point `r` at image `id` (moving the tag if it existed).
    pub fn tag(&self, _id: &str, _r: &ImageRef) -> Result<()> {
        todo!()
    }

    /// Remove a tag, or the image if referenced by id / last tag.
    /// `in_use(image_id)` says whether a container uses it (refuse unless force).
    /// Returns removed tags and image ids.
    pub fn remove(&self, _reference: &str, _force: bool, _in_use: &dyn Fn(&str) -> bool) -> Result<Vec<String>> {
        todo!()
    }

    /// Path to the image's flattened root filesystem (all layers applied,
    /// whiteouts resolved). Cached; containers must treat it as read-only.
    pub fn rootfs(&self, _img: &StoredImage) -> Result<PathBuf> {
        todo!()
    }

    /// Import an uncompressed layer tar (OCI whiteouts allowed) produced by
    /// the builder. Returns its diff_id.
    pub fn add_layer_from_tar(&self, _tar_path: &Path) -> Result<String> {
        todo!()
    }

    /// Create a new image from `parent`'s layers + `new_layers` with `config`.
    pub fn commit(
        &self,
        _parent: Option<&StoredImage>,
        _new_layers: &[String],
        _config: ImageConfig,
        _platform: Platform,
        _tags: &[ImageRef],
    ) -> Result<StoredImage> {
        todo!()
    }

    /// Build cache: key -> image id.
    pub fn cache_get(&self, _key: &str) -> Option<String> {
        todo!()
    }
    pub fn cache_put(&self, _key: &str, _image_id: &str) -> Result<()> {
        todo!()
    }

    /// Delete dangling (untagged) images, or all unused with `all`.
    /// Returns removed ids and bytes freed.
    pub fn prune(&self, _all: bool, _in_use: &dyn Fn(&str) -> bool) -> Result<(Vec<String>, u64)> {
        todo!()
    }

    /// Save registry credentials.
    pub async fn login(&self, _registry: &str, _username: &str, _password: &str) -> Result<()> {
        todo!()
    }
}
