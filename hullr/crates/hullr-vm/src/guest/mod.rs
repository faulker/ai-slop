//! The half of hullr-vm that runs inside the Linux guest.
//!
//! `/init` starts `hullr __guest-agent`, which the CLI dispatches to
//! [`run_guest_agent`]. [`VsockListener`] is also exported for the daemon's
//! `--listen vsock:<port>` mode.

pub mod agent;
pub mod ports;
#[cfg(target_os = "linux")]
pub mod vsock;

pub use agent::run_guest_agent;
#[cfg(target_os = "linux")]
pub use vsock::VsockListener;

#[cfg(test)]
mod tests;
