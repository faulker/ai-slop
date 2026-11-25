use crate::format::{bar, truncate};
use crate::state::{App, Level, Modal, ProgressView};
use crate::theme;
use crate::widgets::{centered, hints, panel};
use hullr_core::container::human_bytes;
use ratatui::{
    layout::Rect,
    text::{Line, Span},
    widgets::{Clear, Paragraph, Wrap},
    Frame,
};

/// Every key binding, grouped. Also the source for the help overlay test.
pub const HELP: &[(&str, &[(&str, &str)])] = &[
    (
        "Everywhere",
        &[
            ("1-5, Tab, arrows", "switch tab (Containers, Images, Volumes, Compose, System)"),
            ("j / k, Up / Down", "move selection"),
            ("g / G, Home / End", "jump to first / last"),
            ("/", "filter the list (Enter keeps it, Esc clears it)"),
            ("F5, ctrl-r", "refresh now (it also refreshes every 2s)"),
            ("P", "prune stopped containers and unused images"),
            ("A", "deep prune: all unused images and volumes"),
            ("?", "this help"),
            ("q, ctrl-c", "quit"),
        ],
    ),
    (
        "Containers",
        &[
            ("r", "run a new container"),
            ("s / x / R", "start / stop / restart"),
            ("d, Delete", "remove (asks first)"),
            ("e", "open a shell inside (back here when you exit)"),
            ("l, Enter", "fullscreen logs"),
            ("PgUp / PgDn", "scroll logs (ctrl-u / ctrl-d too)"),
            ("a", "toggle all / running only"),
        ],
    ),
    (
        "Images",
        &[("p", "pull an image"), ("r, Enter", "run the selected image"), ("d, Delete", "remove (asks first)")],
    ),
    ("Volumes", &[("n", "new volume"), ("d, Delete", "remove (asks first)")]),
    ("Compose", &[("u", "up: bring up a folder with compose.yaml"), ("x, d", "down the selected project (asks first)")]),
    (
        "Fullscreen logs",
        &[("j / k", "scroll"), ("g / G", "oldest / follow"), ("Esc, q", "back")],
    ),
];

pub fn draw(f: &mut Frame, area: Rect, app: &App) {
    match &app.modal {
        Modal::None | Modal::Logs => {}
        Modal::Help => draw_help(f, area),
        Modal::Confirm { prompt, .. } => draw_confirm(f, area, prompt),
        Modal::Input { title, hint, buf, .. } => draw_input(f, area, title, hint, buf),
        Modal::Progress(v) => draw_progress(f, area, v),
    }
}

fn draw_help(f: &mut Frame, area: Rect) {
    let mut lines: Vec<Line> = Vec::new();
    for (group, keys) in HELP {
        lines.push(Line::from(Span::styled(*group, theme::title())));
        for (k, d) in *keys {
            lines.push(Line::from(vec![Span::styled(format!("  {k:<20}"), theme::key()), Span::raw(*d)]));
        }
        lines.push(Line::from(""));
    }
    lines.push(Line::from(Span::styled("Press any key to close", theme::muted())));
    let rect = centered(area, 82, lines.len() as u16 + 2);
    f.render_widget(Clear, rect);
    let block = panel("Help");
    let inner = block.inner(rect);
    f.render_widget(block, rect);
    f.render_widget(Paragraph::new(lines), inner);
}

fn draw_confirm(f: &mut Frame, area: Rect, prompt: &str) {
    let rect = centered(area, 64.min(area.width), 7);
    f.render_widget(Clear, rect);
    let block = panel("Are you sure?").border_style(theme::warn());
    let inner = block.inner(rect);
    f.render_widget(block, rect);
    let lines = vec![
        Line::from(prompt.to_string()),
        Line::from(""),
        hints(&[("y", "yes, do it"), ("n / Esc", "cancel")]),
    ];
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), inner);
}

fn draw_input(f: &mut Frame, area: Rect, title: &str, hint: &str, buf: &str) {
    let rect = centered(area, 78.min(area.width), 7);
    f.render_widget(Clear, rect);
    let block = panel(title);
    let inner = block.inner(rect);
    f.render_widget(block, rect);
    let width = inner.width.saturating_sub(3) as usize;
    let shown: String = {
        let n = buf.chars().count();
        buf.chars().skip(n.saturating_sub(width)).collect()
    };
    let lines = vec![
        Line::from(Span::styled(hint.to_string(), theme::muted())),
        Line::from(""),
        Line::from(vec![Span::styled("> ", theme::key()), Span::raw(shown), Span::styled("_", theme::key())]),
        Line::from(""),
        hints(&[("Enter", "go"), ("Esc", "cancel"), ("ctrl-u", "clear")]),
    ];
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), inner);
}

