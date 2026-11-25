//! Shared, `no_std` building blocks for lora-tracker.
//!
//! - [`packet`]: the sealed position report used on LoRa, WiFi, and LTE.
//! - [`telemetry`]: the sealed heartbeat sent over HTTP, position or not.
//! - [`config`]: the device config blob, where every uplink section is optional.
//!
//! See `docs/protocol.md` for the byte layout.
#![cfg_attr(not(test), no_std)]

pub mod config;
pub mod error;
pub mod packet;
pub mod telemetry;

pub use error::Error;
/// Re-exported so callers build config strings/vecs with the exact same version.
pub use heapless;
