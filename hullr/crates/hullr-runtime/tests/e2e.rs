//! End-to-end runtime tests. They need Linux, namespaces and an alpine
//! rootfs, so they are `#[ignore]`d; run them with
//! `cargo test -p hullr-runtime -- --ignored --test-threads=1`.
//!
//! The rootfs comes from `$HULLR_TEST_ALPINE_TGZ` (a minirootfs or image
//! layer tarball) or is downloaded from dl-cdn.alpinelinux.org. Foreign
//! arch tests use `$HULLR_TEST_ALPINE_ARM64_TGZ`.

#![cfg(target_os = "linux")]

use hullr_core::stream::{read_frame, write_frame, Frame};
use hullr_core::{ExecSpec, LogStream, MountKind, NetworkMode, Platform, RestartPolicy};
use hullr_runtime::{ResolvedMount, RunSpec, Runtime, RuntimeConfig, Status};
use std::path::{Path, PathBuf};
use std::time::Duration;

const ALPINE_URL: &str = "https://dl-cdn.alpinelinux.org/alpine/v3.20/releases/x86_64/alpine-minirootfs-3.20.3-x86_64.tar.gz";

fn exe() -> PathBuf {
    // target/debug/deps/e2e-xxxx -> target/debug/examples/hullr-rt
    let me = std::env::current_exe().unwrap();
    let p = me.parent().unwrap().parent().unwrap().join("examples/hullr-rt");
    assert!(p.exists(), "build the example first: cargo build -p hullr-runtime --example hullr-rt");
    p
}

fn rootfs(var: &str, url: Option<&str>, into: &Path) -> Option<PathBuf> {
    let tgz = match std::env::var_os(var) {
        Some(p) => PathBuf::from(p),
        None => {
            let url = url?;
            let p = std::env::temp_dir().join("hullr-test-alpine.tar.gz");
            if !p.exists() {
                let ok = std::process::Command::new("curl").args(["-sSfL", "-o"]).arg(&p).arg(url).status().ok()?.success();
                if !ok {
                    let _ = std::fs::remove_file(&p);
                    return None;
                }
            }
            p
        }
    };
    std::fs::create_dir_all(into).unwrap();
    let ok = std::process::Command::new("tar").arg("-xzf").arg(&tgz).arg("-C").arg(into).status().unwrap().success();
    ok.then(|| into.to_path_buf())
}

struct Env {
    _tmp: tempfile::TempDir,
    rt: Runtime,
    lower: PathBuf,
    root: PathBuf,
}

/// Kill anything a failed test left running, so panics do not leak shims.
impl Drop for Env {
    fn drop(&mut self) {
        let Ok(rd) = std::fs::read_dir(self.root.join("containers")) else { return };
        for d in rd.flatten() {
            let id = d.file_name().to_string_lossy().into_owned();
            if let Ok(st) = self.rt.state(&id) {
                if let Some(pid) = st.pid {
                    let _ = std::process::Command::new("kill").args(["-9", &pid.to_string()]).status();
                }
            }
            if let Ok(p) = std::fs::read_to_string(d.path().join("shim.pid")) {
                let _ = std::process::Command::new("kill").args(["-9", p.trim()]).status();
            }
        }
        let _ = self.rt.remove_sandbox("proj");
    }
}

fn env() -> Env {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().to_path_buf();
    let lower = rootfs("HULLR_TEST_ALPINE_TGZ", Some(ALPINE_URL), &root.join("alpine")).expect("alpine rootfs");
    let cfg = RuntimeConfig { containers_dir: root.join("containers"), sandboxes_dir: root.join("sandboxes"), exe: exe() };
    std::fs::create_dir_all(&cfg.containers_dir).unwrap();
    Env { rt: Runtime::new(cfg).unwrap(), lower, root, _tmp: tmp }
}

fn spec(e: &Env, id: &str, args: &[&str]) -> RunSpec {
    RunSpec {
        id: id.into(),
        hostname: id.into(),
        lower: e.lower.clone(),
        args: args.iter().map(|s| s.to_string()).collect(),
        env: vec![],
        cwd: "/".into(),
        user: None,
        mounts: vec![],
        ports: vec![],
        network: NetworkMode::None,
        extra_hosts: vec![],
        tty: false,
        interactive: false,
        platform: Platform::host(),
        restart: RestartPolicy::No,
        stop_signal: None,
        memory_limit: None,
        cpus: None,
    }
}

