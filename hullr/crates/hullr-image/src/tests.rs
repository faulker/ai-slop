//! End-to-end tests: an in-process mock registry (bearer auth, blob
//! redirects to a second host, uploads) plus ignored real-network pulls.

use super::*;
use crate::layer::testutil::*;
use hullr_core::api::{no_progress, Progress};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

#[derive(Default)]
struct Mock {
    blobs: Mutex<HashMap<String, Vec<u8>>>,
    manifests: Mutex<HashMap<String, (String, Vec<u8>)>>,
    cdn_port: Mutex<u16>,
    api_port: Mutex<u16>,
    cdn_saw_auth: Mutex<bool>,
    token_requests: Mutex<u32>,
}

struct Req {
    method: String,
    path: String,
    headers: HashMap<String, String>,
    body: Vec<u8>,
}

async fn read_req(s: &mut tokio::net::TcpStream) -> Option<Req> {
    let mut buf = Vec::new();
    let mut tmp = [0u8; 8192];
    let head_end = loop {
        let n = s.read(&mut tmp).await.ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&tmp[..n]);
        if let Some(p) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break p + 4;
        }
    };
    let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
    let mut lines = head.lines();
    let first = lines.next()?;
    let mut parts = first.split(' ');
    let method = parts.next()?.to_string();
    let path = parts.next()?.to_string();
    let headers: HashMap<String, String> = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string()))
        .collect();
    let len: usize = headers.get("content-length").and_then(|v| v.parse().ok()).unwrap_or(0);
    let mut body = buf[head_end..].to_vec();
    while body.len() < len {
        let n = s.read(&mut tmp).await.ok()?;
        if n == 0 {
            break;
        }
        body.extend_from_slice(&tmp[..n]);
    }
    Some(Req { method, path, headers, body })
}

async fn respond(s: &mut tokio::net::TcpStream, status: &str, headers: &[(&str, String)], body: &[u8], head_only: bool) {
    let mut out = format!("HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n", body.len());
    for (k, v) in headers {
        out.push_str(&format!("{k}: {v}\r\n"));
    }
    out.push_str("\r\n");
    let _ = s.write_all(out.as_bytes()).await;
    if !head_only {
        let _ = s.write_all(body).await;
    }
    let _ = s.shutdown().await;
}

async fn start_mock() -> Arc<Mock> {
    let m = Arc::new(Mock::default());
    let api = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let cdn = TcpListener::bind("127.0.0.1:0").await.unwrap();
    *m.api_port.lock().unwrap() = api.local_addr().unwrap().port();
    *m.cdn_port.lock().unwrap() = cdn.local_addr().unwrap().port();

    let mc = m.clone();
    tokio::spawn(async move {
        loop {
            let (mut s, _) = cdn.accept().await.unwrap();
            let m = mc.clone();
            tokio::spawn(async move {
                let Some(req) = read_req(&mut s).await else { return };
                if req.headers.contains_key("authorization") {
                    *m.cdn_saw_auth.lock().unwrap() = true;
                }
                let d = req.path.trim_start_matches("/blob/").to_string();
                let blob = m.blobs.lock().unwrap().get(&d).cloned();
                match blob {
                    Some(b) => respond(&mut s, "200 OK", &[], &b, false).await,
                    None => respond(&mut s, "404 Not Found", &[], b"", false).await,
                }
            });
        }
    });

    let ma = m.clone();
    tokio::spawn(async move {
        loop {
            let (mut s, _) = api.accept().await.unwrap();
            let m = ma.clone();
            tokio::spawn(async move {
                let Some(req) = read_req(&mut s).await else { return };
                handle_api(&m, &mut s, req).await;
            });
        }
    });
    m
}

