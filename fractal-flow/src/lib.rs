//! fractal-flow: endless, high-detail animated fractal visuals.
//!
//! The binary (`main.rs`) owns the window and event loop; everything that can be
//! tested without a GPU lives in these modules.

pub mod app;
pub mod cast;
pub mod deep;
pub mod director;
pub mod gpu;
pub mod palette;
pub mod perf;
pub mod quality;
pub mod rng;
pub mod scene;
pub mod settings;
pub mod snapshot;
pub mod ui;
