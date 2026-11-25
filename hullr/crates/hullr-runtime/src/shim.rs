//! `__shim <container_dir> [--wait-attach]`: the per-container supervisor.
//!
//! Spawned detached by `Runtime::start` (it forks and starts a new session
//! itself), so containers outlive the CLI and no daemon is needed. It:
//! - spawns `__init` for each run and answers its sync protocol (attaching
//!   pasta/slirp4netns and the cgroup between `NS` and `GO`),
//! - owns the container's stdio (a pty when `tty`, else pipes), writes every
//!   output line to `log.jsonl`, keeps a 64 KiB ring buffer replayed to new
//!   attach clients, and serves `shim.sock` with `hullr_core::stream` frames,
//! - records `state.json` and applies the restart policy with backoff,
//! - reports the first start's outcome on the pipe at fd 3 (`OK` or
//!   `ERR <message>`), which is what `Runtime::start` waits on.

use crate::state::{self, Backoff, InitConfig, Layout, NetPlan};
use crate::util::{install_handoff_fd, now_ms, read_line_fd, write_all_fd};
use crate::{cgroup, net, RunSpec, RuntimeState, Status};
use hullr_core::stream::{blocking, Frame};
use hullr_core::{LogLine, LogStream, NetworkMode};
use std::collections::VecDeque;
use std::fs::File;
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::net::{UnixListener, UnixStream};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{sync_channel, SyncSender, TrySendError};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

const RING_BYTES: usize = 64 * 1024;
pub const ENV_EXE: &str = "HULLR_RT_EXE";
pub const ENV_SANDBOXES: &str = "HULLR_RT_SANDBOXES";

/// Where client stdin goes for the current run.
#[derive(Default)]
struct Input {
    stdin: Option<Arc<File>>,
    master: Option<Arc<File>>,
    winsize: Option<(u16, u16)>,
    /// Stdin that arrived before the process had a stdin (attach races
    /// the init's setup); flushed once the target exists.
    pending: Vec<u8>,
    /// CloseStdin arrived before the stdin pipe existed.
    pending_close: bool,
}

impl Input {
    fn set_target(&mut self, tty: bool, f: Arc<File>) {
        if !self.pending.is_empty() {
            let _ = (&*f).write_all(&std::mem::take(&mut self.pending));
        }
        if tty {
            self.master = Some(f);
        } else if !std::mem::take(&mut self.pending_close) {
            self.stdin = Some(f);
        }
    }
}

struct Inner {
    clients: Vec<(u64, SyncSender<Frame>)>,
    next_id: u64,
    ring: VecDeque<Frame>,
    ring_bytes: usize,
    log: Option<File>,
    partial: [Vec<u8>; 2],
    attached_once: bool,
}

struct Hub {
    inner: Mutex<Inner>,
    input: Mutex<Input>,
    attached: Condvar,
    tty: bool,
    interactive: bool,
}

fn frame_len(f: &Frame) -> usize {
    match f {
        Frame::Stdout(d) | Frame::Stderr(d) => d.len(),
        _ => 0,
    }
}

impl Hub {
    fn new(log: &Path, tty: bool, interactive: bool) -> Self {
        Self {
            inner: Mutex::new(Inner {
                clients: vec![],
                next_id: 0,
                ring: VecDeque::new(),
                ring_bytes: 0,
                log: File::options().create(true).append(true).open(log).ok(),
                partial: [vec![], vec![]],
                attached_once: false,
            }),
            input: Mutex::new(Input::default()),
            attached: Condvar::new(),
            tty,
            interactive,
        }
    }

    fn broadcast(inner: &mut Inner, f: &Frame) {
        inner.clients.retain(|(_, tx)| match tx.try_send(f.clone()) {
            Ok(()) => true,
            // A client that cannot keep up is disconnected rather than
            // stalling the container's output.
            Err(TrySendError::Full(_)) | Err(TrySendError::Disconnected(_)) => false,
        });
    }

    fn write_log(inner: &mut Inner, stream: LogStream, line: &[u8]) {
        let mut line = String::from_utf8_lossy(line).into_owned();
        if line.ends_with('\r') {
            line.pop();
        }
        let l = LogLine { ts: now_ms(), stream, line: line.replace('\r', "") };
        if let (Some(f), Ok(mut j)) = (inner.log.as_mut(), serde_json::to_vec(&l)) {
            j.push(b'\n');
            let _ = f.write_all(&j);
        }
    }

