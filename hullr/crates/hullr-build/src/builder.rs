//! The build driver: stage selection, per-instruction execution, layer
//! creation by snapshot diffing, and the step cache.
//!
//! Cache model: each stage keeps a running `chain` hash. Every instruction
//! mixes its resolved text (and, for COPY/ADD, a content digest of the
//! sources) into the chain. Filesystem-changing instructions use the new
//! chain as the cache key; after executing one, an untagged intermediate
//! image is committed and recorded under that key.

use crate::backend::Backend;
use crate::copy::{do_copy, source_digest, CopyCtx, HeredocSrc};
use crate::dockerfile::{self, ArgDecl, CmdForm, Dockerfile, Instruction, Kind};
use crate::fsutil::{clean_abs, copy_tree, resolve_in_root, resolve_owner, rm_rf, unsafe_lchown};
use crate::ignore::DockerIgnore;
use crate::lex;
use crate::snapshot::{snapshot, write_layer_tar, Snapshot};
use hullr_core::api::{Progress, ProgressFn};
use hullr_core::{BuildOptions, Context, Error, HealthCheck, ImageConfig, ImageRef, LogStream, Platform, Result};
use hullr_image::StoredImage;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

const DEFAULT_PATH: &str = "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin";

/// Removes the scratch directory when dropped (success or error).
struct Scratch(PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        rm_rf(&self.0);
    }
}

pub(crate) async fn run_build(
    opts: &BuildOptions,
    be: &dyn Backend,
    work_dir: &Path,
    progress: ProgressFn,
) -> Result<StoredImage> {
    let df_path = match &opts.dockerfile {
        Some(p) if p.is_absolute() || p.exists() => p.clone(),
        Some(p) => opts.context.join(p),
        None => opts.context.join("Dockerfile"),
    };
    let text = std::fs::read_to_string(&df_path)
        .map_err(|e| Error::Build(format!("cannot read Dockerfile {}: {e}", df_path.display())))?;
    let df = dockerfile::parse(&text)?;

    let ignore_text = {
        let mut side = df_path.clone().into_os_string();
        side.push(".dockerignore");
        std::fs::read_to_string(PathBuf::from(side))
            .or_else(|_| std::fs::read_to_string(opts.context.join(".dockerignore")))
            .unwrap_or_default()
    };

    std::fs::create_dir_all(work_dir).context("create build work dir")?;
    let scratch = Scratch(work_dir.join(format!("build-{}", &hullr_core::new_id()[..12])));
    std::fs::create_dir_all(&scratch.0)?;

    let platform = opts.platform.clone().unwrap_or_else(Platform::host);
    let mut global: BTreeMap<String, String> = BTreeMap::new();
    let mut declared: BTreeSet<String> = BTreeSet::new();
    for d in &df.global_args {
        declared.insert(d.name.clone());
        let val = {
            let lk = |k: &str| global.get(k).cloned().or_else(|| builtin_arg(k, &platform));
            match opts.build_args.get(&d.name) {
                Some(v) => Some(v.clone()),
                None => match &d.default {
                    Some(raw) => Some(lex::expand(raw, &lk, df.escape)?),
                    None => None,
                },
            }
        };
        if let Some(v) = val {
            global.insert(d.name.clone(), v);
        }
    }

    let b = Builder {
        opts,
        be,
        progress,
        ctx_dir: opts.context.clone(),
        ignore: DockerIgnore::parse(&ignore_text),
        scratch: scratch.0.clone(),
        platform,
        global,
        step: AtomicUsize::new(0),
        total: 0,
        results: Mutex::new(HashMap::new()),
        ext: Mutex::new(HashMap::new()),
        declared: Mutex::new(declared),
        df,
    };
    let b = {
        let needed = b.df.needed_stages(opts.target.as_deref(), &|s| b.expand_global(s))?;
        let total = needed.iter().map(|i| 1 + b.df.stages[*i].instructions.len()).sum();
        let mut b = b;
        b.total = total;
        (b, needed)
    };
    let (b, needed) = b;
    let mut last = None;
    for (pos, idx) in needed.iter().enumerate() {
        let img = b.build_stage(*idx, pos + 1 == needed.len()).await?;
        b.results.lock().unwrap().insert(*idx, img.clone());
        last = Some(img);
    }
    let unused: Vec<String> = opts
        .build_args
        .keys()
        .filter(|k| !b.declared.lock().unwrap().contains(*k))
        .cloned()
        .collect();
    if !unused.is_empty() {
        b.status(format!("warning: build args not consumed by any ARG: {}", unused.join(", ")));
    }
    last.ok_or_else(|| Error::Build("nothing to build".into()))
}

