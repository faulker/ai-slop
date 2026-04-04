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
use crate::tui::elm_actor::{DtcEntry, ElmHandle};
use crate::tui::widgets::confirm::{self, ConfirmState};

pub struct DtcState {
    pub dtcs: Vec<DtcEntry>,
    pub loading: bool,
    pub list_state: ListState,
    confirm: ConfirmState,
    pending_fetch: Option<oneshot::Receiver<Result<Vec<DtcEntry>>>>,
    pending_clear: Option<oneshot::Receiver<Result<()>>>,
    status_message: String,
}

impl Default for DtcState {
    fn default() -> Self {
        Self {
            dtcs: Vec::new(),
            loading: false,
            list_state: ListState::default(),
            confirm: ConfirmState::default(),
            pending_fetch: None,
            pending_clear: None,
            status_message: "Press 'r' to fetch DTCs".to_string(),
        }
    }
}

impl DtcState {
    pub fn tick(&mut self) {
        if let Some(mut rx) = self.pending_fetch.take() {
            match rx.try_recv() {
                Ok(Ok(dtcs)) => {
                    self.status_message = if dtcs.is_empty() {
                        "No DTCs stored".to_string()
                    } else {
                        format!("{} DTC(s) found", dtcs.len())
                    };
                    self.dtcs = dtcs;
                    self.loading = false;
                    if !self.dtcs.is_empty() {
                        self.list_state.select(Some(0));
                    }
                }
                Ok(Err(e)) => {
                    self.status_message = format!("Error: {}", e);
                    self.loading = false;
                }
                Err(oneshot::error::TryRecvError::Empty) => {
                    self.pending_fetch = Some(rx);
                }
                Err(oneshot::error::TryRecvError::Closed) => {
                    self.loading = false;
                    self.status_message = "Connection lost".to_string();
                }
            }
        }

        if let Some(mut rx) = self.pending_clear.take() {
            match rx.try_recv() {
                Ok(Ok(())) => {
                    self.dtcs.clear();
                    self.list_state.select(None);
                    self.status_message = "DTCs cleared successfully".to_string();
                    self.loading = false;
                }
                Ok(Err(e)) => {
                    self.status_message = format!("Clear failed: {}", e);
                    self.loading = false;
                }
                Err(oneshot::error::TryRecvError::Empty) => {
                    self.pending_clear = Some(rx);
                }
                Err(oneshot::error::TryRecvError::Closed) => {
                    self.loading = false;
                }
            }
        }
    }

    pub fn is_input_focused(&self) -> bool {
        self.confirm.visible
    }
}

pub fn render(state: &mut DtcState, f: &mut Frame, area: Rect, _elm: &ElmHandle) {
    let block = Block::default()
        .title(" Diagnostic Trouble Codes ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Red));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let chunks = Layout::vertical([Constraint::Min(3), Constraint::Length(2)]).split(inner);

    if state.loading {
        let spinner = Paragraph::new("Loading...")
            .style(Style::default().fg(Color::Yellow));
        f.render_widget(spinner, chunks[0]);
    } else if state.dtcs.is_empty() {
        let msg = Paragraph::new(state.status_message.as_str())
            .style(Style::default().fg(Color::DarkGray));
        f.render_widget(msg, chunks[0]);
    } else {
        let items: Vec<ListItem> = state
            .dtcs
            .iter()
            .map(|dtc| {
                let color = match dtc.code.chars().next() {
                    Some('P') => Color::Red,
                    Some('C') => Color::Yellow,
                    Some('B') => Color::Magenta,
                    Some('U') => Color::Blue,
                    _ => Color::White,
                };
                let prefix = match dtc.code.chars().next() {
                    Some('P') => "Powertrain",
                    Some('C') => "Chassis",
                    Some('B') => "Body",
                    Some('U') => "Network",
                    _ => "Unknown",
                };
                let line = Line::from(vec![
                    Span::styled(&dtc.code, Style::default().fg(color).bold()),
                    Span::styled(format!("  ({})", prefix), Style::default().fg(Color::DarkGray)),
                ]);
                ListItem::new(line)
            })
            .collect();

        let list = List::new(items)
            .highlight_style(Style::default().bg(Color::DarkGray).bold())
            .highlight_symbol(">> ");
        f.render_stateful_widget(list, chunks[0], &mut state.list_state);
    }

    let help = Line::from(vec![
        Span::styled("r", Style::default().bold()),
        Span::raw(":refresh "),
        Span::styled("c", Style::default().bold()),
        Span::raw(":clear "),
        Span::styled("Up/Down", Style::default().bold()),
        Span::raw(":select"),
        Span::raw("  "),
        Span::styled(&state.status_message, Style::default().fg(Color::DarkGray)),
    ]);
    f.render_widget(Paragraph::new(help), chunks[1]);

    confirm::render_confirm(&state.confirm, f.area(), f.buffer_mut());
}

pub fn handle_key(state: &mut DtcState, key: KeyEvent, elm: &ElmHandle) -> bool {
    if state.confirm.visible {
        match key.code {
            KeyCode::Left | KeyCode::Right | KeyCode::Tab => {
                state.confirm.toggle_selection();
            }
            KeyCode::Enter => {
                if state.confirm.selected_yes {
                    if let Some(rx) = elm.try_clear_dtcs() {
                        state.pending_clear = Some(rx);
                        state.loading = true;
                        state.status_message = "Clearing DTCs...".to_string();
                    }
                }
                state.confirm.dismiss();
            }
            KeyCode::Esc => {
                state.confirm.dismiss();
            }
            _ => {}
        }
        return true;
    }

    match key.code {
        KeyCode::Char('r') => {
            if !state.loading {
                if let Some(rx) = elm.try_fetch_dtcs() {
                    state.pending_fetch = Some(rx);
                    state.loading = true;
                    state.status_message = "Fetching DTCs...".to_string();
                }
            }
            true
        }
        KeyCode::Char('c') => {
            state.confirm = ConfirmState::show(
                "Clear DTCs",
                "Clear ALL stored diagnostic trouble codes?",
            );
            true
        }
        KeyCode::Up => {
            let i = state.list_state.selected().unwrap_or(0);
            if i > 0 {
                state.list_state.select(Some(i - 1));
            }
            true
        }
        KeyCode::Down => {
            let i = state.list_state.selected().unwrap_or(0);
            if i + 1 < state.dtcs.len() {
                state.list_state.select(Some(i + 1));
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
    fn test_dtc_state_default() {
        let s = DtcState::default();
        assert!(s.dtcs.is_empty());
        assert!(!s.loading);
    }

    #[test]
    fn test_dtc_is_input_focused() {
        let mut s = DtcState::default();
        assert!(!s.is_input_focused());
        s.confirm = ConfirmState::show("T", "M");
        assert!(s.is_input_focused());
    }
}
