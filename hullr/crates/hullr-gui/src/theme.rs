//! Colors and global style.

use eframe::egui::{self, Color32};
use hullr_core::{ContainerInfo, ContainerState};

pub const ACCENT: Color32 = Color32::from_rgb(56, 139, 253);

pub fn success(dark: bool) -> Color32 {
    if dark { Color32::from_rgb(86, 211, 100) } else { Color32::from_rgb(26, 127, 55) }
}

pub fn danger(dark: bool) -> Color32 {
    if dark { Color32::from_rgb(255, 123, 114) } else { Color32::from_rgb(207, 34, 46) }
}

pub fn warning(dark: bool) -> Color32 {
    if dark { Color32::from_rgb(227, 179, 65) } else { Color32::from_rgb(154, 103, 0) }
}

pub fn muted(dark: bool) -> Color32 {
    if dark { Color32::from_gray(150) } else { Color32::from_gray(100) }
}

/// Dot color: green running, yellow transitional, red failed, gray stopped.
pub fn state_color(c: &ContainerInfo, dark: bool) -> Color32 {
    match c.state {
        ContainerState::Running => success(dark),
        ContainerState::Paused | ContainerState::Restarting => warning(dark),
        ContainerState::Dead => danger(dark),
        ContainerState::Exited if c.exit_code.unwrap_or(0) != 0 => danger(dark),
        ContainerState::Exited | ContainerState::Created => muted(dark),
    }
}

/// One-time style tweaks: roomier spacing, no theme override (light or dark
/// follows the system unless the user picks one in System).
pub fn install(ctx: &egui::Context) {
    ctx.style_mut(|s| {
        s.spacing.item_spacing = egui::vec2(8.0, 6.0);
        s.spacing.button_padding = egui::vec2(10.0, 4.0);
        s.visuals.selection.bg_fill = ACCENT.linear_multiply(0.55);
    });
}