    fn output(&self, stream: LogStream, data: &[u8]) {
        let mut g = self.inner.lock().unwrap();
        let f = match stream {
            LogStream::Stdout => Frame::Stdout(data.to_vec()),
            LogStream::Stderr => Frame::Stderr(data.to_vec()),
        };
        Self::broadcast(&mut g, &f);
        g.ring_bytes += data.len();
        g.ring.push_back(f);
        while g.ring_bytes > RING_BYTES {
            match g.ring.pop_front() {
                Some(old) => g.ring_bytes -= frame_len(&old),
                None => break,
            }
        }
        let idx = stream as usize;
        let mut buf = std::mem::take(&mut g.partial[idx]);
        buf.extend_from_slice(data);
        let mut start = 0;
        while let Some(pos) = buf[start..].iter().position(|b| *b == b'\n') {
            Self::write_log(&mut g, stream, &buf[start..start + pos]);
            start += pos + 1;
        }
        buf.drain(..start);
        // Bound a never-ending line.
        if buf.len() > 256 * 1024 {
            Self::write_log(&mut g, stream, &buf);
            buf.clear();
        }
        g.partial[idx] = buf;
    }

    fn flush_partial(&self) {
        let mut g = self.inner.lock().unwrap();
        for (i, s) in [LogStream::Stdout, LogStream::Stderr].into_iter().enumerate() {
            let buf = std::mem::take(&mut g.partial[i]);
            if !buf.is_empty() {
                Self::write_log(&mut g, s, &buf);
            }
        }
    }

    /// A runtime message for the log (and attached clients' stderr).
    fn note(&self, msg: &str) {
        let mut g = self.inner.lock().unwrap();
        Self::write_log(&mut g, LogStream::Stderr, msg.as_bytes());
    }

    /// Send the final frames of a run and disconnect every client.
    fn finish_run(&self, error: Option<&str>, code: i32) {
        let mut g = self.inner.lock().unwrap();
        if let Some(e) = error {
            Self::broadcast(&mut g, &Frame::Error(e.to_string()));
        }
        Self::broadcast(&mut g, &Frame::Exit(code));
        g.clients.clear();
    }

    fn add_client(self: &Arc<Self>, conn: UnixStream) {
        let (tx, rx) = sync_channel::<Frame>(256);
        let id;
        {
            let mut g = self.inner.lock().unwrap();
            id = g.next_id;
            g.next_id += 1;
            for f in g.ring.iter() {
                let _ = tx.try_send(f.clone());
            }
            g.clients.push((id, tx));
            g.attached_once = true;
        }
        self.attached.notify_all();
        let Ok(mut wconn) = conn.try_clone() else { return };
        std::thread::spawn(move || {
            for f in rx {
                if blocking::write_frame(&mut wconn, &f).is_err() {
                    break;
                }
            }
            let _ = wconn.shutdown(std::net::Shutdown::Both);
        });
        let hub = self.clone();
        std::thread::spawn(move || {
            let mut r = conn;
            while let Ok(Some(f)) = blocking::read_frame(&mut r) {
                hub.client_frame(f);
            }
            hub.inner.lock().unwrap().clients.retain(|(i, _)| *i != id);
        });
    }

    fn client_frame(&self, f: Frame) {
        match f {
            Frame::Stdin(d) => {
                if !self.interactive {
                    return;
                }
                let target = {
                    let mut i = self.input.lock().unwrap();
                    let t = if self.tty { i.master.clone() } else { i.stdin.clone() };
                    if t.is_none() && i.pending.len() + d.len() <= 64 * 1024 {
                        i.pending.extend_from_slice(&d);
                    }
                    t
                };
                if let Some(t) = target {
                    let _ = (&*t).write_all(&d);
                }
            }
            Frame::CloseStdin => {
                if !self.tty {
                    let mut i = self.input.lock().unwrap();
                    if i.stdin.take().is_none() {
                        i.pending_close = true;
                    }
                }
            }
            Frame::Resize { cols, rows } => {
                let mut i = self.input.lock().unwrap();
                i.winsize = Some((cols, rows));
                if let Some(m) = &i.master {
                    set_winsize(m.as_raw_fd(), cols, rows);
                }
            }
            _ => {}
        }
    }

    fn wait_first_client(&self, timeout: Duration) {
        let g = self.inner.lock().unwrap();
        let _ = self.attached.wait_timeout_while(g, timeout, |g| !g.attached_once);
    }
}

fn set_winsize(fd: RawFd, cols: u16, rows: u16) {
    let ws = libc::winsize { ws_row: rows, ws_col: cols, ws_xpixel: 0, ws_ypixel: 0 };
    // SAFETY: TIOCSWINSZ with a valid winsize on a pty master fd.
    unsafe { libc::ioctl(fd, libc::TIOCSWINSZ, &ws) };
}

