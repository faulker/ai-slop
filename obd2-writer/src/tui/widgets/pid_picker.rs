use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::{Constraint, Flex, Layout, Rect},
    style::{Color, Style, Stylize},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Widget},
};

use crate::obd::pid::PIDS;
use crate::toyota::enhanced_pids;

/// An item available for selection in the picker.
#[derive(Debug, Clone)]
pub struct PickerItem {
    pub key: String,
    pub label: String,
    pub unit: String,
    pub is_enhanced: bool,
}

/// State for the PID/DID picker modal.
pub struct PidPickerState {
    pub visible: bool,
    pub items: Vec<PickerItem>,
    pub filtered: Vec<usize>,
    pub filter_text: String,
    pub list_state: ListState,
}

impl Default for PidPickerState {
    fn default() -> Self {
        Self {
            visible: false,
            items: Vec::new(),
            filtered: Vec::new(),
            filter_text: String::new(),
            list_state: ListState::default(),
        }
    }
}

impl PidPickerState {
    pub fn open() -> Self {
        let mut items = Vec::new();

        for pid in PIDS {
            items.push(PickerItem {
                key: pid.name.to_string(),
                label: pid.name.to_string(),
                unit: pid.unit.to_string(),
                is_enhanced: false,
            });
        }

        for did in enhanced_pids::cached_dids() {
            items.push(PickerItem {
                key: format!("did:{:04X}:{}", did.id, did.ecu),
                label: format!("{} (0x{:04X})", did.name, did.id),
                unit: did.unit.clone(),
                is_enhanced: true,
            });
        }

        let filtered: Vec<usize> = (0..items.len()).collect();
        let mut state = Self {
            visible: true,
            items,
            filtered,
            filter_text: String::new(),
            list_state: ListState::default(),
        };
        if !state.filtered.is_empty() {
            state.list_state.select(Some(0));
        }
        state
    }

    pub fn apply_filter(&mut self) {
        let lower = self.filter_text.to_lowercase();
        self.filtered = self
            .items
            .iter()
            .enumerate()
            .filter(|(_, item)| {
                lower.is_empty()
                    || item.label.to_lowercase().contains(&lower)
                    || item.key.to_lowercase().contains(&lower)
                    || item.unit.to_lowercase().contains(&lower)
            })
            .map(|(i, _)| i)
            .collect();

        if self.filtered.is_empty() {
            self.list_state.select(None);
        } else {
            self.list_state.select(Some(0));
        }
    }

    pub fn move_up(&mut self) {
        if self.filtered.is_empty() {
            return;
        }
        let i = self.list_state.selected().unwrap_or(0);
        let next = if i == 0 { self.filtered.len() - 1 } else { i - 1 };
        self.list_state.select(Some(next));
    }

    pub fn move_down(&mut self) {
        if self.filtered.is_empty() {
            return;
        }
        let i = self.list_state.selected().unwrap_or(0);
        let next = if i >= self.filtered.len() - 1 { 0 } else { i + 1 };
        self.list_state.select(Some(next));
    }

    /// Returns the selected item's key, or None.
    pub fn confirm_selection(&self) -> Option<String> {
        let sel = self.list_state.selected()?;
        let idx = *self.filtered.get(sel)?;
        Some(self.items[idx].key.clone())
    }

    /// Handle a key event. Returns Some(key) if an item was selected, None otherwise.
    /// Returns Err(()) if the picker should close without selection.
    pub fn handle_key(&mut self, key: KeyEvent) -> std::result::Result<Option<String>, ()> {
        match key.code {
            KeyCode::Esc => Err(()),
            KeyCode::Up => {
                self.move_up();
                Ok(None)
            }
            KeyCode::Down => {
                self.move_down();
                Ok(None)
            }
            KeyCode::Enter => Ok(self.confirm_selection()),
            KeyCode::Backspace => {
                self.filter_text.pop();
                self.apply_filter();
                Ok(None)
            }
            KeyCode::Char(c) => {
                self.filter_text.push(c);
                self.apply_filter();
                Ok(None)
            }
            _ => Ok(None),
        }
    }
}

/// Render the PID picker as a centered overlay.
pub fn render_picker(state: &mut PidPickerState, area: Rect, buf: &mut ratatui::buffer::Buffer) {
    if !state.visible {
        return;
    }

    let picker_w = 60.min(area.width.saturating_sub(4));
    let picker_h = 20.min(area.height.saturating_sub(4));

    let [picker_area] = Layout::horizontal([Constraint::Length(picker_w)])
        .flex(Flex::Center)
        .areas(
            Layout::vertical([Constraint::Length(picker_h)])
                .flex(Flex::Center)
                .areas::<1>(area)[0],
        );

    Clear.render(picker_area, buf);

    let block = Block::default()
        .title(" Select PID/DID (type to filter) ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .style(Style::default().bg(Color::Black));

    let inner = block.inner(picker_area);
    block.render(picker_area, buf);

    if inner.height < 3 {
        return;
    }

    // Filter input at the top
    let filter_area = Rect::new(inner.x, inner.y, inner.width, 1);
    let filter_display = format!("> {}_", state.filter_text);
    Paragraph::new(filter_display)
        .style(Style::default().fg(Color::Yellow))
        .render(filter_area, buf);

    // List area
    let list_area = Rect::new(inner.x, inner.y + 1, inner.width, inner.height.saturating_sub(1));

    let list_items: Vec<ListItem> = state
        .filtered
        .iter()
        .map(|&idx| {
            let item = &state.items[idx];
            let tag = if item.is_enhanced { "DID" } else { "PID" };
            let line = Line::from(vec![
                Span::styled(format!("[{}] ", tag), Style::default().fg(Color::DarkGray)),
                Span::raw(&item.label),
                Span::styled(format!(" ({})", item.unit), Style::default().fg(Color::DarkGray)),
            ]);
            ListItem::new(line)
        })
        .collect();

    let list = List::new(list_items)
        .highlight_style(Style::default().bg(Color::DarkGray).fg(Color::White).bold())
        .highlight_symbol(">> ");

    ratatui::widgets::StatefulWidget::render(list, list_area, buf, &mut state.list_state);
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::buffer::Buffer;

    #[test]
    fn test_picker_open_has_pids() {
        let state = PidPickerState::open();
        assert!(state.visible);
        // Should have at least the standard PIDs
        assert!(state.items.len() >= PIDS.len());
        assert_eq!(state.filtered.len(), state.items.len());
    }

    #[test]
    fn test_picker_filter() {
        let mut state = PidPickerState::open();
        state.filter_text = "rpm".to_string();
        state.apply_filter();
        assert!(state.filtered.len() < state.items.len());
        assert!(!state.filtered.is_empty());
    }

    #[test]
    fn test_picker_navigation() {
        let mut state = PidPickerState::open();
        state.move_down();
        assert_eq!(state.list_state.selected(), Some(1));
        state.move_up();
        assert_eq!(state.list_state.selected(), Some(0));
        // Wrap around
        state.move_up();
        assert_eq!(state.list_state.selected(), Some(state.filtered.len() - 1));
    }

    #[test]
    fn test_picker_render_hidden() {
        let mut state = PidPickerState::default();
        let area = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        render_picker(&mut state, area, &mut buf);
    }
}