fn builtin_arg(k: &str, target: &Platform) -> Option<String> {
    let host = Platform::host();
    let v = |p: &Platform| p.variant.clone().unwrap_or_default();
    Some(match k {
        "TARGETPLATFORM" => target.to_string(),
        "TARGETOS" => target.os.clone(),
        "TARGETARCH" => target.architecture.clone(),
        "TARGETVARIANT" => v(target),
        "BUILDPLATFORM" => host.to_string(),
        "BUILDOS" => host.os.clone(),
        "BUILDARCH" => host.architecture.clone(),
        "BUILDVARIANT" => v(&host),
        _ => return None,
    })
}

fn mix(chain: &str, material: &str) -> String {
    let mut h = Sha256::new();
    h.update(chain.as_bytes());
    h.update([0]);
    h.update(material.as_bytes());
    hex::encode(h.finalize())
}

fn short(text: &str) -> String {
    let first = text.lines().next().unwrap_or("");
    let more = text.contains('\n');
    let mut s: String = first.chars().take(120).collect();
    if more || first.chars().count() > 120 {
        s.push_str(" ...");
    }
    s
}

/// Add line context to a build error.
fn at_line(e: Error, line: usize) -> Error {
    match e {
        Error::Build(m) if !m.starts_with("Dockerfile line") && !m.starts_with("step ") => {
            Error::Build(format!("Dockerfile line {line}: {m}"))
        }
        other => other,
    }
}

struct Builder<'a> {
    opts: &'a BuildOptions,
    be: &'a dyn Backend,
    progress: ProgressFn,
    df: Dockerfile,
    ctx_dir: PathBuf,
    ignore: DockerIgnore,
    scratch: PathBuf,
    platform: Platform,
    global: BTreeMap<String, String>,
    step: AtomicUsize,
    total: usize,
    results: Mutex<HashMap<usize, StoredImage>>,
    ext: Mutex<HashMap<String, StoredImage>>,
    declared: Mutex<BTreeSet<String>>,
}

/// Mutable state of the stage being built.
struct St {
    idx: usize,
    platform: Platform,
    base: Option<StoredImage>,
    /// Latest committed image matching the working state.
    cur: Option<StoredImage>,
    layers: Vec<String>,
    config: ImageConfig,
    workdir: String,
    args: BTreeMap<String, Option<String>>,
    chain: String,
    dir: PathBuf,
    rootfs: Option<PathBuf>,
    snap: Option<Snapshot>,
    overrides: HashMap<String, (u32, u32)>,
    layer_no: usize,
}

impl St {
    fn vars(&self) -> HashMap<String, String> {
        let mut m: HashMap<String, String> = HashMap::new();
        for (k, v) in &self.args {
            if let Some(v) = v {
                m.insert(k.clone(), v.clone());
            }
        }
        for e in &self.config.env {
            if let Some((k, v)) = e.split_once('=') {
                m.insert(k.to_string(), v.to_string());
            }
        }
        m
    }

    fn shell(&self) -> Vec<String> {
        self.config.shell.clone().unwrap_or_else(|| vec!["/bin/sh".into(), "-c".into()])
    }

    fn discard_rootfs(&mut self) {
        if let Some(p) = self.rootfs.take() {
            rm_rf(&p);
        }
        self.snap = None;
    }

