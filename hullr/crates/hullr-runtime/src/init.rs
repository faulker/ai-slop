//! `__init <init.json>`: creates the container's namespaces and runs its
//! process as pid 1.
//!
//! Two processes are involved:
//! - P (this helper, single-threaded after re-exec) enters/creates the user
//!   namespace, the network namespace, then unshares mount, uts, ipc and
//!   pid. It reports `NS <pid>` on the sync socket (fd 3) so the shim can
//!   attach networking and cgroups, waits for `GO`, forks C, and then just
//!   waits for C and exits with its status.
//! - C is pid 1 of the new pid namespace: it builds the root filesystem,
//!   pivots into it, sets hostname, credentials and environment, and execs
//!   the container command. Setup errors go back to P over a close-on-exec
//!   pipe, so a successful exec is seen as EOF.
//!
//! Sync protocol (lines on fd 3): init -> shim `NS <pid>`, `WARN <text>`,
//! `PID <pid>`, `OK`, `ERR <text>`; shim -> init `GO`.

use crate::state::{InitConfig, NetPlan};
use crate::user::{build_env, env_get};
use crate::util::{read_line, rt_err, wait_pid, write_all_fd};
use crate::{mounts, net, ns};
use nix::sched::{unshare, CloneFlags};
use std::ffi::CString;
use std::os::fd::{AsRawFd, RawFd};
use std::path::{Path, PathBuf};

/// Docker's default capability set (by number).
const DEFAULT_CAPS: &[u32] = &[0, 1, 3, 4, 5, 6, 7, 8, 10, 13, 18, 27, 29, 31];

fn send(fd: Option<RawFd>, line: &str) {
    if let Some(fd) = fd {
        write_all_fd(fd, format!("{line}\n").as_bytes());
    }
}

pub fn main(cfg_path: &Path) -> i32 {
    let sync = crate::util::take_handoff_fd();
    let sfd = sync.as_ref().map(|f| f.as_raw_fd());
    let fail = |code: i32, msg: String| -> i32 {
        send(sfd, &format!("ERR {}", msg.replace('\n', " ")));
        code
    };
    let cfg: InitConfig = match std::fs::read(cfg_path)
        .map_err(|e| e.to_string())
        .and_then(|b| serde_json::from_slice(&b).map_err(|e| e.to_string()))
    {
        Ok(c) => c,
        Err(e) => return fail(1, format!("read init config: {e}")),
    };
    match setup_namespaces(&cfg) {
        Ok(()) => {}
        Err(e) => return fail(1, e.to_string().trim_start_matches("runtime error: ").to_string()),
    }
    send(sfd, &format!("NS {}", std::process::id()));
    if sfd.is_some() && read_line(sfd.unwrap()).as_deref() != Some("GO") {
        return 1;
    }

    let (err_r, err_w) = match nix::unistd::pipe2(nix::fcntl::OFlag::O_CLOEXEC) {
        Ok(p) => p,
        Err(e) => return fail(1, format!("pipe: {e}")),
    };
    // SAFETY: this process is single-threaded (fresh re-exec, no threads
    // spawned), so the child may allocate freely.
    let child = unsafe { libc::fork() };
    if child < 0 {
        return fail(1, "fork container process".into());
    }
    if child == 0 {
        drop(err_r);
        let (code, msg) = container_main(&cfg, sfd);
        write_all_fd(err_w.as_raw_fd(), format!("{code} {msg}").as_bytes());
        // SAFETY: terminate the failed child without unwinding.
        unsafe { libc::_exit(code) };
    }
    drop(err_w);
    // P must not keep the container's stdio open (EOF/EIO detection).
    crate::util::redirect_stdio(None);
    let mut buf = Vec::new();
    {
        use std::io::Read;
        let mut f = std::fs::File::from(err_r);
        let _ = f.read_to_end(&mut buf);
    }
    if !buf.is_empty() {
        let s = String::from_utf8_lossy(&buf).into_owned();
        let (code, msg) = s.split_once(' ').unwrap_or(("127", &s));
        let code: i32 = code.parse().unwrap_or(127);
        wait_pid(child);
        return fail(code, msg.to_string());
    }
    send(sfd, &format!("PID {child}"));
    send(sfd, "OK");
    // SAFETY: setting signal dispositions to SIG_IGN for this process only;
    // C has already exec'd with its own dispositions.
    unsafe {
        for s in [libc::SIGINT, libc::SIGTERM, libc::SIGHUP, libc::SIGQUIT, libc::SIGPIPE] {
            libc::signal(s, libc::SIG_IGN);
        }
    }
    wait_pid(child)
}

