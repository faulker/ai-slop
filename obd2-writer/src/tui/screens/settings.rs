use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Color, Style, Stylize},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
    Frame,
};
use tokio::sync::oneshot;

use crate::error::Result;
use crate::toyota::enhanced_pids;
use crate::tui::elm_actor::ElmHandle;
use crate::tui::widgets::confirm::{self, ConfirmState};

struct WritableDid {
    id: u16,
    name: String,
    unit: String,
    ecu: String,
    protocol: String,
    category: String,
    current_value: Option<String>,
    /// Named values for toggle display, e.g. "00" → "OFF", "C0" → "ON"
    values: Option<std::collections::HashMap<String, String>>,
}

pub struct SettingsState {
    dids: Vec<WritableDid>,
    pub list_state: ListState,
    pub editing: bool,
    pub dry_run_mode: bool,
    pub input_buffer: String,
    confirm: ConfirmState,
    pending_write: Option<oneshot::Receiver<Result<()>>>,
    pending_read: Option<(usize, oneshot::Receiver<Result<Vec<u8>>>)>,
    read_all_queue: Vec<usize>,
    status_message: String,
}

impl Default for SettingsState {
    fn default() -> Self {
        let cached = enhanced_pids::cached_dids();
        let dids: Vec<WritableDid> = cached
            .iter()
            .filter(|d| d.writable)
            .map(|d| WritableDid {
                id: d.id,
                name: d.name.clone(),
                unit: d.unit.clone(),
                ecu: d.ecu.clone(),
                protocol: d.protocol.clone(),
                category: d.category.clone().unwrap_or_else(|| "General".to_string()),
                current_value: None,
                values: d.values.clone(),
            })
            .collect();

        let mut list_state = ListState::default();
        if !dids.is_empty() {
            list_state.select(Some(0));
        }

        Self {
            dids,
            list_state,
            editing: false,
            dry_run_mode: false,
            input_buffer: String::new(),
            confirm: ConfirmState::default(),
            pending_write: None,
            pending_read: None,
            read_all_queue: Vec::new(),
            status_message: String::new(),
        }
    }
}

impl SettingsState {
    pub fn tick_with_elm(&mut self, elm: &ElmHandle) {
        // Check pending write
        if let Some(mut rx) = self.pending_write.take() {
            match rx.try_recv() {
                Ok(Ok(())) => {
                    self.status_message = "Write successful".to_string();
                    // Auto-read the value back after a successful write
                    if let Some(idx) = self.list_state.selected() {
                        if idx < self.dids.len() {
                            let did = &self.dids[idx];
                            if let Some(rx) = elm.try_read_did_value(did.id, &did.ecu, &did.protocol) {
                                self.pending_read = Some((idx, rx));
                            }
                        }
                    }
                }
                Ok(Err(e)) => {
                    self.status_message = format!("Write failed: {}", e);
                }
                Err(oneshot::error::TryRecvError::Empty) => {
                    self.pending_write = Some(rx);
                }
                Err(oneshot::error::TryRecvError::Closed) => {
                    self.status_message = "Write channel closed".to_string();
                }
            }
        }

        // Check pending read
        if let Some((idx, mut rx)) = self.pending_read.take() {
            match rx.try_recv() {
                Ok(Ok(data)) => {
                    let hex = data.iter().map(|b| format!("{:02X}", b)).collect::<Vec<_>>().join(" ");
                    if idx < self.dids.len() {
                        self.dids[idx].current_value = Some(hex.clone());
                        self.status_message = format!("Read OK: {}", hex);
                    }
                    // Continue read-all queue
                    self.fire_next_read_all(elm);
                }
                Ok(Err(_)) => {
                    if idx < self.dids.len() {
                        self.dids[idx].current_value = Some("NO RESPONSE".to_string());
                    }
                    self.status_message = format!("0x{:04X}: no response (DID may not exist on this vehicle)",
                        if idx < self.dids.len() { self.dids[idx].id } else { 0 });
                    // Continue read-all queue
                    self.fire_next_read_all(elm);
                }
                Err(oneshot::error::TryRecvError::Empty) => {
                    self.pending_read = Some((idx, rx));
                }
                Err(oneshot::error::TryRecvError::Closed) => {
                    self.status_message = "Read failed: connection lost".to_string();
                }
            }
        }
    }

    fn fire_next_read_all(&mut self, elm: &ElmHandle) {
        while let Some(idx) = self.read_all_queue.pop() {
            if idx < self.dids.len() {
                let did = &self.dids[idx];
                if let Some(rx) = elm.try_read_did_value(did.id, &did.ecu, &did.protocol) {
                    let remaining = self.read_all_queue.len();
                    self.status_message = format!("Reading 0x{:04X}... ({} remaining)", did.id, remaining);
                    self.pending_read = Some((idx, rx));
                    return;
                }
            }
        }
    }

