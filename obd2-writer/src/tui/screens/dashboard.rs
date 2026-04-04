use std::collections::HashMap;
use std::time::Instant;

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Color, Style, Stylize},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};
use tokio::sync::oneshot;

use crate::error::Result;
use crate::tui::elm_actor::ElmHandle;
use crate::tui::widgets::gauge::LabeledGauge;
use crate::tui::widgets::pid_picker::{self, PidPickerState};

/// Per-PID cached value.
#[allow(dead_code)]
struct PidValue {
    value: f64,
    unit: String,
    updated: Instant,
}

pub struct DashboardState {
    pub selected_pids: Vec<String>,
    values: HashMap<String, PidValue>,
    pub poll_interval_ms: u64,
    pub paused: bool,
    pub picker: PidPickerState,
    pub selected_index: usize,
    pending_request: Option<(String, oneshot::Receiver<Result<(String, f64, String)>>)>,
    poll_index: usize,
    last_poll: Instant,
}

impl Default for DashboardState {
    fn default() -> Self {
        Self {
            selected_pids: vec![
                "rpm".to_string(),
                "speed".to_string(),
                "coolant_temp".to_string(),
                "throttle".to_string(),
            ],
            values: HashMap::new(),
            poll_interval_ms: 500,
            paused: false,
            picker: PidPickerState::default(),
            selected_index: 0,
            pending_request: None,
            poll_index: 0,
            last_poll: Instant::now(),
        }
    }
}

impl DashboardState {
    /// Non-blocking tick: check pending results and issue new requests.
    pub fn tick(&mut self, elm: &ElmHandle) {
        // Check if a pending request completed
        if let Some((pid_key, mut rx)) = self.pending_request.take() {
            match rx.try_recv() {
                Ok(Ok((_name, value, unit))) => {
                    self.values.insert(
                        pid_key,
                        PidValue {
                            value,
                            unit,
                            updated: Instant::now(),
                        },
                    );
                }
                Ok(Err(_)) => {
                    // Request failed, skip this PID for now
                }
                Err(oneshot::error::TryRecvError::Empty) => {
                    // Still pending, put it back
                    self.pending_request = Some((pid_key, rx));
                    return;
                }
                Err(oneshot::error::TryRecvError::Closed) => {
                    // Sender dropped
                }
            }
        }

        // If paused or no PIDs, do nothing
        if self.paused || self.selected_pids.is_empty() {
            return;
        }

        // Check poll interval
        if self.last_poll.elapsed().as_millis() < self.poll_interval_ms as u128 {
            return;
        }

        // Fire next PID request
        let pid_key = &self.selected_pids[self.poll_index % self.selected_pids.len()];

        // Enhanced DIDs use "did:XXXX:ECU" format
        if pid_key.starts_with("did:") {
            let parts: Vec<&str> = pid_key.splitn(3, ':').collect();
            if parts.len() == 3 {
                if let Some(rx) = elm.try_fetch_enhanced_did(parts[1], parts[2]) {
                    let key = pid_key.clone();
                    // We need to adapt the receiver type, so wrap in a task
                    let (tx, orx) = oneshot::channel();
                    tokio::spawn(async move {
                        let result = rx.await;
                        let mapped = match result {
                            Ok(Ok(r)) => Ok((
                                r.name,
                                r.value.unwrap_or(0.0),
                                r.unit,
                            )),
                            Ok(Err(e)) => Err(e),
                            Err(_) => Err(crate::error::Error::NotConnected),
                        };
                        let _ = tx.send(mapped);
                    });
                    self.pending_request = Some((key, orx));
                    self.poll_index += 1;
                    self.last_poll = Instant::now();
                }
            }
        } else if let Some(rx) = elm.try_read_pid(pid_key) {
            self.pending_request = Some((pid_key.clone(), rx));
            self.poll_index += 1;
            self.last_poll = Instant::now();
        }
    }
}

