//! Minimal Docker Registry v2 / OCI distribution client over reqwest.
//!
//! One `Registry` talks to one repository. Authentication follows the
//! `WWW-Authenticate` challenge: bearer tokens are fetched from the realm
//! (anonymously, or with Basic credentials when we have them, which is how
//! Docker Hub and GHCR work), and plain Basic challenges reuse the
//! credentials directly. Redirects are followed by hand so the registry
//! `Authorization` header is never sent to a different host (blob redirects
//! to a CDN).

use crate::auth::{self, Challenge, Credentials};
use crate::manifest::{self, Descriptor, ImageManifest, Manifest};
use hullr_core::{Error, ImageRef, Platform, Result};
use reqwest::{header, Client, RequestBuilder, Response, StatusCode, Url};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const MAX_REDIRECTS: usize = 6;
const MAX_MANIFEST_BYTES: usize = 8 * 1024 * 1024;

#[derive(Clone)]
enum AuthState {
    None,
    Bearer(String),
    Basic,
}

/// A manifest chosen for a platform, together with its config blob.
#[derive(Debug, Clone)]
pub struct Resolved {
    /// Digest of the manifest or index that the reference points at.
    pub digest: String,
    pub manifest: ImageManifest,
    pub config_digest: String,
    pub config_blob: Vec<u8>,
}

pub struct Registry {
    http: Client,
    base: String,
    host: String,
    repo: String,
    scope: String,
    creds: Option<Credentials>,
    state: Mutex<AuthState>,
}

/// `http` for loopback registries and ones listed in `HULLR_INSECURE_REGISTRIES`
/// (comma separated), `https` otherwise.
pub fn scheme_for(host: &str) -> &'static str {
    let bare = host.rsplit_once(':').filter(|(h, p)| !h.ends_with(']') || p.parse::<u16>().is_ok()).map(|(h, _)| h).unwrap_or(host);
    let bare = bare.trim_matches(|c| c == '[' || c == ']');
    if bare == "localhost" || bare == "::1" || bare.starts_with("127.") {
        return "http";
    }
    if let Ok(list) = std::env::var("HULLR_INSECURE_REGISTRIES") {
        if list.split(',').any(|h| h.trim() == host) {
            return "http";
        }
    }
    "https"
}

