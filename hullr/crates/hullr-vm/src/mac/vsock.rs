//! Host side vsock: a [`Connector`] that dials guest ports through the VM's
//! `VZVirtioSocketDevice`.
//!
//! `VZVirtualMachine` and its devices must only be touched on the queue the
//! VM was created with, which is the main queue. The VM lives in a
//! main-thread `thread_local`; `connect` hops to the main queue with
//! `exec_async`, calls `connectToPort:completionHandler:` there, and the
//! completion handler (also on the main queue) sends back a `dup` of the
//! connection's fd. The original connection is then closed, so the socket's
//! lifetime is exactly that of the returned stream.

use crate::transport::Connector;
use block2::RcBlock;
use dispatch2::DispatchQueue;
use objc2::rc::Retained;
use objc2_foundation::NSError;
use objc2_virtualization::{VZVirtioSocketConnection, VZVirtioSocketDevice, VZVirtualMachine};
use std::cell::{Cell, RefCell};
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, IntoRawFd, OwnedFd};
use std::time::Duration;
use tokio::sync::oneshot;

thread_local! {
    /// The running VM. Only ever set and read on the main thread.
    static VM: RefCell<Option<Retained<VZVirtualMachine>>> = const { RefCell::new(None) };
}

/// Store the VM (main thread only).
pub fn set_vm(vm: Retained<VZVirtualMachine>) {
    VM.with(|v| *v.borrow_mut() = Some(vm));
}

/// Run `f` with the VM (main thread only).
pub fn with_vm<R>(f: impl FnOnce(&VZVirtualMachine) -> R) -> Option<R> {
    VM.with(|v| v.borrow().as_deref().map(f))
}

/// Dials guest vsock ports.
#[derive(Debug, Clone, Copy, Default)]
pub struct VsockConnector;

type FdResult = io::Result<OwnedFd>;

impl Connector for VsockConnector {
    type Stream = tokio::net::UnixStream;

    async fn connect(&self, port: u32) -> io::Result<Self::Stream> {
        let (tx, rx) = oneshot::channel::<FdResult>();
        DispatchQueue::main().exec_async(move || connect_on_main(port, tx));
        let fd = tokio::time::timeout(Duration::from_secs(10), rx)
            .await
            .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, format!("vsock connect to port {port} timed out")))?
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "vm queue dropped the connect request"))??;
        set_nonblocking(&fd)?;
        // tokio only needs a nonblocking stream socket fd; the unix stream type
        // works for AF_VSOCK fds too.
        // SAFETY: the fd is an owned, connected stream socket.
        let std_stream = unsafe { std::os::unix::net::UnixStream::from_raw_fd(fd.into_raw_fd()) };
        tokio::net::UnixStream::from_std(std_stream)
    }
}

fn set_nonblocking(fd: &OwnedFd) -> io::Result<()> {
    // SAFETY: fcntl on a valid owned fd.
    unsafe {
        let flags = libc::fcntl(fd.as_raw_fd(), libc::F_GETFL);
        if flags < 0 || libc::fcntl(fd.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) < 0 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}

fn socket_device(vm: &VZVirtualMachine) -> Option<Retained<VZVirtioSocketDevice>> {
    // SAFETY: called on the VM's queue (main), as the framework requires.
    let dev = unsafe { vm.socketDevices() }.firstObject()?;
    dev.downcast::<VZVirtioSocketDevice>().ok()
}

/// Runs on the main queue.
fn connect_on_main(port: u32, tx: oneshot::Sender<FdResult>) {
    let Some(Some(dev)) = with_vm(socket_device) else {
        let _ = tx.send(Err(io::Error::new(io::ErrorKind::NotConnected, "vm is not running")));
        return;
    };
    // The block type is `Fn`, so the one-shot sender is taken out of a Cell.
    let tx = Cell::new(Some(tx));
    let handler = RcBlock::new(move |conn: *mut VZVirtioSocketConnection, err: *mut NSError| {
        let Some(tx) = tx.take() else { return };
        // SAFETY: the framework passes either a valid connection or a valid
        // error, both live for the duration of this call.
        let res = unsafe {
            match conn.as_ref() {
                Some(conn) => {
                    let fd = libc::dup(conn.fileDescriptor());
                    conn.close();
                    if fd < 0 {
                        Err(io::Error::last_os_error())
                    } else {
                        Ok(OwnedFd::from_raw_fd(fd))
                    }
                }
                None => {
                    let msg = err
                        .as_ref()
                        .map(|e| e.localizedDescription().to_string())
                        .unwrap_or_else(|| "unknown error".into());
                    Err(io::Error::new(io::ErrorKind::ConnectionRefused, format!("vsock port {port}: {msg}")))
                }
            }
        };
        let _ = tx.send(res);
    });
    // SAFETY: on the VM's queue; the block is retained by the framework.
    unsafe { dev.connectToPort_completionHandler(port, &handler) };
}