fn setup_namespaces(cfg: &InitConfig) -> hullr_core::Result<()> {
    let rootless = crate::util::euid() != 0;
    match cfg.net {
        NetPlan::Join(pid) if rootless => {
            if !ns::join_userns_of(pid)? {
                ns::enter_userns(&ns::IdMapping::detect())?;
            }
        }
        _ if rootless => ns::enter_userns(&ns::IdMapping::detect())?,
        _ => {}
    }
    ns::become_root();
    match cfg.net {
        NetPlan::Host => {}
        NetPlan::New => {
            unshare(CloneFlags::CLONE_NEWNET).map_err(|e| rt_err(format!("unshare network namespace: {e}")))?;
            net::loopback_up().map_err(|e| rt_err(format!("bring up lo: {e}")))?;
        }
        NetPlan::Join(pid) => ns::join(pid, "net", CloneFlags::CLONE_NEWNET)?,
    }
    let flags = CloneFlags::CLONE_NEWNS | CloneFlags::CLONE_NEWUTS | CloneFlags::CLONE_NEWIPC | CloneFlags::CLONE_NEWPID;
    unshare(flags).map_err(|e| rt_err(format!("unshare namespaces: {e}")))?;
    Ok(())
}

/// Runs in C. Only returns on error: (exit code, message).
fn container_main(cfg: &InitConfig, sync: Option<RawFd>) -> (i32, String) {
    // SAFETY: setsid has no memory-safety preconditions.
    unsafe { libc::setsid() };
    // Enter a cgroup namespace rooted at our (already joined) cgroup.
    let _ = unshare(CloneFlags::CLONE_NEWCGROUP);
    let own_netns = cfg.net != NetPlan::Host;
    match mounts::setup(cfg, own_netns) {
        Ok(notes) => {
            for n in notes {
                send(sync, &format!("WARN {n}"));
            }
        }
        Err(e) => return (1, strip(e)),
    }
    if cfg.spec.tty {
        if let Err(e) = container_pty(sync) {
            return (1, e);
        }
    }
    if !cfg.spec.hostname.is_empty() {
        if let Err(e) = nix::unistd::sethostname(&cfg.spec.hostname) {
            return (1, format!("sethostname: {e}"));
        }
    }
    let s = &cfg.spec;
    exec_in_container(&s.args, &s.env, &[], &s.hostname, s.tty, s.user.as_deref(), &s.cwd, true)
}

/// Allocate the container's terminal from its own devpts instance (so
/// `tty` resolves to /dev/pts/N inside), hand the master to the shim over
/// the sync socket, and make the slave our stdio and controlling terminal.
fn container_pty(sync: Option<RawFd>) -> Result<(), String> {
    let sync = sync.ok_or("tty needs the shim sync socket")?;
    let pty = nix::pty::openpty(None, None).map_err(|e| format!("openpty in container: {e}"))?;
    let slave_path = std::fs::read_link(format!("/proc/self/fd/{}", pty.slave.as_raw_fd())).ok();
    if !crate::util::send_fd(sync, b"PTY\n", pty.master.as_raw_fd()) {
        return Err(format!("send pty to shim: {}", std::io::Error::last_os_error()));
    }
    drop(pty.master);
    // SAFETY: wiring the slave onto stdio; TIOCSCTTY makes it the
    // controlling terminal of our fresh session (nobody else's).
    unsafe {
        for i in 0..3 {
            libc::dup2(pty.slave.as_raw_fd(), i);
        }
        if libc::ioctl(0, libc::TIOCSCTTY, 0) != 0 {
            return Err(format!("set controlling terminal: {}", std::io::Error::last_os_error()));
        }
    }
    drop(pty.slave);
    // Docker also exposes the terminal as /dev/console.
    if let Some(p) = slave_path {
        let c = Path::new("/dev/console");
        if std::fs::File::create(c).is_ok() {
            let _ = nix::mount::mount(Some(&p), c, None::<&str>, nix::mount::MsFlags::MS_BIND, None::<&str>);
        }
    }
    Ok(())
}

pub fn strip(e: hullr_core::Error) -> String {
    match e {
        hullr_core::Error::Runtime(m) | hullr_core::Error::Invalid(m) => m,
        other => other.to_string(),
    }
}

