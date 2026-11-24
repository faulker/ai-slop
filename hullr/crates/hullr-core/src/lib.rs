//! Shared types for every hullr crate.
//!
//! `api::Api` is the contract the CLI, TUI and GUI talk to. It is implemented
//! twice: in-process by `hullr-engine`, and over a unix socket by
//! `hullr-api::Client`. Front ends never know which one they have.

pub mod api;
pub mod container;
pub mod error;
pub mod image;
pub mod paths;
pub mod platform;
pub mod reference;
pub mod stream;

pub use api::{Api, EventStream, ProgressFn};
pub use container::*;
pub use error::{Context, Error, Result};
pub use image::*;
pub use platform::Platform;
pub use reference::ImageRef;

/// Random 64-hex-char id (like Docker container ids).
pub fn new_id() -> String {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};
    let mut out = String::with_capacity(64);
    for i in 0..4u64 {
        let mut h = RandomState::new().build_hasher();
        h.write_u64(i);
        h.write_u128(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0),
        );
        h.write_u32(std::process::id());
        out.push_str(&format!("{:016x}", h.finish()));
    }
    out
}

/// Seconds since the unix epoch.
pub fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
