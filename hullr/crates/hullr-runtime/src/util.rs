//! Small Linux process helpers shared by the re-exec'd roles.

use hullr_core::Error;
use std::fs::File;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::path::{Component, Path, PathBuf};

/// The fd number used to hand a pipe/socket to a re-exec'd helper.
pub const HANDOFF_FD: RawFd = 3;

pub fn rt_err(msg: impl std::fmt::Display) -> Error {
    Error::Runtime(msg.to_string())
}

/// Take ownership of fd 3 if it is open, marking it close-on-exec.
pub fn take_handoff_fd() -> Option<OwnedFd> {
    // SAFETY: fcntl on a possibly closed fd number only returns EBADF.
    let ok = unsafe { libc::fcntl(HANDOFF_FD, libc::F_GETFD) } >= 0;
    if !ok {
        return None;
    }
    // SAFETY: as above; FD_CLOEXEC keeps it out of the container process.
    unsafe { libc::fcntl(HANDOFF_FD, libc::F_SETFD, libc::FD_CLOEXEC) };
    // SAFETY: fd 3 is open and nothing else in this fresh process owns it.
    Some(unsafe { OwnedFd::from_raw_fd(HANDOFF_FD) })
}

/// For use in `pre_exec`: place `fd` at fd 3 without close-on-exec.
/// Async-signal-safe (only dup2/fcntl).
pub fn install_handoff_fd(fd: RawFd) -> std::io::Result<()> {
    // SAFETY: dup2/fcntl are async-signal-safe and act on fds we own.
    unsafe {
        if fd == HANDOFF_FD {
            if libc::fcntl(fd, libc::F_SETFD, 0) < 0 {
                return Err(std::io::Error::last_os_error());
            }
        } else if libc::dup2(fd, HANDOFF_FD) < 0 {
            return Err(std::io::Error::last_os_error());
        }
    }
    Ok(())
}

/// Detach from the caller: fork, the parent exits 0 immediately (so the
/// spawner can reap it at once), the child starts a new session. Must be
/// called while single-threaded.
pub fn daemonize() {
    // SAFETY: called first thing in a freshly exec'd single-threaded
    // helper, so fork cannot leave locks held by other threads.
    match unsafe { libc::fork() } {
        -1 => {}
        0 => {
            // SAFETY: setsid has no memory-safety preconditions.
            unsafe { libc::setsid() };
        }
        _ => unsafe { libc::_exit(0) },
    }
}

/// Redirect stdin/stdout to /dev/null and stderr to `log` (or /dev/null).
pub fn redirect_stdio(log: Option<&Path>) {
    let null = File::options().read(true).write(true).open("/dev/null").ok();
    let err = log.and_then(|p| File::options().create(true).append(true).open(p).ok());
    // SAFETY: dup2 onto the standard fds; sources are valid open files.
    unsafe {
        if let Some(n) = &null {
            libc::dup2(n.as_raw_fd(), 0);
            libc::dup2(n.as_raw_fd(), 1);
        }
        if let Some(e) = err.as_ref().or(null.as_ref()) {
            libc::dup2(e.as_raw_fd(), 2);
        }
    }
}

/// A short path to `dir/name` through `/proc/self/fd`, so unix socket paths
/// stay under the 108-byte limit however deep the container dir is. The
/// returned `File` (the directory) must stay open while the path is used.
pub fn short_path(dir: &Path, name: &str) -> std::io::Result<(File, PathBuf)> {
    let d = File::open(dir)?;
    let p = PathBuf::from(format!("/proc/self/fd/{}/{name}", d.as_raw_fd()));
    Ok((d, p))
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Join `path` (absolute, container view) onto `root`, resolving symlinks
/// inside `root` so the result can never point outside it.
pub fn secure_join(root: &Path, path: &str) -> PathBuf {
    let mut parts: Vec<String> = Path::new(path)
        .components()
        .filter_map(|c| match c {
            Component::Normal(s) => Some(s.to_string_lossy().into_owned()),
            Component::ParentDir => Some("..".into()),
            _ => None,
        })
        .rev()
        .collect();
    let mut cur: Vec<String> = vec![];
    let mut hops = 0;
    while let Some(p) = parts.pop() {
        if p == ".." {
            cur.pop();
            continue;
        }
        let candidate: PathBuf = root.join(cur.iter().collect::<PathBuf>()).join(&p);
        match std::fs::read_link(&candidate) {
            Ok(target) if hops < 40 => {
                hops += 1;
                let t = target.to_string_lossy().into_owned();
                if t.starts_with('/') {
                    cur.clear();
                }
                for c in Path::new(&t).components().rev() {
                    match c {
                        Component::Normal(s) => parts.push(s.to_string_lossy().into_owned()),
                        Component::ParentDir => parts.push("..".into()),
                        _ => {}
                    }
                }
            }
            _ => cur.push(p),
        }
    }
    root.join(cur.iter().collect::<PathBuf>())
}

/// Read one `\n`-terminated line from a blocking fd, byte by byte (no
/// buffering, so nothing past the line is consumed). `None` on EOF.
pub fn read_line(fd: RawFd) -> Option<String> {
    let mut out = Vec::new();
    loop {
        let mut b = [0u8; 1];
        // SAFETY: reading into a valid 1-byte buffer.
        let n = unsafe { libc::read(fd, b.as_mut_ptr().cast(), 1) };
        if n < 0 && std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
            continue;
        }
        if n <= 0 {
            return if out.is_empty() { None } else { Some(String::from_utf8_lossy(&out).into_owned()) };
        }
        if b[0] == b'\n' {
            return Some(String::from_utf8_lossy(&out).into_owned());
        }
        out.push(b[0]);
    }
}

