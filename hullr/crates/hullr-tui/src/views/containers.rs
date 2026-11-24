use crate::format::*;
use crate::state::{App, Detail, LogEntry};
use crate::theme;
use crate::widgets::{empty_state, panel};
use hullr_core::container::human_bytes;
use hullr_core::{ContainerInfo, ContainerState};
use ratatui::{
    layout::{Constraint, Layout, Rect},
    text::{Line, Span},
    widgets::{Cell, Paragraph, Row, Sparkline, Table, TableState},
    Frame,
};

pub fn draw(f: &mut Frame, area: Rect, app: &App) {
    let wide = area.width >= 110;
    let chunks = if wide {
        Layout::horizontal([Constraint::Percentage(55), Constraint::Percentage(45)]).split(area)
    } else {
        Layout::vertical([Constraint::Percentage(40), Constraint::Percentage(60)]).split(area)
    };
    draw_list(f, chunks[0], app);
    draw_detail(f, chunks[1], app);
}

fn draw_list(f: &mut Frame, area: Rect, app: &App) {
    let scope = if app.show_all { "all" } else { "running" };
    let mut title = format!("Containers ({scope})");
    if !app.filter.is_empty() || app.filter_edit {
        title.push_str(&format!(" / {}", app.filter));
        if app.filter_edit {
            title.push('_');
        }
    }
    let block = panel(&title);
    let inner = block.inner(area);
    f.render_widget(block, area);

    let list = app.visible_containers();
    if list.is_empty() {
        let (head, hint) = if let Some(e) = &app.conn_error {
            ("Can't reach the engine".to_string(), e.clone())
        } else if !app.loaded[0] {
            ("Loading containers...".to_string(), String::new())
        } else if !app.filter.is_empty() {
            (format!("No containers match '{}'", app.filter), "Press Esc to clear the filter".into())
        } else if !app.show_all {
            ("No running containers".into(), "Press a to show stopped ones, or r to run one".into())
        } else {
            ("No containers yet".into(), "Press r to run one, or 2 then p to pull an image first".into())
        };
        empty_state(f, inner, &head, &hint);
        return;
    }

    let rows: Vec<Row> = list
        .iter()
        .map(|c| {
            Row::new(vec![
                Cell::from(Span::styled("\u{25cf}", ratatui::style::Style::default().fg(theme::state_color(c)))),
                Cell::from(truncate(&c.name, 22)),
                Cell::from(truncate(short_ref(&c.image), 24)),
                Cell::from(c.status_text(app.now)),
                Cell::from(ports_summary(&c.spec.ports)),
                Cell::from(Span::styled(c.short_id().to_string(), theme::muted())),
            ])
        })
        .collect();
    let widths = [
        Constraint::Length(1),
        Constraint::Length(22),
        Constraint::Length(24),
        Constraint::Min(14),
        Constraint::Length(14),
        Constraint::Length(12),
    ];
    let header = Row::new(["", "NAME", "IMAGE", "STATUS", "PORTS", "ID"]).style(theme::header());
    let table = Table::new(rows, widths)
        .header(header)
        .column_spacing(1)
        .row_highlight_style(theme::selected())
        .highlight_symbol("\u{25b8}");
    let mut state = TableState::default().with_selected(Some(app.sel[0]));
    f.render_stateful_widget(table, inner, &mut state);
}

fn draw_detail(f: &mut Frame, area: Rect, app: &App) {
    let Some(c) = app.selected_container() else {
        let block = panel("Details");
        let inner = block.inner(area);
        f.render_widget(block, area);
        empty_state(f, inner, "Nothing selected", "Select a container to see its details, stats and logs");
        return;
    };
    let running = c.state == ContainerState::Running;
    let summary_h = 11.min(area.height / 2);
    let stats_h = if running { 6 } else { 0 };
    let chunks = Layout::vertical([
        Constraint::Length(summary_h),
        Constraint::Length(stats_h),
        Constraint::Min(3),
    ])
    .split(area);
    draw_summary(f, chunks[0], app, c);
    if running {
        draw_stats(f, chunks[1], &app.detail);
    }
    draw_logs(f, chunks[2], &app.detail, false);
}