    fn ensure_rootfs(&mut self, be: &dyn Backend) -> Result<PathBuf> {
        if let Some(p) = &self.rootfs {
            return Ok(p.clone());
        }
        let dir = self.dir.join("rootfs");
        rm_rf(&dir);
        match &self.cur {
            Some(img) => copy_tree(&be.rootfs(img)?, &dir)?,
            None => std::fs::create_dir_all(&dir)?,
        }
        self.snap = Some(snapshot(&dir)?);
        self.rootfs = Some(dir.clone());
        Ok(dir)
    }

    /// Diff the working rootfs against the last snapshot, store the layer
    /// (if any) and commit + cache an intermediate image under `key`.
    fn finish_layer(&mut self, be: &dyn Backend, key: &str) -> Result<()> {
        let root = self.rootfs.clone().ok_or_else(|| Error::Build("internal: no rootfs".into()))?;
        let after = snapshot(&root)?;
        let before = self.snap.take().unwrap_or_default();
        self.layer_no += 1;
        let tar = self.dir.join(format!("layer-{}.tar", self.layer_no));
        let n = write_layer_tar(&root, &before, &after, &self.overrides, &|u, g| be.id_to_container(u, g), &tar)?;
        if n > 0 {
            let diff = be.add_layer_from_tar(&tar)?;
            let _ = std::fs::remove_file(&tar);
            self.layers.push(diff);
        }
        self.snap = Some(after);
        self.overrides.clear();
        let img = be.commit(self.base.as_ref(), &self.layers, self.config.clone(), self.platform.clone(), &[])?;
        let _ = be.cache_put(key, &img.info.id);
        self.cur = Some(img);
        Ok(())
    }

    fn mix(&mut self, material: &str) -> String {
        self.chain = mix(&self.chain, material);
        self.chain.clone()
    }
}

fn set_env(env: &mut Vec<String>, k: &str, v: &str) {
    let prefix = format!("{k}=");
    let entry = format!("{k}={v}");
    match env.iter().position(|e| e.starts_with(&prefix)) {
        Some(i) => env[i] = entry,
        None => env.push(entry),
    }
}

fn parse_dur_secs(s: &str) -> Result<u64> {
    let bad = || Error::Build(format!("bad duration '{s}'"));
    if let Ok(n) = s.parse::<f64>() {
        return Ok(n.ceil() as u64);
    }
    let mut total = 0.0;
    let mut rest = s.trim();
    while !rest.is_empty() {
        let ne = rest.find(|c: char| !(c.is_ascii_digit() || c == '.')).ok_or_else(bad)?;
        if ne == 0 {
            return Err(bad());
        }
        let n: f64 = rest[..ne].parse().map_err(|_| bad())?;
        rest = &rest[ne..];
        let ue = rest.find(|c: char| c.is_ascii_digit() || c == '.').unwrap_or(rest.len());
        let m = match &rest[..ue] {
            "ns" => 1e-9,
            "us" => 1e-6,
            "ms" => 1e-3,
            "s" => 1.0,
            "m" => 60.0,
            "h" => 3600.0,
            _ => return Err(bad()),
        };
        total += n * m;
        rest = &rest[ue..];
    }
    Ok(total.ceil() as u64)
}

/// Expand `80`, `80/udp`, `8000-8002` into normalized `port/proto` entries.
fn expand_ports(spec: &str) -> Result<Vec<String>> {
    let (ports, proto) = spec.split_once('/').map(|(p, pr)| (p, pr.to_ascii_lowercase())).unwrap_or((spec, "tcp".into()));
    let bad = || Error::Build(format!("invalid EXPOSE '{spec}'"));
    let (a, b) = match ports.split_once('-') {
        Some((a, b)) => (a.parse::<u16>().map_err(|_| bad())?, b.parse::<u16>().map_err(|_| bad())?),
        None => {
            let p = ports.parse::<u16>().map_err(|_| bad())?;
            (p, p)
        }
    };
    if b < a {
        return Err(bad());
    }
    Ok((a..=b).map(|p| format!("{p}/{proto}")).collect())
}

struct LineSink {
    out: Mutex<[Vec<u8>; 2]>,
    progress: ProgressFn,
    source: String,
}