async fn handle_api(m: &Arc<Mock>, s: &mut tokio::net::TcpStream, req: Req) {
    let api_port = *m.api_port.lock().unwrap();
    let cdn_port = *m.cdn_port.lock().unwrap();
    let (path, query) = req.path.split_once('?').map(|(a, b)| (a.to_string(), b.to_string())).unwrap_or((req.path.clone(), String::new()));
    if path == "/token" {
        *m.token_requests.lock().unwrap() += 1;
        assert!(query.contains("service=mock"));
        respond(s, "200 OK", &[], br#"{"token":"tok123"}"#, false).await;
        return;
    }
    let authed = req.headers.get("authorization").map(|v| v == "Bearer tok123").unwrap_or(false);
    if !authed {
        let ch = format!(r#"Bearer realm="http://127.0.0.1:{api_port}/token",service="mock""#);
        respond(s, "401 Unauthorized", &[("WWW-Authenticate", ch)], b"{}", false).await;
        return;
    }
    let head = req.method == "HEAD";
    if path == "/v2/" {
        respond(s, "200 OK", &[], b"{}", false).await;
        return;
    }
    if let Some(rest) = path.strip_prefix("/v2/test/img/blobs/") {
        if rest == "uploads/" && req.method == "POST" {
            respond(s, "202 Accepted", &[("Location", "/v2/test/img/blobs/uploads/u1".into())], b"", false).await;
        } else if rest.starts_with("uploads/") && req.method == "PUT" {
            let digest = query.split('&').find_map(|p| p.strip_prefix("digest=")).unwrap_or("").replace("%3A", ":");
            if manifest::sha256_digest(&req.body) != digest {
                respond(s, "400 Bad Request", &[], b"digest mismatch", false).await;
                return;
            }
            m.blobs.lock().unwrap().insert(digest, req.body);
            respond(s, "201 Created", &[], b"", false).await;
        } else if req.method == "GET" || head {
            let known = m.blobs.lock().unwrap().contains_key(rest);
            if !known {
                respond(s, "404 Not Found", &[], b"", false).await;
            } else if head {
                respond(s, "200 OK", &[], b"", true).await;
            } else {
                respond(s, "307 Temporary Redirect", &[("Location", format!("http://127.0.0.1:{cdn_port}/blob/{rest}"))], b"", false).await;
            }
        }
        return;
    }
    if let Some(reference) = path.strip_prefix("/v2/test/img/manifests/") {
        if req.method == "PUT" {
            let ct = req.headers.get("content-type").cloned().unwrap_or_default();
            let digest = manifest::sha256_digest(&req.body);
            {
                let mut ms = m.manifests.lock().unwrap();
                ms.insert(reference.to_string(), (ct.clone(), req.body.clone()));
                ms.insert(digest, (ct, req.body));
            }
            respond(s, "201 Created", &[], b"", false).await;
            return;
        }
        let found = m.manifests.lock().unwrap().get(reference).cloned();
        match found {
            Some((ct, body)) => {
                let d = manifest::sha256_digest(&body);
                respond(s, "200 OK", &[("Content-Type", ct), ("Docker-Content-Digest", d)], &body, head).await
            }
            None => respond(s, "404 Not Found", &[], b"{}", false).await,
        }
        return;
    }
    respond(s, "404 Not Found", &[], b"", false).await;
}

/// Publish a one-layer image on the mock under `test/img:latest` (docker v2 types).
fn publish_sample(m: &Mock) -> String {
    use std::io::Write;
    let tar = build_tar(&[
        E::Dir("bin/", 0o755),
        E::File("bin/busybox", b"#!busybox", 0o755),
        E::File("old", b"x", 0o644),
    ]);
    let tar2 = build_tar(&[E::File(".wh.old", b"", 0o644), E::File("new", b"n", 0o644)]);
    let mut layers = Vec::new();
    let mut diff_ids = Vec::new();
    for t in [&tar, &tar2] {
        let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        e.write_all(t).unwrap();
        let gz = e.finish().unwrap();
        let d = manifest::sha256_digest(&gz);
        m.blobs.lock().unwrap().insert(d.clone(), gz.clone());
        layers.push(serde_json::json!({
            "mediaType": "application/vnd.docker.image.rootfs.diff.tar.gzip",
            "digest": d, "size": gz.len()
        }));
        diff_ids.push(manifest::sha256_digest(t));
    }
    let cfg = serde_json::json!({
        "architecture": "amd64", "os": "linux", "created": "2024-01-01T00:00:00Z",
        "config": {"Cmd": ["/bin/busybox"], "Env": ["A=B"]},
        "rootfs": {"type": "layers", "diff_ids": diff_ids},
        "history": [{"created_by": "a"}, {"created_by": "b"}]
    });
    let cfg_bytes = serde_json::to_vec(&cfg).unwrap();
    let cfg_digest = manifest::sha256_digest(&cfg_bytes);
    m.blobs.lock().unwrap().insert(cfg_digest.clone(), cfg_bytes.clone());
    let mf = serde_json::json!({
        "schemaVersion": 2,
        "mediaType": manifest::DOCKER_MANIFEST,
        "config": {"mediaType": "application/vnd.docker.container.image.v1+json", "digest": cfg_digest, "size": cfg_bytes.len()},
        "layers": layers
    });
    // Serve it behind a manifest list to exercise platform selection.
    let mf_bytes = serde_json::to_vec(&mf).unwrap();
    let mf_digest = manifest::sha256_digest(&mf_bytes);
    let list = serde_json::json!({
        "schemaVersion": 2, "mediaType": manifest::DOCKER_LIST,
        "manifests": [
            {"mediaType": manifest::DOCKER_MANIFEST, "digest": mf_digest, "size": mf_bytes.len(),
             "platform": {"architecture": "amd64", "os": "linux"}},
            {"mediaType": manifest::DOCKER_MANIFEST, "digest": format!("sha256:{}", "0".repeat(64)), "size": 1,
             "platform": {"architecture": "arm64", "os": "linux", "variant": "v8"}}
        ]
    });
    let list_bytes = serde_json::to_vec(&list).unwrap();
    let mut ms = m.manifests.lock().unwrap();
    ms.insert(mf_digest.clone(), (manifest::DOCKER_MANIFEST.into(), mf_bytes));
    ms.insert("latest".into(), (manifest::DOCKER_LIST.into(), list_bytes));
    cfg_digest
}

fn recording_progress() -> (ProgressFn, Arc<Mutex<Vec<Progress>>>) {
    let log = Arc::new(Mutex::new(Vec::new()));
    let l2 = log.clone();
    (Arc::new(move |p| l2.lock().unwrap().push(p)), log)
}

#[tokio::test]
async fn pull_from_mock_with_token_auth_and_cross_host_redirect() {
    let m = start_mock().await;
    let image_id = publish_sample(&m);
    let port = *m.api_port.lock().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let store = ImageStore::open(dir.path().join("images")).unwrap();
    let r = ImageRef::parse(&format!("127.0.0.1:{port}/test/img:latest")).unwrap();
    let (prog, log) = recording_progress();

    let img = store.pull(&r, &Platform::parse("linux/amd64").unwrap(), prog).await.unwrap();
    assert_eq!(img.info.id, image_id);
    assert_eq!(img.info.repo_tags, vec![r.to_string()]);
    assert_eq!(img.info.config.cmd, Some(vec!["/bin/busybox".to_string()]));
    assert_eq!(img.diff_ids.len(), 2);
    assert!(!*m.cdn_saw_auth.lock().unwrap(), "Authorization leaked to the redirect target");
    assert!(*m.token_requests.lock().unwrap() >= 1);

    let events = log.lock().unwrap().clone();
    assert!(events.iter().any(|p| matches!(p, Progress::Status { message } if message.starts_with("Pulling from"))));
    assert!(events.iter().any(|p| matches!(p, Progress::Status { message } if message.starts_with("Digest: sha256:"))));
    assert!(events.iter().any(|p| matches!(p, Progress::Status { message } if message.contains("Downloaded newer image"))));
    assert_eq!(events.iter().filter(|p| matches!(p, Progress::Layer { done: true, .. })).count(), 2);

    let root = store.rootfs(&img).unwrap();
    assert!(root.join("bin/busybox").exists());
    assert!(root.join("new").exists());
    assert!(!root.join("old").exists() && !root.join(".wh.old").exists());

    // Second pull: everything cached, reported up to date.
    let (prog, log) = recording_progress();
    store.pull(&r, &Platform::parse("linux/amd64").unwrap(), prog).await.unwrap();
    assert!(log
        .lock()
        .unwrap()
        .iter()
        .any(|p| matches!(p, Progress::Status { message } if message.contains("up to date"))));

    // Requesting a platform with a missing manifest fails cleanly.
    let err = store.pull(&r, &Platform::parse("linux/riscv64").unwrap(), no_progress()).await.unwrap_err();
    assert!(matches!(err, Error::NotFound(_)), "{err:?}");
}

#[tokio::test]
async fn corrupted_blob_is_rejected() {
    let m = start_mock().await;
    publish_sample(&m);
    // Corrupt one layer blob while keeping its digest key.
    {
        let mut blobs = m.blobs.lock().unwrap();
        let key = blobs.keys().find(|k| blobs[*k].starts_with(&[0x1f, 0x8b])).unwrap().clone();
        blobs.get_mut(&key).unwrap().push(0);
    }
    let port = *m.api_port.lock().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let store = ImageStore::open(dir.path().join("images")).unwrap();
    let r = ImageRef::parse(&format!("127.0.0.1:{port}/test/img:latest")).unwrap();
    let err = store.pull(&r, &Platform::parse("linux/amd64").unwrap(), no_progress()).await.unwrap_err();
    assert!(err.to_string().contains("digest mismatch"), "{err}");
    assert!(store.list().unwrap().is_empty());
}

#[tokio::test]
async fn commit_then_push_then_pull_round_trip() {
    let m = start_mock().await;
    let port = *m.api_port.lock().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let a = ImageStore::open(dir.path().join("a")).unwrap();

    let tar_dir = tempfile::tempdir().unwrap();
    let tar_path = tar_dir.path().join("layer.tar");
    std::fs::write(&tar_path, build_tar(&[E::Dir("app/", 0o755), E::File("app/run", b"#!/bin/sh\n", 0o755)])).unwrap();
    let diff = a.add_layer_from_tar(&tar_path).unwrap();
    let r = ImageRef::parse(&format!("127.0.0.1:{port}/test/img:v1")).unwrap();
    let cfg = ImageConfig { cmd: Some(vec!["/app/run".into()]), env: vec!["X=1".into()], ..Default::default() };
    let img = a.commit(None, &[diff.clone()], cfg.clone(), Platform::new("linux", "amd64", None), &[r.clone()]).unwrap();
    assert_eq!(img.info.repo_tags, vec![r.to_string()]);

    let (prog, log) = recording_progress();
    a.push(&r, prog).await.unwrap();
    assert!(log.lock().unwrap().iter().any(|p| matches!(p, Progress::Layer { done: true, .. })));
    // Pushing again finds everything already there.
    a.push(&r, no_progress()).await.unwrap();

    let b = ImageStore::open(dir.path().join("b")).unwrap();
    let pulled = b.pull(&r, &Platform::parse("linux/amd64").unwrap(), no_progress()).await.unwrap();
    assert_eq!(pulled.info.id, img.info.id, "image id (config digest) must survive a push/pull cycle");
    assert_eq!(pulled.diff_ids, vec![diff]);
    assert_eq!(pulled.info.config, cfg);
    assert!(b.rootfs(&pulled).unwrap().join("app/run").exists());
}

#[test]
fn get_find_and_list_resolution_rules() {
    let dir = tempfile::tempdir().unwrap();
    let s = ImageStore::open(dir.path().join("images")).unwrap();
    let t = tempfile::tempdir().unwrap();
    let p = t.path().join("l.tar");
    std::fs::write(&p, build_tar(&[E::File("f", b"1", 0o644)])).unwrap();
    let d = s.add_layer_from_tar(&p).unwrap();
    let plat = Platform::new("linux", "arm64", Some("v8"));
    let img = s
        .commit(None, &[d], ImageConfig::default(), plat.clone(), &[ImageRef::parse("web:1").unwrap()])
        .unwrap();
    assert_eq!(s.get("web:1").unwrap().info.id, img.info.id);
    assert_eq!(s.get("docker.io/library/web:1").unwrap().info.id, img.info.id);
    assert_eq!(s.get(&img.info.id[7..19]).unwrap().info.id, img.info.id);
    assert!(matches!(s.get("web:2"), Err(Error::NotFound(_))));
    assert!(s.find("web:2", None).unwrap().is_none());
    assert!(s.find("web:1", Some(&Platform::parse("linux/arm64").unwrap())).unwrap().is_some());
    assert!(s.find("web:1", Some(&Platform::parse("linux/amd64").unwrap())).unwrap().is_none());
    let list = s.list().unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].repo_tags, vec!["docker.io/library/web:1"]);
    assert_eq!(list[0].platform, plat);
}

