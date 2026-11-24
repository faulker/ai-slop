//! End-to-end builder tests over an in-memory fake store and a fake runner
//! that executes RUN commands directly on the host inside the rootfs
//! (no isolation; fine for `touch`/`mkdir`/`echo` style steps).
//!
//! Tests against the real `ImageStore` are marked `#[ignore]` until it is
//! usable in this environment.

use crate::backend::Backend;
use crate::builder::run_build;
use hullr_core::api::{Progress, ProgressFn};
use hullr_core::{BuildOptions, Error, ImageConfig, ImageInfo, ImageRef, LogStream, Platform, Result};
use hullr_image::StoredImage;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct State {
    layers: HashMap<String, Vec<u8>>,
    images: HashMap<String, StoredImage>,
    cache: HashMap<String, String>,
    n: usize,
}

struct Fake {
    dir: tempfile::TempDir,
    st: Mutex<State>,
    runs: AtomicUsize,
}

impl Fake {
    fn new() -> Self {
        let f = Fake { dir: tempfile::tempdir().unwrap(), st: Mutex::new(State::default()), runs: AtomicUsize::new(0) };
        // a tiny "base" image with /etc/os and ENV
        let tar_path = f.dir.path().join("base.tar");
        {
            let mut b = tar::Builder::new(std::fs::File::create(&tar_path).unwrap());
            let mut h = tar::Header::new_gnu();
            h.set_size(5);
            h.set_mode(0o644);
            b.append_data(&mut h, "etc/os", &b"base\n"[..]).unwrap();
            b.finish().unwrap();
        }
        let d = f.add_layer_from_tar(&tar_path).unwrap();
        let cfg = ImageConfig { env: vec!["BASEVAR=1".into()], working_dir: Some("/".into()), ..Default::default() };
        f.commit(None, &[d], cfg, Platform::host(), &[ImageRef::parse("base:latest").unwrap()]).unwrap();
        f
    }
    fn files(&self, img: &StoredImage) -> Vec<String> {
        let root = self.rootfs(img).unwrap();
        let mut out = Vec::new();
        fn walk(d: &Path, rel: &str, out: &mut Vec<String>) {
            for e in std::fs::read_dir(d).unwrap().flatten() {
                let n = format!("{rel}/{}", e.file_name().to_string_lossy());
                out.push(n.clone());
                if e.file_type().unwrap().is_dir() {
                    walk(&e.path(), &n, out);
                }
            }
        }
        walk(&root, "", &mut out);
        out.sort();
        out
    }
}

#[async_trait::async_trait]
impl Backend for Fake {
    fn rootfs(&self, img: &StoredImage) -> Result<PathBuf> {
        let mut st = self.st.lock().unwrap();
        st.n += 1;
        let dir = self.dir.path().join(format!("rootfs-{}", st.n));
        std::fs::create_dir_all(&dir)?;
        for d in &img.diff_ids {
            let bytes = st.layers[d].clone();
            let mut a = tar::Archive::new(&bytes[..]);
            for e in a.entries()? {
                let mut e = e?;
                let p = e.path()?.into_owned();
                let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                if let Some(target) = name.strip_prefix(".wh.") {
                    let t = dir.join(p.parent().unwrap()).join(target);
                    if t.is_dir() {
                        std::fs::remove_dir_all(&t)?;
                    } else {
                        let _ = std::fs::remove_file(&t);
                    }
                } else {
                    e.unpack_in(&dir)?;
                }
            }
        }
        Ok(dir)
    }
    fn add_layer_from_tar(&self, p: &Path) -> Result<String> {
        let bytes = std::fs::read(p)?;
        let id = format!("sha256:{}", hex::encode(Sha256::digest(&bytes)));
        self.st.lock().unwrap().layers.insert(id.clone(), bytes);
        Ok(id)
    }
    fn commit(&self, parent: Option<&StoredImage>, new_layers: &[String], config: ImageConfig, platform: Platform, tags: &[ImageRef]) -> Result<StoredImage> {
        let mut diffs = parent.map(|p| p.diff_ids.clone()).unwrap_or_default();
        diffs.extend(new_layers.iter().cloned());
        let tag_strs: Vec<String> = tags.iter().map(|t| t.to_string()).collect();
        let id = format!(
            "sha256:{}",
            hex::encode(Sha256::digest(format!("{}|{:?}|{:?}", serde_json::to_string(&config).unwrap(), diffs, tag_strs)))
        );
        let img = StoredImage {
            info: ImageInfo { id: id.clone(), repo_tags: tag_strs, size: 0, created: 0, platform, config },
            diff_ids: diffs,
        };
        self.st.lock().unwrap().images.insert(id, img.clone());
        Ok(img)
    }
    fn cache_get(&self, key: &str) -> Option<String> {
        self.st.lock().unwrap().cache.get(key).cloned()
    }
    fn cache_put(&self, key: &str, id: &str) -> Result<()> {
        self.st.lock().unwrap().cache.insert(key.into(), id.into());
        Ok(())
    }
    fn get(&self, r: &str) -> Result<StoredImage> {
        let st = self.st.lock().unwrap();
        st.images
            .values()
            .find(|i| i.info.id == r || i.info.repo_tags.iter().any(|t| t.ends_with(r) || t == r))
            .cloned()
            .ok_or_else(|| Error::NotFound(r.into()))
    }
    async fn base_image(&self, reference: &str, _p: &Platform, _pull: bool, _pr: ProgressFn) -> Result<StoredImage> {
        self.get(reference)
    }
    async fn run(
        &self,
        rootfs: &Path,
        args: Vec<String>,
        env: Vec<String>,
        _cwd: String,
        _user: Option<String>,
        _platform: &Platform,
        output: &(dyn for<'b> Fn(LogStream, &'b [u8]) + Send + Sync),
    ) -> Result<i32> {
        self.runs.fetch_add(1, Ordering::SeqCst);
        let mut c = std::process::Command::new(&args[0]);
        c.args(&args[1..]).current_dir(rootfs).env_clear();
        for e in env {
            if let Some((k, v)) = e.split_once('=') {
                c.env(k, v);
            }
        }
        let o = c.output()?;
        output(LogStream::Stdout, &o.stdout);
        output(LogStream::Stderr, &o.stderr);
        Ok(o.status.code().unwrap_or(-1))
    }
    fn id_to_container(&self, u: u32, g: u32) -> (u32, u32) {
        crate::default_id_map(u, g)
    }
}

