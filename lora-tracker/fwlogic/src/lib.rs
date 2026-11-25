//! Firmware logic that doesn't touch hardware, kept out of `firmware/` so it
//! runs under `cargo test` on the host.
//!
//! - [`nmea`]: GPS sentences to a fix.
//! - [`geoip`]: coarse location from the network's public IP when there's no GPS.
//! - [`location`]: picks the best position source and turns it into report fields.
//! - [`http`], [`sntp`]: the tiny bits of HTTP/1.0 and SNTP the board speaks.
//! - [`log`], [`screens`]: the event log and the OLED pages.
//! - [`telemetry`]: the heartbeat sent to the server.
#![cfg_attr(not(test), no_std)]

pub mod fixed;
pub mod geoip;
pub mod http;
pub mod location;
pub mod log;
pub mod nmea;
pub mod screens;
pub mod sntp;
pub mod telemetry;
pub mod time;