impl LineSink {
    fn push(&self, stream: LogStream, data: &[u8]) {
        let i = if stream == LogStream::Stdout { 0 } else { 1 };
        let mut g = self.out.lock().unwrap();
        g[i].extend_from_slice(data);
        while let Some(pos) = g[i].iter().position(|b| *b == b'\n') {
            let line: Vec<u8> = g[i].drain(..=pos).collect();
            self.emit(&line);
        }
    }
    fn flush(&self) {
        let mut g = self.out.lock().unwrap();
        for i in 0..2 {
            if !g[i].is_empty() {
                let line = std::mem::take(&mut g[i]);
                self.emit(&line);
            }
        }
    }
    fn emit(&self, raw: &[u8]) {
        let s = String::from_utf8_lossy(raw);
        (self.progress)(Progress::Output { source: self.source.clone(), line: s.trim_end_matches(['\n', '\r']).to_string() });
    }
}

impl<'a> Builder<'a> {
    fn status(&self, message: String) {
        (self.progress)(Progress::Status { message });
    }

    fn expand_global(&self, s: &str) -> String {
        let lk = |k: &str| self.global.get(k).cloned().or_else(|| builtin_arg(k, &self.platform));
        lex::expand(s, &lk, self.df.escape).unwrap_or_else(|_| s.to_string())
    }

    fn emit_step(&self, n: usize, text: &str, cached: bool) {
        (self.progress)(Progress::Step { step: n, total: self.total, text: text.to_string(), cached });
    }

    async fn build_stage(&self, idx: usize, is_final: bool) -> Result<StoredImage> {
        let stage = &self.df.stages[idx];
        let n = self.step.fetch_add(1, Ordering::SeqCst) + 1;
        let lk = |k: &str| self.global.get(k).cloned().or_else(|| builtin_arg(k, &self.platform));
        let base_ref = lex::expand(&stage.base, &lk, self.df.escape).map_err(|e| at_line(e, stage.line))?;
        let mut platform = match &stage.platform {
            Some(p) => Platform::parse(&lex::expand(p, &lk, self.df.escape).map_err(|e| at_line(e, stage.line))?)?,
            None => self.platform.clone(),
        };
        self.emit_step(n, &format!("FROM {base_ref}"), false);

        let dir = self.scratch.join(format!("stage-{idx}"));
        std::fs::create_dir_all(&dir)?;

        let base: Option<StoredImage> = if base_ref.eq_ignore_ascii_case("scratch") {
            None
        } else if let Some(j) = self.df.find_stage(&base_ref).filter(|j| *j < idx) {
            let img = self
                .results
                .lock()
                .unwrap()
                .get(&j)
                .cloned()
                .ok_or_else(|| Error::Build(format!("stage '{base_ref}' was not built")))?;
            if stage.platform.is_none() {
                platform = img.info.platform.clone();
            }
            Some(img)
        } else {
            Some(self.be.base_image(&base_ref, &platform, self.opts.pull, self.progress.clone()).await?)
        };

        let config = base.as_ref().map(|b| b.info.config.clone()).unwrap_or_default();
        let workdir = config.working_dir.clone().filter(|w| w.starts_with('/')).unwrap_or_else(|| "/".into());
        let mut st = St {
            idx,
            chain: mix(
                "base",
                &format!("{}|{}", base.as_ref().map(|b| b.info.id.as_str()).unwrap_or("scratch"), platform),
            ),
            platform,
            cur: base.clone(),
            base,
            layers: Vec::new(),
            config,
            workdir,
            args: BTreeMap::new(),
            dir,
            rootfs: None,
            snap: None,
            overrides: HashMap::new(),
            layer_no: 0,
        };

        for ins in &stage.instructions {
            let n = self.step.fetch_add(1, Ordering::SeqCst) + 1;
            self.exec(&mut st, ins, n).await.map_err(|e| at_line(e, ins.line))?;
        }

        let tags: Vec<ImageRef> = if is_final {
            st.config.labels.extend(self.opts.labels.clone());
            self.opts.tags.iter().map(|t| ImageRef::parse(t)).collect::<Result<_>>()?
        } else {
            Vec::new()
        };
        let img = self.be.commit(st.base.as_ref(), &st.layers, st.config.clone(), st.platform.clone(), &tags)?;
        st.discard_rootfs();
        Ok(img)
    }

