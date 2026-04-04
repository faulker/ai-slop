use ratatui::{
    layout::Rect,
    style::{Color, Style, Stylize},
    text::Span,
    widgets::Widget,
    buffer::Buffer,
};

/// A labeled horizontal bar gauge showing a value within a range.
pub struct LabeledGauge<'a> {
    pub label: &'a str,
    pub value: f64,
    pub unit: &'a str,
    pub min: f64,
    pub max: f64,
    pub bar_color: Color,
}

impl<'a> LabeledGauge<'a> {
    pub fn new(label: &'a str, value: f64, unit: &'a str) -> Self {
        Self {
            label,
            value,
            unit,
            min: 0.0,
            max: 100.0,
            bar_color: Color::Cyan,
        }
    }

    pub fn range(mut self, min: f64, max: f64) -> Self {
        self.min = min;
        self.max = max;
        self
    }

    pub fn color(mut self, c: Color) -> Self {
        self.bar_color = c;
        self
    }
}

impl Widget for LabeledGauge<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.height < 1 || area.width < 10 {
            return;
        }

        // Layout: "label_______  [====        ] value unit"
        let label_width = 18.min(area.width as usize / 3);
        let value_text = format!("{:.1} {}", self.value, self.unit);
        let value_width = value_text.len() + 1;
        let bar_width = area.width as usize - label_width - value_width - 4;

        if bar_width < 3 {
            // Not enough room for the bar, just show label + value
            let text = format!("{:width$} {}", self.label, value_text, width = label_width);
            let span = Span::raw(text);
            buf.set_span(area.x, area.y, &span, area.width);
            return;
        }

        // Label (safe for non-ASCII: truncate at char boundary)
        let label_str = if self.label.len() > label_width {
            match self.label.char_indices().nth(label_width) {
                Some((byte_idx, _)) => &self.label[..byte_idx],
                None => self.label,
            }
        } else {
            self.label
        };
        buf.set_span(
            area.x,
            area.y,
            &Span::styled(
                format!("{:width$}", label_str, width = label_width),
                Style::default().bold(),
            ),
            label_width as u16,
        );

        // Bar brackets
        let bar_x = area.x + label_width as u16 + 1;
        buf.set_string(bar_x, area.y, "[", Style::default().dark_gray());

        let ratio = if self.max > self.min {
            ((self.value - self.min) / (self.max - self.min)).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let filled = (ratio * bar_width as f64) as usize;

        for i in 0..bar_width {
            let ch = if i < filled { '=' } else { ' ' };
            let style = if i < filled {
                Style::default().fg(self.bar_color)
            } else {
                Style::default().dark_gray()
            };
            buf.set_string(bar_x + 1 + i as u16, area.y, &ch.to_string(), style);
        }

        buf.set_string(
            bar_x + 1 + bar_width as u16,
            area.y,
            "]",
            Style::default().dark_gray(),
        );

        // Value
        let val_x = bar_x + 2 + bar_width as u16 + 1;
        buf.set_span(
            val_x,
            area.y,
            &Span::styled(value_text, Style::default().fg(Color::White)),
            area.width.saturating_sub(val_x - area.x),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_labeled_gauge_creation() {
        let g = LabeledGauge::new("RPM", 3500.0, "RPM")
            .range(0.0, 8000.0)
            .color(Color::Green);
        assert_eq!(g.label, "RPM");
        assert_eq!(g.value, 3500.0);
        assert_eq!(g.min, 0.0);
        assert_eq!(g.max, 8000.0);
    }

    #[test]
    fn test_gauge_renders_without_panic() {
        let g = LabeledGauge::new("test", 50.0, "%");
        let area = Rect::new(0, 0, 60, 1);
        let mut buf = Buffer::empty(area);
        g.render(area, &mut buf);
    }

    #[test]
    fn test_gauge_handles_tiny_area() {
        let g = LabeledGauge::new("test", 50.0, "%");
        let area = Rect::new(0, 0, 5, 1);
        let mut buf = Buffer::empty(area);
        g.render(area, &mut buf);
    }
}
