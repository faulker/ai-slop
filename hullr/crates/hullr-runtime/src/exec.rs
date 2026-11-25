//! `__exec <container_dir> <pid>`: run an extra process in a running
//! container. The `ExecSpec` comes as JSON in `$_HULLR_EXEC_SPEC`, the
//! client's frame socket at fd 3.
//!
//! The helper joins the user (if different), ipc, uts, net, pid, cgroup and
//! mount namespaces of the container's pid 1, chroots to its root, then
//! forks (so the child lands in the pid namespace) and execs. It relays the
//! child's stdio as `hullr_core::stream` frames and ends with `Exit(code)`;
//! setup failures are sent as `Error` + `Exit(126|127)`.

use crate::state::Layout;
use crate::util::write_all_fd;
use hullr_core::stream::{blocking, Frame};
use hullr_core::ExecSpec;
use nix::sched::CloneFlags;
use std::fs::File;
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, OwnedFd, RawFd};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::sync::Mutex;

pub const ENV_SPEC: &str = "_HULLR_EXEC_SPEC";

pub fn main(dir: &Path, pid: u32) -> i32 {
    let Some(fd) = crate::util::take_handoff_fd() else { return 1 };
    crate::util::daemonize();
    crate::util::redirect_stdio(None);
    let sock = UnixStream::from(fd);
    let out = Mutex::new(sock.try_clone().expect("dup socket"));
    let fail = |code: i32, msg: String| -> i32 {
        let mut w = out.lock().unwrap();
        let _ = blocking::write_frame(&mut *w, &Frame::Error(msg));
        let _ = blocking::write_frame(&mut *w, &Frame::Exit(code));
        code
    };
    let spec: ExecSpec = match std::env::var(ENV_SPEC).map_err(|e| e.to_string()).and_then(|s| serde_json::from_str(&s).map_err(|e| e.to_string())) {
        Ok(s) => s,
        Err(e) => return fail(126, format!("exec spec: {e}")),
    };
    let layout = Layout::new(dir);
    let run = match layout.read_spec() {
        Ok(r) => r,
        Err(e) => return fail(126, e.to_string()),
    };
    if let Some(cg) = crate::cgroup::path_for(&layout.id()) {
        let _ = crate::cgroup::add_pid(&cg, std::process::id());
    }
    if let Err(e) = enter(pid) {
        return fail(126, e);
    }

    // Stdio for the child, created inside the container's mount namespace
    // so a pty comes from the container's devpts.
    let (master, child_io): (Option<File>, [OwnedFd; 3]) = match make_stdio(&spec) {
        Ok(x) => x,
        Err(e) => return fail(126, e),
    };
    let (err_r, err_w) = match nix::unistd::pipe2(nix::fcntl::OFlag::O_CLOEXEC) {
        Ok(p) => p,
        Err(e) => return fail(126, format!("pipe: {e}")),
    };
    let child_fds: Vec<RawFd> = child_io.iter().map(|f| f.as_raw_fd()).collect();
    // SAFETY: still single-threaded here (no threads spawned yet).
    let child = unsafe { libc::fork() };
    if child < 0 {
        return fail(126, "fork".into());
    }
    if child == 0 {
        // SAFETY: wiring our own fds onto stdio in the child.
        unsafe {
            libc::setsid();
            for (i, fd) in child_fds.iter().enumerate() {
                libc::dup2(*fd, i as RawFd);
            }
            if spec.tty {
                libc::ioctl(0, libc::TIOCSCTTY, 0);
            }
        }
        let user = spec.user.as_deref().or(run.user.as_deref());
        let cwd = spec.working_dir.clone().unwrap_or_else(|| run.cwd.clone());
        let (code, msg) =
            crate::init::exec_in_container(&spec.cmd, &run.env, &spec.env, &run.hostname, spec.tty, user, &cwd, false);
        write_all_fd(err_w.as_raw_fd(), format!("{code} {msg}").as_bytes());
        // SAFETY: leave the failed child immediately.
        unsafe { libc::_exit(code) };
    }
    drop(err_w);
    let [stdin_c, stdout_c, stderr_c] = child_io;
    drop((stdout_c, stderr_c));
    let mut errbuf = Vec::new();
    let _ = File::from(err_r).read_to_end(&mut errbuf);
    if !errbuf.is_empty() {
        crate::util::wait_pid(child);
        let s = String::from_utf8_lossy(&errbuf).into_owned();
        let (c, m) = s.split_once(' ').unwrap_or(("126", &s));
        return fail(c.parse().unwrap_or(126), m.to_string());
    }

    drop(stdin_c);
    let io = ParentIo {
        stdin: if spec.tty { master.as_ref().and_then(|m| m.try_clone().ok()) } else { STDIN_W.with(|s| s.borrow_mut().take()) },
        outputs: match master {
            Some(m) => vec![(m, true)],
            None => STDOUT_R.with(|s| s.borrow_mut().drain(..).collect()),
        },
    };
    let code = relay(&sock, io, &spec, child);
    let mut w = out.lock().unwrap();
    let _ = blocking::write_frame(&mut *w, &Frame::Exit(code));
    let _ = w.shutdown(std::net::Shutdown::Both);
    code
}