pub fn build_http_client() -> Result<Client> {
    let mut b = Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(concat!("hullr/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(30))
        .read_timeout(Duration::from_secs(60));
    // Honour a custom CA bundle (corporate proxies), like most tools do.
    if let Some(path) = std::env::var_os("SSL_CERT_FILE").or_else(|| std::env::var_os("HULLR_CA_BUNDLE")) {
        if let Ok(pem) = std::fs::read(&path) {
            if let Ok(certs) = reqwest::Certificate::from_pem_bundle(&pem) {
                for c in certs {
                    b = b.add_root_certificate(c);
                }
            }
        }
    }
    b.build().map_err(|e| Error::Registry(format!("http client: {e}")))
}

fn net_err(what: &str, e: reqwest::Error) -> Error {
    Error::Registry(format!("{what}: {e}"))
}

#[derive(Deserialize)]
struct TokenResponse {
    token: Option<String>,
    access_token: Option<String>,
}

impl Registry {
    /// Client for the repository of `r`. `push` requests push scope too.
    pub fn new(r: &ImageRef, creds: Option<Credentials>, push: bool) -> Result<Self> {
        let host = r.api_host().to_string();
        let scope = format!("repository:{}:{}", r.repository, if push { "pull,push" } else { "pull" });
        Self::with_parts(host, r.repository.clone(), scope, creds)
    }

    /// Client for `/v2/` only, used to verify credentials.
    pub fn for_login(registry: &str, creds: Option<Credentials>) -> Result<Self> {
        let reg = auth::normalize_registry(registry);
        let host = if reg == "docker.io" { hullr_core::reference::DOCKER_HUB_API.to_string() } else { reg };
        Self::with_parts(host, String::new(), String::new(), creds)
    }

    fn with_parts(host: String, repo: String, scope: String, creds: Option<Credentials>) -> Result<Self> {
        Ok(Self {
            http: build_http_client()?,
            base: format!("{}://{}", scheme_for(&host), host),
            host,
            repo,
            scope,
            creds,
            state: Mutex::new(AuthState::None),
        })
    }

    #[cfg(test)]
    pub(crate) fn base(&self) -> &str {
        &self.base
    }

    fn state(&self) -> AuthState {
        self.state.lock().unwrap().clone()
    }

    fn set_state(&self, s: AuthState) {
        *self.state.lock().unwrap() = s;
    }

    async fn send_once(&self, rb: RequestBuilder) -> Result<Response> {
        let rb = match self.state() {
            AuthState::None => rb,
            AuthState::Bearer(t) => rb.bearer_auth(t),
            AuthState::Basic => match &self.creds {
                Some(c) => rb.header(header::AUTHORIZATION, c.basic_header()),
                None => rb,
            },
        };
        rb.send().await.map_err(|e| net_err("registry request failed", e))
    }

    /// Send a request to the registry host, answering one 401 challenge.
    async fn send(&self, build: impl Fn(&Client) -> RequestBuilder) -> Result<Response> {
        let resp = self.send_once(build(&self.http)).await?;
        if resp.status() != StatusCode::UNAUTHORIZED {
            return Ok(resp);
        }
        let challenge = resp
            .headers()
            .get_all(header::WWW_AUTHENTICATE)
            .iter()
            .filter_map(|v| v.to_str().ok())
            .find_map(|v| auth::parse_challenge(v).ok());
        let Some(challenge) = challenge else { return Ok(resp) };
        self.authenticate(challenge).await?;
        self.send_once(build(&self.http)).await
    }

    async fn authenticate(&self, challenge: Challenge) -> Result<()> {
        match challenge {
            Challenge::Basic => {
                if self.creds.is_none() {
                    return Err(Error::Registry(format!(
                        "{} requires credentials; run login first",
                        self.host
                    )));
                }
                self.set_state(AuthState::Basic);
            }
            Challenge::Bearer { realm, service, scope } => {
                let mut url = Url::parse(&realm).map_err(|e| Error::Registry(format!("bad auth realm '{realm}': {e}")))?;
                {
                    let mut q = url.query_pairs_mut();
                    if let Some(s) = &service {
                        q.append_pair("service", s);
                    }
                    let scope = if !self.scope.is_empty() { Some(self.scope.clone()) } else { scope };
                    if let Some(s) = scope {
                        q.append_pair("scope", &s);
                    }
                }
                let mut rb = self.http.get(url);
                if let Some(c) = &self.creds {
                    rb = rb.header(header::AUTHORIZATION, c.basic_header());
                }
                let resp = rb.send().await.map_err(|e| net_err("token request failed", e))?;
                let status = resp.status();
                if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
                    return Err(Error::Registry(format!("authentication failed for {} ({status})", self.host)));
                }
                if !status.is_success() {
                    return Err(Error::Registry(format!("token endpoint returned {status}")));
                }
                let t: TokenResponse = resp.json().await.map_err(|e| net_err("bad token response", e))?;
                let token = t
                    .token
                    .or(t.access_token)
                    .ok_or_else(|| Error::Registry("token response without token".into()))?;
                self.set_state(AuthState::Bearer(token));
            }
        }
        Ok(())
    }

    /// Verify credentials by completing the `/v2/` auth handshake.
    pub async fn check_login(&self) -> Result<()> {
        let url = format!("{}/v2/", self.base);
        let resp = self.send(|c| c.get(&url)).await?;
        match resp.status() {
            s if s.is_success() => Ok(()),
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => {
                Err(Error::Registry(format!("login to {} failed: unauthorized", self.host)))
            }
            s => Err(Error::Registry(format!("login to {} failed: {s}", self.host))),
        }
    }

    async fn check(&self, resp: Response, what: &str) -> Result<Response> {
        let status = resp.status();
        if status.is_success() {
            return Ok(resp);
        }
        let body = resp.text().await.unwrap_or_default();
        let body: String = body.chars().take(300).collect();
        Err(match status {
            StatusCode::NOT_FOUND => Error::NotFound(format!("{what}: not found ({})", body.trim())),
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => Error::Registry(format!(
                "{what}: unauthorized ({status}); the repository may not exist or needs login {}",
                body.trim()
            )),
            _ => Error::Registry(format!("{what}: {status} {}", body.trim())),
        })
    }

    // ---- manifests ----

    /// Fetch a manifest or index. Returns body, content type and the
    /// computed `sha256:` digest (verified against the reference when it is
    /// itself a digest, and against `Docker-Content-Digest` when present).
    pub async fn get_manifest(&self, reference: &str) -> Result<(Vec<u8>, Option<String>, String)> {
        let url = format!("{}/v2/{}/manifests/{}", self.base, self.repo, reference);
        let what = format!("manifest {}:{}", self.repo, reference);
        let resp = self
            .send(|c| c.get(&url).header(header::ACCEPT, manifest::ACCEPT))
            .await?;
        let resp = self.check(resp, &what).await?;
        let ctype = resp.headers().get(header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).map(String::from);
        let header_digest = resp
            .headers()
            .get("docker-content-digest")
            .and_then(|v| v.to_str().ok())
            .map(String::from);
        let body = resp.bytes().await.map_err(|e| net_err("reading manifest", e))?;
        if body.len() > MAX_MANIFEST_BYTES {
            return Err(Error::Registry("manifest too large".into()));
        }
        let digest = manifest::sha256_digest(&body);
        if reference.starts_with("sha256:") && reference != digest {
            return Err(Error::Registry(format!("manifest digest mismatch: wanted {reference}, got {digest}")));
        }
        if let Some(h) = header_digest.filter(|h| h.starts_with("sha256:")) {
            if h != digest {
                return Err(Error::Registry(format!("manifest digest mismatch: header {h}, body {digest}")));
            }
        }
        Ok((body.to_vec(), ctype, digest))
    }

    /// Resolve `reference` to a single-platform image manifest plus config.
    pub async fn resolve(&self, reference: &str, platform: &Platform) -> Result<Resolved> {
        let (body, ctype, top_digest) = self.get_manifest(reference).await?;
        let parsed = manifest::parse_manifest(&body, ctype.as_deref())?;
        let image = match parsed {
            Manifest::Image(m) => m,
            Manifest::Index(entries) => {
                let d = manifest::select_platform(&entries, platform).ok_or_else(|| {
                    Error::NotFound(format!(
                        "no matching manifest for {platform} (available: {})",
                        manifest::available_platforms(&entries)
                    ))
                })?;
                let (cbody, cctype, _) = self.get_manifest(&d.digest).await?;
                match manifest::parse_manifest(&cbody, cctype.as_deref())? {
                    Manifest::Image(m) => m,
                    Manifest::Index(_) => return Err(Error::Registry("nested manifest index".into())),
                }
            }
        };
        let config_blob = self.get_blob_bytes(&image.config.digest).await?;
        let parsed = manifest::parse_config(&config_blob)?;
        if !platform.matches(&parsed.platform) {
            return Err(Error::NotFound(format!(
                "image is {} but {platform} was requested",
                parsed.platform
            )));
        }
        Ok(Resolved { digest: top_digest, config_digest: image.config.digest.clone(), manifest: image, config_blob })
    }

    // ---- blobs ----

    fn blob_url(&self, digest: &str) -> String {
        format!("{}/v2/{}/blobs/{}", self.base, self.repo, digest)
    }

    /// GET a blob, following redirects without leaking credentials.
    async fn open_blob(&self, digest: &str) -> Result<Response> {
        let mut url = self.blob_url(digest);
        for _ in 0..MAX_REDIRECTS {
            let parsed = Url::parse(&url).map_err(|e| Error::Registry(format!("bad blob url: {e}")))?;
            let same_host = host_port(&parsed) == self.host;
            let resp = if same_host {
                self.send(|c| c.get(&url)).await?
            } else {
                self.http.get(&url).send().await.map_err(|e| net_err("blob request failed", e))?
            };
            if resp.status().is_redirection() {
                let loc = resp
                    .headers()
                    .get(header::LOCATION)
                    .and_then(|v| v.to_str().ok())
                    .ok_or_else(|| Error::Registry("redirect without location".into()))?;
                url = parsed.join(loc).map_err(|e| Error::Registry(format!("bad redirect: {e}")))?.to_string();
                continue;
            }
            return self.check(resp, &format!("blob {digest}")).await;
        }
        Err(Error::Registry("too many redirects fetching blob".into()))
    }

    /// Small blob (image config) into memory, digest verified.
    pub async fn get_blob_bytes(&self, digest: &str) -> Result<Vec<u8>> {
        manifest::digest_hex(digest)?;
        let resp = self.open_blob(digest).await?;
        let body = resp.bytes().await.map_err(|e| net_err("reading blob", e))?;
        if body.len() > 64 * 1024 * 1024 {
            return Err(Error::Registry("config blob too large".into()));
        }
        let got = manifest::sha256_digest(&body);
        if got != digest {
            return Err(Error::Registry(format!("blob digest mismatch: wanted {digest}, got {got}")));
        }
        Ok(body.to_vec())
    }

    /// Stream a blob into `dest`, verifying its sha256. `on_bytes(total_so_far,
    /// content_length)` is called as data arrives. Retries transient failures.
    pub async fn download_blob(
        &self,
        digest: &str,
        dest: &Path,
        on_bytes: &mut (dyn FnMut(u64, u64) + Send),
    ) -> Result<()> {
        manifest::digest_hex(digest)?;
        let mut last = Error::Registry("download failed".into());
        for attempt in 0..3u64 {
            if attempt > 0 {
                tokio::time::sleep(Duration::from_millis(500 * attempt)).await;
            }
            match self.download_once(digest, dest, on_bytes).await {
                Ok(()) => return Ok(()),
                Err(e @ (Error::NotFound(_) | Error::Unsupported(_))) => return Err(e),
                Err(e) => {
                    if matches!(&e, Error::Registry(m) if m.contains("unauthorized") || m.contains("digest mismatch")) {
                        return Err(e);
                    }
                    last = e;
                }
            }
        }
        Err(last)
    }

    async fn download_once(
        &self,
        digest: &str,
        dest: &Path,
        on_bytes: &mut (dyn FnMut(u64, u64) + Send),
    ) -> Result<()> {
        use futures::StreamExt;
        let resp = self.open_blob(digest).await?;
        let total = resp.content_length().unwrap_or(0);
        let mut file = tokio::fs::File::create(dest).await?;
        let mut hasher = Sha256::new();
        let mut got = 0u64;
        let mut stream = resp.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| net_err("blob download interrupted", e))?;
            hasher.update(&chunk);
            file.write_all(&chunk).await?;
            got += chunk.len() as u64;
            on_bytes(got, total);
        }
        file.flush().await?;
        drop(file);
        let have = format!("sha256:{}", hex::encode(hasher.finalize()));
        if have != digest {
            let _ = tokio::fs::remove_file(dest).await;
            return Err(Error::Registry(format!("blob digest mismatch: wanted {digest}, got {have}")));
        }
        Ok(())
    }

    pub async fn blob_exists(&self, digest: &str) -> Result<bool> {
        let url = self.blob_url(digest);
        let resp = self.send(|c| c.head(&url)).await?;
        match resp.status() {
            s if s.is_success() => Ok(true),
            StatusCode::NOT_FOUND => Ok(false),
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => Err(Error::Registry(format!(
                "push to {}/{} denied; run login with an account that can push",
                self.host, self.repo
            ))),
            s => Err(Error::Registry(format!("checking blob {digest}: {s}"))),
        }
    }

    /// Monolithic blob upload from a file.
    pub async fn upload_blob(
        &self,
        digest: &str,
        path: &Path,
        size: u64,
        on_bytes: std::sync::Arc<dyn Fn(u64) + Send + Sync>,
    ) -> Result<()> {
        let start = format!("{}/v2/{}/blobs/uploads/", self.base, self.repo);
        let resp = self.send(|c| c.post(&start).header(header::CONTENT_LENGTH, 0)).await?;
        let resp = self.check(resp, "starting blob upload").await?;
        let loc = resp
            .headers()
            .get(header::LOCATION)
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| Error::Registry("upload without Location".into()))?;
        let mut url = Url::parse(&start).unwrap().join(loc).map_err(|e| Error::Registry(format!("bad upload location: {e}")))?;
        url.query_pairs_mut().append_pair("digest", digest);
        let same_host = host_port(&url) == self.host;
        let make = |c: &Client| {
            c.put(url.clone())
                .header(header::CONTENT_TYPE, "application/octet-stream")
                .header(header::CONTENT_LENGTH, size)
                .body(file_body(path.to_path_buf(), on_bytes.clone()))
        };
        let resp = if same_host {
            self.send(make).await?
        } else {
            make(&self.http).send().await.map_err(|e| net_err("blob upload failed", e))?
        };
        self.check(resp, &format!("uploading blob {digest}")).await?;
        Ok(())
    }

    /// Upload small in-memory content (config blob) via monolithic upload.
    pub async fn upload_bytes(&self, digest: &str, data: Vec<u8>) -> Result<()> {
        let start = format!("{}/v2/{}/blobs/uploads/", self.base, self.repo);
        let resp = self.send(|c| c.post(&start).header(header::CONTENT_LENGTH, 0)).await?;
        let resp = self.check(resp, "starting blob upload").await?;
        let loc = resp
            .headers()
            .get(header::LOCATION)
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| Error::Registry("upload without Location".into()))?;
        let mut url = Url::parse(&start).unwrap().join(loc).map_err(|e| Error::Registry(format!("bad upload location: {e}")))?;
        url.query_pairs_mut().append_pair("digest", digest);
        let same_host = host_port(&url) == self.host;
        let make = |c: &Client| {
            c.put(url.clone()).header(header::CONTENT_TYPE, "application/octet-stream").body(data.clone())
        };
        let resp = if same_host {
            self.send(make).await?
        } else {
            make(&self.http).send().await.map_err(|e| net_err("blob upload failed", e))?
        };
        self.check(resp, &format!("uploading blob {digest}")).await?;
        Ok(())
    }

    /// PUT a manifest under `reference` (tag or digest).
    pub async fn put_manifest(&self, reference: &str, body: Vec<u8>, media_type: &str) -> Result<()> {
        let url = format!("{}/v2/{}/manifests/{}", self.base, self.repo, reference);
        let resp = self
            .send(|c| c.put(&url).header(header::CONTENT_TYPE, media_type).body(body.clone()))
            .await?;
        self.check(resp, "pushing manifest").await?;
        Ok(())
    }
}

