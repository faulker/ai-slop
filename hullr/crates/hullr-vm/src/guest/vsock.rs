//! AF_VSOCK listener for the guest (Linux only), integrated with tokio.
//!
//! Accepted connections are returned as `tokio::net::UnixStream`: tokio only
//! needs a nonblocking stream socket fd for epoll readiness, read, write and
//! shutdown, so the unix stream type works for vsock fds as well. This lets
//! the daemon serve vsock with the same code path as its unix socket.

use crate::transport::Acceptor;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
#[cfg(test)]
use std::time::Duration;
use tokio::io::unix::AsyncFd;
use tokio::io::Interest;

/// A listening AF_VSOCK socket bound to `VMADDR_CID_ANY:<port>`.
pub struct VsockListener {
    fd: AsyncFd<OwnedFd>,
    port: u32,
}

fn cvt(rc: libc::c_int) -> io::Result<libc::c_int> {
    if rc < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(rc)
    }
}

impl VsockListener {
    /// Bind and listen. Must be called inside a tokio runtime.
    pub fn bind(port: u32) -> io::Result<Self> {
        // SAFETY: plain socket(2) call; the returned fd is owned immediately.
        let raw = cvt(unsafe {
            libc::socket(libc::AF_VSOCK, libc::SOCK_STREAM | libc::SOCK_NONBLOCK | libc::SOCK_CLOEXEC, 0)
        })?;
        // SAFETY: `raw` is a fresh fd that nothing else owns.
        let fd = unsafe { OwnedFd::from_raw_fd(raw) };
        // SAFETY: sockaddr_vm is plain data; all-zero is a valid starting value.
        let mut addr: libc::sockaddr_vm = unsafe { std::mem::zeroed() };
        addr.svm_family = libc::AF_VSOCK as libc::sa_family_t;
        addr.svm_port = port;
        addr.svm_cid = libc::VMADDR_CID_ANY;
        // SAFETY: addr points to a correctly sized, initialized sockaddr_vm.
        cvt(unsafe {
            libc::bind(
                fd.as_raw_fd(),
                &addr as *const libc::sockaddr_vm as *const libc::sockaddr,
                std::mem::size_of::<libc::sockaddr_vm>() as libc::socklen_t,
            )
        })?;
        // SAFETY: listen on a valid bound socket.
        cvt(unsafe { libc::listen(fd.as_raw_fd(), 128) })?;
        Ok(Self { fd: register(fd)?, port })
    }

    pub fn port(&self) -> u32 {
        self.port
    }

    /// Accept one connection.
    pub async fn accept(&self) -> io::Result<tokio::net::UnixStream> {
        loop {
            let mut guard = self.fd.readable().await?;
            let res = guard.try_io(|fd| {
                // SAFETY: accept4 with null address pointers is allowed; the
                // returned fd is owned by the caller.
                cvt(unsafe {
                    libc::accept4(
                        fd.as_raw_fd(),
                        std::ptr::null_mut(),
                        std::ptr::null_mut(),
                        libc::SOCK_NONBLOCK | libc::SOCK_CLOEXEC,
                    )
                })
            });
            match res {
                Ok(Ok(raw)) => return stream_from_fd(raw),
                Ok(Err(e)) => return Err(e),
                Err(_would_block) => continue,
            }
        }
    }
}

/// `with_interest` is deprecated in newer tokio in favour of the unsafe
/// `register_with_interest`, which older tokio 1.x lacks. Its I/O safety
/// concern does not apply here: the `OwnedFd` is owned by the `AsyncFd` and
/// never closed or replaced while registered.
#[allow(deprecated)]
fn register(fd: OwnedFd) -> io::Result<AsyncFd<OwnedFd>> {
    AsyncFd::with_interest(fd, Interest::READABLE)
}

fn stream_from_fd(raw: RawFd) -> io::Result<tokio::net::UnixStream> {
    // SAFETY: `raw` is a freshly accepted, nonblocking stream socket owned
    // by nobody else.
    let std_stream = unsafe { std::os::unix::net::UnixStream::from_raw_fd(raw) };
    tokio::net::UnixStream::from_std(std_stream)
}

impl Acceptor for VsockListener {
    type Stream = tokio::net::UnixStream;
    async fn accept(&self) -> io::Result<Self::Stream> {
        VsockListener::accept(self).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// Blocking connect to `VMADDR_CID_LOCAL:<port>` (vsock loopback).
    fn connect_local(port: u32) -> io::Result<std::os::unix::net::UnixStream> {
        // SAFETY: as in `bind`; the fd is owned right away.
        let fd = unsafe { OwnedFd::from_raw_fd(cvt(libc::socket(libc::AF_VSOCK, libc::SOCK_STREAM | libc::SOCK_CLOEXEC, 0))?) };
        // SAFETY: plain data, zero is valid.
        let mut addr: libc::sockaddr_vm = unsafe { std::mem::zeroed() };
        addr.svm_family = libc::AF_VSOCK as libc::sa_family_t;
        addr.svm_port = port;
        addr.svm_cid = 1; // VMADDR_CID_LOCAL
        // SAFETY: valid sockaddr_vm pointer and length.
        cvt(unsafe {
            libc::connect(
                fd.as_raw_fd(),
                &addr as *const libc::sockaddr_vm as *const libc::sockaddr,
                std::mem::size_of::<libc::sockaddr_vm>() as libc::socklen_t,
            )
        })?;
        Ok(std::os::unix::net::UnixStream::from(fd))
    }

    /// Runs only where the kernel has vsock loopback (`vsock_loopback`);
    /// otherwise it reports and passes.
    #[tokio::test]
    async fn vsock_loopback_accept() {
        let port = 40_000 + (std::process::id() % 20_000);
        let listener = match VsockListener::bind(port) {
            Ok(l) => l,
            Err(e) => {
                eprintln!("skipping: no AF_VSOCK here ({e})");
                return;
            }
        };
        let client = tokio::task::spawn_blocking(move || connect_local(port));
        let mut server = match tokio::time::timeout(Duration::from_secs(2), listener.accept()).await {
            Ok(s) => s.unwrap(),
            Err(_) => {
                eprintln!("skipping: vsock loopback not available");
                return;
            }
        };
        let Ok(Ok(client)) = client.await else {
            eprintln!("skipping: vsock loopback connect failed");
            return;
        };
        client.set_nonblocking(true).unwrap();
        let mut client = tokio::net::UnixStream::from_std(client).unwrap();
        client.write_all(b"ping").await.unwrap();
        let mut buf = [0; 4];
        server.read_exact(&mut buf).await.unwrap();
        assert_eq!(&buf, b"ping");
    }
}