/// Final steps shared by the container init and `__exec`, run with `/`
/// already the container root: credentials, env, cwd, exec. Only returns on
/// failure.
#[allow(clippy::too_many_arguments)]
pub fn exec_in_container(
    args: &[String],
    base_env: &[String],
    extra_env: &[String],
    hostname: &str,
    tty: bool,
    user: Option<&str>,
    cwd: &str,
    create_cwd: bool,
) -> (i32, String) {
    if args.is_empty() {
        return (127, "no command specified".into());
    }
    let passwd = std::fs::read_to_string("/etc/passwd").unwrap_or_default();
    let group = std::fs::read_to_string("/etc/group").unwrap_or_default();
    let id = match crate::user::resolve(user, &passwd, &group) {
        Ok(i) => i,
        Err(e) => return (126, strip(e)),
    };
    let env = build_env(base_env, extra_env, hostname, tty, id.home.as_deref());
    let cwd = if cwd.is_empty() { "/" } else { cwd };
    if create_cwd {
        let _ = std::fs::create_dir_all(cwd);
    }
    if let Err(e) = nix::unistd::chdir(cwd) {
        return (126, format!("chdir to cwd (\"{cwd}\") set in config: {e}"));
    }
    let path = match lookup(&args[0], env_get(&env, "PATH").unwrap_or(crate::user::DEFAULT_PATH)) {
        Ok(p) => p,
        Err(e) => return (127, e),
    };
    drop_capabilities();
    if let Err(e) = set_identity(&id) {
        return (126, e);
    }
    reset_signals();
    let c = |s: &str| CString::new(s.as_bytes()).unwrap_or_default();
    let argv: Vec<CString> = args.iter().map(|a| c(a)).collect();
    let envp: Vec<CString> = env.iter().map(|a| c(a)).collect();
    let cpath = CString::new(path.as_os_str().as_encoded_bytes()).unwrap_or_default();
    let err = nix::unistd::execve(&cpath, &argv, &envp).unwrap_err();
    let p = path.display();
    match err {
        nix::errno::Errno::ENOENT => (127, format!("exec: no such file: {p} (or its interpreter is missing)")),
        nix::errno::Errno::EACCES => (126, format!("exec: permission denied: {p}")),
        nix::errno::Errno::ENOEXEC => (126, format!("exec format error: {p} (wrong architecture? see `hullr system emulation`)")),
        e => (126, format!("exec {p}: {e}")),
    }
}

/// execvp-style lookup against the container's PATH.
pub fn lookup(cmd: &str, path: &str) -> Result<PathBuf, String> {
    if cmd.contains('/') {
        if Path::new(cmd).exists() {
            return Ok(cmd.into());
        }
        return Err(format!("exec: no such file: {cmd}"));
    }
    for dir in path.split(':').filter(|d| !d.is_empty()) {
        let p = Path::new(dir).join(cmd);
        if nix::unistd::access(&p, nix::unistd::AccessFlags::X_OK).is_ok() && p.is_file() {
            return Ok(p);
        }
    }
    Err(format!("exec: \"{cmd}\": executable file not found in $PATH"))
}

fn set_identity(id: &crate::user::Identity) -> Result<(), String> {
    let groups: Vec<libc::gid_t> = id.groups.clone();
    // SAFETY: credential syscalls with a valid groups buffer.
    unsafe {
        let deny = std::fs::read_to_string("/proc/self/setgroups").map(|s| s.trim() == "deny").unwrap_or(false);
        if !deny && libc::setgroups(groups.len(), groups.as_ptr()) != 0 && id.uid != 0 {
            return Err(format!("setgroups: {}", std::io::Error::last_os_error()));
        }
        if libc::setresgid(id.gid, id.gid, id.gid) != 0 {
            return Err(format!(
                "setgid {}: {} (is the gid mapped? rootless without /etc/subgid maps only root)",
                id.gid,
                std::io::Error::last_os_error()
            ));
        }
        if libc::setresuid(id.uid, id.uid, id.uid) != 0 {
            return Err(format!(
                "setuid {}: {} (is the uid mapped? rootless without /etc/subuid maps only root)",
                id.uid,
                std::io::Error::last_os_error()
            ));
        }
    }
    Ok(())
}

/// Limit the bounding set to Docker's defaults (root in the container then
/// gets only those after exec) and clear ambient capabilities.
fn drop_capabilities() {
    let last: u32 = std::fs::read_to_string("/proc/sys/kernel/cap_last_cap")
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(40);
    // SAFETY: prctl with integer arguments only.
    unsafe {
        libc::prctl(libc::PR_CAP_AMBIENT, libc::PR_CAP_AMBIENT_CLEAR_ALL, 0, 0, 0);
        for cap in 0..=last {
            if !DEFAULT_CAPS.contains(&cap) {
                libc::prctl(libc::PR_CAPBSET_DROP, cap as libc::c_ulong, 0, 0, 0);
            }
        }
    }
}

/// Default dispositions and an empty mask (Rust ignores SIGPIPE, and the
/// container must not inherit that).
pub fn reset_signals() {
    // SAFETY: resetting dispositions and the mask of the current process.
    unsafe {
        for s in 1..32 {
            if s != libc::SIGKILL && s != libc::SIGSTOP {
                libc::signal(s, libc::SIG_DFL);
            }
        }
        let mut set: libc::sigset_t = std::mem::zeroed();
        libc::sigemptyset(&mut set);
        libc::sigprocmask(libc::SIG_SETMASK, &set, std::ptr::null_mut());
    }
}