fn draw_progress(f: &mut Frame, area: Rect, v: &ProgressView) {
    let h = (area.height.saturating_sub(4)).clamp(8, 22);
    let rect = centered(area, 84.min(area.width), h);
    f.render_widget(Clear, rect);
    let border = match &v.done {
        Some(Err(_)) => theme::err(),
        Some(Ok(_)) => theme::ok(),
        None => theme::muted(),
    };
    let block = panel(&v.title).border_style(border);
    let inner = block.inner(rect);
    f.render_widget(block, rect);

    let mut lines: Vec<Line> = Vec::new();
    if let Some((cur, total)) = v.overall() {
        let pct = cur * 100 / total.max(1);
        lines.push(Line::from(vec![
            Span::styled(bar(cur, total, 30), theme::ok()),
            Span::raw(format!(" {pct}%  {} / {}", human_bytes(cur), human_bytes(total))),
        ]));
        lines.push(Line::from(""));
    }
    let footer = 2;
    let budget = (inner.height as usize).saturating_sub(lines.len() + footer);
    let layer_rows = v.layers.len().min(budget.min(8));
    let skip = v.layers.len().saturating_sub(layer_rows);
    for l in v.layers.iter().skip(skip) {
        let id: String = l.id.trim_start_matches("sha256:").chars().take(12).collect();
        let mark = if l.done { "done" } else { "" };
        lines.push(Line::from(format!(
            "{id:<12} {} {:>9} {mark}",
            bar(if l.done { l.total.max(1) } else { l.current }, l.total, 20),
            if l.total > 0 { human_bytes(l.total) } else { "?".into() }
        )));
    }
    let text_budget = budget.saturating_sub(layer_rows);
    let skip = v.lines.len().saturating_sub(text_budget);
    for l in v.lines.iter().skip(skip) {
        lines.push(Line::from(truncate(l, inner.width as usize)));
    }
    if v.layers.is_empty() && v.lines.is_empty() && v.done.is_none() {
        lines.push(Line::from(Span::styled("Working...", theme::muted())));
    }
    // Pin the footer to the bottom.
    while lines.len() + footer < inner.height as usize {
        lines.push(Line::from(""));
    }
    match &v.done {
        None => lines.push(hints(&[("Esc", "hide (keeps running)")])),
        Some(Ok(m)) => {
            lines.push(Line::from(Span::styled(format!("Done: {m}"), theme::ok())));
            lines.push(hints(&[("Esc / Enter", "close")]));
        }
        Some(Err(e)) => {
            lines.push(Line::from(Span::styled(truncate(&format!("Failed: {e}"), inner.width as usize), theme::err())));
            lines.push(hints(&[("Esc / Enter", "close")]));
        }
    }
    f.render_widget(Paragraph::new(lines), inner);
}

/// The bottom status bar: a message if there is one, else context hints.
pub fn status_line(app: &App) -> Line<'static> {
    if app.filter_edit {
        return hints(&[("Enter", "keep filter"), ("Esc", "clear filter")]);
    }
    if let Some(s) = &app.status {
        let style = match s.level {
            Level::Info => theme::muted(),
            Level::Success => theme::ok(),
            Level::Error => theme::err(),
        };
        return Line::from(Span::styled(format!(" {}", s.text), style));
    }
    use crate::state::Tab;
    let mut pairs: Vec<(&str, &str)> = match app.tab {
        Tab::Containers => vec![("r", "run"), ("s", "start"), ("x", "stop"), ("R", "restart"), ("d", "remove"), ("e", "shell"), ("l", "logs"), ("a", "all/running")],
        Tab::Images => vec![("p", "pull"), ("r", "run"), ("d", "remove")],
        Tab::Volumes => vec![("n", "new"), ("d", "remove")],
        Tab::Compose => vec![("u", "up"), ("x", "down")],
        Tab::System => vec![("P", "prune")],
    };
    pairs.extend([("/", "filter"), ("?", "help"), ("q", "quit")]);
    hints(&pairs)
}