pub fn summary_lines(c: &ContainerInfo, now: i64) -> Vec<Line<'static>> {
    let label = |s: &str| Span::styled(format!("{s:<9}"), theme::muted());
    let ports = ports_summary(&c.spec.ports);
    let mounts = if c.spec.mounts.is_empty() {
        "none".to_string()
    } else {
        c.spec
            .mounts
            .iter()
            .map(|m| format!("{}:{}{}", truncate(&m.source, 24), m.target, if m.read_only { ":ro" } else { "" }))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let native = if c.platform.is_native() { "native" } else { "emulated" };
    let state_span = Span::styled(
        format!("\u{25cf} {}", c.status_text(now)),
        ratatui::style::Style::default().fg(theme::state_color(c)),
    );
    vec![
        Line::from(vec![label("State"), state_span]),
        Line::from(vec![label("Image"), Span::raw(short_ref(&c.image).to_string())]),
        Line::from(vec![label("ID"), Span::raw(c.short_id().to_string())]),
        Line::from(vec![label("Command"), Span::raw(truncate(&c.command.join(" "), 60))]),
        Line::from(vec![label("Created"), Span::raw(format!("{} ({})", fmt_datetime(c.created), ago(now, c.created)))]),
        Line::from(vec![label("Platform"), Span::raw(format!("{} ({native})", c.platform))]),
        Line::from(vec![label("Ports"), Span::raw(if ports.is_empty() { "none".into() } else { ports })]),
        Line::from(vec![label("Mounts"), Span::raw(mounts)]),
        Line::from(vec![label("Env"), Span::raw(format!("{} variable(s)", c.spec.env.len()))]),
    ]
}

fn draw_summary(f: &mut Frame, area: Rect, app: &App, c: &ContainerInfo) {
    let block = panel(&c.name);
    let inner = block.inner(area);
    f.render_widget(block, area);
    f.render_widget(Paragraph::new(summary_lines(c, app.now)), inner);
}

fn draw_stats(f: &mut Frame, area: Rect, d: &Detail) {
    let block = panel("Live stats");
    let inner = block.inner(area);
    f.render_widget(block, area);
    let rows = Layout::vertical([Constraint::Length(1), Constraint::Length(1), Constraint::Length(1), Constraint::Length(1)])
        .split(inner);
    let Some(s) = &d.stats else {
        f.render_widget(Paragraph::new(Span::styled("Waiting for the first sample...", theme::muted())), rows[0]);
        return;
    };
    let cpu_h: Vec<u64> = d.cpu_hist.iter().copied().collect();
    let mem_h: Vec<u64> = d.mem_hist.iter().copied().collect();
    let split = |r: Rect| Layout::horizontal([Constraint::Length(26), Constraint::Min(4)]).split(r);

    let c = split(rows[0]);
    f.render_widget(Paragraph::new(format!("CPU  {:>6.1}%", s.cpu_percent)), c[0]);
    f.render_widget(Sparkline::default().data(&cpu_h).style(theme::ok()), c[1]);

    let m = split(rows[1]);
    let mem = match s.memory_limit {
        Some(l) if l > 0 => format!("MEM  {} / {}", human_bytes(s.memory_bytes), human_bytes(l)),
        _ => format!("MEM  {}", human_bytes(s.memory_bytes)),
    };
    f.render_widget(Paragraph::new(mem), m[0]);
    f.render_widget(Sparkline::default().data(&mem_h).style(theme::warn()), m[1]);

    f.render_widget(Paragraph::new(format!("PIDS {}", s.pids)), rows[2]);
    f.render_widget(
        Paragraph::new(format!("NET  rx {}  tx {}", human_bytes(s.net_rx_bytes), human_bytes(s.net_tx_bytes))),
        rows[3],
    );
}

/// Lines to show for a viewport of `height` rows, honouring the scroll offset.
pub fn log_window(logs: &std::collections::VecDeque<LogEntry>, scroll: usize, height: usize) -> Vec<&LogEntry> {
    let end = logs.len().saturating_sub(scroll);
    let start = end.saturating_sub(height);
    logs.iter().skip(start).take(end - start).collect()
}

pub fn draw_logs(f: &mut Frame, area: Rect, d: &Detail, fullscreen: bool) {
    let follow = if d.log_scroll == 0 { "following" } else { "scrolled, G to follow" };
    let mut title = format!("Logs ({follow})");
    if !fullscreen {
        title.push_str(" l: fullscreen");
    }
    let block = panel(&title);
    let inner = block.inner(area);
    f.render_widget(block, area);
    if d.logs.is_empty() {
        f.render_widget(Paragraph::new(Span::styled("No output yet.", theme::muted())), inner);
        return;
    }
    let lines: Vec<Line> = log_window(&d.logs, d.log_scroll, inner.height as usize)
        .into_iter()
        .map(|e| {
            let text = truncate(&e.text, inner.width as usize);
            if e.stderr {
                Line::from(Span::styled(text, theme::err()))
            } else {
                Line::from(text)
            }
        })
        .collect();
    f.render_widget(Paragraph::new(lines), inner);
}