struct Env {
    fake: Fake,
    ctx: tempfile::TempDir,
    events: Arc<Mutex<Vec<Progress>>>,
}

impl Env {
    fn new(dockerfile: &str) -> Self {
        let ctx = tempfile::tempdir().unwrap();
        std::fs::write(ctx.path().join("Dockerfile"), dockerfile).unwrap();
        Env { fake: Fake::new(), ctx, events: Arc::new(Mutex::new(Vec::new())) }
    }
    fn file(&self, rel: &str, content: &str) {
        let p = self.ctx.path().join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, content).unwrap();
    }
    fn opts(&self) -> BuildOptions {
        BuildOptions { context: self.ctx.path().to_path_buf(), ..Default::default() }
    }
    async fn build(&self, opts: &BuildOptions) -> Result<StoredImage> {
        self.events.lock().unwrap().clear();
        let ev = self.events.clone();
        let progress: ProgressFn = Arc::new(move |p| ev.lock().unwrap().push(p));
        let work = self.fake.dir.path().join("work");
        run_build(opts, &self.fake, &work, progress).await
    }
    fn steps(&self) -> Vec<(String, bool)> {
        self.events
            .lock()
            .unwrap()
            .iter()
            .filter_map(|e| match e {
                Progress::Step { text, cached, .. } => Some((text.clone(), *cached)),
                _ => None,
            })
            .collect()
    }
    fn work_is_clean(&self) -> bool {
        let w = self.fake.dir.path().join("work");
        !w.exists() || std::fs::read_dir(w).unwrap().next().is_none()
    }
}

#[tokio::test]
async fn basic_build_layers_config_and_tags() {
    let e = Env::new(
        "FROM scratch\nENV A=1 B=two\nLABEL k=v\nEXPOSE 80 53/udp\nWORKDIR /app\nCOPY hello.txt /app/\nRUN mkdir -p data && echo hi > data/f.txt\nUSER nobody\nCMD [\"run\"]\nENTRYPOINT /start\nHEALTHCHECK --interval=5s CMD true\nSTOPSIGNAL SIGINT\nVOLUME /data\n",
    );
    e.file("hello.txt", "hello");
    let mut o = e.opts();
    o.tags = vec!["myimg:1".into(), "other".into()];
    o.labels.insert("extra".into(), "yes".into());
    let img = e.build(&o).await.unwrap();

    let files = e.fake.files(&img);
    for f in ["/app", "/app/hello.txt", "/data/f.txt"] {
        assert!(files.contains(&f.to_string()), "{f} in {files:?}");
    }
    let c = &img.info.config;
    assert_eq!(c.env, ["A=1", "B=two"]);
    assert_eq!(c.labels["k"], "v");
    assert_eq!(c.labels["extra"], "yes");
    assert_eq!(c.exposed_ports, ["80/tcp", "53/udp"]);
    assert_eq!(c.working_dir.as_deref(), Some("/app"));
    assert_eq!(c.user.as_deref(), Some("nobody"));
    assert_eq!(c.entrypoint.as_ref().unwrap(), &["/bin/sh", "-c", "/start"]);
    assert_eq!(c.cmd, None, "ENTRYPOINT after CMD resets CMD");
    assert_eq!(c.stop_signal.as_deref(), Some("SIGINT"));
    assert_eq!(c.volumes, ["/data"]);
    let hc = c.healthcheck.as_ref().unwrap();
    assert_eq!((hc.interval_secs, hc.test.clone()), (5, vec!["CMD-SHELL".to_string(), "true".to_string()]));
    assert_eq!(img.info.repo_tags.len(), 2);
    assert!(img.info.repo_tags[0].contains("myimg"));
    // WORKDIR, COPY, RUN each produced a layer
    assert_eq!(img.diff_ids.len(), 3);
    let steps = e.steps();
    assert_eq!(steps[0].0, "FROM scratch");
    assert_eq!(steps.len(), 13);
    assert!(e.work_is_clean());
}