pub fn render(state: &mut DashboardState, f: &mut Frame, area: Rect, _elm: &ElmHandle) {
    let block = Block::default()
        .title(" Dashboard ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));
    let inner = block.inner(area);
    f.render_widget(block, area);

    if state.selected_pids.is_empty() {
        let msg = Paragraph::new("No PIDs selected. Press Enter to add PIDs.")
            .style(Style::default().fg(Color::DarkGray));
        f.render_widget(msg, inner);
        // Render picker if open
        pid_picker::render_picker(&mut state.picker, f.area(), f.buffer_mut());
        return;
    }

    // Help line at bottom
    let help_height = 2;
    let gauge_height = inner.height.saturating_sub(help_height);

    let chunks = Layout::vertical([
        Constraint::Length(gauge_height),
        Constraint::Length(help_height),
    ])
    .split(inner);

    // Render gauges
    let gauge_area = chunks[0];
    let max_visible = gauge_area.height as usize;

    for (i, pid_key) in state.selected_pids.iter().enumerate() {
        if i >= max_visible {
            break;
        }
        let y = gauge_area.y + i as u16;
        let row = Rect::new(gauge_area.x, y, gauge_area.width, 1);

        let (label, value, unit, max) = if let Some(pv) = state.values.get(pid_key) {
            let display_name = if pid_key.starts_with("did:") {
                // Show the friendly name from the value
                pid_key
                    .splitn(3, ':')
                    .nth(1)
                    .unwrap_or(pid_key)
                    .to_string()
            } else {
                pid_key.clone()
            };
            let max = default_max_for_pid(pid_key, pv.value);
            (display_name, pv.value, pv.unit.clone(), max)
        } else {
            let display_name = if pid_key.starts_with("did:") {
                pid_key.splitn(3, ':').nth(1).unwrap_or(pid_key).to_string()
            } else {
                pid_key.clone()
            };
            (display_name, 0.0, "?".to_string(), 100.0)
        };

        let color = if i == state.selected_index {
            Color::Yellow
        } else {
            gauge_color_for_pid(pid_key)
        };

        let g = LabeledGauge::new(&label, value, &unit)
            .range(0.0, max)
            .color(color);
        f.render_widget(g, row);
    }

    // Help text
    let status = if state.paused { "PAUSED" } else { "LIVE" };
    let help = Line::from(vec![
        Span::styled(
            format!(" [{}] ", status),
            Style::default()
                .fg(if state.paused { Color::Red } else { Color::Green })
                .bold(),
        ),
        Span::styled(
            format!("{}ms ", state.poll_interval_ms),
            Style::default().fg(Color::DarkGray),
        ),
        Span::raw("| "),
        Span::styled("Enter", Style::default().bold()),
        Span::raw(":add "),
        Span::styled("d", Style::default().bold()),
        Span::raw(":remove "),
        Span::styled("Space", Style::default().bold()),
        Span::raw(":pause "),
        Span::styled("+/-", Style::default().bold()),
        Span::raw(":interval "),
        Span::styled("Up/Down", Style::default().bold()),
        Span::raw(":select"),
    ]);
    f.render_widget(Paragraph::new(help), chunks[1]);

    // Render picker overlay if open
    pid_picker::render_picker(&mut state.picker, f.area(), f.buffer_mut());
}

pub fn handle_key(state: &mut DashboardState, key: KeyEvent, _elm: &ElmHandle) -> bool {
    // If picker is open, delegate to it
    if state.picker.visible {
        match state.picker.handle_key(key) {
            Ok(Some(selected_key)) => {
                if !state.selected_pids.contains(&selected_key) {
                    state.selected_pids.push(selected_key);
                }
                state.picker.visible = false;
            }
            Ok(None) => {}
            Err(()) => {
                state.picker.visible = false;
            }
        }
        return true;
    }

    match key.code {
        KeyCode::Enter => {
            state.picker = PidPickerState::open();
            true
        }
        KeyCode::Char(' ') => {
            state.paused = !state.paused;
            true
        }
        KeyCode::Char('+') | KeyCode::Char('=') => {
            if state.poll_interval_ms > 100 {
                state.poll_interval_ms -= 100;
            }
            true
        }
        KeyCode::Char('-') => {
            state.poll_interval_ms += 100;
            true
        }
        KeyCode::Char('d') => {
            if !state.selected_pids.is_empty() {
                let idx = state.selected_index.min(state.selected_pids.len() - 1);
                state.selected_pids.remove(idx);
                if state.selected_index > 0 && state.selected_index >= state.selected_pids.len() {
                    state.selected_index = state.selected_pids.len().saturating_sub(1);
                }
            }
            true
        }
        KeyCode::Up => {
            if state.selected_index > 0 {
                state.selected_index -= 1;
            }
            true
        }
        KeyCode::Down => {
            if state.selected_index + 1 < state.selected_pids.len() {
                state.selected_index += 1;
            }
            true
        }
        _ => false,
    }
}

fn default_max_for_pid(key: &str, current: f64) -> f64 {
    match key {
        "rpm" => 8000.0,
        "speed" => 200.0,
        "coolant_temp" | "intake_temp" | "ambient_temp" | "oil_temp" => 150.0,
        "throttle" | "load" | "fuel_level" => 100.0,
        "battery_voltage" => 16.0,
        "maf" => 300.0,
        "intake_pressure" | "baro_pressure" => 255.0,
        "timing_advance" => 64.0,
        "runtime" => 65535.0,
        _ => {
            // For unknown/enhanced, auto-scale
            if current > 0.0 {
                current * 1.5
            } else {
                100.0
            }
        }
    }
}

fn gauge_color_for_pid(key: &str) -> Color {
    match key {
        "rpm" => Color::Green,
        "speed" => Color::Cyan,
        "coolant_temp" | "oil_temp" => Color::Red,
        "throttle" | "load" => Color::Yellow,
        "fuel_level" => Color::Magenta,
        "battery_voltage" => Color::Blue,
        _ => Color::Cyan,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dashboard_state_default() {
        let s = DashboardState::default();
        assert!(!s.paused);
        assert_eq!(s.poll_interval_ms, 500);
        assert!(!s.selected_pids.is_empty());
    }

    #[test]
    fn test_default_max_for_pid() {
        assert_eq!(default_max_for_pid("rpm", 0.0), 8000.0);
        assert_eq!(default_max_for_pid("speed", 0.0), 200.0);
        assert_eq!(default_max_for_pid("unknown", 50.0), 75.0);
    }

    #[test]
    fn test_gauge_color_for_pid() {
        assert_eq!(gauge_color_for_pid("rpm"), Color::Green);
        assert_eq!(gauge_color_for_pid("unknown"), Color::Cyan);
    }
}