    pub fn is_input_focused(&self) -> bool {
        self.editing || self.confirm.visible
    }
}

pub fn render(state: &mut SettingsState, f: &mut Frame, area: Rect, _elm: &ElmHandle) {
    let block = Block::default()
        .title(" Settings (Writable DIDs) ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow));
    let inner = block.inner(area);
    f.render_widget(block, area);

    if state.dids.is_empty() {
        let msg = Paragraph::new("No writable DIDs defined in toyota_dids.toml")
            .style(Style::default().fg(Color::DarkGray));
        f.render_widget(msg, inner);
        return;
    }

    let chunks = Layout::vertical([
        Constraint::Min(3),
        Constraint::Length(3),
    ])
    .split(inner);

    // DID list
    let items: Vec<ListItem> = state
        .dids
        .iter()
        .map(|did| {
            let (val_display, val_color) = match did.current_value.as_deref() {
                Some("NO RESPONSE") => ("NO RESPONSE".to_string(), Color::Red),
                Some(v) => {
                    // Look up named value if available
                    let label = did.values.as_ref()
                        .and_then(|vals| vals.get(&v.to_uppercase()))
                        .or_else(|| did.values.as_ref().and_then(|vals| vals.get(&v.to_lowercase())));
                    match label {
                        Some(name) => (format!("{} ({})", name, v), Color::Green),
                        None => (format!("{} {}", v, did.unit), Color::White),
                    }
                }
                None => ("(not read)".to_string(), Color::DarkGray),
            };
            let line = Line::from(vec![
                Span::styled(format!("0x{:04X}", did.id), Style::default().fg(Color::Cyan)),
                Span::raw(" "),
                Span::styled(&did.name, Style::default().bold()),
                Span::styled(format!(" [{}]", did.category), Style::default().fg(Color::DarkGray)),
                Span::styled(format!("  {}", val_display), Style::default().fg(val_color)),
            ]);
            ListItem::new(line)
        })
        .collect();

    let list = List::new(items)
        .highlight_style(Style::default().bg(Color::DarkGray).bold())
        .highlight_symbol(">> ");

    f.render_stateful_widget(list, chunks[0], &mut state.list_state);

    // Help / edit area
    let help_content = if state.editing {
        Line::from(vec![
            Span::styled(" EDIT: ", Style::default().fg(Color::Yellow).bold()),
            Span::raw(format!("New hex value: {}_", state.input_buffer)),
            Span::styled("  Enter:confirm  Esc:cancel", Style::default().fg(Color::DarkGray)),
        ])
    } else {
        let mut spans = vec![
            Span::styled("t", Style::default().bold()),
            Span::raw(":toggle "),
            Span::styled("Enter", Style::default().bold()),
            Span::raw(":edit "),
            Span::styled("r", Style::default().bold()),
            Span::raw(":read "),
            Span::styled("R", Style::default().bold()),
            Span::raw(":read all "),
            Span::styled("d", Style::default().bold()),
            Span::raw(":dry-run "),
            Span::styled("Up/Down", Style::default().bold()),
            Span::raw(":select"),
        ];
        if !state.status_message.is_empty() {
            spans.push(Span::raw("  "));
            spans.push(Span::styled(
                &state.status_message,
                Style::default().fg(Color::Green),
            ));
        }
        Line::from(spans)
    };
    f.render_widget(Paragraph::new(help_content), chunks[1]);

    // Confirm overlay
    confirm::render_confirm(&state.confirm, f.area(), f.buffer_mut());
}

pub fn handle_key(state: &mut SettingsState, key: KeyEvent, elm: &ElmHandle) -> bool {
    // Confirm dialog
    if state.confirm.visible {
        match key.code {
            KeyCode::Left | KeyCode::Right | KeyCode::Tab => {
                state.confirm.toggle_selection();
            }
            KeyCode::Enter => {
                if state.confirm.selected_yes {
                    if let Some(idx) = state.list_state.selected() {
                        if idx < state.dids.len() {
                            let did = &state.dids[idx];
                            let did_hex = format!("{:04X}", did.id);
                            let data = state.input_buffer.replace(' ', "");
                            let ecu = did.ecu.clone();
                            let dry_run = state.dry_run_mode;
                            let protocol = did.protocol.clone();
                            if let Some(rx) = elm.try_verified_write(&did_hex, &data, &ecu, &protocol, dry_run)
                            {
                                state.pending_write = Some(rx);
                                state.status_message = if dry_run {
                                    "Dry-run...".to_string()
                                } else {
                                    "Writing...".to_string()
                                };
                            }
                        }
                    }
                }
                state.confirm.dismiss();
                state.editing = false;
                state.dry_run_mode = false;
                state.input_buffer.clear();
            }
            KeyCode::Esc => {
                state.confirm.dismiss();
            }
            _ => {}
        }
        return true;
    }

    // Editing mode
    if state.editing {
        match key.code {
            KeyCode::Enter => {
                if !state.input_buffer.is_empty() {
                    let title = if state.dry_run_mode { "Confirm Dry-Run" } else { "Confirm Write" };
                    let msg = if state.dry_run_mode {
                        format!("Dry-run {} on selected DID? (no changes will be made)", state.input_buffer)
                    } else {
                        format!("Write {} to selected DID?", state.input_buffer)
                    };
                    state.confirm = ConfirmState::show(title, msg);
                }
            }
            KeyCode::Esc => {
                state.editing = false;
                state.input_buffer.clear();
            }
            KeyCode::Backspace => {
                state.input_buffer.pop();
            }
            KeyCode::Char(c) if c.is_ascii_hexdigit() || c == ' ' => {
                state.input_buffer.push(c.to_ascii_uppercase());
            }
            _ => {}
        }
        return true;
    }

    // Normal mode
    match key.code {
        KeyCode::Up => {
            let i = state.list_state.selected().unwrap_or(0);
            if i > 0 {
                state.list_state.select(Some(i - 1));
            }
            true
        }
        KeyCode::Down => {
            let i = state.list_state.selected().unwrap_or(0);
            if i + 1 < state.dids.len() {
                state.list_state.select(Some(i + 1));
            }
            true
        }
        KeyCode::Enter => {
            state.editing = true;
            state.dry_run_mode = false;
            state.input_buffer.clear();
            state.status_message.clear();
            true
        }
        KeyCode::Char('t') => {
            // Toggle: cycle to the next named value and write it
            if let Some(idx) = state.list_state.selected() {
                if idx < state.dids.len() && state.pending_write.is_none() {
                    let did = &state.dids[idx];
                    if let (Some(cur), Some(vals)) = (&did.current_value, &did.values) {
                        // Find the next value in the map
                        let cur_upper = cur.to_uppercase();
                        let mut keys: Vec<&String> = vals.keys().collect();
                        keys.sort(); // deterministic order
                        let cur_idx = keys.iter().position(|k| k.to_uppercase() == cur_upper);
                        let next_idx = match cur_idx {
                            Some(i) => (i + 1) % keys.len(),
                            None => 0,
                        };
                        let next_val = keys[next_idx].to_uppercase();
                        let next_label = &vals[keys[next_idx]];
                        let did_hex = format!("{:04X}", did.id);
                        let ecu = did.ecu.clone();
                        let protocol = did.protocol.clone();
                        if let Some(rx) = elm.try_verified_write(&did_hex, &next_val, &ecu, &protocol, false) {
                            state.pending_write = Some(rx);
                            state.status_message = format!("Setting to {} ({})...", next_label, next_val);
                        }
                    } else if did.values.is_some() {
                        state.status_message = "Read the value first (press 'r')".to_string();
                    } else {
                        state.status_message = "No toggle values defined for this setting".to_string();
                    }
                }
            }
            true
        }
        KeyCode::Char('r') => {
            // Read current value of selected DID
            if let Some(idx) = state.list_state.selected() {
                if idx < state.dids.len() && state.pending_read.is_none() {
                    let did = &state.dids[idx];
                    if let Some(rx) = elm.try_read_did_value(did.id, &did.ecu, &did.protocol) {
                        state.pending_read = Some((idx, rx));
                        state.status_message = format!("Reading 0x{:04X}...", did.id);
                    }
                }
            }
            true
        }
        KeyCode::Char('R') => {
            // Read all DIDs sequentially
            if state.pending_read.is_none() && !state.dids.is_empty() {
                // Queue all indices in reverse so pop() gives us index 0 first
                state.read_all_queue = (0..state.dids.len()).rev().collect();
                state.status_message = "Reading all DIDs...".to_string();
                state.fire_next_read_all(elm);
            }
            true
        }
        KeyCode::Char('d') => {
            if let Some(idx) = state.list_state.selected() {
                if idx < state.dids.len() {
                    state.editing = true;
                    state.dry_run_mode = true;
                    state.input_buffer.clear();
                    state.status_message = "Enter value for dry-run (no changes will be made)".to_string();
                }
            }
            true
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_settings_state_default() {
        let s = SettingsState::default();
        assert!(!s.editing);
        assert!(s.input_buffer.is_empty());
    }

    #[test]
    fn test_settings_is_input_focused() {
        let mut s = SettingsState::default();
        assert!(!s.is_input_focused());
        s.editing = true;
        assert!(s.is_input_focused());
    }
}