#[tokio::test]
async fn run_output_is_streamed_and_failure_reports_code() {
    let e = Env::new("FROM scratch\nRUN echo out1; echo out2\nRUN echo bad >&2; exit 7\n");
    let err = e.build(&e.opts()).await.unwrap_err();
    let m = err.to_string();
    assert!(matches!(err, Error::Build(_)), "{m}");
    assert!(m.contains("exit code 7") && m.contains("exit 7"), "{m}");
    let lines: Vec<String> = e
        .events
        .lock()
        .unwrap()
        .iter()
        .filter_map(|p| match p {
            Progress::Output { source, line } => Some(format!("{source}:{line}")),
            _ => None,
        })
        .collect();
    assert!(lines.contains(&"step 2:out1".to_string()) && lines.contains(&"step 2:out2".to_string()), "{lines:?}");
    assert!(lines.contains(&"step 3:bad".to_string()), "{lines:?}");
    assert!(e.work_is_clean(), "scratch dir must be removed on error");
}

#[tokio::test]
async fn cache_hits_skip_execution_and_no_cache_disables() {
    let e = Env::new("FROM scratch\nWORKDIR /w\nCOPY a.txt .\nRUN echo built > out.txt\nENV X=1\n");
    e.file("a.txt", "one");
    let o = e.opts();
    let first = e.build(&o).await.unwrap();
    assert_eq!(e.fake.runs.load(Ordering::SeqCst), 1);
    assert!(e.steps().iter().all(|(_, c)| !c));

    let second = e.build(&o).await.unwrap();
    assert_eq!(e.fake.runs.load(Ordering::SeqCst), 1, "RUN must be cached");
    let cached: Vec<bool> = e.steps().into_iter().map(|s| s.1).collect();
    assert_eq!(cached, [false, true, true, true, false]);
    assert_eq!(first.diff_ids, second.diff_ids);
    assert!(e.fake.files(&second).contains(&"/out.txt".to_string()));

    // changing a source file invalidates COPY and everything after it
    e.file("a.txt", "two");
    e.build(&o).await.unwrap();
    assert_eq!(e.fake.runs.load(Ordering::SeqCst), 2);
    assert_eq!(e.steps()[1..4].iter().map(|s| s.1).collect::<Vec<_>>(), [true, false, false]);

    let mut nc = o.clone();
    nc.no_cache = true;
    e.build(&nc).await.unwrap();
    assert_eq!(e.fake.runs.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn build_args_change_invalidates_cache_and_reach_run() {
    let e = Env::new("ARG BASE=scratch\nFROM ${BASE}\nARG V=1\nARG UNSET\nRUN echo \"$V\" > v.txt\n");
    let mut o = e.opts();
    let img = e.build(&o).await.unwrap();
    let read = |img: &StoredImage| std::fs::read_to_string(e.fake.rootfs(img).unwrap().join("v.txt")).unwrap();
    assert_eq!(read(&img).trim(), "1");
    o.build_args.insert("V".into(), "2".into());
    let img2 = e.build(&o).await.unwrap();
    assert_eq!(read(&img2).trim(), "2");
    assert_eq!(e.fake.runs.load(Ordering::SeqCst), 2);
    // unconsumed build arg warning
    o.build_args.insert("NOPE".into(), "x".into());
    e.build(&o).await.unwrap();
    assert!(e.events.lock().unwrap().iter().any(|p| matches!(p, Progress::Status { message } if message.contains("NOPE"))));
}

#[tokio::test]
async fn multi_stage_copy_from_and_target() {
    let e = Env::new(
        "FROM scratch AS builder\nRUN mkdir out && echo artifact > out/app && echo junk > out/junk\n\nFROM scratch AS tools\nRUN echo t > tool\n\nFROM base:latest AS final\nCOPY --from=builder /out/app /usr/bin/app\nRUN cat /dev/null\n",
    );
    let o = e.opts();
    let img = e.build(&o).await.unwrap();
    let files = e.fake.files(&img);
    assert!(files.contains(&"/usr/bin/app".to_string()), "{files:?}");
    assert!(files.contains(&"/etc/os".to_string()), "base content inherited");
    assert!(!files.iter().any(|f| f.contains("junk") || f == "/tool"));
    assert_eq!(img.info.config.env, ["BASEVAR=1"]);
    // `tools` is not needed and must not run: builder RUN + final RUN
    assert_eq!(e.fake.runs.load(Ordering::SeqCst), 2);

    let mut t = o.clone();
    t.target = Some("tools".into());
    t.no_cache = true;
    let img = e.build(&t).await.unwrap();
    assert!(e.fake.files(&img).contains(&"/tool".to_string()));
    assert_eq!(e.fake.runs.load(Ordering::SeqCst), 3);
    t.target = Some("missing".into());
    assert!(e.build(&t).await.is_err());
}

#[tokio::test]
async fn deletions_become_whiteouts() {
    let e = Env::new("FROM scratch\nRUN mkdir d && echo x > d/f && echo y > top\nRUN rm -r d && rm top\n");
    let img = e.build(&e.opts()).await.unwrap();
    assert_eq!(img.diff_ids.len(), 2);
    assert!(e.fake.files(&img).is_empty(), "{:?}", e.fake.files(&img));
}

#[tokio::test]
async fn copy_heredoc_chmod_and_dockerignore() {
    let e = Env::new(
        "FROM scratch\nENV NAME=world\nCOPY <<EOF /etc/greeting\nhello $NAME\nEOF\nCOPY --chmod=755 run.sh /bin/run.sh\nCOPY . /ctx/\nRUN <<EOT\nmkdir made\necho in-heredoc > made/f\nEOT\n",
    );
    e.file("run.sh", "#!/bin/sh\n");
    e.file("keep.txt", "k");
    e.file("skip.log", "s");
    e.file(".dockerignore", "*.log\nDockerfile\n.dockerignore\n");
    let img = e.build(&e.opts()).await.unwrap();
    let root = e.fake.rootfs(&img).unwrap();
    assert_eq!(std::fs::read_to_string(root.join("etc/greeting")).unwrap(), "hello world\n");
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(std::fs::metadata(root.join("bin/run.sh")).unwrap().permissions().mode() & 0o777, 0o755);
    assert!(root.join("ctx/keep.txt").exists());
    assert!(!root.join("ctx/skip.log").exists() && !root.join("ctx/Dockerfile").exists());
    assert_eq!(std::fs::read_to_string(root.join("made/f")).unwrap().trim(), "in-heredoc");
}

#[tokio::test]
async fn shell_instruction_changes_run_shell() {
    let e = Env::new("FROM scratch\nSHELL [\"/bin/sh\", \"-ec\"]\nRUN echo ok > f\nCMD echo hi\n");
    let img = e.build(&e.opts()).await.unwrap();
    assert_eq!(img.info.config.cmd.as_ref().unwrap(), &["/bin/sh", "-ec", "echo hi"]);
    assert_eq!(img.info.config.shell.as_ref().unwrap()[1], "-ec");
}

#[tokio::test]
async fn unknown_instruction_and_missing_source_errors() {
    let e = Env::new("FROM scratch\nBOGUS x\n");
    assert!(e.build(&e.opts()).await.unwrap_err().to_string().contains("line 2"));
    let e = Env::new("FROM scratch\nCOPY nothere /x\n");
    let err = e.build(&e.opts()).await.unwrap_err().to_string();
    assert!(err.contains("line 2") && err.contains("nothere"), "{err}");
    let e = Env::new("FROM scratch\nRUN true\n");
    std::fs::remove_file(e.ctx.path().join("Dockerfile")).unwrap();
    assert!(e.build(&e.opts()).await.is_err());
}

#[tokio::test]
async fn onbuild_and_run_mount_warn() {
    let e = Env::new("FROM scratch\nONBUILD RUN echo later\nRUN --mount=type=cache,target=/c true\n");
    e.build(&e.opts()).await.unwrap();
    let msgs: Vec<String> = e
        .events
        .lock()
        .unwrap()
        .iter()
        .filter_map(|p| if let Progress::Status { message } = p { Some(message.clone()) } else { None })
        .collect();
    assert!(msgs.iter().any(|m| m.contains("ONBUILD")), "{msgs:?}");
    assert!(msgs.iter().any(|m| m.contains("--mount")), "{msgs:?}");
    assert_eq!(e.fake.runs.load(Ordering::SeqCst), 1);
}

#[tokio::test]
#[ignore = "needs a working hullr-image ImageStore and a BuildHost; run manually"]
async fn real_store_smoke() {
    // Placeholder for an integration test once the image crate and engine
    // host are available: build `FROM scratch` + COPY and inspect the image.
}