fn host_port(u: &Url) -> String {
    match (u.host_str(), u.port()) {
        (Some(h), Some(p)) => format!("{h}:{p}"),
        (Some(h), None) => h.to_string(),
        _ => String::new(),
    }
}

/// Lazily-opened streaming request body over a file, reporting bytes sent.
fn file_body(path: std::path::PathBuf, on_bytes: std::sync::Arc<dyn Fn(u64) + Send + Sync>) -> reqwest::Body {
    struct St {
        path: std::path::PathBuf,
        file: Option<tokio::fs::File>,
        sent: u64,
        cb: std::sync::Arc<dyn Fn(u64) + Send + Sync>,
    }
    let st = St { path, file: None, sent: 0, cb: on_bytes };
    let s = futures::stream::unfold(st, |mut st| async move {
        if st.file.is_none() {
            match tokio::fs::File::open(&st.path).await {
                Ok(f) => st.file = Some(f),
                Err(e) => return Some((Err(e), st)),
            }
        }
        let mut buf = vec![0u8; 256 * 1024];
        match st.file.as_mut().unwrap().read(&mut buf).await {
            Ok(0) => None,
            Ok(n) => {
                buf.truncate(n);
                st.sent += n as u64;
                (st.cb)(st.sent);
                Some((Ok::<Vec<u8>, std::io::Error>(buf), st))
            }
            Err(e) => Some((Err(e), st)),
        }
    });
    reqwest::Body::wrap_stream(s)
}

/// Descriptor helper used by push.
pub fn descriptor(media_type: &str, digest: &str, size: u64) -> Descriptor {
    Descriptor { media_type: media_type.into(), digest: digest.into(), size, platform: None, annotations: None }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scheme_selection() {
        assert_eq!(scheme_for("registry-1.docker.io"), "https");
        assert_eq!(scheme_for("localhost:5000"), "http");
        assert_eq!(scheme_for("127.0.0.1:5000"), "http");
        assert_eq!(scheme_for("ghcr.io"), "https");
    }

    #[test]
    fn base_url_uses_api_host() {
        let r = ImageRef::parse("alpine").unwrap();
        let reg = Registry::new(&r, None, false).unwrap();
        assert_eq!(reg.base(), "https://registry-1.docker.io");
    }
}
