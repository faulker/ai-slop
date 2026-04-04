use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Color, Style, Stylize},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
    Frame,
};
use tokio::sync::{mpsc, oneshot};

use crate::error::Result;
use crate::toyota::did_scan::{self, DiscoveredDid};
use crate::tui::elm_actor::{ElmHandle, FoundEcu, KwpResult, ProgressUpdate};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanPane {
    Ecu,
    Did,
}

pub struct ScansState {
    pub active_pane: ScanPane,
    // ECU scan
    pub ecu_results: Vec<FoundEcu>,
    pub ecu_list_state: ListState,
    pub ecu_scanning: bool,
    // DID scan
    pub did_results: Vec<DiscoveredDid>,
    pub kwp_results: Vec<KwpResult>,
    pub did_list_state: ListState,
    pub did_scanning: bool,
    pub scan_mode: ScanMode,
    pub ecu_input: String,
    pub range_input: String,
    pub writable_toggle: bool,
    pub input_field: DidInputField,
    pub input_focused: bool,
    // Progress
    pub scan_progress: String,
    // Pending operations
    pending_ecu_scan: Option<oneshot::Receiver<Result<Vec<FoundEcu>>>>,
    pending_did_scan: Option<oneshot::Receiver<Result<Vec<DiscoveredDid>>>>,
    pending_kwp_scan: Option<oneshot::Receiver<Result<Vec<KwpResult>>>>,
    progress_rx: Option<mpsc::UnboundedReceiver<ProgressUpdate>>,
    status_message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanMode {
    Uds,   // Service 0x22 ReadDataByIdentifier (2-byte DID)
    Kwp21, // Service 0x21 ReadDataByLocalIdentifier (1-byte ID)
    Kwp1A, // Service 0x1A ReadEcuIdentification (1-byte ID)
}

impl ScanMode {
    fn label(self) -> &'static str {
        match self {
            ScanMode::Uds => "UDS 0x22",
            ScanMode::Kwp21 => "KWP 0x21",
            ScanMode::Kwp1A => "KWP 0x1A",
        }
    }

