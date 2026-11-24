//! Linux implementation behind the public `Runtime` methods. Each method
//! is a thin client of the files and helpers the shim maintains.

use crate::state::{self, InitConfig, Layout, NetPlan};
use crate::util::{rt_err, short_path};
use crate::{cgroup, emulation, net, sandbox, shim, Capabilities, RunSpec, Runtime, RuntimeState, Status};
use hullr_core::{Error, ExecSpec, LogStream, NetworkMode, Platform, PortMapping, RestartPolicy, Result, Stats};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, Instant};
use tokio::net::UnixStream;

impl Runtime {
    fn layout(&self, id: &str) -> Result<Layout> {
        if id.is_empty() || id.contains('/') || id.starts_with('.') {
            return Err(Error::Invalid(format!("bad container id '{id}'")));
        }
        Ok(Layout::new(self.cfg.containers_dir.join(id)))
    }

    fn existing(&self, id: &str) -> Result<Layout> {
        let l = self.layout(id)?;
        if !l.spec().exists() {
            return Err(Error::NotFound(format!("container {id}")));
        }
        Ok(l)
    }
}

fn overlay_supported(rootless: bool) -> bool {
    static CACHE: OnceLock<bool> = OnceLock::new();
    *CACHE.get_or_init(|| {
        let listed = std::fs::read_to_string("/proc/filesystems").map(|s| s.contains("\toverlay")).unwrap_or(false);
        if !listed || std::env::var_os("HULLR_NO_OVERLAY").is_some() {
            return false;
        }
        if !rootless {
            return true;
        }
        // Unprivileged overlay mounts need Linux 5.11+.
        let rel = nix::sys::utsname::uname().map(|u| u.release().to_string_lossy().into_owned()).unwrap_or_default();
        let mut it = rel.split(|c: char| !c.is_ascii_digit()).filter_map(|s| s.parse::<u32>().ok());
        let (maj, min) = (it.next().unwrap_or(0), it.next().unwrap_or(0));
        (maj, min) >= (5, 11)
    })
}

pub fn capabilities(_rt: &Runtime) -> Capabilities {
    let rootless = crate::util::euid() != 0;
    let mut warnings = vec![];
    let overlay = overlay_supported(rootless);
    if !overlay {
        warnings.push("overlayfs unavailable: containers get a full copy of their image".into());
    }
    let user_networking = net::detect_user_net().is_some();
    if !user_networking {
        warnings.push("pasta/slirp4netns not found: private networking falls back to host networking, ports are not published".into());
    }
    let cgroups = cgroup::base().is_some();
    if !cgroups {
        warnings.push("no delegated cgroup v2: memory/cpu limits are ignored, stats come from /proc".into());
    }
    if rootless && !crate::ns::IdMapping::detect().full() {
        warnings.push("no /etc/subuid range (or newuidmap): containers can only use uid 0".into());
    }
    Capabilities { rootless, overlay, user_networking, cgroups, emulated: emulation::emulated(), warnings }
}

pub fn create(rt: &Runtime, spec: &RunSpec) -> Result<()> {
    let l = rt.layout(&spec.id)?;
    if l.spec().exists() {
        return Err(Error::Conflict(format!("container {}", spec.id)));
    }
    if !spec.lower.is_dir() {
        return Err(Error::NotFound(format!("image rootfs {}", spec.lower.display())));
    }
    for d in [l.dir.clone(), l.rootfs(), l.upper(), l.work()] {
        std::fs::create_dir_all(&d)?;
    }
    let host_hosts = std::fs::read_to_string("/etc/hosts").unwrap_or_default();
    let host_net = matches!(spec.network, NetworkMode::Host);
    std::fs::write(l.file("hosts"), net::hosts_file(&spec.hostname, &spec.network, &spec.extra_hosts, &host_hosts))?;
    std::fs::write(l.file("resolv.conf"), net::host_resolv_conf(host_net))?;
    std::fs::write(l.file("hostname"), format!("{}\n", spec.hostname))?;
    l.write_state(&state::initial_state())?;
    hullr_core::paths::write_atomic(&l.spec(), &serde_json::to_vec_pretty(spec)?)?;
    Ok(())
}