struct Shim {
    layout: Layout,
    spec: RunSpec,
    exe: PathBuf,
    sandboxes: PathBuf,
    hub: Arc<Hub>,
    start_fd: Option<OwnedFd>,
    cgroup: Option<PathBuf>,
    st: RuntimeState,
}

enum Outcome {
    Exited(i32),
    Failed(i32, String),
}

pub fn main(dir: &Path, wait_attach: bool) -> i32 {
    let start_fd = crate::util::take_handoff_fd();
    crate::util::daemonize();
    let layout = Layout::new(dir);
    crate::util::redirect_stdio(Some(&layout.shim_log()));
    let report = |fd: &Option<OwnedFd>, msg: &str| {
        if let Some(f) = fd {
            write_all_fd(f.as_raw_fd(), format!("{msg}\n").as_bytes());
        }
    };
    let spec = match layout.read_spec() {
        Ok(s) => s,
        Err(e) => {
            report(&start_fd, &format!("ERR {e}"));
            return 1;
        }
    };
    let _ = std::fs::write(layout.shim_pid(), std::process::id().to_string());
    let hub = Arc::new(Hub::new(&layout.log(), spec.tty, spec.interactive));

    // Bind the attach socket through /proc/self/fd to dodge the 108-byte
    // sun_path limit.
    let _ = std::fs::remove_file(layout.sock());
    let listener = match crate::util::short_path(&layout.dir, "shim.sock").and_then(|(_d, p)| UnixListener::bind(p)) {
        Ok(l) => l,
        Err(e) => {
            report(&start_fd, &format!("ERR bind attach socket: {e}"));
            let _ = std::fs::remove_file(layout.shim_pid());
            return 1;
        }
    };
    {
        let hub = hub.clone();
        std::thread::spawn(move || {
            for c in listener.incoming().flatten() {
                hub.add_client(c);
            }
        });
    }

    let exe = std::env::var_os(ENV_EXE).map(PathBuf::from).or_else(|| std::env::current_exe().ok()).unwrap_or_default();
    let sandboxes = std::env::var_os(ENV_SANDBOXES).map(PathBuf::from).unwrap_or_default();
    let mut st = layout.read_state().unwrap_or_else(|_| state::initial_state());
    st.restart_count = 0;
    st.stopped_by_user = false;
    st.pid = None;
    let mut shim = Shim { layout, spec, exe, sandboxes, hub, start_fd, cgroup: None, st };
    shim.setup_cgroup();

    if wait_attach {
        shim.st.status = Status::Created;
        let _ = shim.layout.write_state(&shim.st);
        shim.notify_start("OK");
        shim.hub.wait_first_client(Duration::from_secs(30));
    }
    let code = shim.run_loop();
    let _ = std::fs::remove_file(shim.layout.sock());
    if let Some(cg) = &shim.cgroup {
        let _ = std::fs::remove_dir(cg);
    }
    let _ = std::fs::remove_file(shim.layout.shim_pid());
    let _ = code;
    0
}

impl Shim {
    fn notify_start(&mut self, msg: &str) {
        if let Some(f) = self.start_fd.take() {
            write_all_fd(f.as_raw_fd(), format!("{msg}\n").as_bytes());
        }
    }

    fn save(&self) {
        let _ = self.layout.write_state(&self.st);
    }

    fn setup_cgroup(&mut self) {
        let id = self.layout.id();
        match cgroup::create(&id, self.spec.memory_limit, self.spec.cpus) {
            Ok(p) => self.cgroup = Some(p),
            Err(e) => {
                if self.spec.memory_limit.is_some() || self.spec.cpus.is_some() {
                    self.hub.note(&format!("hullr: warning: resource limits not applied: {e}"));
                }
            }
        }
    }