struct ParentIo {
    stdin: Option<File>,
    /// (reader, is_stdout); for a tty the pty master.
    outputs: Vec<(File, bool)>,
}

/// Single-threaded poll loop between the client socket and the child.
/// (No threads: after setns into a pid namespace, CLONE_THREAD fails.)
fn relay(sock: &UnixStream, mut io: ParentIo, spec: &ExecSpec, child: libc::pid_t) -> i32 {
    let mut sock_w = sock.try_clone().expect("dup socket");
    let master_fd = if spec.tty { io.outputs.first().map(|(f, _)| f.as_raw_fd()) } else { None };
    let mut inbuf: Vec<u8> = vec![];
    let mut sock_open = true;
    let mut buf = vec![0u8; 32 * 1024];
    let mut exit: Option<i32> = None;
    let mut drain_deadline = None;
    loop {
        if exit.is_none() {
            let mut st = 0;
            // SAFETY: non-blocking wait for our own child.
            if unsafe { libc::waitpid(child, &mut st, libc::WNOHANG) } == child {
                exit = Some(if libc::WIFEXITED(st) { libc::WEXITSTATUS(st) } else { 128 + libc::WTERMSIG(st) });
                drain_deadline = Some(std::time::Instant::now() + std::time::Duration::from_secs(2));
            }
        }
        if io.outputs.is_empty() && exit.is_some() {
            break;
        }
        if drain_deadline.map(|d| std::time::Instant::now() > d).unwrap_or(false) {
            break;
        }
        let mut fds: Vec<libc::pollfd> =
            io.outputs.iter().map(|(f, _)| libc::pollfd { fd: f.as_raw_fd(), events: libc::POLLIN, revents: 0 }).collect();
        if sock_open {
            fds.push(libc::pollfd { fd: sock.as_raw_fd(), events: libc::POLLIN, revents: 0 });
        }
        // SAFETY: poll over a valid, correctly sized pollfd array.
        let n = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, 100) };
        if n <= 0 {
            continue;
        }
        let mut closed = vec![];
        for (i, p) in fds.iter().enumerate() {
            if p.revents == 0 {
                continue;
            }
            if i < io.outputs.len() {
                let (f, is_out) = &mut io.outputs[i];
                match f.read(&mut buf) {
                    Ok(0) | Err(_) => closed.push(i),
                    Ok(n) => {
                        let fr = if *is_out { Frame::Stdout(buf[..n].to_vec()) } else { Frame::Stderr(buf[..n].to_vec()) };
                        // A gone client must not stop the process: keep
                        // draining output and discard it.
                        let _ = blocking::write_frame(&mut sock_w, &fr);
                    }
                }
            } else {
                let mut r = sock;
                match r.read(&mut buf) {
                    Ok(0) | Err(_) => sock_open = false,
                    Ok(n) => inbuf.extend_from_slice(&buf[..n]),
                }
                while let Some(f) = take_frame(&mut inbuf) {
                    match f {
                        Frame::Stdin(d) if spec.interactive => {
                            if let Some(w) = io.stdin.as_mut() {
                                let _ = w.write_all(&d);
                            }
                        }
                        Frame::CloseStdin if !spec.tty => io.stdin = None,
                        Frame::Resize { cols, rows } => {
                            if let Some(m) = master_fd {
                                let ws = libc::winsize { ws_row: rows, ws_col: cols, ws_xpixel: 0, ws_ypixel: 0 };
                                // SAFETY: TIOCSWINSZ on our pty master.
                                unsafe { libc::ioctl(m, libc::TIOCSWINSZ, &ws) };
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
        for i in closed.into_iter().rev() {
            io.outputs.remove(i);
        }
    }
    match exit {
        Some(c) => c,
        None => crate::util::wait_pid(child),
    }
}

/// Pop one complete frame off the front of `buf`.
fn take_frame(buf: &mut Vec<u8>) -> Option<Frame> {
    if buf.len() < 5 {
        return None;
    }
    let len = u32::from_be_bytes([buf[1], buf[2], buf[3], buf[4]]) as usize;
    if buf.len() < 5 + len {
        return None;
    }
    let mut r = &buf[..5 + len];
    let f = blocking::read_frame(&mut r).ok().flatten();
    buf.drain(..5 + len);
    f
}

thread_local! {
    static STDIN_W: std::cell::RefCell<Option<File>> = const { std::cell::RefCell::new(None) };
    static STDOUT_R: std::cell::RefCell<Vec<(File, bool)>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Child stdio fds; parent-side ends are stashed (pty master returned,
/// pipe ends in thread-locals of this single-purpose process).
fn make_stdio(spec: &ExecSpec) -> Result<(Option<File>, [OwnedFd; 3]), String> {
    let cloexec = |fd: RawFd| {
        // SAFETY: fcntl on an fd we own.
        unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) };
    };
    if spec.tty {
        let pty = nix::pty::openpty(None, None).map_err(|e| format!("openpty: {e}"))?;
        cloexec(pty.master.as_raw_fd());
        cloexec(pty.slave.as_raw_fd());
        let s = pty.slave;
        let s2 = s.try_clone().map_err(|e| e.to_string())?;
        let s3 = s.try_clone().map_err(|e| e.to_string())?;
        return Ok((Some(File::from(pty.master)), [s, s2, s3]));
    }
    let pipe = || nix::unistd::pipe2(nix::fcntl::OFlag::O_CLOEXEC).map_err(|e| format!("pipe: {e}"));
    let stdin_c = if spec.interactive {
        let (r, w) = pipe()?;
        STDIN_W.with(|s| *s.borrow_mut() = Some(File::from(w)));
        r
    } else {
        let f = File::open("/dev/null").map_err(|e| format!("/dev/null: {e}"))?;
        OwnedFd::from(f)
    };
    let (or, ow) = pipe()?;
    let (er, ew) = pipe()?;
    STDOUT_R.with(|s| s.borrow_mut().extend([(File::from(or), true), (File::from(er), false)]));
    Ok((None, [stdin_c, ow, ew]))
}

/// Join the namespaces of `pid` and chroot into its root.
fn enter(pid: u32) -> Result<(), String> {
    let open = |n: &str| File::open(format!("/proc/{pid}/{n}")).map_err(|e| format!("container process {pid}: {e}"));
    let root = open("root")?;
    let mut fds = vec![];
    for (name, flag) in [
        ("user", CloneFlags::CLONE_NEWUSER),
        ("ipc", CloneFlags::CLONE_NEWIPC),
        ("uts", CloneFlags::CLONE_NEWUTS),
        ("net", CloneFlags::CLONE_NEWNET),
        ("pid", CloneFlags::CLONE_NEWPID),
        ("cgroup", CloneFlags::CLONE_NEWCGROUP),
        ("mnt", CloneFlags::CLONE_NEWNS),
    ] {
        let same = crate::ns::ns_id(&pid.to_string(), name) == crate::ns::ns_id("self", name);
        if same {
            continue;
        }
        match File::open(format!("/proc/{pid}/ns/{name}")) {
            Ok(f) => fds.push((name, flag, f)),
            Err(_) if name == "cgroup" => {}
            Err(e) => return Err(format!("open {name} namespace: {e}")),
        }
    }
    for (name, flag, f) in &fds {
        nix::sched::setns(f, *flag).map_err(|e| format!("join {name} namespace: {e}"))?;
        if *name == "user" {
            crate::ns::become_root();
        }
    }
    // SAFETY: fchdir/chroot on a directory fd we hold open.
    unsafe {
        if libc::fchdir(root.as_raw_fd()) != 0 || libc::chroot(c".".as_ptr()) != 0 {
            return Err(format!("chroot into container: {}", std::io::Error::last_os_error()));
        }
    }
    let _ = nix::unistd::chdir("/");
    Ok(())
}

/// Client side: spawn the helper and return our end of its socket.
pub fn spawn(exe: &Path, dir: &Path, pid: u32, spec: &ExecSpec) -> hullr_core::Result<UnixStream> {
    use std::os::unix::process::CommandExt;
    let (ours, theirs) = UnixStream::pair()?;
    let tfd = theirs.as_raw_fd();
    let mut cmd = std::process::Command::new(exe);
    cmd.arg("__exec")
        .arg(dir)
        .arg(pid.to_string())
        .env(ENV_SPEC, serde_json::to_string(spec)?)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    // SAFETY: only dup2/fcntl between fork and exec.
    unsafe {
        cmd.pre_exec(move || crate::util::install_handoff_fd(tfd));
    }
    let mut child = cmd.spawn().map_err(|e| crate::util::rt_err(format!("spawn exec helper: {e}")))?;
    drop(theirs);
    // The helper daemonizes; reap the short-lived parent.
    let _ = child.wait();
    Ok(ours)
}