#[tokio::test]
async fn login_stores_credentials_after_verification() {
    let m = start_mock().await;
    let port = *m.api_port.lock().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let s = ImageStore::open(dir.path().join("images")).unwrap();
    // The mock hands out tokens to anyone, so verification succeeds.
    s.login(&format!("127.0.0.1:{port}"), "alice", "secret").await.unwrap();
    let raw = std::fs::read_to_string(dir.path().join("images/auth.json")).unwrap();
    let v: serde_json::Value = serde_json::from_str(&raw).unwrap();
    assert!(v["auths"][format!("127.0.0.1:{port}")]["auth"].is_string());
    assert!(s.login("x", "", "").await.is_err());
}

// ---- real network ----

async fn pull_and_check(r: &str, platform: &str, expect: &[&str]) {
    let dir = tempfile::tempdir().unwrap();
    let store = ImageStore::open(dir.path().join("images")).unwrap();
    let (prog, log) = recording_progress();
    let r = ImageRef::parse(r).unwrap();
    let plat = Platform::parse(platform).unwrap();
    let img = store.pull(&r, &plat, prog).await.unwrap();
    assert!(plat.matches(&img.info.platform), "{} vs {plat}", img.info.platform);
    let root = store.rootfs(&img).unwrap();
    for p in expect {
        assert!(std::fs::symlink_metadata(root.join(p)).is_ok(), "{p} missing in rootfs of {r} ({platform})");
    }
    fn has_whiteout(p: &std::path::Path) -> bool {
        std::fs::read_dir(p).unwrap().flatten().any(|e| {
            e.file_name().to_string_lossy().starts_with(".wh.")
                || (std::fs::symlink_metadata(e.path()).unwrap().is_dir() && has_whiteout(&e.path()))
        })
    }
    assert!(!has_whiteout(&root));
    let events = log.lock().unwrap();
    println!("{r} {platform}: id={} layers={} status lines={}", img.info.short_id(), img.diff_ids.len(),
        events.iter().filter(|p| matches!(p, Progress::Status { .. })).count());
    for p in events.iter() {
        if let Progress::Status { message } = p {
            println!("  {message}");
        }
    }
}