    fn try_cache(&self, st: &mut St, key: &str) -> bool {
        if self.opts.no_cache {
            return false;
        }
        let Some(id) = self.be.cache_get(key) else { return false };
        let Ok(img) = self.be.get(&id) else { return false };
        let base_len = st.base.as_ref().map(|b| b.diff_ids.len()).unwrap_or(0);
        if img.diff_ids.len() < base_len {
            return false;
        }
        st.layers = img.diff_ids[base_len..].to_vec();
        st.cur = Some(img);
        st.discard_rootfs();
        true
    }

    /// Resolve the source of `COPY --from=<r>` to a stored image.
    async fn stage_or_image(&self, st: &St, r: &str) -> Result<StoredImage> {
        if let Some(j) = self.df.find_stage(r) {
            if j >= st.idx {
                return Err(Error::Build(format!("COPY --from={r} refers to a stage that is not earlier")));
            }
            return self
                .results
                .lock()
                .unwrap()
                .get(&j)
                .cloned()
                .ok_or_else(|| Error::Build(format!("stage '{r}' was not built")));
        }
        if let Some(i) = self.ext.lock().unwrap().get(r) {
            return Ok(i.clone());
        }
        let img = self.be.base_image(r, &st.platform, false, self.progress.clone()).await?;
        self.ext.lock().unwrap().insert(r.to_string(), img.clone());
        Ok(img)
    }