fn logs(rt: &Runtime, id: &str) -> Vec<String> {
    std::fs::read_to_string(rt.log_path(id))
        .unwrap_or_default()
        .lines()
        .map(|l| serde_json::from_str::<hullr_core::LogLine>(l).unwrap().line)
        .collect()
}

/// Collect output frames until Exit; returns (stdout+stderr text, code).
async fn drain(s: tokio::net::UnixStream) -> (String, i32) {
    let (mut r, _w) = s.into_split();
    let mut out = Vec::new();
    while let Some(f) = read_frame(&mut r).await.unwrap() {
        match f {
            Frame::Stdout(d) | Frame::Stderr(d) => out.extend(d),
            Frame::Error(e) => out.extend(e.into_bytes()),
            Frame::Exit(c) => return (String::from_utf8_lossy(&out).into_owned(), c),
            _ => {}
        }
    }
    (String::from_utf8_lossy(&out).into_owned(), -1)
}

async fn run(e: &Env, s: RunSpec) -> (Vec<String>, i32) {
    let id = s.id.clone();
    e.rt.create(&s).unwrap();
    e.rt.start(&id, false).await.unwrap();
    let code = tokio::time::timeout(Duration::from_secs(30), e.rt.wait(&id)).await.unwrap().unwrap();
    let l = logs(&e.rt, &id);
    e.rt.remove(&id).unwrap();
    (l, code)
}

#[tokio::test]
#[ignore]
async fn echo_detached_and_errors() {
    let e = env();
    let (mut l, code) = run(&e, spec(&e, "echo", &["sh", "-c", "echo hi; echo oops >&2; exit 3"])).await;
    assert_eq!(code, 3);
    // stdout and stderr are separate pipes, so their relative order is not
    // guaranteed (same as Docker).
    l.sort();
    assert_eq!(l, ["hi", "oops"].map(String::from).to_vec());
    // A missing binary is reported by start itself.
    e.rt.create(&spec(&e, "missing", &["/bin/foo"])).unwrap();
    let err = e.rt.start("missing", false).await.unwrap_err().to_string();
    assert!(err.contains("exec: no such file: /bin/foo"), "{err}");
    assert_eq!(e.rt.state("missing").unwrap().status, Status::Exited);
    e.rt.remove("missing").unwrap();
}

#[tokio::test]
#[ignore]
async fn tty_shell_via_attach() {
    let e = env();
    let mut s = spec(&e, "tty", &["sh"]);
    s.tty = true;
    s.interactive = true;
    e.rt.create(&s).unwrap();
    e.rt.start("tty", true).await.unwrap();
    let conn = e.rt.attach("tty").await.unwrap();
    let (mut r, mut w) = conn.into_split();
    write_frame(&mut w, &Frame::Resize { cols: 123, rows: 45 }).await.unwrap();
    write_frame(&mut w, &Frame::Stdin(b"tty; stty size; echo $TERM; exit 7\n".to_vec())).await.unwrap();
    let mut out = Vec::new();
    let code = loop {
        match tokio::time::timeout(Duration::from_secs(15), read_frame(&mut r)).await.unwrap().unwrap() {
            Some(Frame::Stdout(d)) => out.extend(d),
            Some(Frame::Exit(c)) => break c,
            Some(_) => {}
            None => panic!("eof"),
        }
    };
    let out = String::from_utf8_lossy(&out);
    assert_eq!(code, 7, "{out}");
    assert!(out.contains("/dev/pts/0"), "{out}");
    assert!(out.contains("45 123"), "{out}");
    assert!(out.contains("xterm"), "{out}");
    assert_eq!(e.rt.wait("tty").await.unwrap(), 7);
    // tty logs are split on newlines with \r stripped.
    assert!(logs(&e.rt, "tty").iter().any(|l| l == "/dev/pts/0"));
    e.rt.remove("tty").unwrap();
}

