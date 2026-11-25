use crate::state::App;
use crate::theme;
use crate::widgets::{empty_state, panel};
use ratatui::{
    layout::{Constraint, Layout, Rect},
    text::{Line, Span},
    widgets::{Paragraph, Wrap},
    Frame,
};

pub fn draw(f: &mut Frame, area: Rect, app: &App) {
    let Some(i) = &app.info else {
        let block = panel("System");
        let inner = block.inner(area);
        f.render_widget(block, area);
        match &app.conn_error {
            Some(e) => empty_state(f, inner, "Can't reach the engine", e),
            None => empty_state(f, inner, "Loading system info...", ""),
        }
        return;
    };
    let warn_h = if i.warnings.is_empty() { 0 } else { i.warnings.len() as u16 + 2 };
    let chunks = Layout::vertical([Constraint::Length(warn_h), Constraint::Min(5)]).split(area);
    if warn_h > 0 {
        let block = panel("Warnings").border_style(theme::warn());
        let inner = block.inner(chunks[0]);
        f.render_widget(block, chunks[0]);
        let lines: Vec<Line> = i
            .warnings
            .iter()
            .map(|w| Line::from(vec![Span::styled("! ", theme::warn()), Span::raw(w.clone())]))
            .collect();
        f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), inner);
    }
    let block = panel("System");
    let inner = block.inner(chunks[1]);
    f.render_widget(block, chunks[1]);
    let l = |k: &str, v: String| Line::from(vec![Span::styled(format!("{k:<12}"), theme::muted()), Span::raw(v)]);
    let emu = if i.emulated_platforms.is_empty() { "none".to_string() } else { i.emulated_platforms.join(", ") };
    let lines = vec![
        l("Version", i.version.clone()),
        l("Host", format!("{} ({})", i.os, i.host_platform)),
        l("Mode", i.mode.clone()),
        l("Rootless", if i.rootless { "yes".into() } else { "no".into() }),
        l("Data root", i.data_root.display().to_string()),
        l("Emulation", emu),
        l("Containers", format!("{} ({} running)", i.containers, i.running)),
        l("Images", i.images.to_string()),
        Line::from(""),
        Line::from(vec![
            Span::styled("P", theme::key()),
            Span::styled(" prune stopped containers and unused images   ", theme::muted()),
            Span::styled("A", theme::key()),
            Span::styled(" deep prune (images and volumes)", theme::muted()),
        ]),
    ];
    f.render_widget(Paragraph::new(lines), inner);
}