pub async fn start(rt: &Runtime, id: &str, wait_for_attach: bool) -> Result<()> {
    let l = rt.existing(id)?;
    let spec = l.read_spec()?;
    emulation::check(&spec.platform)?;
    if state::shim_pid(&l).is_some() {
        return Err(Error::Conflict(format!("container {id} is already running")));
    }
    let _ = std::fs::remove_file(l.stopped());
    let exe = rt.cfg.exe.clone();
    let sandboxes = rt.cfg.sandboxes_dir.clone();
    let dir = l.dir.clone();
    let line = tokio::task::spawn_blocking(move || {
        let mut args: Vec<&std::ffi::OsStr> = vec!["__shim".as_ref(), dir.as_os_str()];
        if wait_for_attach {
            args.push("--wait-attach".as_ref());
        }
        crate::util::spawn_detached(
            &exe,
            &args,
            &[(shim::ENV_EXE, exe.as_os_str()), (shim::ENV_SANDBOXES, sandboxes.as_os_str())],
            // Generous: the copy fallback may duplicate a large image.
            Duration::from_secs(600),
        )
    })
    .await
    .map_err(|e| rt_err(format!("start task: {e}")))??;
    match line.strip_prefix("ERR ") {
        None if line == "OK" => Ok(()),
        Some(m) => Err(Error::Runtime(m.to_string())),
        None => Err(rt_err(format!("unexpected shim reply: {line}"))),
    }
}

pub fn state(rt: &Runtime, id: &str) -> Result<RuntimeState> {
    let l = rt.existing(id)?;
    let st = l.read_state().unwrap_or_else(|_| state::initial_state());
    Ok(state::reconcile(st, state::shim_pid(&l).is_some()))
}