#[tokio::test]
#[ignore = "needs network access to registry-1.docker.io"]
async fn network_pull_alpine_both_arches() {
    pull_and_check("alpine:3.20", "linux/amd64", &["bin/busybox", "etc/alpine-release"]).await;
    pull_and_check("alpine:3.20", "linux/arm64", &["bin/busybox", "etc/alpine-release"]).await;
}

/// GHCR serves blobs from pkg-containers.githubusercontent.com; some
/// sandboxes block that host, so the full pull is opt-in via
/// `HULLR_TEST_GHCR_BLOBS=1`. Token auth, index selection and the config
/// fetch are always exercised.
#[tokio::test]
#[ignore = "needs network access to ghcr.io"]
async fn network_ghcr_anonymous() {
    let r = ImageRef::parse("ghcr.io/linuxcontainers/alpine:latest").unwrap();
    let reg = registry::Registry::new(&r, None, false).unwrap();
    match reg.resolve(r.manifest_reference(), &Platform::parse("linux/amd64").unwrap()).await {
        Ok(res) => println!("ghcr resolved: manifest {} with {} layers", res.digest, res.manifest.layers.len()),
        // Config blobs are redirected to the blob host, which may be blocked.
        Err(Error::Registry(m)) if m.contains("blob request failed") => {
            println!("ghcr token auth + manifest ok; blob host unreachable here: {m:.80}");
        }
        Err(e) => panic!("{e}"),
    }
    if std::env::var("HULLR_TEST_GHCR_BLOBS").is_ok() {
        pull_and_check("ghcr.io/linuxcontainers/alpine:latest", "linux/amd64", &["bin/busybox"]).await;
    }
}

#[tokio::test]
#[ignore = "needs network access to registry-1.docker.io"]
async fn network_pull_multi_layer_image() {
    pull_and_check("nginx:alpine-slim", "linux/amd64", &["usr/sbin/nginx", "etc/nginx/nginx.conf"]).await;
}