    async fn exec(&self, st: &mut St, ins: &Instruction, n: usize) -> Result<()> {
        let vars = st.vars();
        let lk = |k: &str| vars.get(k).cloned();
        let esc = self.df.escape;
        let ex = |w: &str| lex::expand(w, &lk, esc);
        let text = short(&ins.text);
        let meta = |st: &mut St, material: String| {
            self.emit_step(n, &text, false);
            st.mix(&material);
        };

        match &ins.kind {
            Kind::Nop => meta(st, String::new()),
            Kind::Onbuild(raw) => {
                self.status(format!("warning: ONBUILD is not executed ({raw})"));
                meta(st, format!("ONBUILD {raw}"));
            }
            Kind::Env(pairs) => {
                let mut res = Vec::new();
                for (k, v) in pairs {
                    res.push((ex(k)?, ex(v)?));
                }
                for (k, v) in &res {
                    set_env(&mut st.config.env, k, v);
                }
                meta(st, format!("ENV {res:?}"));
            }
            Kind::Label(pairs) => {
                let mut res = Vec::new();
                for (k, v) in pairs {
                    res.push((ex(k)?, ex(v)?));
                }
                for (k, v) in &res {
                    st.config.labels.insert(k.clone(), v.clone());
                }
                meta(st, format!("LABEL {res:?}"));
            }
            Kind::Arg(decls) => {
                let mut mat = Vec::new();
                for ArgDecl { name, default } in decls {
                    self.declared.lock().unwrap().insert(name.clone());
                    let val = match self.opts.build_args.get(name) {
                        Some(v) => Some(v.clone()),
                        None => match default {
                            Some(d) => Some(ex(d)?),
                            None => self.global.get(name).cloned().or_else(|| builtin_arg(name, &st.platform)),
                        },
                    };
                    mat.push(format!("{name}={val:?}"));
                    st.args.insert(name.clone(), val);
                }
                meta(st, format!("ARG {}", mat.join(" ")));
            }
            Kind::Expose(words) => {
                for w in words {
                    for p in expand_ports(&ex(w)?)? {
                        if !st.config.exposed_ports.contains(&p) {
                            st.config.exposed_ports.push(p);
                        }
                    }
                }
                meta(st, format!("EXPOSE {words:?}"));
            }
            Kind::Volume(words) => {
                for w in words {
                    let v = ex(w)?;
                    if !st.config.volumes.contains(&v) {
                        st.config.volumes.push(v);
                    }
                }
                meta(st, format!("VOLUME {words:?}"));
            }
            Kind::User(u) => {
                let u = ex(u)?;
                st.config.user = Some(u.clone());
                meta(st, format!("USER {u}"));
            }
            Kind::Stopsignal(s) => {
                let s = ex(s)?;
                st.config.stop_signal = Some(s.clone());
                meta(st, format!("STOPSIGNAL {s}"));
            }
            Kind::Shell(v) => {
                st.config.shell = Some(v.clone());
                meta(st, format!("SHELL {v:?}"));
            }
            Kind::Cmd(c) => {
                st.config.cmd = Some(self.argv(st, c));
                let m = format!("CMD {:?}", st.config.cmd);
                meta(st, m);
            }
            Kind::Entrypoint(c) => {
                st.config.entrypoint = Some(self.argv(st, c));
                st.config.cmd = None;
                let m = format!("ENTRYPOINT {:?}", st.config.entrypoint);
                meta(st, m);
            }
            Kind::Healthcheck(h) => {
                let hc = if h.none {
                    HealthCheck { test: vec!["NONE".into()], interval_secs: 30, timeout_secs: 30, retries: 3, start_period_secs: 0 }
                } else {
                    let secs = |o: &Option<String>, d: u64| -> Result<u64> {
                        match o {
                            Some(s) => parse_dur_secs(&ex(s)?),
                            None => Ok(d),
                        }
                    };
                    let mut test = Vec::new();
                    match h.cmd.as_ref().unwrap() {
                        CmdForm::Exec(v) => {
                            test.push("CMD".to_string());
                            test.extend(v.clone());
                        }
                        CmdForm::Shell(s) => test.extend(["CMD-SHELL".to_string(), s.clone()]),
                    }
                    HealthCheck {
                        test,
                        interval_secs: secs(&h.interval, 30)?,
                        timeout_secs: secs(&h.timeout, 30)?,
                        retries: match &h.retries {
                            Some(r) => ex(r)?.parse().map_err(|_| Error::Build("bad HEALTHCHECK --retries".into()))?,
                            None => 3,
                        },
                        start_period_secs: secs(&h.start_period, 0)?,
                    }
                };
                let m = format!("HEALTHCHECK {hc:?}");
                st.config.healthcheck = Some(hc);
                meta(st, m);
            }
            Kind::Workdir(raw) => {
                let p = ex(raw)?;
                let abs = if p.starts_with('/') { clean_abs(&p) } else { clean_abs(&format!("{}/{}", st.workdir, p)) };
                st.workdir = abs.clone();
                st.config.working_dir = Some(abs.clone());
                let key = st.mix(&format!("WORKDIR {abs}"));
                let hit = self.try_cache(st, &key);
                self.emit_step(n, &text, hit);
                if !hit {
                    let root = st.ensure_rootfs(self.be)?;
                    let host = resolve_in_root(&root, &abs, true)?;
                    std::fs::create_dir_all(&host).context(format!("create workdir {abs}"))?;
                    st.finish_layer(self.be, &key)?;
                }
            }
            Kind::Run(spec) => {
                for m in spec.mounts.iter().chain(spec.other_flags.iter()) {
                    self.status(format!("warning: ignoring unsupported RUN flag {m}"));
                }
                let cmd = spec.cmd.as_ref().ok_or_else(|| Error::Build("RUN has no command".into()))?;
                let args = self.argv(st, cmd);
                let key = st.mix(&format!("RUN {args:?}|{}|{:?}", st.workdir, st.config.user));
                let hit = self.try_cache(st, &key);
                self.emit_step(n, &text, hit);
                if !hit {
                    let root = st.ensure_rootfs(self.be)?;
                    let mut env = st.config.env.clone();
                    for (k, v) in &st.args {
                        if let Some(v) = v {
                            if !env.iter().any(|e| e.starts_with(&format!("{k}="))) {
                                env.push(format!("{k}={v}"));
                            }
                        }
                    }
                    if !env.iter().any(|e| e.starts_with("PATH=")) {
                        env.push(format!("PATH={DEFAULT_PATH}"));
                    }
                    let sink = Arc::new(LineSink {
                        out: Mutex::new([Vec::new(), Vec::new()]),
                        progress: self.progress.clone(),
                        source: format!("step {n}"),
                    });
                    let cb = {
                        let sink = sink.clone();
                        move |s: LogStream, d: &[u8]| sink.push(s, d)
                    };
                    let code = self
                        .be
                        .run(&root, args, env, st.workdir.clone(), st.config.user.clone(), &st.platform, &cb)
                        .await?;
                    sink.flush();
                    if code != 0 {
                        return Err(Error::Build(format!("step {n}: `{text}` failed with exit code {code}")));
                    }
                    st.finish_layer(self.be, &key)?;
                }
            }
            Kind::Copy(c) | Kind::Add(c) => {
                let is_add = matches!(ins.kind, Kind::Add(_));
                let kw = if is_add { "ADD" } else { "COPY" };
                let sources: Vec<String> = c.sources.iter().map(|s| if s.starts_with("<<") { Ok(s.clone()) } else { ex(s) }).collect::<Result<_>>()?;
                let dest = ex(&c.dest)?;
                let from = c.from.as_ref().map(|f| ex(f)).transpose()?;
                let chown = c.chown.as_ref().map(|f| ex(f)).transpose()?;
                let chmod = match &c.chmod {
                    Some(m) => {
                        let m = ex(m)?;
                        Some(u32::from_str_radix(m.trim_start_matches("0o"), 8).map_err(|_| {
                            Error::Unsupported(format!("--chmod={m}: only octal modes are supported"))
                        })?)
                    }
                    None => None,
                };
                let mut heredocs: Vec<HeredocSrc> = Vec::new();
                for h in &c.heredocs {
                    let body = if h.expand { lex::substitute_plain(&h.body, &lk, esc)? } else { h.body.clone() };
                    heredocs.push((h.name.clone(), body));
                }
                let (src_root, ignore, id_part): (PathBuf, Option<&DockerIgnore>, String) = match &from {
                    Some(f) => {
                        let img = self.stage_or_image(st, f).await?;
                        // Not the image id: a rebuilt stage gets a new id
                        // (its config has a timestamp) even when its files
                        // are identical. `digest` already covers the content.
                        (self.be.rootfs(&img)?, None, String::new())
                    }
                    None => (self.ctx_dir.clone(), Some(&self.ignore), String::new()),
                };
                let cctx = CopyCtx { src_root: &src_root, ignore, dest_root: &st.dir, workdir: &st.workdir, is_add };
                let digest = source_digest(&cctx, &sources, &heredocs)?;
                let key = st.mix(&format!(
                    "{kw} {sources:?} -> {dest}|from={id_part}|chown={chown:?}|chmod={chmod:?}|{digest}|wd={}",
                    st.workdir
                ));
                let hit = self.try_cache(st, &key);
                self.emit_step(n, &text, hit);
                if !hit {
                    let root = st.ensure_rootfs(self.be)?;
                    let cctx = CopyCtx { src_root: &src_root, ignore, dest_root: &root, workdir: &st.workdir, is_add };
                    let touched = do_copy(&cctx, &sources, &dest, &heredocs, chmod)?;
                    if let Some(spec) = chown {
                        let (uid, gid) = resolve_owner(&root, &spec)?;
                        let is_root = unsafe { libc::geteuid() } == 0;
                        for p in touched {
                            if is_root {
                                unsafe_lchown(&p, uid, gid).map_err(|e| Error::Build(format!("chown {}: {e}", p.display())))?;
                            } else if let Ok(rel) = p.strip_prefix(&root) {
                                st.overrides.insert(crate::fsutil::rel_string(rel), (uid, gid));
                            }
                        }
                    }
                    st.finish_layer(self.be, &key)?;
                }
            }
        }
        Ok(())
    }

    /// argv for CMD/ENTRYPOINT/RUN: exec form as is, shell form via SHELL.
    fn argv(&self, st: &St, c: &CmdForm) -> Vec<String> {
        match c {
            CmdForm::Exec(v) => v.clone(),
            CmdForm::Shell(s) => {
                let mut v = st.shell();
                v.push(s.clone());
                v
            }
        }
    }
}
