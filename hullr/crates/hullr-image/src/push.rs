//! Pushing a local image to its registry as an OCI manifest.
//!
//! Layer blobs are the compressed blobs kept in the store (the originals for
//! pulled layers, gzip for layers created by builds), so digests match what
//! the registry already knows.

use crate::auth;
use crate::manifest::{self, Descriptor};
use crate::pull::short_id;
use crate::registry::{descriptor, Registry};
use crate::store::Store;
use crate::StoredImage;
use futures::{StreamExt, TryStreamExt};
use hullr_core::api::{Progress, ProgressFn};
use hullr_core::{Error, ImageRef, Result};
use std::sync::Arc;

pub async fn push(store: &Arc<Store>, r: &ImageRef, img: &StoredImage, progress: ProgressFn) -> Result<()> {
    let creds = auth::lookup(&store.root, &r.registry);
    let reg = Registry::new(r, creds, true)?;
    progress(Progress::Status {
        message: format!("The push refers to repository [{}/{}]", r.registry, r.repository),
    });

    let mut layers: Vec<(Descriptor, std::path::PathBuf)> = Vec::new();
    for d in &img.diff_ids {
        let meta = store
            .layer_meta(d)
            .ok_or_else(|| Error::NotFound(format!("layer {d} is missing from the store")))?;
        let path = store.blob_path(&meta.digest)?;
        if !path.is_file() {
            return Err(Error::NotFound(format!("blob {} for layer {d} is missing", meta.digest)));
        }
        layers.push((descriptor(manifest::oci_layer_type(&meta.media_type), &meta.digest, meta.size), path));
    }

    let reg_ref = &reg;
    futures::stream::iter(layers.iter().cloned())
        .map(|(desc, path)| {
            let progress = progress.clone();
            async move {
                let id = short_id(&desc.digest);
                if reg_ref.blob_exists(&desc.digest).await? {
                    progress(Progress::Layer { id, current: desc.size, total: desc.size, done: true });
                    return Ok::<(), Error>(());
                }
                progress(Progress::Layer { id: id.clone(), current: 0, total: desc.size, done: false });
                let (p2, id2, total) = (progress.clone(), id.clone(), desc.size);
                let last = std::sync::Mutex::new(std::time::Instant::now() - std::time::Duration::from_secs(1));
                let cb: Arc<dyn Fn(u64) + Send + Sync> = Arc::new(move |sent| {
                    let mut l = last.lock().unwrap();
                    if l.elapsed() >= std::time::Duration::from_millis(100) {
                        *l = std::time::Instant::now();
                        p2(Progress::Layer { id: id2.clone(), current: sent, total, done: false });
                    }
                });
                reg_ref.upload_blob(&desc.digest, &path, desc.size, cb).await?;
                progress(Progress::Layer { id, current: desc.size, total: desc.size, done: true });
                Ok(())
            }
        })
        .buffer_unordered(3)
        .try_collect::<Vec<()>>()
        .await?;

    let config_blob = store.read_blob(&img.info.id)?;
    if !reg.blob_exists(&img.info.id).await? {
        reg.upload_bytes(&img.info.id, config_blob.clone()).await?;
    }
    let config_desc = descriptor(manifest::OCI_CONFIG, &img.info.id, config_blob.len() as u64);
    let descs: Vec<Descriptor> = layers.into_iter().map(|(d, _)| d).collect();
    let body = manifest::build_manifest(&config_desc, &descs);
    let digest = manifest::sha256_digest(&body);
    let size = body.len();
    reg.put_manifest(r.manifest_reference(), body, manifest::OCI_MANIFEST).await?;
    progress(Progress::Status {
        message: format!("{}: digest: {digest} size: {size}", r.manifest_reference()),
    });
    Ok(())
}