/// Send `line` plus one fd (SCM_RIGHTS) over a unix socket.
pub fn send_fd(sock: RawFd, line: &[u8], fd: RawFd) -> bool {
    // SAFETY: building a msghdr with one iovec over `line` and a cmsg
    // buffer sized by CMSG_SPACE for exactly one int.
    unsafe {
        let mut iov = libc::iovec { iov_base: line.as_ptr() as *mut _, iov_len: line.len() };
        let space = libc::CMSG_SPACE(std::mem::size_of::<RawFd>() as u32) as usize;
        let mut cbuf = vec![0u8; space];
        let mut msg: libc::msghdr = std::mem::zeroed();
        msg.msg_iov = &mut iov;
        msg.msg_iovlen = 1;
        msg.msg_control = cbuf.as_mut_ptr().cast();
        msg.msg_controllen = space as _;
        let c = libc::CMSG_FIRSTHDR(&msg);
        (*c).cmsg_level = libc::SOL_SOCKET;
        (*c).cmsg_type = libc::SCM_RIGHTS;
        (*c).cmsg_len = libc::CMSG_LEN(std::mem::size_of::<RawFd>() as u32) as _;
        std::ptr::write_unaligned(libc::CMSG_DATA(c) as *mut RawFd, fd);
        libc::sendmsg(sock, &msg, 0) == line.len() as isize
    }
}

/// Like `read_line`, but also returns an fd passed with SCM_RIGHTS
/// (received close-on-exec).
pub fn read_line_fd(sock: RawFd) -> Option<(String, Option<OwnedFd>)> {
    let mut out = Vec::new();
    let mut got: Option<OwnedFd> = None;
    loop {
        let mut b = [0u8; 1];
        // SAFETY: recvmsg into a 1-byte iovec and a cmsg buffer sized for
        // one fd; any received fd is taken into an OwnedFd exactly once.
        let n = unsafe {
            let mut iov = libc::iovec { iov_base: b.as_mut_ptr().cast(), iov_len: 1 };
            let space = libc::CMSG_SPACE(std::mem::size_of::<RawFd>() as u32) as usize;
            let mut cbuf = vec![0u8; space];
            let mut msg: libc::msghdr = std::mem::zeroed();
            msg.msg_iov = &mut iov;
            msg.msg_iovlen = 1;
            msg.msg_control = cbuf.as_mut_ptr().cast();
            msg.msg_controllen = space as _;
            let n = libc::recvmsg(sock, &mut msg, libc::MSG_CMSG_CLOEXEC);
            if n > 0 {
                let c = libc::CMSG_FIRSTHDR(&msg);
                if !c.is_null() && (*c).cmsg_level == libc::SOL_SOCKET && (*c).cmsg_type == libc::SCM_RIGHTS {
                    let fd = std::ptr::read_unaligned(libc::CMSG_DATA(c) as *const RawFd);
                    got = Some(OwnedFd::from_raw_fd(fd));
                }
            }
            n
        };
        if n < 0 && std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
            continue;
        }
        if n <= 0 {
            return if out.is_empty() { None } else { Some((String::from_utf8_lossy(&out).into_owned(), got)) };
        }
        if b[0] == b'\n' {
            return Some((String::from_utf8_lossy(&out).into_owned(), got));
        }
        out.push(b[0]);
    }
}

