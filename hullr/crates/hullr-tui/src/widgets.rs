//! Small reusable drawing helpers.

use crate::theme;
use ratatui::{
    layout::{Constraint, Flex, Layout, Rect},
    text::{Line, Span},
    widgets::{Block, BorderType, Paragraph, Wrap},
    Frame,
};

/// A rectangle centered in `area`, `w` x `h` (clamped to fit).
pub fn centered(area: Rect, w: u16, h: u16) -> Rect {
    let w = w.min(area.width);
    let h = h.min(area.height);
    let [v] = Layout::vertical([Constraint::Length(h)]).flex(Flex::Center).areas(area);
    let [r] = Layout::horizontal([Constraint::Length(w)]).flex(Flex::Center).areas(v);
    r
}

pub fn panel(title: &str) -> Block<'static> {
    Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(theme::muted())
        .title(Span::styled(format!(" {title} "), theme::title()))
}

/// A friendly message for an empty list: a headline plus a hint line.
pub fn empty_state(f: &mut Frame, area: Rect, headline: &str, hint: &str) {
    let lines = vec![
        Line::from(""),
        Line::from(Span::styled(headline.to_string(), theme::title())).centered(),
        Line::from(""),
        Line::from(Span::styled(hint.to_string(), theme::muted())).centered(),
    ];
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), area);
}

/// `key label` pairs rendered as a line of hints.
pub fn hints(pairs: &[(&str, &str)]) -> Line<'static> {
    let mut spans = Vec::new();
    for (i, (k, l)) in pairs.iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw("  "));
        }
        spans.push(Span::styled((*k).to_string(), theme::key()));
        spans.push(Span::styled(format!(" {l}"), theme::muted()));
    }
    Line::from(spans)
}
