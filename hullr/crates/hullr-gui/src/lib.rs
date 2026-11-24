//! Desktop app for hullr, built on egui/eframe (macOS and Linux).
//!
//! `state` holds all UI state and window-free logic; `backend` runs API calls
//! on the tokio runtime and reports back over a channel; `views` draw.

pub mod app;
pub mod backend;
pub mod state;
pub mod state_text;
pub mod sys;
pub mod theme;
pub mod views;
pub mod widgets;

#[cfg(test)]
mod mock;
#[cfg(test)]
mod testutil;

use anyhow::anyhow;
use hullr_core::Api;
use std::sync::Arc;

/// Open the desktop window and run until it is closed. Blocks the calling
/// thread (it must be the main thread); API calls run on `rt`.
pub fn run(api: Arc<dyn Api>, rt: tokio::runtime::Handle) -> anyhow::Result<()> {
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("hullr")
            .with_inner_size([1180.0, 760.0])
            .with_min_inner_size([820.0, 520.0]),
        ..Default::default()
    };
    eframe::run_native("hullr", options, Box::new(move |cc| Ok(Box::new(app::HullrApp::new(cc, api, rt)))))
        .map_err(|e| anyhow!("could not start the window: {e}"))
}
