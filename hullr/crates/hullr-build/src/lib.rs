//! Dockerfile parser and image builder.
//!
//! * [`dockerfile`]: the parser (`parse`) and model.
//! * [`build`]: executes a Dockerfile against a [`BuildHost`], producing
//!   layers by diffing filesystem snapshots, with a content-addressed
//!   step cache.

use hullr_core::{api::ProgressFn, BuildOptions, LogStream, Platform, Result};
use hullr_image::{ImageStore, StoredImage};
use std::path::Path;

mod backend;
mod builder;
mod copy;
pub mod dockerfile;
mod fsutil;
pub mod ignore;
pub mod lex;
pub mod snapshot;

#[cfg(test)]
mod build_tests;

/// What the builder needs from the engine.
#[allow(clippy::too_many_arguments)]
#[async_trait::async_trait]
pub trait BuildHost: Send + Sync {
    fn store(&self) -> &ImageStore;
    /// Resolve (pulling if missing or `pull_always`) a FROM image.
    async fn base_image(&self, reference: &str, platform: &Platform, pull_always: bool, progress: ProgressFn) -> Result<StoredImage>;
    /// Run a RUN step inside the writable `rootfs` dir. Returns exit code.
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

    /// Map a host uid/gid seen in the build rootfs to the id recorded in the
    /// layer. The default assumes a rootless setup where container root is
    /// the invoking user (so the user's own ids become 0) and an identity
    /// mapping when running as root. Override for other id mappings.
    fn id_to_container(&self, uid: u32, gid: u32) -> (u32, u32) {
        default_id_map(uid, gid)
    }
}

/// See [`BuildHost::id_to_container`].
pub fn default_id_map(uid: u32, gid: u32) -> (u32, u32) {
    // SAFETY: geteuid/getegid have no preconditions and cannot fail.
    let (eu, eg) = unsafe { (libc::geteuid(), libc::getegid()) };
    if eu == 0 {
        (uid, gid)
    } else {
        (if uid == eu { 0 } else { uid }, if gid == eg { 0 } else { gid })
    }
}

/// Build per `opts`, using `work_dir` (`Paths::build()`) for scratch space.
pub async fn build(opts: &BuildOptions, host: &dyn BuildHost, work_dir: &Path, progress: ProgressFn) -> Result<StoredImage> {
    builder::run_build(opts, &backend::HostBackend(host), work_dir, progress).await
}