pub async fn wait(rt: &Runtime, id: &str) -> Result<i32> {
    let l = rt.existing(id)?;
    loop {
        let alive = state::shim_pid(&l).is_some();
        let st = state::reconcile(l.read_state().unwrap_or_else(|_| state::initial_state()), alive);
        if st.status == Status::Exited && !alive {
            return Ok(st.exit_code.unwrap_or(0));
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

fn signal_pid(pid: u32, sig: i32) -> Result<()> {
    // SAFETY: kill(2) with a pid recorded by the shim.
    if unsafe { libc::kill(pid as i32, sig) } != 0 {
        let e = std::io::Error::last_os_error();
        if e.raw_os_error() != Some(libc::ESRCH) {
            return Err(rt_err(format!("kill {pid}: {e}")));
        }
    }
    Ok(())
}

fn stop_signal(spec: &RunSpec) -> i32 {
    spec.stop_signal.as_deref().and_then(|s| crate::parse_signal(s).ok()).unwrap_or(libc::SIGTERM)
}

pub fn kill(rt: &Runtime, id: &str, signal: i32) -> Result<()> {
    let l = rt.existing(id)?;
    let st = state(rt, id)?;
    if !matches!(st.status, Status::Running | Status::Restarting) {
        return Err(Error::Conflict(format!("container {id} is not running")));
    }
    let spec = l.read_spec()?;
    if signal == libc::SIGKILL || signal == libc::SIGTERM || signal == stop_signal(&spec) {
        std::fs::write(l.stopped(), "")?;
    }
    if let Some(pid) = st.pid {
        signal_pid(pid, signal)?;
    }
    Ok(())
}

pub async fn stop(rt: &Runtime, id: &str, timeout_secs: u32) -> Result<()> {
    let l = rt.existing(id)?;
    let spec = l.read_spec()?;
    if state::shim_pid(&l).is_none() {
        return Ok(());
    }
    std::fs::write(l.stopped(), "")?;
    let gone = |l: &Layout| state::shim_pid(l).is_none();
    let wait_gone = |deadline: Instant| {
        let l = l.clone();
        async move {
            while !gone(&l) && Instant::now() < deadline {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            gone(&l)
        }
    };
    if let Some(pid) = l.read_state().ok().and_then(|s| s.pid) {
        signal_pid(pid, stop_signal(&spec))?;
    }
    if wait_gone(Instant::now() + Duration::from_secs(timeout_secs as u64)).await {
        return Ok(());
    }
    if let Some(pid) = l.read_state().ok().and_then(|s| s.pid) {
        signal_pid(pid, libc::SIGKILL)?;
    }
    if wait_gone(Instant::now() + Duration::from_secs(10)).await {
        Ok(())
    } else {
        Err(rt_err(format!("container {id} did not stop")))
    }
}

pub async fn attach(rt: &Runtime, id: &str) -> Result<UnixStream> {
    let l = rt.existing(id)?;
    if state::shim_pid(&l).is_none() {
        return Err(Error::Conflict(format!("container {id} is not running")));
    }
    let (_dir, p) = short_path(&l.dir, "shim.sock")?;
    let s = std::os::unix::net::UnixStream::connect(&p).map_err(|e| rt_err(format!("attach: {e}")))?;
    s.set_nonblocking(true)?;
    Ok(UnixStream::from_std(s)?)
}

pub async fn exec(rt: &Runtime, id: &str, spec: &ExecSpec) -> Result<UnixStream> {
    let l = rt.existing(id)?;
    let st = state(rt, id)?;
    let pid = match (st.status, st.pid) {
        (Status::Running, Some(p)) => p,
        _ => return Err(Error::Conflict(format!("container {id} is not running"))),
    };
    if spec.cmd.is_empty() {
        return Err(Error::Invalid("exec needs a command".into()));
    }
    let s = crate::exec::spawn(&rt.cfg.exe, &l.dir, pid, spec)?;
    s.set_nonblocking(true)?;
    Ok(UnixStream::from_std(s)?)
}

pub fn stats(rt: &Runtime, id: &str) -> Result<Stats> {
    let l = rt.existing(id)?;
    let st = state(rt, id)?;
    let (Status::Running, Some(pid)) = (st.status, st.pid) else { return Ok(Stats::default()) };
    let spec = l.read_spec()?;
    Ok(rt.tracker.stats(id, pid, spec.memory_limit))
}

pub fn remove(rt: &Runtime, id: &str) -> Result<()> {
    let l = rt.existing(id)?;
    if state::shim_pid(&l).is_some() {
        return Err(Error::Conflict(format!("container {id} is running")));
    }
    for m in [l.rootfs(), l.file("netns")] {
        let _ = nix::mount::umount2(&m, nix::mount::MntFlags::MNT_DETACH);
    }
    cgroup::remove(id);
    remove_tree(&rt.cfg.exe, &l.dir)
}

/// Remove a directory that may contain files owned by subordinate ids or
/// overlay's mode-000 work dirs: retry inside a user namespace if needed.
fn remove_tree(exe: &Path, dir: &Path) -> Result<()> {
    match std::fs::remove_dir_all(dir) {
        Ok(()) => return Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) if crate::util::euid() == 0 => return Err(e.into()),
        Err(_) => {}
    }
    let out = std::process::Command::new(exe)
        .arg("__rm")
        .arg(dir)
        .output()
        .map_err(|e| rt_err(format!("spawn remove helper: {e}")))?;
    if !out.status.success() || dir.exists() {
        return Err(rt_err(format!("remove {}: {}", dir.display(), String::from_utf8_lossy(&out.stderr).trim())));
    }
    Ok(())
}

/// `__rm <dir>`: delete a tree as root of a fresh user namespace.
pub fn rm_main(dir: &Path) -> i32 {
    if crate::util::euid() != 0 {
        if let Err(e) = crate::ns::enter_userns(&crate::ns::IdMapping::detect()) {
            eprintln!("{e}");
            return 1;
        }
        crate::ns::become_root();
    }
    match std::fs::remove_dir_all(dir) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("{e}");
            1
        }
    }
}

pub fn ensure_sandbox(rt: &Runtime, name: &str, ports: &[PortMapping]) -> Result<()> {
    let dir = sandbox::dir_for(&rt.cfg.sandboxes_dir, name)?;
    sandbox::ensure(&rt.cfg.exe, &dir, &rt.cfg.sandboxes_dir, ports)
}

pub fn remove_sandbox(rt: &Runtime, name: &str) -> Result<()> {
    sandbox::remove(&sandbox::dir_for(&rt.cfg.sandboxes_dir, name)?)
}

static RUN_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

#[allow(clippy::too_many_arguments)]
pub async fn run_in_rootfs(
    rt: &Runtime,
    rootfs: &Path,
    args: Vec<String>,
    env: Vec<String>,
    cwd: String,
    user: Option<String>,
    platform: &Platform,
    output: &(dyn Fn(LogStream, &[u8]) + Send + Sync),
) -> Result<i32> {
    use std::os::fd::AsRawFd;
    use tokio::io::AsyncReadExt;
    emulation::check(platform)?;
    let rootfs = std::fs::canonicalize(rootfs).map_err(|e| rt_err(format!("rootfs {}: {e}", rootfs.display())))?;
    let seq = RUN_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let tmp: PathBuf = std::env::temp_dir().join(format!("hullr-run-{}-{seq}", std::process::id()));
    std::fs::create_dir_all(&tmp)?;
    struct Cleanup(PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(tmp.clone());
    let hostname = "hullr-build".to_string();
    let host_hosts = std::fs::read_to_string("/etc/hosts").unwrap_or_default();
    std::fs::write(tmp.join("hosts"), net::hosts_file(&hostname, &NetworkMode::Host, &[], &host_hosts))?;
    std::fs::write(tmp.join("resolv.conf"), net::host_resolv_conf(true))?;
    let spec = RunSpec {
        id: "build".into(),
        hostname,
        lower: rootfs.clone(),
        args,
        env,
        cwd,
        user,
        mounts: vec![],
        ports: vec![],
        network: NetworkMode::Host,
        extra_hosts: vec![],
        tty: false,
        interactive: false,
        platform: platform.clone(),
        restart: RestartPolicy::No,
        stop_signal: None,
        memory_limit: None,
        cpus: None,
    };
    let cfg = InitConfig {
        spec,
        rootfs,
        overlay: None,
        copied_marker: None,
        etc_files: vec![
            (tmp.join("hosts"), "/etc/hosts".into()),
            (tmp.join("resolv.conf"), "/etc/resolv.conf".into()),
        ],
        net: NetPlan::Host,
    };
    let cfg_path = tmp.join("init.json");
    std::fs::write(&cfg_path, serde_json::to_vec(&cfg)?)?;

    let (ours, theirs) = std::os::unix::net::UnixStream::pair()?;
    let tfd = theirs.as_raw_fd();
    let mut cmd = tokio::process::Command::new(&rt.cfg.exe);
    cmd.arg("__init")
        .arg(&cfg_path)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    // SAFETY: only async-signal-safe calls between fork and exec.
    unsafe {
        cmd.pre_exec(move || {
            libc::setsid();
            crate::util::install_handoff_fd(tfd)
        });
    }
    let mut child = cmd.spawn().map_err(|e| rt_err(format!("spawn init: {e}")))?;
    drop(cmd);
    drop(theirs);
    let sync = tokio::task::spawn_blocking(move || -> (std::result::Result<(), String>, Vec<String>) {
        let fd = ours.as_raw_fd();
        let mut warns = vec![];
        while let Some(line) = crate::util::read_line(fd) {
            let (c, a) = line.split_once(' ').unwrap_or((line.as_str(), ""));
            match c {
                "NS" => {
                    crate::util::write_all_fd(fd, b"GO\n");
                }
                "WARN" => warns.push(a.to_string()),
                "OK" => return (Ok(()), warns),
                "ERR" => return (Err(a.to_string()), warns),
                _ => {}
            }
        }
        (Err("build step init exited during setup".into()), warns)
    });
    let mut out = child.stdout.take().expect("piped");
    let mut err = child.stderr.take().expect("piped");
    let (mut ob, mut eb) = (vec![0u8; 16384], vec![0u8; 16384]);
    let (mut o_open, mut e_open) = (true, true);
    while o_open || e_open {
        tokio::select! {
            r = out.read(&mut ob), if o_open => match r {
                Ok(0) | Err(_) => o_open = false,
                Ok(n) => output(LogStream::Stdout, &ob[..n]),
            },
            r = err.read(&mut eb), if e_open => match r {
                Ok(0) | Err(_) => e_open = false,
                Ok(n) => output(LogStream::Stderr, &eb[..n]),
            },
        }
    }
    let status = child.wait().await?;
    let (res, warns) = sync.await.map_err(|e| rt_err(format!("sync task: {e}")))?;
    for w in warns {
        output(LogStream::Stderr, format!("hullr: warning: {w}\n").as_bytes());
    }
    res.map_err(Error::Runtime)?;
    Ok(crate::util::exit_code(status))
}
