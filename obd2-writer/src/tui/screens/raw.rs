use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Color, Style, Stylize},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame,
};
use tokio::sync::oneshot;

use crate::error::Result;
use crate::tui::elm_actor::ElmHandle;

const MAX_OUTPUT_LINES: usize = 2000;

pub struct RawState {
    pub input_buffer: String,
    pub command_history: Vec<String>,
    output_lines: Vec<(String, OutputKind)>,
    pub history_index: Option<usize>,
    pending_response: Option<oneshot::Receiver<Result<String>>>,
    pending_response_uds: Option<oneshot::Receiver<Result<Vec<u8>>>>,
    pending_response_dbg: Option<oneshot::Receiver<Result<String>>>,
    pub input_active: bool,
}

#[derive(Clone)]
enum OutputKind {
    Command,
    Response,
    Error,
}

impl Default for RawState {
    fn default() -> Self {
        Self {
            input_buffer: String::new(),
            command_history: Vec::new(),
            output_lines: vec![("Type AT or hex commands. Enter to send. Esc to navigate tabs.".to_string(), OutputKind::Response)],
            history_index: None,
            pending_response: None,
            pending_response_uds: None,
            pending_response_dbg: None,
            input_active: true,
        }
    }
}

impl RawState {
    pub fn tick(&mut self) {
        if let Some(mut rx) = self.pending_response.take() {
            match rx.try_recv() {
                Ok(Ok(response)) => {
                    let lines: Vec<&str> = response.lines()
                        .map(|l| l.trim())
                        .filter(|l| !l.is_empty())
                        .collect();
                    if lines.len() > 1 {
                        // Multi-line response — show all lines clearly
                        for (i, line) in lines.iter().enumerate() {
                            self.output_lines.push((
                                format!("[{}] {}", i + 1, line),
                                OutputKind::Response,
                            ));
                        }
                    } else {
                        for line in &lines {
                            self.output_lines
                                .push((line.to_string(), OutputKind::Response));
                        }
                    }
                    self.trim_output();
                }
                Ok(Err(e)) => {
                    self.output_lines
                        .push((format!("ERROR: {}", e), OutputKind::Error));
                    self.trim_output();
                }
                Err(oneshot::error::TryRecvError::Empty) => {
                    self.pending_response = Some(rx);
                }
                Err(oneshot::error::TryRecvError::Closed) => {
                    self.output_lines
                        .push(("Connection lost".to_string(), OutputKind::Error));
                }
            }
        }

        if let Some(mut rx) = self.pending_response_dbg.take() {
            match rx.try_recv() {
                Ok(Ok(response)) => {
                    // Show raw bytes as hex dump
                    let bytes: Vec<u8> = response.bytes().collect();
                    self.output_lines.push((
                        format!("RAW ({} bytes):", bytes.len()),
                        OutputKind::Response,
                    ));
                    // Show hex dump in chunks of 32 bytes
                    for chunk in bytes.chunks(32) {
                        let hex = chunk.iter()
                            .map(|b| format!("{:02X}", b))
                            .collect::<Vec<_>>()
                            .join(" ");
                        let ascii: String = chunk.iter()
                            .map(|&b| if b >= 0x20 && b < 0x7F { b as char } else { '.' })
                            .collect();
                        self.output_lines.push((
                            format!("  {} | {}", hex, ascii),
                            OutputKind::Response,
                        ));
                    }
                    // Also show as lines split by \r and \n
                    self.output_lines.push(("LINES:".to_string(), OutputKind::Response));
                    for (i, line) in response.split(|c| c == '\r' || c == '\n').enumerate() {
                        let trimmed = line.trim();
                        if !trimmed.is_empty() {
                            self.output_lines.push((
                                format!("  [{}] {}", i, trimmed),
                                OutputKind::Response,
                            ));
                        }
                    }
                    self.trim_output();
                }
                Ok(Err(e)) => {
                    self.output_lines.push((format!("DBG ERROR: {}", e), OutputKind::Error));
                    self.trim_output();
                }
                Err(oneshot::error::TryRecvError::Empty) => {
                    self.pending_response_dbg = Some(rx);
                }
                Err(oneshot::error::TryRecvError::Closed) => {}
            }
        }

        if let Some(mut rx) = self.pending_response_uds.take() {
            match rx.try_recv() {
                Ok(Ok(bytes)) => {
                    let hex = bytes.iter()
                        .map(|b| format!("{:02X}", b))
                        .collect::<Vec<_>>()
                        .join(" ");
                    self.output_lines.push((
                        format!("UDS response ({} bytes): {}", bytes.len(), hex),
                        OutputKind::Response,
                    ));
                    self.trim_output();
                }
                Ok(Err(e)) => {
                    self.output_lines.push((format!("UDS ERROR: {}", e), OutputKind::Error));
                    self.trim_output();
                }
                Err(oneshot::error::TryRecvError::Empty) => {
                    self.pending_response_uds = Some(rx);
                }
                Err(oneshot::error::TryRecvError::Closed) => {
                    self.output_lines.push(("Connection lost".to_string(), OutputKind::Error));
                }
            }
        }
    }

