//! Terminal UI for hullr, built on ratatui.
//!
//! Layout: `state` (pure reducer) -> `ui`/`views` (pure render) and `app`
//! (runtime that talks to the API through channels). `term` holds the
//! reusable interactive terminal pump.

pub mod app;
pub mod format;
pub mod state;
pub mod term;
pub mod theme;
pub mod ui;
pub mod views;
pub mod widgets;

#[cfg(test)]
mod testutil;

pub use app::run;
