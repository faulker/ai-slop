use crate::state::App;
use crate::theme;
use crate::widgets::{empty_state, panel};
use hullr_core::{ComposeStatus, ContainerState};
use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::Style,
    text::{Line, Span},
    widgets::{Cell, Paragraph, Row, Table, TableState},
    Frame,
};

/// `2/3 running`
pub fn running_summary(p: &ComposeStatus) -> String {
    let running = p
        .services
        .iter()
        .filter(|s| s.container.as_ref().is_some_and(|c| c.state == ContainerState::Running))
        .count();
    format!("{running}/{} running", p.services.len())
}

pub fn draw(f: &mut Frame, area: Rect, app: &App) {
    let chunks = Layout::horizontal([Constraint::Percentage(45), Constraint::Percentage(55)]).split(area);
    let block = panel("Compose projects");
    let inner = block.inner(chunks[0]);
    f.render_widget(block, chunks[0]);
    let list = app.visible_projects();
    if list.is_empty() {
        let (head, hint) = if let Some(e) = &app.conn_error {
            ("Can't reach the engine".to_string(), e.clone())
        } else if !app.loaded[3] {
            ("Loading projects...".into(), String::new())
        } else {
            (
                "No compose projects running".into(),
                "Press u and enter a folder with a compose.yaml to bring one up".into(),
            )
        };
        empty_state(f, inner, &head, &hint);
    } else {
        let rows: Vec<Row> = list
            .iter()
            .map(|p| Row::new(vec![Cell::from(p.project.clone()), Cell::from(running_summary(p))]))
            .collect();
        let table = Table::new(rows, [Constraint::Min(14), Constraint::Length(16)])
            .header(Row::new(["PROJECT", "SERVICES"]).style(theme::header()))
            .row_highlight_style(theme::selected())
            .highlight_symbol("\u{25b8}");
        let mut st = TableState::default().with_selected(Some(app.sel[3]));
        f.render_stateful_widget(table, inner, &mut st);
    }

    let block = panel("Services");
    let inner = block.inner(chunks[1]);
    f.render_widget(block, chunks[1]);
    match app.selected_project() {
        None => empty_state(f, inner, "Nothing selected", "u up  x down"),
        Some(p) => {
            let lines: Vec<Line> = p
                .services
                .iter()
                .map(|s| match &s.container {
                    Some(c) => Line::from(vec![
                        Span::styled("\u{25cf} ", Style::default().fg(theme::state_color(c))),
                        Span::raw(format!("{:<18}", s.service)),
                        Span::raw(c.status_text(app.now)),
                    ]),
                    None => Line::from(vec![
                        Span::styled("\u{25cb} ", theme::muted()),
                        Span::raw(format!("{:<18}", s.service)),
                        Span::styled("not created", theme::muted()),
                    ]),
                })
                .collect();
            f.render_widget(Paragraph::new(lines), inner);
        }
    }
}