    fn run_loop(&mut self) -> i32 {
        let mut backoff = Backoff::default();
        let mut first = true;
        loop {
            let t0 = Instant::now();
            let outcome = self.run_once();
            let (code, err) = match outcome {
                Outcome::Exited(c) => (c, None),
                Outcome::Failed(c, e) => (c, Some(e)),
            };
            self.hub.flush_partial();
            self.hub.finish_run(err.as_deref(), code);
            self.st.pid = None;
            self.st.exit_code = Some(code);
            self.st.finished_at = Some(hullr_core::now_unix());
            if let Some(e) = &err {
                self.hub.note(&format!("hullr: error: {e}"));
                if first && self.start_fd.is_some() {
                    // `start` reports this error; no restarts.
                    self.st.status = Status::Exited;
                    self.save();
                    self.notify_start(&format!("ERR {e}"));
                    return code;
                }
            }
            first = false;
            let stopped = self.layout.stopped().exists();
            if !state::should_restart(self.spec.restart, code, self.st.restart_count, stopped) {
                self.st.status = Status::Exited;
                self.st.stopped_by_user = stopped;
                self.save();
                return code;
            }
            self.st.status = Status::Restarting;
            self.save();
            let delay = backoff.next_delay(t0.elapsed());
            let until = Instant::now() + delay;
            while Instant::now() < until {
                if self.layout.stopped().exists() {
                    self.st.status = Status::Exited;
                    self.st.stopped_by_user = true;
                    self.save();
                    return code;
                }
                std::thread::sleep(Duration::from_millis(50).min(until - Instant::now()));
            }
            self.st.restart_count += 1;
        }
    }

    fn plan_net(&self) -> Result<(NetPlan, Option<net::UserNet>), String> {
        let alive_pid = |p: &Path| -> Option<u32> {
            let pid: u32 = std::fs::read_to_string(p).ok()?.trim().parse().ok()?;
            state::is_pid_alive(pid).then_some(pid)
        };
        Ok(match &self.spec.network {
            NetworkMode::Host => (NetPlan::Host, None),
            NetworkMode::None => (NetPlan::New, None),
            NetworkMode::Private => match net::detect_user_net() {
                Some(t) => (NetPlan::New, Some(t)),
                None => {
                    self.hub.note("hullr: warning: no usable pasta/slirp4netns (or no access to /dev/net/tun); using host networking");
                    (NetPlan::Host, None)
                }
            },
            NetworkMode::Sandbox(name) => {
                let pid = alive_pid(&self.sandboxes.join(name).join("holder.pid"))
                    .ok_or_else(|| format!("network sandbox {name} is not running"))?;
                (NetPlan::Join(pid), None)
            }
            NetworkMode::Container(id) => {
                let other = Layout::new(self.layout.dir.parent().unwrap_or(Path::new("/")).join(id));
                let pid = other
                    .read_state()
                    .ok()
                    .filter(|s| s.status == Status::Running)
                    .and_then(|s| s.pid)
                    .filter(|p| state::is_pid_alive(*p))
                    .ok_or_else(|| format!("container {id} is not running"))?;
                (NetPlan::Join(pid), None)
            }
        })
    }

    fn init_config(&self, net: NetPlan) -> InitConfig {
        let l = &self.layout;
        InitConfig {
            spec: self.spec.clone(),
            rootfs: l.rootfs(),
            overlay: Some((l.upper(), l.work())),
            copied_marker: Some(l.copied_marker()),
            etc_files: vec![
                (l.file("hosts"), "/etc/hosts".into()),
                (l.file("resolv.conf"), "/etc/resolv.conf".into()),
                (l.file("hostname"), "/etc/hostname".into()),
            ],
            net,
        }
    }

