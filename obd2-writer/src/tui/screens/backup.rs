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
use crate::toyota::backup::BackupStore;
use crate::tui::elm_actor::{ElmHandle, ProgressUpdate};
use crate::tui::widgets::confirm::{self, ConfirmState};

#[allow(dead_code)]
struct BackupItem {
    key: String,
    did: String,
    ecu: String,
    original_data: String,
    timestamp: String,
}

pub struct BackupState {
    items: Vec<BackupItem>,
    pub list_state: ListState,
    pub loading: bool,
    confirm: ConfirmState,
    confirm_action: ConfirmAction,
    pending_backup: Option<oneshot::Receiver<Result<()>>>,
    pending_restore: Option<oneshot::Receiver<Result<()>>>,
    progress_rx: Option<mpsc::UnboundedReceiver<ProgressUpdate>>,
    progress_text: String,
    status_message: String,
}

#[derive(Debug, Clone, Copy)]
enum ConfirmAction {
    None,
    BackupAll,
    Restore,
}

impl Default for BackupState {
    fn default() -> Self {
        let mut state = Self {
            items: Vec::new(),
            list_state: ListState::default(),
            loading: false,
            confirm: ConfirmState::default(),
            confirm_action: ConfirmAction::None,
            pending_backup: None,
            pending_restore: None,
            progress_rx: None,
            progress_text: String::new(),
            status_message: String::new(),
        };
        state.refresh_from_disk();
        state
    }
}

impl BackupState {
    fn refresh_from_disk(&mut self) {
        self.items.clear();
        if let Ok(store) = BackupStore::load() {
            for (key, entry) in store.list() {
                self.items.push(BackupItem {
                    key: key.clone(),
                    did: entry.did.clone(),
                    ecu: entry.ecu.clone(),
                    original_data: entry.original_data.clone(),
                    timestamp: entry.timestamp.clone(),
                });
            }
        }
        if !self.items.is_empty() && self.list_state.selected().is_none() {
            self.list_state.select(Some(0));
        }
    }

