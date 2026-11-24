//! The Linux VM that hosts containers on macOS.
//!
//! On Linux hullr runs containers natively and this crate is only used for
//! its guest half. On macOS there is no Linux kernel, so a small Linux VM is
//! booted with Apple's Virtualization.framework, and the Linux build of the
//! same `hullr` binary runs inside it as the daemon.
//!
//! Pieces:
//!
//! * [`config`]: VM settings stored in `<root>/vm/config.json`.
//! * [`state`]: pid and state files of the detached VM host process, used to
//!   answer [`status`] and implement [`stop`].
//! * [`boot`]: boot file lookup, disk image and kernel command line.
//! * [`protocol`]: vsock port numbers and the small wire formats between host
//!   and guest.
//! * [`transport`]: the `Connector`/`Acceptor` abstraction over vsock, so the
//!   forwarding logic is testable with TCP on Linux.
//! * [`bridge`]: host unix socket to guest vsock API bridge.
//! * [`forward`]: host side of port forwarding (listens on published ports).
//! * [`guest`]: the agent that runs inside the VM (`hullr __guest-agent`).
//! * `mac`: the Virtualization.framework host process (`hullr __vm`), macOS
//!   only.
//!
//! Hidden CLI subcommands that must be dispatched by `hullr-cli`:
//! `hullr __vm` calls [`vm_main`] and `hullr __guest-agent` calls
//! [`guest::run_guest_agent`].

pub mod boot;
pub mod bridge;
pub mod config;
pub mod forward;
pub mod guest;
pub mod protocol;
pub mod state;
pub mod transport;

#[cfg(target_os = "macos")]
mod mac;

use hullr_core::paths::Paths;
#[cfg(not(target_os = "macos"))]
use hullr_core::Error;
use hullr_core::Result;
use std::path::{Path, PathBuf};

pub use config::{load_config, save_config, VmConfig};
pub use state::{status, stop, VmStatus};

/// The hidden subcommand that runs the VM host process.
pub const VM_SUBCOMMAND: &str = "__vm";
/// The hidden subcommand that runs the agent inside the guest.
pub const GUEST_AGENT_SUBCOMMAND: &str = "__guest-agent";

/// Ensure the VM is running (spawning the detached `hullr __vm` host process
/// if needed) and the API socket answers. Returns the socket path.
pub async fn ensure_running(paths: &Paths, exe: &Path) -> Result<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        mac::ensure_running(paths, exe).await
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (paths, exe);
        Err(Error::Unsupported(
            "the hullr VM is only needed on macOS; on Linux containers run natively".into(),
        ))
    }
}

/// Entry for the detached VM host process (`hullr __vm`). The CLI calls this
/// from main when argv[1] == "__vm". Blocks for the VM lifetime and returns
/// the process exit code.
pub fn vm_main(paths: &Paths) -> i32 {
    #[cfg(target_os = "macos")]
    {
        mac::vm_main(paths)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = paths;
        eprintln!("hullr: {VM_SUBCOMMAND} is only supported on macOS");
        2
    }
}
