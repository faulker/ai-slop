//! The narrow interface the builder needs, implemented over [`BuildHost`]
//! in production and by an in-memory fake in tests (so build logic is
//! testable without a real image store).

use hullr_core::{api::ProgressFn, ImageConfig, ImageRef, LogStream, Platform, Result};
use hullr_image::StoredImage;
use std::path::{Path, PathBuf};

use crate::BuildHost;

#[async_trait::async_trait]
pub(crate) trait Backend: Send + Sync {
    fn rootfs(&self, img: &StoredImage) -> Result<PathBuf>;
    fn add_layer_from_tar(&self, tar_path: &Path) -> Result<String>;
    fn commit(
        &self,
        parent: Option<&StoredImage>,
        new_layers: &[String],
        config: ImageConfig,
        platform: Platform,
        tags: &[ImageRef],
    ) -> Result<StoredImage>;
    fn cache_get(&self, key: &str) -> Option<String>;
    fn cache_put(&self, key: &str, image_id: &str) -> Result<()>;
    fn get(&self, reference: &str) -> Result<StoredImage>;
    async fn base_image(
        &self,
        reference: &str,
        platform: &Platform,
        pull_always: bool,
        progress: ProgressFn,
    ) -> Result<StoredImage>;
    #[allow(clippy::too_many_arguments)]
    async fn run(
        &self,
        rootfs: &Path,
        args: Vec<String>,
        env: Vec<String>,
        cwd: String,
        user: Option<String>,
        platform: &Platform,
        output: &(dyn for<'b> Fn(LogStream, &'b [u8]) + Send + Sync),
    ) -> Result<i32>;
    fn id_to_container(&self, uid: u32, gid: u32) -> (u32, u32);
}

/// Production backend: a [`BuildHost`] and its image store.
pub(crate) struct HostBackend<'a>(pub &'a dyn BuildHost);

#[async_trait::async_trait]
impl Backend for HostBackend<'_> {
    fn rootfs(&self, img: &StoredImage) -> Result<PathBuf> {
        self.0.store().rootfs(img)
    }
    fn add_layer_from_tar(&self, p: &Path) -> Result<String> {
        self.0.store().add_layer_from_tar(p)
    }
    fn commit(
        &self,
        parent: Option<&StoredImage>,
        new_layers: &[String],
        config: ImageConfig,
        platform: Platform,
        tags: &[ImageRef],
    ) -> Result<StoredImage> {
        self.0.store().commit(parent, new_layers, config, platform, tags)
    }
    fn cache_get(&self, key: &str) -> Option<String> {
        self.0.store().cache_get(key)
    }
    fn cache_put(&self, key: &str, id: &str) -> Result<()> {
        self.0.store().cache_put(key, id)
    }
    fn get(&self, reference: &str) -> Result<StoredImage> {
        self.0.store().get(reference)
    }
    async fn base_image(&self, reference: &str, platform: &Platform, pull_always: bool, progress: ProgressFn) -> Result<StoredImage> {
        self.0.base_image(reference, platform, pull_always, progress).await
    }
    async fn run(
        &self,
        rootfs: &Path,
        args: Vec<String>,
        env: Vec<String>,
        cwd: String,
        user: Option<String>,
        platform: &Platform,
        output: &(dyn for<'b> Fn(LogStream, &'b [u8]) + Send + Sync),
    ) -> Result<i32> {
        self.0.run(rootfs, args, env, cwd, user, platform, output).await
    }
    fn id_to_container(&self, uid: u32, gid: u32) -> (u32, u32) {
        self.0.id_to_container(uid, gid)
    }
}