/// Write all bytes to a raw fd (retrying on EINTR / short writes).
pub fn write_all_fd(fd: RawFd, mut data: &[u8]) -> bool {
    while !data.is_empty() {
        // SAFETY: writing from a valid slice.
        let n = unsafe { libc::write(fd, data.as_ptr().cast(), data.len()) };
        if n < 0 {
            if std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return false;
        }
        data = &data[n as usize..];
    }
    true
}

/// Exit status of a wait() result as a shell-style code.
pub fn exit_code(st: std::process::ExitStatus) -> i32 {
    use std::os::unix::process::ExitStatusExt;
    st.code().unwrap_or_else(|| 128 + st.signal().unwrap_or(0))
}

/// waitpid wrapper returning a shell-style code.
pub fn wait_pid(pid: libc::pid_t) -> i32 {
    let mut status = 0;
    loop {
        // SAFETY: waiting for our own child with a valid status pointer.
        let r = unsafe { libc::waitpid(pid, &mut status, 0) };
        if r < 0 && std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
            continue;
        }
        if r < 0 {
            return 255;
        }
        break;
    }
    if libc::WIFEXITED(status) {
        libc::WEXITSTATUS(status)
    } else if libc::WIFSIGNALED(status) {
        128 + libc::WTERMSIG(status)
    } else {
        255
    }
}

pub fn euid() -> u32 {
    // SAFETY: geteuid has no preconditions.
    unsafe { libc::geteuid() }
}

/// Copy `src/.` into `dst` preserving everything (`cp -a`).
pub fn copy_tree(src: &Path, dst: &Path) -> hullr_core::Result<()> {
    std::fs::create_dir_all(dst)?;
    let st = std::process::Command::new("cp")
        .arg("-a")
        .arg("--reflink=auto")
        .arg(format!("{}/.", src.display()))
        .arg(dst)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .output()
        .map_err(|e| rt_err(format!("cp: {e}")))?;
    if !st.status.success() {
        return Err(rt_err(format!(
            "copy {} -> {}: {}",
            src.display(),
            dst.display(),
            String::from_utf8_lossy(&st.stderr).trim()
        )));
    }
    Ok(())
}

/// Spawn a self-daemonizing helper (`exe args...`) with a status pipe at
/// fd 3, reap the short-lived parent, and return the first line the
/// helper writes there (`OK` or `ERR ...`). Blocking.
pub fn spawn_detached(
    exe: &Path,
    args: &[&std::ffi::OsStr],
    envs: &[(&str, &std::ffi::OsStr)],
    timeout: std::time::Duration,
) -> hullr_core::Result<String> {
    use std::os::unix::process::CommandExt;
    let (r, w) = nix::unistd::pipe2(nix::fcntl::OFlag::O_CLOEXEC).map_err(|e| rt_err(format!("pipe: {e}")))?;
    let wfd = w.as_raw_fd();
    let mut cmd = std::process::Command::new(exe);
    cmd.args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    for (k, v) in envs {
        cmd.env(k, v);
    }
    // SAFETY: only dup2/fcntl run between fork and exec.
    unsafe {
        cmd.pre_exec(move || install_handoff_fd(wfd));
    }
    let mut child = cmd.spawn().map_err(|e| rt_err(format!("spawn {}: {e}", exe.display())))?;
    drop(w);
    let _ = child.wait();
    let mut pfd = libc::pollfd { fd: r.as_raw_fd(), events: libc::POLLIN, revents: 0 };
    // SAFETY: poll on one valid pollfd.
    let n = unsafe { libc::poll(&mut pfd, 1, timeout.as_millis() as i32) };
    if n <= 0 {
        return Err(rt_err("helper did not report status in time"));
    }
    read_line(r.as_raw_fd()).ok_or_else(|| rt_err("helper exited without reporting status"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secure_join_stays_inside() {
        let d = tempfile::tempdir().unwrap();
        let r = d.path();
        std::fs::create_dir_all(r.join("usr/lib")).unwrap();
        std::os::unix::fs::symlink("/usr/lib", r.join("lib")).unwrap();
        std::os::unix::fs::symlink("../../../../etc", r.join("usr/evil")).unwrap();
        assert_eq!(secure_join(r, "/lib/x"), r.join("usr/lib/x"));
        assert_eq!(secure_join(r, "/usr/evil/passwd"), r.join("etc/passwd"));
        assert_eq!(secure_join(r, "/../../a"), r.join("a"));
        assert_eq!(secure_join(r, "/"), r.to_path_buf());
    }
}
