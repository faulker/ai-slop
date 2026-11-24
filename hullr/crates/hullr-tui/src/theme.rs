//! Colors and styles in one place.

use hullr_core::{ContainerInfo, ContainerState};
use ratatui::style::{Color, Modifier, Style};

pub const ACCENT: Color = Color::Cyan;
pub const MUTED: Color = Color::DarkGray;

pub fn title() -> Style {
    Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
}

pub fn muted() -> Style {
    Style::default().fg(MUTED)
}

pub fn selected() -> Style {
    Style::default().bg(Color::Rgb(40, 50, 70)).add_modifier(Modifier::BOLD)
}

pub fn header() -> Style {
    Style::default().fg(Color::Gray).add_modifier(Modifier::BOLD)
}

pub fn key() -> Style {
    Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
}

pub fn ok() -> Style {
    Style::default().fg(Color::Green)
}

pub fn warn() -> Style {
    Style::default().fg(Color::Yellow)
}

pub fn err() -> Style {
    Style::default().fg(Color::Red)
}

/// Dot color for a container: green running, yellow transitional, red failed,
/// gray stopped.
pub fn state_color(c: &ContainerInfo) -> Color {
    match c.state {
        ContainerState::Running => Color::Green,
        ContainerState::Paused | ContainerState::Restarting => Color::Yellow,
        ContainerState::Dead => Color::Red,
        ContainerState::Exited if c.exit_code.unwrap_or(0) != 0 => Color::Red,
        ContainerState::Exited | ContainerState::Created => Color::DarkGray,
    }
}