    pub fn tick(&mut self) {
        // Drain progress
        if let Some(ref mut rx) = self.progress_rx {
            while let Ok(update) = rx.try_recv() {
                match update {
                    ProgressUpdate::Step(cur, total, msg) => {
                        self.progress_text = format!("{} ({}/{})", msg, cur, total);
                    }
                    ProgressUpdate::Done(msg) => {
                        self.progress_text = msg;
                    }
                }
            }
        }

        if let Some(mut rx) = self.pending_backup.take() {
            match rx.try_recv() {
                Ok(Ok(())) => {
                    self.status_message = "Backup complete".to_string();
                    self.loading = false;
                    self.refresh_from_disk();
                }
                Ok(Err(e)) => {
                    self.status_message = format!("Backup failed: {}", e);
                    self.loading = false;
                }
                Err(oneshot::error::TryRecvError::Empty) => {
                    self.pending_backup = Some(rx);
                }
                Err(oneshot::error::TryRecvError::Closed) => {
                    self.loading = false;
                }
            }
        }

        if let Some(mut rx) = self.pending_restore.take() {
            match rx.try_recv() {
                Ok(Ok(())) => {
                    self.status_message = "Restore complete".to_string();
                    self.loading = false;
                }
                Ok(Err(e)) => {
                    self.status_message = format!("Restore failed: {}", e);
                    self.loading = false;
                }
                Err(oneshot::error::TryRecvError::Empty) => {
                    self.pending_restore = Some(rx);
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

pub fn render(state: &mut BackupState, f: &mut Frame, area: Rect, _elm: &ElmHandle) {
    let block = Block::default()
        .title(" Backups ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Blue));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let chunks = Layout::vertical([Constraint::Min(3), Constraint::Length(2)]).split(inner);

    if state.loading {
        let msg = if state.progress_text.is_empty() {
            "Working...".to_string()
        } else {
            state.progress_text.clone()
        };
        f.render_widget(
            Paragraph::new(msg).style(Style::default().fg(Color::Yellow)),
            chunks[0],
        );
    } else if state.items.is_empty() {
        f.render_widget(
            Paragraph::new("No backups stored. Press 'a' to backup all.").style(Style::default().fg(Color::DarkGray)),
            chunks[0],
        );
    } else {
        let items: Vec<ListItem> = state
            .items
            .iter()
            .map(|b| {
                let line = Line::from(vec![
                    Span::styled(
                        format!("0x{:<6}", b.did),
                        Style::default().fg(Color::Cyan),
                    ),
                    Span::styled(
                        format!("@{:<4}", b.ecu),
                        Style::default().fg(Color::DarkGray),
                    ),
                    Span::raw("  "),
                    Span::styled(
                        &b.original_data,
                        Style::default().fg(Color::White).bold(),
                    ),
                    Span::raw("  "),
                    Span::styled(
                        &b.timestamp,
                        Style::default().fg(Color::DarkGray),
                    ),
                ]);
                ListItem::new(line)
            })
            .collect();

        let header = Line::from(vec![
            Span::styled(
                format!("{} backup(s) stored", state.items.len()),
                Style::default().fg(Color::Green),
            ),
        ]);
        let list_area = Layout::vertical([Constraint::Length(1), Constraint::Min(1)]).split(chunks[0]);
        f.render_widget(Paragraph::new(header), list_area[0]);

        let list = List::new(items)
            .highlight_style(Style::default().bg(Color::DarkGray).bold())
            .highlight_symbol(">> ");
        f.render_stateful_widget(list, list_area[1], &mut state.list_state);
    }

    let mut help_spans = vec![
        Span::styled("a", Style::default().bold()),
        Span::raw(":backup all "),
        Span::styled("r", Style::default().bold()),
        Span::raw(":restore selected "),
        Span::styled("Up/Down", Style::default().bold()),
        Span::raw(":select"),
    ];
    if !state.status_message.is_empty() {
        help_spans.push(Span::raw("  "));
        help_spans.push(Span::styled(
            &state.status_message,
            Style::default().fg(Color::Green),
        ));
    }
    f.render_widget(Paragraph::new(Line::from(help_spans)), chunks[1]);

    confirm::render_confirm(&state.confirm, f.area(), f.buffer_mut());
}

pub fn handle_key(state: &mut BackupState, key: KeyEvent, elm: &ElmHandle) -> bool {
    if state.confirm.visible {
        match key.code {
            KeyCode::Left | KeyCode::Right | KeyCode::Tab => {
                state.confirm.toggle_selection();
            }
            KeyCode::Enter => {
                if state.confirm.selected_yes {
                    match state.confirm_action {
                        ConfirmAction::BackupAll => {
                            let (ptx, prx) = mpsc::unbounded_channel();
                            if let Some(rx) = elm.try_backup_all(ptx) {
                                state.pending_backup = Some(rx);
                                state.progress_rx = Some(prx);
                                state.loading = true;
                                state.status_message = "Backing up...".to_string();
                            }
                        }
                        ConfirmAction::Restore => {
                            if let Some(idx) = state.list_state.selected() {
                                if idx < state.items.len() {
                                    let item = &state.items[idx];
                                    if let Some(rx) =
                                        elm.try_restore_did(&item.did, &item.ecu)
                                    {
                                        state.pending_restore = Some(rx);
                                        state.loading = true;
                                        state.status_message = "Restoring...".to_string();
                                    }
                                }
                            }
                        }
                        ConfirmAction::None => {}
                    }
                }
                state.confirm.dismiss();
                state.confirm_action = ConfirmAction::None;
            }
            KeyCode::Esc => {
                state.confirm.dismiss();
                state.confirm_action = ConfirmAction::None;
            }
            _ => {}
        }
        return true;
    }

    match key.code {
        KeyCode::Char('a') => {
            state.confirm = ConfirmState::show(
                "Backup All",
                "Read and backup all configured DID values?",
            );
            state.confirm_action = ConfirmAction::BackupAll;
            true
        }
        KeyCode::Char('r') => {
            if state.list_state.selected().is_some() {
                state.confirm = ConfirmState::show(
                    "Restore DID",
                    "Write the backed-up value back to the ECU?",
                );
                state.confirm_action = ConfirmAction::Restore;
            }
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
            if i + 1 < state.items.len() {
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
    fn test_backup_state_default() {
        let s = BackupState::default();
        assert!(!s.loading);
    }

    #[test]
    fn test_backup_is_input_focused() {
        let mut s = BackupState::default();
        assert!(!s.is_input_focused());
        s.confirm = ConfirmState::show("T", "M");
        assert!(s.is_input_focused());
    }
}
