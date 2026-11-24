use crate::format::*;
use crate::state::App;
use crate::theme;
use crate::widgets::{empty_state, panel};
use ratatui::{
    layout::{Constraint, Layout, Rect},
    text::{Line, Span},
    widgets::{Cell, Paragraph, Row, Table, TableState, Wrap},
    Frame,
};

pub fn draw(f: &mut Frame, area: Rect, app: &App) {
    let chunks = Layout::horizontal([Constraint::Percentage(55), Constraint::Percentage(45)]).split(area);
    let block = panel("Volumes");
    let inner = block.inner(chunks[0]);
    f.render_widget(block, chunks[0]);
    let list = app.visible_volumes();
    if list.is_empty() {
        let (head, hint) = if let Some(e) = &app.conn_error {
            ("Can't reach the engine".to_string(), e.clone())
        } else if !app.loaded[2] {
            ("Loading volumes...".into(), String::new())
        } else if !app.filter.is_empty() {
            (format!("No volumes match '{}'", app.filter), "Press Esc to clear the filter".into())
        } else {
            ("No volumes yet".into(), "Press n to create one. Volumes keep data after a container is removed".into())
        };
        empty_state(f, inner, &head, &hint);
    } else {
        let rows: Vec<Row> = list
            .iter()
            .map(|v| Row::new(vec![Cell::from(truncate(&v.name, 40)), Cell::from(ago(app.now, v.created))]))
            .collect();
        let table = Table::new(rows, [Constraint::Min(16), Constraint::Length(18)])
            .header(Row::new(["NAME", "CREATED"]).style(theme::header()))
            .row_highlight_style(theme::selected())
            .highlight_symbol("\u{25b8}");
        let mut st = TableState::default().with_selected(Some(app.sel[2]));
        f.render_stateful_widget(table, inner, &mut st);
    }

    let block = panel("Details");
    let inner = block.inner(chunks[1]);
    f.render_widget(block, chunks[1]);
    match app.selected_volume() {
        None => empty_state(f, inner, "Nothing selected", "n new  d remove"),
        Some(v) => {
            let l = |k: &str, s: String| Line::from(vec![Span::styled(format!("{k:<9}"), theme::muted()), Span::raw(s)]);
            let mut lines = vec![
                l("Name", v.name.clone()),
                l("Path", v.path.display().to_string()),
                l("Created", fmt_datetime(v.created)),
            ];
            for (k, val) in &v.labels {
                lines.push(l("Label", format!("{k}={val}")));
            }
            f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
        }
    }
}