    fn next(self) -> Self {
        match self {
            ScanMode::Uds => ScanMode::Kwp21,
            ScanMode::Kwp21 => ScanMode::Kwp1A,
            ScanMode::Kwp1A => ScanMode::Uds,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DidInputField {
    Ecu,
    Range,
}

impl Default for ScansState {
    fn default() -> Self {
        Self {
            active_pane: ScanPane::Ecu,
            ecu_results: Vec::new(),
            ecu_list_state: ListState::default(),
            ecu_scanning: false,
            did_results: Vec::new(),
            kwp_results: Vec::new(),
            did_list_state: ListState::default(),
            did_scanning: false,
            scan_mode: ScanMode::Kwp21,
            ecu_input: "7C0".to_string(),
            range_input: "00-FF".to_string(),
            writable_toggle: false,
            input_field: DidInputField::Ecu,
            input_focused: false,
            scan_progress: String::new(),
            pending_ecu_scan: None,
            pending_did_scan: None,
            pending_kwp_scan: None,
            progress_rx: None,
            status_message: String::new(),
        }
    }
}

impl ScansState {
    pub fn tick(&mut self) {
        // Drain progress updates
        if let Some(ref mut rx) = self.progress_rx {
            while let Ok(update) = rx.try_recv() {
                match update {
                    ProgressUpdate::Step(cur, total, msg) => {
                        self.scan_progress = format!("{} ({}/{})", msg, cur, total);
                    }
                    ProgressUpdate::Done(msg) => {
                        self.scan_progress = msg;
                    }
                }
            }
        }

        if let Some(mut rx) = self.pending_ecu_scan.take() {
            match rx.try_recv() {
                Ok(Ok(ecus)) => {
                    self.status_message = format!("Found {} ECU(s)", ecus.len());
                    self.ecu_results = ecus;
                    self.ecu_scanning = false;
                    if !self.ecu_results.is_empty() {
                        self.ecu_list_state.select(Some(0));
                    }
                }
                Ok(Err(e)) => {
                    self.status_message = format!("ECU scan error: {}", e);
                    self.ecu_scanning = false;
                }
                Err(oneshot::error::TryRecvError::Empty) => {
                    self.pending_ecu_scan = Some(rx);
                }
                Err(oneshot::error::TryRecvError::Closed) => {
                    self.ecu_scanning = false;
                }
            }
        }

        if let Some(mut rx) = self.pending_did_scan.take() {
            match rx.try_recv() {
                Ok(Ok(dids)) => {
                    self.status_message = format!("Found {} DID(s)", dids.len());
                    self.did_results = dids;
                    self.did_scanning = false;
                    if !self.did_results.is_empty() {
                        self.did_list_state.select(Some(0));
                    }
                }
                Ok(Err(e)) => {
                    self.status_message = format!("DID scan error: {}", e);
                    self.did_scanning = false;
                }
                Err(oneshot::error::TryRecvError::Empty) => {
                    self.pending_did_scan = Some(rx);
                }
                Err(oneshot::error::TryRecvError::Closed) => {
                    self.did_scanning = false;
                }
            }
        }

        if let Some(mut rx) = self.pending_kwp_scan.take() {
            match rx.try_recv() {
                Ok(Ok(results)) => {
                    self.status_message = format!("Found {} KWP ID(s)", results.len());
                    self.kwp_results = results;
                    self.did_scanning = false;
                    if !self.kwp_results.is_empty() {
                        self.did_list_state.select(Some(0));
                    }
                }
                Ok(Err(e)) => {
                    self.status_message = format!("KWP scan error: {}", e);
                    self.did_scanning = false;
                }
                Err(oneshot::error::TryRecvError::Empty) => {
                    self.pending_kwp_scan = Some(rx);
                }
                Err(oneshot::error::TryRecvError::Closed) => {
                    self.did_scanning = false;
                }
            }
        }
    }

    pub fn is_input_focused(&self) -> bool {
        self.input_focused
    }
}

pub fn render(state: &mut ScansState, f: &mut Frame, area: Rect, _elm: &ElmHandle) {
    let block = Block::default()
        .title(" ECU & DID Scans ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Magenta));
    let inner = block.inner(area);
    f.render_widget(block, area);

    // Split horizontally: ECU pane | DID pane
    let panes = Layout::horizontal([Constraint::Percentage(40), Constraint::Percentage(60)])
        .split(inner);

    render_ecu_pane(state, f, panes[0]);
    render_did_pane(state, f, panes[1]);
}

fn render_ecu_pane(state: &mut ScansState, f: &mut Frame, area: Rect) {
    let is_active = state.active_pane == ScanPane::Ecu;
    let border_color = if is_active { Color::Cyan } else { Color::DarkGray };
    let block = Block::default()
        .title(" ECUs ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let chunks = Layout::vertical([Constraint::Min(3), Constraint::Length(2)]).split(inner);

    if state.ecu_scanning {
        let msg = if state.scan_progress.is_empty() {
            "Scanning ECUs...".to_string()
        } else {
            state.scan_progress.clone()
        };
        f.render_widget(
            Paragraph::new(msg).style(Style::default().fg(Color::Yellow)),
            chunks[0],
        );
    } else if state.ecu_results.is_empty() {
        f.render_widget(
            Paragraph::new("Press 'e' to scan").style(Style::default().fg(Color::DarkGray)),
            chunks[0],
        );
    } else {
        let items: Vec<ListItem> = state
            .ecu_results
            .iter()
            .map(|ecu| {
                let line = Line::from(vec![
                    Span::styled(
                        format!("0x{}", ecu.tx_address),
                        Style::default().fg(Color::Cyan),
                    ),
                    Span::raw(" "),
                    Span::raw(&ecu.name),
                ]);
                ListItem::new(line)
            })
            .collect();

        let list = List::new(items)
            .highlight_style(Style::default().bg(Color::DarkGray).bold())
            .highlight_symbol(">> ");
        f.render_stateful_widget(list, chunks[0], &mut state.ecu_list_state);
    }

    let help = Line::from(vec![
        Span::styled("e", Style::default().bold()),
        Span::raw(":scan "),
        Span::styled("Enter", Style::default().bold()),
        Span::raw(":use in DID scan"),
    ]);
    f.render_widget(Paragraph::new(help), chunks[1]);
}

fn render_did_pane(state: &mut ScansState, f: &mut Frame, area: Rect) {
    let is_active = state.active_pane == ScanPane::Did;
    let border_color = if is_active { Color::Cyan } else { Color::DarkGray };
    let block = Block::default()
        .title(" DID Scan ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let chunks = Layout::vertical([
        Constraint::Length(3), // Input fields
        Constraint::Min(3),   // Results
        Constraint::Length(2), // Help
    ])
    .split(inner);

    // Input fields
    let ecu_style = if state.input_focused && state.input_field == DidInputField::Ecu {
        Style::default().fg(Color::Yellow).bold()
    } else {
        Style::default().fg(Color::White)
    };
    let range_style = if state.input_focused && state.input_field == DidInputField::Range {
        Style::default().fg(Color::Yellow).bold()
    } else {
        Style::default().fg(Color::White)
    };

    let input_lines = vec![
        Line::from(vec![
            Span::styled("ECU: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                if state.input_focused && state.input_field == DidInputField::Ecu {
                    format!("{}_", state.ecu_input)
                } else {
                    state.ecu_input.clone()
                },
                ecu_style,
            ),
            Span::raw("  "),
            Span::styled("Range: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                if state.input_focused && state.input_field == DidInputField::Range {
                    format!("{}_", state.range_input)
                } else {
                    state.range_input.clone()
                },
                range_style,
            ),
        ]),
        Line::from(vec![
            Span::styled("Mode: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                state.scan_mode.label(),
                Style::default().fg(Color::Magenta).bold(),
            ),
            Span::raw("  "),
            Span::styled("Writable test: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                if state.writable_toggle { "ON" } else { "OFF" },
                Style::default().fg(if state.writable_toggle {
                    Color::Green
                } else {
                    Color::Red
                }),
            ),
        ]),
        Line::from(vec![Span::styled(
            &state.scan_progress,
            Style::default().fg(Color::Yellow),
        )]),
    ];
    f.render_widget(Paragraph::new(input_lines), chunks[0]);

    // Results
    if state.did_scanning {
        f.render_widget(
            Paragraph::new("Scanning...").style(Style::default().fg(Color::Yellow)),
            chunks[1],
        );
    } else {
        let has_results = match state.scan_mode {
            ScanMode::Uds => !state.did_results.is_empty(),
            ScanMode::Kwp21 | ScanMode::Kwp1A => !state.kwp_results.is_empty(),
        };

        if !has_results {
            f.render_widget(
                Paragraph::new("Press 's' to scan").style(Style::default().fg(Color::DarkGray)),
                chunks[1],
            );
        } else {
            match state.scan_mode {
                ScanMode::Uds => {
                    let items: Vec<ListItem> = state
                        .did_results
                        .iter()
                        .map(|d| {
                            let hex = d
                                .data
                                .iter()
                                .map(|b| format!("{:02X}", b))
                                .collect::<Vec<_>>()
                                .join(" ");
                            let line = Line::from(vec![
                                Span::styled(
                                    format!("0x{:04X}", d.did),
                                    Style::default().fg(Color::Cyan),
                                ),
                                Span::raw(format!(" = {} ", hex)),
                                if d.writable {
                                    Span::styled("[W]", Style::default().fg(Color::Green).bold())
                                } else {
                                    Span::styled("[R]", Style::default().fg(Color::DarkGray))
                                },
                            ]);
                            ListItem::new(line)
                        })
                        .collect();

                    let list = List::new(items)
                        .highlight_style(Style::default().bg(Color::DarkGray).bold())
                        .highlight_symbol(">> ");
                    f.render_stateful_widget(list, chunks[1], &mut state.did_list_state);
                }
                ScanMode::Kwp21 | ScanMode::Kwp1A => {
                    let items: Vec<ListItem> = state
                        .kwp_results
                        .iter()
                        .map(|r| {
                            let hex = r
                                .data
                                .iter()
                                .map(|b| format!("{:02X}", b))
                                .collect::<Vec<_>>()
                                .join(" ");
                            let ascii: String = r
                                .data
                                .iter()
                                .map(|&b| if b.is_ascii_graphic() || b == b' ' { b as char } else { '.' })
                                .collect();
                            let line = Line::from(vec![
                                Span::styled(
                                    format!("0x{:02X}", r.local_id),
                                    Style::default().fg(Color::Cyan),
                                ),
                                Span::raw(format!(" = {} ", hex)),
                                Span::styled(
                                    format!(" {}", ascii),
                                    Style::default().fg(Color::DarkGray),
                                ),
                            ]);
                            ListItem::new(line)
                        })
                        .collect();

                    let list = List::new(items)
                        .highlight_style(Style::default().bg(Color::DarkGray).bold())
                        .highlight_symbol(">> ");
                    f.render_stateful_widget(list, chunks[1], &mut state.did_list_state);
                }
            }
        }
    }

    let help = Line::from(vec![
        Span::styled("s", Style::default().bold()),
        Span::raw(":scan "),
        Span::styled("m", Style::default().bold()),
        Span::raw(":mode "),
        Span::styled("Esc", Style::default().bold()),
        Span::raw(":cancel "),
        Span::styled("i", Style::default().bold()),
        Span::raw(":edit "),
        Span::styled("w", Style::default().bold()),
        Span::raw(":writable "),
        Span::styled("Tab", Style::default().bold()),
        Span::raw(":pane"),
        if !state.status_message.is_empty() {
            Span::styled(
                format!("  {}", state.status_message),
                Style::default().fg(Color::DarkGray),
            )
        } else {
            Span::raw("")
        },
    ]);
    f.render_widget(Paragraph::new(help), chunks[2]);
}

pub fn handle_key(state: &mut ScansState, key: KeyEvent, elm: &ElmHandle) -> bool {
    // Input editing mode
    if state.input_focused {
        match key.code {
            KeyCode::Esc => {
                state.input_focused = false;
            }
            KeyCode::Tab => {
                state.input_field = match state.input_field {
                    DidInputField::Ecu => DidInputField::Range,
                    DidInputField::Range => DidInputField::Ecu,
                };
            }
            KeyCode::Enter => {
                state.input_focused = false;
            }
            KeyCode::Backspace => {
                match state.input_field {
                    DidInputField::Ecu => { state.ecu_input.pop(); }
                    DidInputField::Range => { state.range_input.pop(); }
                }
            }
            KeyCode::Char(c) => {
                match state.input_field {
                    DidInputField::Ecu => {
                        if c.is_ascii_hexdigit() {
                            state.ecu_input.push(c.to_ascii_uppercase());
                        }
                    }
                    DidInputField::Range => {
                        if c.is_ascii_hexdigit() || c == '-' {
                            state.range_input.push(c.to_ascii_uppercase());
                        }
                    }
                }
            }
            _ => {}
        }
        return true;
    }

    match key.code {
        KeyCode::Esc => {
            if state.did_scanning {
                // Cancel scan by dropping the progress receiver
                state.progress_rx = None;
                state.pending_did_scan = None;
                state.pending_kwp_scan = None;
                state.did_scanning = false;
                state.status_message = "Scan cancelled".to_string();
            }
            true
        }
        KeyCode::Tab => {
            state.active_pane = match state.active_pane {
                ScanPane::Ecu => ScanPane::Did,
                ScanPane::Did => ScanPane::Ecu,
            };
            true
        }
        KeyCode::Char('e') => {
            if !state.ecu_scanning {
                let (ptx, prx) = mpsc::unbounded_channel();
                if let Some(rx) = elm.try_scan_ecus(ptx) {
                    state.pending_ecu_scan = Some(rx);
                    state.progress_rx = Some(prx);
                    state.ecu_scanning = true;
                    state.status_message = "Scanning ECUs...".to_string();
                }
            }
            true
        }
        KeyCode::Char('s') => {
            if !state.did_scanning {
                match state.scan_mode {
                    ScanMode::Uds => {
                        if let Ok((start, end)) = did_scan::parse_range(&state.range_input) {
                            let (ptx, prx) = mpsc::unbounded_channel();
                            if let Some(rx) = elm.try_scan_did_range(
                                &state.ecu_input,
                                start,
                                end,
                                state.writable_toggle,
                                ptx,
                            ) {
                                state.pending_did_scan = Some(rx);
                                state.progress_rx = Some(prx);
                                state.did_scanning = true;
                                state.status_message = "Scanning UDS DIDs...".to_string();
                            }
                        } else {
                            state.status_message = "Invalid range (e.g. 1A00-1AFF)".to_string();
                        }
                    }
                    ScanMode::Kwp21 | ScanMode::Kwp1A => {
                        let service = match state.scan_mode {
                            ScanMode::Kwp21 => 0x21,
                            ScanMode::Kwp1A => 0x1A,
                            _ => unreachable!(),
                        };
                        if let Ok((start, end)) = parse_kwp_range(&state.range_input) {
                            let (ptx, prx) = mpsc::unbounded_channel();
                            if let Some(rx) = elm.try_scan_kwp(
                                &state.ecu_input,
                                service,
                                start,
                                end,
                                ptx,
                            ) {
                                state.pending_kwp_scan = Some(rx);
                                state.progress_rx = Some(prx);
                                state.did_scanning = true;
                                state.status_message = format!(
                                    "Scanning KWP 0x{:02X} 0x{:02X}-0x{:02X}...",
                                    service, start, end
                                );
                            }
                        } else {
                            state.status_message = "Invalid range (e.g. 00-FF)".to_string();
                        }
                    }
                }
            }
            true
        }
        KeyCode::Enter => {
            // Select ECU from left pane and auto-fill DID scan ECU field
            if state.active_pane == ScanPane::Ecu {
                if let Some(idx) = state.ecu_list_state.selected() {
                    if let Some(ecu) = state.ecu_results.get(idx) {
                        state.ecu_input = ecu.tx_address.clone();
                        state.active_pane = ScanPane::Did;
                        state.status_message = format!("ECU set to 0x{}", ecu.tx_address);
                    }
                }
            }
            true
        }
        KeyCode::Char('i') => {
            state.input_focused = true;
            state.active_pane = ScanPane::Did;
            true
        }
        KeyCode::Char('m') => {
            state.scan_mode = state.scan_mode.next();
            // Update default range when switching modes
            match state.scan_mode {
                ScanMode::Uds => state.range_input = "1A00-1AFF".to_string(),
                ScanMode::Kwp21 | ScanMode::Kwp1A => state.range_input = "00-FF".to_string(),
            }
            state.status_message = format!("Mode: {}", state.scan_mode.label());
            true
        }
        KeyCode::Char('w') => {
            state.writable_toggle = !state.writable_toggle;
            true
        }
        KeyCode::Up => {
            match state.active_pane {
                ScanPane::Ecu => {
                    let i = state.ecu_list_state.selected().unwrap_or(0);
                    if i > 0 {
                        state.ecu_list_state.select(Some(i - 1));
                    }
                }
                ScanPane::Did => {
                    let i = state.did_list_state.selected().unwrap_or(0);
                    if i > 0 {
                        state.did_list_state.select(Some(i - 1));
                    }
                }
            }
            true
        }
        KeyCode::Down => {
            match state.active_pane {
                ScanPane::Ecu => {
                    let i = state.ecu_list_state.selected().unwrap_or(0);
                    if i + 1 < state.ecu_results.len() {
                        state.ecu_list_state.select(Some(i + 1));
                    }
                }
                ScanPane::Did => {
                    let i = state.did_list_state.selected().unwrap_or(0);
                    if i + 1 < state.did_results.len() {
                        state.did_list_state.select(Some(i + 1));
                    }
                }
            }
            true
        }
        _ => false,
    }
}

/// Parse a KWP2000 local identifier range like "00-FF" into (start, end).
fn parse_kwp_range(s: &str) -> std::result::Result<(u8, u8), ()> {
    let parts: Vec<&str> = s.split('-').collect();
    if parts.len() != 2 {
        return Err(());
    }
    let start = u8::from_str_radix(parts[0].trim(), 16).map_err(|_| ())?;
    let end = u8::from_str_radix(parts[1].trim(), 16).map_err(|_| ())?;
    if start > end {
        return Err(());
    }
    Ok((start, end))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scans_state_default() {
        let s = ScansState::default();
        assert_eq!(s.active_pane, ScanPane::Ecu);
        assert!(!s.ecu_scanning);
        assert!(!s.did_scanning);
        assert_eq!(s.ecu_input, "7C0");
        assert!(matches!(s.scan_mode, ScanMode::Kwp21));
        assert_eq!(s.range_input, "00-FF");
    }

    #[test]
    fn test_parse_kwp_range() {
        assert_eq!(parse_kwp_range("00-FF"), Ok((0x00, 0xFF)));
        assert_eq!(parse_kwp_range("80-8F"), Ok((0x80, 0x8F)));
        assert!(parse_kwp_range("FF-00").is_err());
        assert!(parse_kwp_range("GG-FF").is_err());
        assert!(parse_kwp_range("00FF").is_err());
    }

    #[test]
    fn test_scan_mode_cycle() {
        assert!(matches!(ScanMode::Uds.next(), ScanMode::Kwp21));
        assert!(matches!(ScanMode::Kwp21.next(), ScanMode::Kwp1A));
        assert!(matches!(ScanMode::Kwp1A.next(), ScanMode::Uds));
    }

    #[test]
    fn test_scans_is_input_focused() {
        let mut s = ScansState::default();
        assert!(!s.is_input_focused());
        s.input_focused = true;
        assert!(s.is_input_focused());
    }
}