#[tokio::test]
#[ignore]
async fn exec_stop_and_stats() {
    let e = env();
    e.rt.create(&spec(&e, "sleeper", &["sleep", "300"])).unwrap();
    e.rt.start("sleeper", false).await.unwrap();
    assert_eq!(e.rt.state("sleeper").unwrap().status, Status::Running);
    let (out, code) = drain(e.rt.exec("sleeper", &ExecSpec { cmd: vec!["ps".into()], ..Default::default() }).await.unwrap()).await;
    assert_eq!(code, 0);
    let first = out.lines().nth(1).unwrap_or_default();
    assert!(first.trim_start().starts_with("1 ") && first.contains("sleep 300"), "{out}");
    let (out, code) = drain(
        e.rt.exec("sleeper", &ExecSpec { cmd: vec!["sh".into(), "-c".into(), "tty; exit 4".into()], tty: true, ..Default::default() })
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(code, 4);
    assert!(out.contains("/dev/pts/"), "{out}");
    let (out, code) = drain(e.rt.exec("sleeper", &ExecSpec { cmd: vec!["nope".into()], ..Default::default() }).await.unwrap()).await;
    assert_eq!(code, 127, "{out}");
    let st = e.rt.stats("sleeper").unwrap();
    assert!(st.pids >= 1 && st.memory_bytes > 0, "{st:?}");
    let t = std::time::Instant::now();
    // sleep as pid 1 ignores SIGTERM, so this needs the SIGKILL fallback.
    e.rt.stop("sleeper", 1).await.unwrap();
    assert!(t.elapsed() < Duration::from_secs(5));
    let st = e.rt.state("sleeper").unwrap();
    assert_eq!((st.status, st.exit_code, st.stopped_by_user), (Status::Exited, Some(137), true));
    e.rt.remove("sleeper").unwrap();
    assert!(!e.root.join("containers/sleeper").exists());
}

#[tokio::test]
#[ignore]
async fn restart_on_failure() {
    let e = env();
    let mut s = spec(&e, "flaky", &["sh", "-c", "echo run; exit 1"]);
    s.restart = RestartPolicy::OnFailure(2);
    let (l, code) = run(&e, s).await;
    assert_eq!(code, 1);
    assert_eq!(l, ["run", "run", "run"].map(String::from).to_vec());
}

#[tokio::test]
#[ignore]
async fn mounts_and_networks() {
    let e = env();
    let host = e.root.join("shared");
    std::fs::create_dir_all(&host).unwrap();
    std::fs::write(host.join("in.txt"), "from-host\n").unwrap();
    let mut s = spec(&e, "mnt", &["sh", "-c", "cat /data/in.txt; echo back > /data/out.txt; echo x > /ro/f || echo ro-ok; ls /scratch && echo tmpfs-ok"]);
    s.mounts = vec![
        ResolvedMount { kind: MountKind::Bind, source: host.clone(), target: "/data".into(), read_only: false },
        ResolvedMount { kind: MountKind::Bind, source: host.clone(), target: "/ro".into(), read_only: true },
        ResolvedMount { kind: MountKind::Tmpfs, source: PathBuf::new(), target: "/scratch".into(), read_only: false },
    ];
    let (l, code) = run(&e, s).await;
    assert_eq!(code, 0, "{l:?}");
    assert!(l.contains(&"from-host".to_string()) && l.contains(&"ro-ok".to_string()) && l.contains(&"tmpfs-ok".to_string()), "{l:?}");
    assert_eq!(std::fs::read_to_string(host.join("out.txt")).unwrap(), "back\n");

    let ifaces = "cat /proc/net/dev | tail -n +3 | cut -d: -f1 | tr -d ' ' | sort | tr '\\n' ,";
    let (l, _) = run(&e, spec(&e, "netnone", &["sh", "-c", ifaces])).await;
    assert_eq!(l, ["lo,"].map(String::from).to_vec());
    let mut s = spec(&e, "nethost", &["sh", "-c", ifaces]);
    s.network = NetworkMode::Host;
    let (l, _) = run(&e, s).await;
    let host_ifaces = std::fs::read_to_string("/proc/net/dev").unwrap().lines().count() - 2;
    assert_eq!(l[0].matches(',').count(), host_ifaces, "{l:?}");
}

#[tokio::test]
#[ignore]
async fn private_network_with_ports() {
    let e = env();
    if !e.rt.capabilities().user_networking {
        eprintln!("skipping: no pasta/slirp4netns");
        return;
    }
    let mut s = spec(&e, "web", &["sh", "-c", "ip -4 addr show | grep -q 'scope global' && echo has-addr; echo hello-port | nc -l -p 80"]);
    s.network = NetworkMode::Private;
    s.ports = vec![hullr_core::PortMapping::parse("127.0.0.1:18089:80").unwrap()];
    e.rt.create(&s).unwrap();
    e.rt.start("web", false).await.unwrap();
    let mut got = String::new();
    for _ in 0..50 {
        if let Ok(mut c) = std::net::TcpStream::connect("127.0.0.1:18089") {
            use std::io::Read;
            c.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
            let _ = c.read_to_string(&mut got);
            if !got.is_empty() {
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(got.trim(), "hello-port");
    let _ = tokio::time::timeout(Duration::from_secs(10), e.rt.wait("web")).await;
    assert!(logs(&e.rt, "web").contains(&"has-addr".to_string()));
    let _ = e.rt.stop("web", 1).await;
    e.rt.remove("web").unwrap();
}

#[tokio::test]
#[ignore]
async fn sandbox_shared_netns() {
    let e = env();
    e.rt.ensure_sandbox("proj", &[]).unwrap();
    let mut a = spec(&e, "srv", &["sh", "-c", "echo from-srv | nc -l -p 9000"]);
    a.network = NetworkMode::Sandbox("proj".into());
    e.rt.create(&a).unwrap();
    e.rt.start("srv", false).await.unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    let mut b = spec(&e, "cli", &["sh", "-c", "nc 127.0.0.1 9000 </dev/null"]);
    b.network = NetworkMode::Sandbox("proj".into());
    let (l, _) = run(&e, b).await;
    assert_eq!(l, ["from-srv"].map(String::from).to_vec());
    let _ = tokio::time::timeout(Duration::from_secs(5), e.rt.wait("srv")).await;
    let _ = e.rt.stop("srv", 1).await;
    e.rt.remove("srv").unwrap();
    // Adding ports later restarts the forwarder with the union.
    e.rt.ensure_sandbox("proj", &[hullr_core::PortMapping::parse("18090:80").unwrap()]).unwrap();
    e.rt.remove_sandbox("proj").unwrap();
    assert!(!e.root.join("sandboxes/proj").exists());
}

#[tokio::test]
#[ignore]
async fn run_in_rootfs_build_step() {
    let e = env();
    let out = std::sync::Mutex::new(Vec::new());
    let cb = |s: LogStream, d: &[u8]| {
        if s == LogStream::Stdout {
            out.lock().unwrap().extend_from_slice(d)
        }
    };
    let code = e
        .rt
        .run_in_rootfs(&e.lower, vec!["sh".into(), "-c".into(), "apk --version && touch /built".into()], vec![], "/".into(), None, &Platform::host(), &cb)
        .await
        .unwrap();
    assert_eq!(code, 0);
    assert!(String::from_utf8_lossy(&out.lock().unwrap()).contains("apk-tools"));
    assert!(e.lower.join("built").exists(), "writes land in the rootfs dir");
}

#[tokio::test]
#[ignore]
async fn foreign_arch() {
    let e = env();
    let arm = rootfs("HULLR_TEST_ALPINE_ARM64_TGZ", None, &e.root.join("arm64"));
    let Some(arm) = arm else {
        eprintln!("skipping: set HULLR_TEST_ALPINE_ARM64_TGZ");
        return;
    };
    let p = Platform::parse("linux/arm64").unwrap();
    if !e.rt.capabilities().emulated.iter().any(|x| x.architecture == "arm64") {
        let err = hullr_runtime::emulation::check(&p).unwrap_err().to_string();
        assert!(err.contains("hullr system emulation enable"));
        eprintln!("skipping: arm64 binfmt not registered");
        return;
    }
    let mut s = spec(&e, "arm", &["uname", "-m"]);
    s.lower = arm;
    s.platform = p;
    let (l, code) = run(&e, s).await;
    assert_eq!((code, l), (0, vec!["aarch64".to_string()]));
}
