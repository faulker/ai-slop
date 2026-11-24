//! Dockerfile parser + builder. INTERFACE STUB.

use hullr_core::{api::ProgressFn, BuildOptions, LogStream, Platform, Result};
use hullr_image::{ImageStore, StoredImage};
use std::path::Path;

pub mod dockerfile {
    //! Parsed Dockerfile model; `parse` is the entry point.
}

/// What the builder needs from the engine.
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
        output: &(dyn Fn(LogStream, &[u8]) + Send + Sync),
    ) -> Result<i32>;
}

/// Build per `opts`, using `work_dir` (`Paths::build()`) for scratch space.
pub async fn build(_opts: &BuildOptions, _host: &dyn BuildHost, _work_dir: &Path, _progress: ProgressFn) -> Result<StoredImage> {
    todo!()
}
