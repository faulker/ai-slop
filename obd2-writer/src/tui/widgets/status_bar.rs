use ratatui::{
    layout::Rect,
    style::{Color, Style, Stylize},
    text::{Line, Span},
    widgets::{Paragraph, Widget},
};

/// Status bar showing connection info and session state.
pub struct StatusBar<'a> {
    pub connection_info: &'a str,
    pub session: &'a str,
    pub extra: &'a str,
}

#[allow(dead_code)]
impl<'a> StatusBar<'a> {
    pub fn new(connection_info: &'a str) -> Self {
        Self {
            connection_info,
            session: "Default",
            extra: "",
        }
    }

    pub fn session(mut self, s: &'a str) -> Self {
        self.session = s;
        self
    }

    pub fn extra(mut self, s: &'a str) -> Self {
        self.extra = s;
        self
    }
}

impl Widget for StatusBar<'_> {
    fn render(self, area: Rect, buf: &mut ratatui::buffer::Buffer) {
        let mut spans = vec![
            Span::styled(" Connected: ", Style::default().fg(Color::Green).bold()),
            Span::raw(self.connection_info),
            Span::raw("  "),
            Span::styled("Session: ", Style::default().fg(Color::Yellow).bold()),
            Span::raw(self.session),
        ];
        if !self.extra.is_empty() {
            spans.push(Span::raw("  "));
            spans.push(Span::styled(self.extra, Style::default().fg(Color::Cyan)));
        }
        let line = Line::from(spans);
        let p = Paragraph::new(line).style(Style::default().bg(Color::DarkGray));
        p.render(area, buf);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::buffer::Buffer;

    #[test]
    fn test_status_bar_renders() {
        let bar = StatusBar::new("ELM327 v1.5").session("Extended").extra("Polling");
        let area = Rect::new(0, 0, 80, 1);
        let mut buf = Buffer::empty(area);
        bar.render(area, &mut buf);
    }
}
