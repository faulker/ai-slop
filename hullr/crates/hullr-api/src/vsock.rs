//! Minimal AF_VSOCK listener (Linux only), used when the daemon runs inside
//! the macOS VM and the host talks to it over virtio-vsock.

#[cfg(target_os = "linux")]
pub fn listen(port: u32) -> std::io::Result<std::os::fd::OwnedFd> {
    use std::os::fd::FromRawFd;
    // SAFETY: plain socket/bind/listen syscalls on a fresh fd; the fd is
    // wrapped in OwnedFd immediately so it is closed on every error path.
    unsafe {
        let fd = libc::socket(libc::AF_VSOCK, libc::SOCK_STREAM | libc::SOCK_CLOEXEC, 0);
        if fd < 0 {
            return Err(std::io::Error::last_os_error());
        }
        let owned = std::os::fd::OwnedFd::from_raw_fd(fd);
        let mut addr: libc::sockaddr_vm = std::mem::zeroed();
        addr.svm_family = libc::AF_VSOCK as libc::sa_family_t;
        addr.svm_cid = libc::VMADDR_CID_ANY;
        addr.svm_port = port;
        let rc = libc::bind(
            fd,
            &addr as *const libc::sockaddr_vm as *const libc::sockaddr,
            std::mem::size_of::<libc::sockaddr_vm>() as libc::socklen_t,
        );
        if rc < 0 {
            return Err(std::io::Error::last_os_error());
        }
        if libc::listen(fd, 128) < 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(owned)
    }
}

/// Blocking accept; returns the connection as a std UnixStream, which works
/// for any stream socket fd (we only read and write it).
#[cfg(target_os = "linux")]
pub fn accept(listener: &std::os::fd::OwnedFd) -> std::io::Result<std::os::unix::net::UnixStream> {
    use std::os::fd::{AsRawFd, FromRawFd};
    // SAFETY: accept4 on a listening fd we own; null addr pointers are allowed.
    let fd = unsafe {
        libc::accept4(listener.as_raw_fd(), std::ptr::null_mut(), std::ptr::null_mut(), libc::SOCK_CLOEXEC)
    };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: fd is a freshly accepted, owned stream socket.
    Ok(unsafe { std::os::unix::net::UnixStream::from_raw_fd(fd) })
}