    fn trim_output(&mut self) {
        if self.output_lines.len() > MAX_OUTPUT_LINES {
            let excess = self.output_lines.len() - MAX_OUTPUT_LINES;
            self.output_lines.drain(..excess);
        }
    }

    pub fn is_input_focused(&self) -> bool {
        self.input_active
    }
}

pub fn render(state: &mut RawState, f: &mut Frame, area: Rect, _elm: &ElmHandle) {
    let block = Block::default()
        .title(" Raw Commands ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::White));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let chunks = Layout::vertical([Constraint::Min(3), Constraint::Length(1)]).split(inner);

    // Output area
    let output_area = chunks[0];
    let visible_lines = output_area.height as usize;
    let total = state.output_lines.len();
    let start = if total > visible_lines {
        total - visible_lines
    } else {
        0
    };

    let lines: Vec<Line> = state.output_lines[start..]
        .iter()
        .map(|(text, kind)| {
            let style = match kind {
                OutputKind::Command => Style::default().fg(Color::Yellow).bold(),
                OutputKind::Response => Style::default().fg(Color::Green),
                OutputKind::Error => Style::default().fg(Color::Red),
            };
            Line::styled(text.as_str(), style)
        })
        .collect();

    let output = Paragraph::new(lines).wrap(Wrap { trim: false });
    f.render_widget(output, output_area);

    // Input line
    let pending_indicator = if state.pending_response.is_some() {
        Span::styled(" [waiting] ", Style::default().fg(Color::Yellow))
    } else {
        Span::raw("")
    };
    let mode_indicator = if !state.input_active {
        Span::styled(" [Esc — type to resume] ", Style::default().fg(Color::DarkGray))
    } else {
        Span::styled(" [Esc to navigate] ", Style::default().fg(Color::DarkGray))
    };
    let prompt_color = if state.input_active { Color::Cyan } else { Color::DarkGray };
    let input = Line::from(vec![
        Span::styled("> ", Style::default().fg(prompt_color).bold()),
        Span::raw(&state.input_buffer),
        if state.input_active {
            Span::styled("_", Style::default().fg(Color::Cyan))
        } else {
            Span::raw("")
        },
        pending_indicator,
        mode_indicator,
    ]);
    f.render_widget(Paragraph::new(input), chunks[1]);
}

pub fn handle_key(state: &mut RawState, key: KeyEvent, elm: &ElmHandle) -> bool {
    // If not in input mode, any key except Esc re-enters input mode
    // F5 clears output, F6 saves output to file
    if key.code == KeyCode::F(5) {
        state.output_lines.clear();
        state.output_lines.push(("Output cleared. F5:clear F6:save".to_string(), OutputKind::Response));
        return true;
    }
    if key.code == KeyCode::F(6) {
        let path = "raw-output.txt";
        let content: String = state.output_lines.iter()
            .map(|(text, _)| text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        match std::fs::write(path, &content) {
            Ok(_) => {
                state.output_lines.push((
                    format!("Saved {} lines to {}", state.output_lines.len(), path),
                    OutputKind::Response,
                ));
            }
            Err(e) => {
                state.output_lines.push((
                    format!("Save failed: {}", e),
                    OutputKind::Error,
                ));
            }
        }
        return true;
    }

    if !state.input_active {
        match key.code {
            KeyCode::Char(c) => {
                state.input_active = true;
                state.input_buffer.push(c);
                return true;
            }
            KeyCode::Enter | KeyCode::Up | KeyCode::Down => {
                state.input_active = true;
                // fall through to normal handling below
            }
            _ => return false, // let app handle tab switching etc.
        }
    }

    match key.code {
        KeyCode::Enter => {
            let cmd = state.input_buffer.trim().to_string();
            if !cmd.is_empty() && state.pending_response.is_none() {
                state
                    .output_lines
                    .push((format!("> {}", cmd), OutputKind::Command));

                // Route commands based on prefix:
                //   AT...     → AT command (no OBD parsing)
                //   uds ...   → hex bytes sent through UDS/ISO-TP (reassembles multi-frame)
                //   sec ECU L → security access: request seed for level L on ECU
                //   (other)   → raw passthrough
                let upper = cmd.to_uppercase();
                if upper.starts_with("AT") {
                    if let Some(rx) = elm.try_at_command(&cmd) {
                        state.pending_response = Some(rx);
                    }
                } else if upper.starts_with("UNLOCK ") {
                    // unlock 7C0 61 → try common Toyota security algorithms
                    let parts: Vec<&str> = cmd.split_whitespace().collect();
                    if parts.len() >= 3 {
                        let ecu = parts[1];
                        if let Ok(level) = u8::from_str_radix(parts[2], 16) {
                            state.output_lines.push((
                                format!("Attempting security unlock on {} level 0x{:02X}...", ecu, level),
                                OutputKind::Command,
                            ));
                            if let Some(rx) = elm.try_security_unlock(ecu, level) {
                                state.pending_response = Some(rx);
                            }
                        } else {
                            state.output_lines.push(("Invalid level (hex)".to_string(), OutputKind::Error));
                        }
                    } else {
                        state.output_lines.push(("Usage: unlock <ECU> <level> (e.g. unlock 7C0 61)".to_string(), OutputKind::Error));
                    }
                } else if upper.starts_with("DBG ") {
                    // dbg <cmd> → send command and show raw byte dump of response
                    let raw_cmd = cmd[4..].trim().to_string();
                    if let Some(rx) = elm.try_raw_command(&raw_cmd) {
                        state.pending_response_dbg = Some(rx);
                    }
                } else if upper.starts_with("SEC ") {
                    // sec 7C0 61 → security access on ECU 7C0, level 0x61
                    let parts: Vec<&str> = cmd.split_whitespace().collect();
                    if parts.len() >= 3 {
                        let ecu = parts[1];
                        if let Ok(level) = u8::from_str_radix(parts[2], 16) {
                            if let Some(rx) = elm.try_security_access(ecu, level) {
                                state.pending_response_uds = Some(rx);
                            }
                        } else {
                            state.output_lines.push(("Invalid level (hex)".to_string(), OutputKind::Error));
                        }
                    } else {
                        state.output_lines.push(("Usage: sec <ECU> <level> (e.g. sec 7C0 61)".to_string(), OutputKind::Error));
                    }
                } else if upper.starts_with("KEY ") {
                    // key 7C0 62 AA BB CC DD → send security key to ECU
                    let parts: Vec<&str> = cmd.split_whitespace().collect();
                    if parts.len() >= 4 {
                        let key_bytes: std::result::Result<Vec<u8>, _> = parts[2..]
                            .iter()
                            .map(|s| u8::from_str_radix(s, 16))
                            .collect();
                        match key_bytes {
                            Ok(data) if !data.is_empty() => {
                                let hex = data.iter()
                                    .map(|b| format!("{:02X}", b))
                                    .collect::<Vec<_>>()
                                    .join(" ");
                                let full_cmd = format!("27 {}", hex);
                                state.output_lines.push((
                                    format!("Sending {} ({} bytes)", full_cmd, data.len() + 1),
                                    OutputKind::Command,
                                ));
                                // Use multi-frame for >7 byte messages
                                if data.len() + 1 > 7 {
                                    if let Some(rx) = elm.try_send_multi_frame(&full_cmd) {
                                        state.pending_response = Some(rx);
                                    }
                                } else {
                                    if let Some(rx) = elm.try_raw_command(&full_cmd) {
                                        state.pending_response = Some(rx);
                                    }
                                }
                            }
                            _ => {
                                state.output_lines.push((
                                    "Usage: key <ECU> <level+key_hex> (e.g. key 7C0 62 AA BB CC DD)".to_string(),
                                    OutputKind::Error,
                                ));
                            }
                        }
                    } else {
                        state.output_lines.push((
                            "Usage: key <ECU> <level+key_hex> (e.g. key 7C0 62 AA BB CC DD)".to_string(),
                            OutputKind::Error,
                        ));
                    }
                } else if upper.starts_with("UDS ") {
                    // uds 27 61 → send hex bytes through UDS with ISO-TP reassembly
                    let hex_part = cmd[4..].trim();
                    let bytes: std::result::Result<Vec<u8>, _> = hex_part
                        .split_whitespace()
                        .map(|s| u8::from_str_radix(s, 16))
                        .collect();
                    match bytes {
                        Ok(data) if !data.is_empty() => {
                            let hex_str = data.iter()
                                .map(|b| format!("{:02X}", b))
                                .collect::<Vec<_>>()
                                .join(" ");
                            if let Some(rx) = elm.try_raw_command(&hex_str) {
                                state.pending_response = Some(rx);
                            }
                        }
                        _ => {
                            state.output_lines.push(("Usage: uds <hex bytes> (e.g. uds 27 61)".to_string(), OutputKind::Error));
                        }
                    }
                } else {
                    if let Some(rx) = elm.try_raw_command(&cmd) {
                        state.pending_response = Some(rx);
                    }
                }

                if state.command_history.last().map(|s| s.as_str()) != Some(&cmd) {
                    state.command_history.push(cmd);
                }
                state.input_buffer.clear();
                state.history_index = None;
                state.trim_output();
            }
            true
        }
        KeyCode::Up => {
            if !state.command_history.is_empty() {
                let idx = match state.history_index {
                    Some(i) if i > 0 => i - 1,
                    Some(i) => i,
                    None => state.command_history.len() - 1,
                };
                state.history_index = Some(idx);
                state.input_buffer = state.command_history[idx].clone();
            }
            true
        }
        KeyCode::Down => {
            if let Some(idx) = state.history_index {
                if idx + 1 < state.command_history.len() {
                    let next = idx + 1;
                    state.history_index = Some(next);
                    state.input_buffer = state.command_history[next].clone();
                } else {
                    state.history_index = None;
                    state.input_buffer.clear();
                }
            }
            true
        }
        KeyCode::Backspace => {
            state.input_buffer.pop();
            true
        }
        KeyCode::Char(c) => {
            state.input_buffer.push(c);
            true
        }
        KeyCode::Esc => {
            state.input_active = false;
            state.input_buffer.clear();
            state.history_index = None;
            true
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_raw_state_default() {
        let s = RawState::default();
        assert!(s.input_buffer.is_empty());
        assert!(s.command_history.is_empty());
        assert!(!s.output_lines.is_empty());
    }

    #[test]
    fn test_raw_input_focus_toggle() {
        let mut s = RawState::default();
        // Starts active
        assert!(s.is_input_focused());
        // Esc deactivates
        s.input_active = false;
        assert!(!s.is_input_focused());
        // Re-activate
        s.input_active = true;
        assert!(s.is_input_focused());
    }

    #[test]
    fn test_raw_trim_output() {
        let mut s = RawState::default();
        for i in 0..MAX_OUTPUT_LINES + 500 {
            s.output_lines.push((format!("line {}", i), OutputKind::Response));
        }
        s.trim_output();
        assert!(s.output_lines.len() <= MAX_OUTPUT_LINES);
    }
}
