//! Pulling images: resolve the manifest, download and extract layers
//! (up to three at a time), then register the image and its tag.

use crate::auth;
use crate::manifest::{self, Descriptor};
use crate::registry::Registry;
use crate::store::{ImageRecord, LayerMeta, Store};
use crate::{fsutil, layer, StoredImage};
use futures::{StreamExt, TryStreamExt};
use hullr_core::api::{Progress, ProgressFn};
use hullr_core::{Error, ImageRef, Platform, Result};
use std::sync::Arc;
use std::time::{Duration, Instant};

const CONCURRENT_LAYERS: usize = 3;
const PROGRESS_INTERVAL: Duration = Duration::from_millis(100);

/// Short layer id shown in progress output (12 hex chars of the digest).
pub fn short_id(digest: &str) -> String {
    digest.strip_prefix("sha256:").unwrap_or(digest).chars().take(12).collect()
}

pub async fn pull(store: &Arc<Store>, r: &ImageRef, platform: &Platform, progress: ProgressFn) -> Result<StoredImage> {
    let creds = auth::lookup(&store.root, &r.registry);
    let reg = Registry::new(r, creds, false)?;
    progress(Progress::Status { message: format!("Pulling from {}", manifest::repo_display(r)) });

    let resolved = reg.resolve(r.manifest_reference(), platform).await?;
    progress(Progress::Status { message: format!("Digest: {}", resolved.digest) });

    let cfg = manifest::parse_config(&resolved.config_blob)?;
    let layers = &resolved.manifest.layers;
    if cfg.diff_ids.len() != layers.len() {
        return Err(Error::Registry(format!(
            "manifest has {} layers but config lists {} diff_ids",
            layers.len(),
            cfg.diff_ids.len()
        )));
    }
    store.put_blob(&resolved.config_blob)?;

    let reg = &reg;
    let results: Vec<LayerMeta> = futures::stream::iter(layers.iter().cloned().zip(cfg.diff_ids.iter().cloned()))
        .map(|(desc, diff)| {
            let store = store.clone();
            let progress = progress.clone();
            async move { ensure_layer(&store, reg, desc, diff, progress).await }
        })
        .buffered(CONCURRENT_LAYERS)
        .try_collect()
        .await?;

    let size = results.iter().map(|m| m.unpacked).sum();
    let rec = ImageRecord {
        id: resolved.config_digest.clone(),
        config: cfg.config,
        diff_ids: cfg.diff_ids,
        platform: cfg.platform,
        created: cfg.created,
        size,
    };
    let tag_key = if r.tag.is_some() || r.digest.is_some() { vec![r.to_string()] } else { vec![] };
    let previous = tag_key.first().and_then(|k| store.load_tags().ok().and_then(|t| t.get(k).cloned()));
    let store2 = store.clone();
    let rec2 = rec.clone();
    tokio::task::spawn_blocking(move || store2.register_image(&rec2, &tag_key))
        .await
        .map_err(|e| Error::Other(format!("join: {e}")))??;
    let msg = if previous.as_deref() != Some(rec.id.as_str()) {
        format!("Status: Downloaded newer image for {r}")
    } else {
        format!("Status: Image is up to date for {r}")
    };
    progress(Progress::Status { message: msg });

    let tags = store.load_tags()?;
    let info = store.info_for(&rec, &tags);
    Ok(StoredImage { info, diff_ids: rec.diff_ids })
}

/// Make sure one layer is downloaded, verified and extracted.
async fn ensure_layer(
    store: &Arc<Store>,
    reg: &Registry,
    desc: Descriptor,
    diff_id: String,
    progress: ProgressFn,
) -> Result<LayerMeta> {
    let id = short_id(&desc.digest);
    if store.layer_ready(&diff_id) {
        store.touch_layer(&diff_id);
        progress(Progress::Layer { id, current: desc.size, total: desc.size, done: true });
        return Ok(store.layer_meta(&diff_id).expect("ready layer has metadata"));
    }

    let blob = store.blob_path(&desc.digest)?;
    if !blob.is_file() {
        progress(Progress::Layer { id: id.clone(), current: 0, total: desc.size, done: false });
        let tmp = store.tmp_path("dl");
        let mut last = Instant::now() - PROGRESS_INTERVAL;
        let (p, lid, size) = (progress.clone(), id.clone(), desc.size);
        let mut cb = move |got: u64, len: u64| {
            if last.elapsed() >= PROGRESS_INTERVAL {
                last = Instant::now();
                let total = if size > 0 { size } else { len };
                p(Progress::Layer { id: lid.clone(), current: got, total, done: false });
            }
        };
        if let Err(e) = reg.download_blob(&desc.digest, &tmp, &mut cb).await {
            let _ = tokio::fs::remove_file(&tmp).await;
            return Err(e);
        }
        store.install_blob(&tmp, &desc.digest)?;
    }

    let st = store.clone();
    let (d2, diff2, mt) = (desc.clone(), diff_id.clone(), desc.media_type.clone());
    let meta = tokio::task::spawn_blocking(move || -> Result<LayerMeta> {
        let tmp_dir = st.tmp_path("layer");
        std::fs::create_dir(&tmp_dir)?;
        let blob = st.blob_path(&d2.digest)?;
        let ex = match layer::extract_layer(&blob, &mt, &tmp_dir, None) {
            Ok(e) => e,
            Err(e) => {
                let _ = fsutil::remove_tree(&tmp_dir);
                return Err(e);
            }
        };
        if ex.diff_id != diff2 {
            let _ = fsutil::remove_tree(&tmp_dir);
            return Err(Error::Registry(format!(
                "layer {} diff_id mismatch: config says {diff2}, content is {}",
                d2.digest, ex.diff_id
            )));
        }
        let meta = LayerMeta {
            diff_id: diff2,
            digest: d2.digest.clone(),
            size: d2.size,
            media_type: d2.media_type.clone(),
            unpacked: ex.unpacked,
        };
        st.install_layer(&tmp_dir, &meta)?;
        Ok(meta)
    })
    .await
    .map_err(|e| Error::Other(format!("join: {e}")))??;

    progress(Progress::Layer { id, current: desc.size, total: desc.size, done: true });
    Ok(meta)
}