    fn run_once(&mut self) -> Outcome {
        let (plan, tool) = match self.plan_net() {
            Ok(p) => p,
            Err(e) => return Outcome::Failed(127, e),
        };
        let cfg = self.init_config(plan);
        if let Err(e) = std::fs::write(self.layout.init_cfg(), serde_json::to_vec(&cfg).unwrap_or_default()) {
            return Outcome::Failed(127, format!("write init config: {e}"));
        }
        let (child, sync, readers) = match self.spawn_init() {
            Ok(x) => x,
            Err(e) => return Outcome::Failed(127, e),
        };
        let mut child = child;
        let sfd = sync.as_raw_fd();
        let mut helper = None;
        let mut result: Result<u32, String> = Err("container init exited during setup".into());
        let mut pid = 0;
        let mut readers = readers;
        while let Some((line, fd)) = read_line_fd(sfd) {
            let (cmd, arg) = line.split_once(' ').unwrap_or((line.as_str(), ""));
            match cmd {
                "NS" => {
                    let p: u32 = arg.parse().unwrap_or(0);
                    if let Some(cg) = &self.cgroup {
                        if let Err(e) = cgroup::add_pid(cg, p) {
                            self.hub.note(&format!("hullr: warning: {e}"));
                        }
                    }
                    if let Some(t) = &tool {
                        match net::attach(t, p, &self.spec.ports, &self.layout.dir) {
                            Ok(h) => helper = Some(h),
                            // Degrade rather than refuse to start: the
                            // container keeps its loopback-only netns.
                            Err(e) => self.hub.note(&format!(
                                "hullr: warning: private networking unavailable ({e}); loopback only"
                            )),
                        }
                    }
                    write_all_fd(sfd, b"GO\n");
                }
                "PTY" => {
                    let Some(fd) = fd else { continue };
                    let master = File::from(fd);
                    if let Ok(mut reader) = master.try_clone() {
                        let ws = self.hub.input.lock().unwrap().winsize.unwrap_or((80, 24));
                        set_winsize(master.as_raw_fd(), ws.0, ws.1);
                        self.hub.input.lock().unwrap().set_target(true, Arc::new(master));
                        let hub = self.hub.clone();
                        readers.push(std::thread::spawn(move || pump(&mut reader, &hub, LogStream::Stdout)));
                    }
                }
                "WARN" => self.hub.note(&format!("hullr: warning: {arg}")),
                "PID" => pid = arg.parse().unwrap_or(0),
                "OK" => {
                    result = Ok(pid);
                    break;
                }
                "ERR" => {
                    result = Err(arg.to_string());
                    break;
                }
                _ => {}
            }
        }
        drop(sync);
        match result {
            Ok(pid) => {
                self.st.status = Status::Running;
                self.st.pid = Some(pid);
                self.st.started_at = Some(hullr_core::now_unix());
                self.st.exit_code = None;
                self.st.finished_at = None;
                self.save();
                self.notify_start("OK");
                let code = child.wait().map(crate::util::exit_code).unwrap_or(255);
                self.finish_io(readers);
                drop(helper);
                Outcome::Exited(code)
            }
            Err(e) => {
                let _ = child.kill();
                let code = child.wait().map(crate::util::exit_code).unwrap_or(127);
                self.finish_io(readers);
                drop(helper);
                // A process we killed during setup reports a setup failure.
                Outcome::Failed(if code == 0 || code == 137 { 127 } else { code }, e)
            }
        }
    }

    /// Wait (bounded) for the output readers to drain, then drop the input.
    fn finish_io(&self, readers: Vec<std::thread::JoinHandle<()>>) {
        let deadline = Instant::now() + Duration::from_secs(3);
        for r in readers {
            while !r.is_finished() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(5));
            }
            if r.is_finished() {
                let _ = r.join();
            }
        }
        let mut i = self.hub.input.lock().unwrap();
        i.stdin = None;
        i.master = None;
    }

    fn spawn_init(&self) -> Result<(std::process::Child, UnixStream, Vec<std::thread::JoinHandle<()>>), String> {
        let (ours, theirs) = UnixStream::pair().map_err(|e| format!("socketpair: {e}"))?;
        let mut cmd = Command::new(&self.exe);
        cmd.arg("__init").arg(self.layout.init_cfg());
        let mut readers = vec![];
        let theirs_fd = theirs.as_raw_fd();
        // SAFETY: the closure only calls async-signal-safe functions
        // (setsid, dup2, fcntl) between fork and exec.
        unsafe {
            cmd.pre_exec(move || {
                libc::setsid();
                install_handoff_fd(theirs_fd)
            });
        }
        if self.spec.tty {
            // The init allocates the pty inside the container and sends us
            // the master (`PTY` on the sync socket).
            cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
            let child = cmd.spawn().map_err(|e| format!("spawn init: {e}"))?;
            drop(cmd);
            drop(theirs);
            return Ok((child, ours, readers));
        }
        cmd.stdin(if self.spec.interactive { Stdio::piped() } else { Stdio::null() })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = cmd.spawn().map_err(|e| format!("spawn init: {e}"))?;
        drop(cmd);
        drop(theirs);
        if let Some(si) = child.stdin.take() {
            // SAFETY: converting an owned ChildStdin into an owned File.
            let f = unsafe { File::from_raw_fd(std::os::fd::IntoRawFd::into_raw_fd(si)) };
            self.hub.input.lock().unwrap().set_target(false, Arc::new(f));
        }
        for (s, stream) in [
            (child.stdout.take().map(|o| Box::new(o) as Box<dyn Read + Send>), LogStream::Stdout),
            (child.stderr.take().map(|o| Box::new(o) as Box<dyn Read + Send>), LogStream::Stderr),
        ] {
            if let Some(mut r) = s {
                let hub = self.hub.clone();
                readers.push(std::thread::spawn(move || pump(&mut r, &hub, stream)));
            }
        }
        Ok((child, ours, readers))
    }
}

fn pump(r: &mut dyn Read, hub: &Hub, stream: LogStream) {
    let mut buf = vec![0u8; 32 * 1024];
    loop {
        match r.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => hub.output(stream, &buf[..n]),
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            // EIO: every pty slave closed (the container exited).
            Err(_) => break,
        }
    }
}
