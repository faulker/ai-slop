use crate::format::*;
use crate::state::App;
use crate::theme;
use crate::widgets::{empty_state, panel};
use hullr_core::container::human_bytes;
use ratatui::{
    layout::{Constraint, Layout, Rect},
    text::{Line, Span},
    widgets::{Cell, Paragraph, Row, Table, TableState, Wrap},
    Frame,
};

pub fn draw(f: &mut Frame, area: Rect, app: &App) {
    let chunks = Layout::horizontal([Constraint::Percentage(60), Constraint::Percentage(40)]).split(area);
    let mut title = "Images".to_string();
    if !app.filter.is_empty() {
        title.push_str(&format!(" / {}", app.filter));
    }
    let block = panel(&title);
    let inner = block.inner(chunks[0]);
    f.render_widget(block, chunks[0]);

    let list = app.visible_images();
    if list.is_empty() {
        let (head, hint) = if let Some(e) = &app.conn_error {
            ("Can't reach the engine".to_string(), e.clone())
        } else if !app.loaded[1] {
            ("Loading images...".into(), String::new())
        } else if !app.filter.is_empty() {
            (format!("No images match '{}'", app.filter), "Press Esc to clear the filter".into())
        } else {
            ("No images yet".into(), "Press p to pull one, for example nginx:latest".into())
        };
        empty_state(f, inner, &head, &hint);
    } else {
        let rows: Vec<Row> = list
            .iter()
            .map(|i| {
                let tag = i.repo_tags.first().map(|t| short_ref(t).to_string()).unwrap_or_else(|| "<untagged>".into());
                Row::new(vec![
                    Cell::from(truncate(&tag, 38)),
                    Cell::from(human_bytes(i.size)),
                    Cell::from(i.platform.to_string()),
                    Cell::from(ago(app.now, i.created)),
                    Cell::from(Span::styled(i.short_id().to_string(), theme::muted())),
                ])
            })
            .collect();
        let widths = [
            Constraint::Min(20),
            Constraint::Length(9),
            Constraint::Length(13),
            Constraint::Length(16),
            Constraint::Length(12),
        ];
        let table = Table::new(rows, widths)
            .header(Row::new(["REPOSITORY:TAG", "SIZE", "PLATFORM", "CREATED", "ID"]).style(theme::header()))
            .row_highlight_style(theme::selected())
            .highlight_symbol("\u{25b8}");
        let mut st = TableState::default().with_selected(Some(app.sel[1]));
        f.render_stateful_widget(table, inner, &mut st);
    }

    let block = panel("Details");
    let inner = block.inner(chunks[1]);
    f.render_widget(block, chunks[1]);
    match app.selected_image() {
        None => empty_state(f, inner, "Nothing selected", "p pull  r run  d remove"),
        Some(i) => {
            let l = |k: &str, v: String| Line::from(vec![Span::styled(format!("{k:<9}"), theme::muted()), Span::raw(v)]);
            let mut lines = vec![
                l("Tags", i.repo_tags.iter().map(|t| short_ref(t)).collect::<Vec<_>>().join(", ")),
                l("ID", i.short_id().to_string()),
                l("Size", human_bytes(i.size)),
                l("Created", fmt_datetime(i.created)),
                l("Platform", i.platform.to_string()),
            ];
            if let Some(e) = &i.config.entrypoint {
                lines.push(l("Entry", e.join(" ")));
            }
            if let Some(c) = &i.config.cmd {
                lines.push(l("Cmd", c.join(" ")));
            }
            if !i.config.exposed_ports.is_empty() {
                lines.push(l("Ports", i.config.exposed_ports.join(", ")));
            }
            if let Some(w) = &i.config.working_dir {
                lines.push(l("Workdir", w.clone()));
            }
            lines.push(l("Env", format!("{} variable(s)", i.config.env.len())));
            f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
        }
    }
}
