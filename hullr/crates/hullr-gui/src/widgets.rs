//! Small reusable widgets, including the painter-based line plot.

use crate::theme;
use eframe::egui::{self, pos2, vec2, Align2, Color32, FontId, Pos2, Rect, RichText, Sense, Shape, Stroke, Ui};
use std::collections::VecDeque;

/// A filled status dot.
pub fn dot(ui: &mut Ui, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
    ui.painter().circle_filled(rect.center(), 5.0, color);
}

/// Centered friendly message for an empty page. Returns true if the button
/// (when given) was clicked.
pub fn empty_state(ui: &mut Ui, title: &str, hint: &str, button: Option<&str>) -> bool {
    let mut clicked = false;
    ui.vertical_centered(|ui| {
        ui.add_space((ui.available_height() * 0.28).clamp(16.0, 160.0));
        ui.label(RichText::new(title).size(20.0).strong());
        ui.add_space(4.0);
        ui.label(RichText::new(hint).color(theme::muted(ui.visuals().dark_mode)));
        if let Some(b) = button {
            ui.add_space(12.0);
            clicked = ui.add(egui::Button::new(RichText::new(b).strong()).fill(theme::ACCENT.linear_multiply(0.6))).clicked();
        }
    });
    clicked
}

/// A red-ish button for destructive actions.
pub fn danger_button(ui: &mut Ui, text: &str) -> egui::Response {
    let c = theme::danger(ui.visuals().dark_mode);
    ui.add(egui::Button::new(RichText::new(text).color(c)))
}

pub fn muted_label(ui: &mut Ui, text: impl Into<String>) {
    ui.label(RichText::new(text.into()).color(theme::muted(ui.visuals().dark_mode)));
}

/// Pick a plot ceiling: at least `floor`, rounded up to a tidy number.
pub fn nice_max(value_max: f32, floor: f32) -> f32 {
    let v = value_max.max(floor).max(f32::EPSILON);
    let mag = 10f32.powf(v.log10().floor());
    let n = v / mag;
    let step = if n <= 1.0 { 1.0 } else if n <= 2.0 { 2.0 } else if n <= 5.0 { 5.0 } else { 10.0 };
    step * mag
}

/// Map samples to points inside `rect`: oldest on the left, newest on the
/// right edge, `y_max` at the top. A single sample is drawn as a flat line.
pub fn plot_points(values: &[f32], rect: Rect, y_max: f32) -> Vec<Pos2> {
    let n = values.len();
    if n == 0 || y_max <= 0.0 {
        return vec![];
    }
    let x_at = |i: usize| if n == 1 { rect.right() } else { rect.left() + rect.width() * i as f32 / (n - 1) as f32 };
    let pts: Vec<Pos2> = values
        .iter()
        .enumerate()
        .map(|(i, v)| pos2(x_at(i), rect.bottom() - rect.height() * (v / y_max).clamp(0.0, 1.0)))
        .collect();
    if n == 1 {
        return vec![pos2(rect.left(), pts[0].y), pts[0]];
    }
    pts
}

/// A simple line plot drawn with the painter.
pub fn line_plot(ui: &mut Ui, values: &VecDeque<f32>, size: egui::Vec2, color: Color32, y_max: f32, title: &str, current: &str) {
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let p = ui.painter_at(rect);
    let vis = ui.visuals();
    p.rect_filled(rect, egui::CornerRadius::same(4), vis.extreme_bg_color);
    let grid = Stroke::new(1.0_f32, vis.widgets.noninteractive.bg_stroke.color.linear_multiply(0.6));
    for f in [0.25, 0.5, 0.75] {
        let y = rect.bottom() - rect.height() * f;
        p.line_segment([pos2(rect.left(), y), pos2(rect.right(), y)], grid);
    }
    let inner = rect.shrink2(vec2(6.0, 22.0));
    let vals: Vec<f32> = values.iter().copied().collect();
    let pts = plot_points(&vals, inner, y_max);
    if pts.len() >= 2 {
        // Soft fill under the line, then the line itself.
        let mut fill = pts.clone();
        fill.push(pos2(pts[pts.len() - 1].x, inner.bottom()));
        fill.push(pos2(pts[0].x, inner.bottom()));
        p.add(Shape::convex_polygon(fill, color.linear_multiply(0.12), Stroke::NONE));
        p.add(Shape::line(pts, Stroke::new(1.8_f32, color)));
    }
    let text = vis.text_color();
    p.text(rect.left_top() + vec2(8.0, 4.0), Align2::LEFT_TOP, title, FontId::proportional(12.0), theme::muted(vis.dark_mode));
    p.text(rect.right_top() + vec2(-8.0, 4.0), Align2::RIGHT_TOP, current, FontId::monospace(12.0), text);
    p.text(
        rect.left_bottom() + vec2(8.0, -3.0),
        Align2::LEFT_BOTTOM,
        format!("max {}", trim_float(y_max)),
        FontId::monospace(10.0),
        theme::muted(vis.dark_mode),
    );
}

fn trim_float(v: f32) -> String {
    if v >= 1_000_000.0 {
        hullr_core::container::human_bytes(v as u64)
    } else {
        format!("{v:.0}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nice_max_rounds_up() {
        assert_eq!(nice_max(0.0, 100.0), 100.0);
        assert_eq!(nice_max(130.0, 100.0), 200.0);
        assert_eq!(nice_max(420.0, 100.0), 500.0);
        assert_eq!(nice_max(900.0, 100.0), 1000.0);
        assert!(nice_max(0.0, 0.0) > 0.0);
        assert_eq!(nice_max(3.0e8, 0.0), 5.0e8);
    }

    #[test]
    fn points_span_the_rect() {
        let rect = Rect::from_min_max(pos2(0.0, 0.0), pos2(100.0, 50.0));
        let pts = plot_points(&[0.0, 50.0, 100.0], rect, 100.0);
        assert_eq!(pts, vec![pos2(0.0, 50.0), pos2(50.0, 25.0), pos2(100.0, 0.0)]);
        // Values above the ceiling clamp to the top.
        assert_eq!(plot_points(&[0.0, 500.0], rect, 100.0)[1].y, 0.0);
        assert!(plot_points(&[], rect, 100.0).is_empty());
        let single = plot_points(&[100.0], rect, 100.0);
        assert_eq!(single.len(), 2);
        assert_eq!(single[0].y, single[1].y);
    }
}
