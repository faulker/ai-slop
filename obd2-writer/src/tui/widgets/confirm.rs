use ratatui::{
    layout::{Constraint, Flex, Layout, Rect},
    style::{Color, Style, Stylize},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Widget},
};

/// Modal confirmation dialog state.
#[derive(Debug, Clone)]
pub struct ConfirmState {
    pub visible: bool,
    pub title: String,
    pub message: String,
    pub selected_yes: bool,
}

impl Default for ConfirmState {
    fn default() -> Self {
        Self {
            visible: false,
            title: String::new(),
            message: String::new(),
            selected_yes: false,
        }
    }
}

impl ConfirmState {
    pub fn show(title: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            visible: true,
            title: title.into(),
            message: message.into(),
            selected_yes: false,
        }
    }

    pub fn toggle_selection(&mut self) {
        self.selected_yes = !self.selected_yes;
    }

    pub fn dismiss(&mut self) {
        self.visible = false;
    }
}

/// Render a modal confirmation dialog centered in the given area.
pub fn render_confirm(state: &ConfirmState, area: Rect, buf: &mut ratatui::buffer::Buffer) {
    if !state.visible {
        return;
    }

    let dialog_width = 50.min(area.width.saturating_sub(4));
    let dialog_height = 7.min(area.height.saturating_sub(2));

    let [dialog_area] = Layout::horizontal([Constraint::Length(dialog_width)])
        .flex(Flex::Center)
        .areas(
            Layout::vertical([Constraint::Length(dialog_height)])
                .flex(Flex::Center)
                .areas::<1>(area)[0],
        );

    Clear.render(dialog_area, buf);

    let block = Block::default()
        .title(format!(" {} ", state.title))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow))
        .style(Style::default().bg(Color::Black));

    let inner = block.inner(dialog_area);
    block.render(dialog_area, buf);

    // Message
    if inner.height >= 3 {
        let msg = Paragraph::new(state.message.as_str())
            .style(Style::default().fg(Color::White));
        let msg_area = Rect::new(inner.x + 1, inner.y + 1, inner.width.saturating_sub(2), 2);
        msg.render(msg_area, buf);
    }

    // Buttons
    if inner.height >= 1 {
        let button_y = inner.y + inner.height.saturating_sub(1);
        let yes_style = if state.selected_yes {
            Style::default().fg(Color::Black).bg(Color::Green).bold()
        } else {
            Style::default().fg(Color::Green)
        };
        let no_style = if !state.selected_yes {
            Style::default().fg(Color::Black).bg(Color::Red).bold()
        } else {
            Style::default().fg(Color::Red)
        };

        let buttons = Line::from(vec![
            Span::raw("      "),
            Span::styled(" Yes ", yes_style),
            Span::raw("    "),
            Span::styled(" No ", no_style),
        ]);
        let btn_area = Rect::new(inner.x, button_y, inner.width, 1);
        Paragraph::new(buttons).render(btn_area, buf);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::buffer::Buffer;

    #[test]
    fn test_confirm_state_default_hidden() {
        let s = ConfirmState::default();
        assert!(!s.visible);
    }

    #[test]
    fn test_confirm_state_show() {
        let s = ConfirmState::show("Title", "Are you sure?");
        assert!(s.visible);
        assert!(!s.selected_yes);
    }

    #[test]
    fn test_confirm_toggle() {
        let mut s = ConfirmState::show("T", "M");
        assert!(!s.selected_yes);
        s.toggle_selection();
        assert!(s.selected_yes);
        s.toggle_selection();
        assert!(!s.selected_yes);
    }

    #[test]
    fn test_confirm_render_hidden() {
        let s = ConfirmState::default();
        let area = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        render_confirm(&s, area, &mut buf);
    }

    #[test]
    fn test_confirm_render_visible() {
        let s = ConfirmState::show("Confirm", "Clear all DTCs?");
        let area = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        render_confirm(&s, area, &mut buf);
    }
}
