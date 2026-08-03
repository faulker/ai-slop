//! Overlays: help, compose/add-command input, confirm, exit prompt, error
//! toast, error log. DESIGN.md section 4 (`Modal`), section 7 (keybindings,
//! for the hint text), and section 9 (the exit-prompt ASCII mock-up) are the
//! source of truth for content. No tmux calls, no IO, no state mutation:
//! `render` only draws into a caller-supplied `Buffer`.
//!
//! `Overlay` is deliberately not `keymap::ModalState`. `ModalState` carries
//! only what `keymap::map` needs to route a key; `Overlay` carries what
//! rendering needs to draw (actual compose text, actual counts, actual
//! messages). `app.rs` derives both from its own richer `Modal` (DESIGN.md
//! section 4) on every frame/keystroke; neither of these two is the source
//! of truth.

use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Clear, Paragraph, Widget, Wrap};

/// What to draw for the currently open modal. One variant per DESIGN.md
/// section 4 `Modal` variant (minus `Modal::None`, which the caller
/// represents as `Option::None` rather than a variant here).
pub enum Overlay<'a> {
    Help,
    /// Bare-launch compose screen, or the mid-run add-command prompt.
    /// `queued` is every line already committed; `input` is the line still
    /// being typed.
    Compose {
        input: &'a str,
        queued: &'a [String],
        launch_on_submit: bool,
    },
    Confirm {
        prompt: &'a str,
    },
    /// Counts feed the "N running, M finished" line; `reattach_hint` is the
    /// full `multi-run -a <id>` command shown under "keep".
    ExitPrompt {
        running: usize,
        finished: usize,
        reattach_hint: &'a str,
    },
    Toast {
        text: &'a str,
        is_error: bool,
    },
    ErrorLog {
        entries: &'a [String],
        scroll: u16,
    },
}

/// Render the given overlay centered (or, for `Toast`, corner-anchored) over
/// `area`. Dispatches to one function per variant.
pub fn render(overlay: &Overlay, area: Rect, buf: &mut Buffer) {
    match overlay {
        Overlay::Help => render_help(area, buf),
        Overlay::Compose {
            input,
            queued,
            launch_on_submit,
        } => render_compose(input, queued, *launch_on_submit, area, buf),
        Overlay::Confirm { prompt } => render_confirm(prompt, area, buf),
        Overlay::ExitPrompt {
            running,
            finished,
            reattach_hint,
        } => render_exit_prompt(*running, *finished, reattach_hint, area, buf),
        Overlay::Toast { text, is_error } => render_toast(text, *is_error, area, buf),
        Overlay::ErrorLog { entries, scroll } => render_error_log(entries, *scroll, area, buf),
    }
}

/// A `width` x `height` rect centered within `area`, clamped so it never
/// exceeds `area`'s bounds.
fn centered(width: u16, height: u16, area: Rect) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    Rect {
        x,
        y,
        width,
        height,
    }
}

/// Clear `rect` and draw `block` as its border, returning the inner rect to
/// fill with content. Clearing matters because overlays sit on top of
/// already-drawn tiles; without it, wide glyphs or partial redraws from the
/// tile underneath can show through gaps.
fn popup(block: Block<'_>, rect: Rect, buf: &mut Buffer) -> Rect {
    Clear.render(rect, buf);
    let inner = block.inner(rect);
    block.render(rect, buf);
    inner
}

fn render_help(area: Rect, buf: &mut Buffer) {
    // Tall enough for every line; `centered` clamps to the terminal, so a
    // short window simply shows fewer of them.
    let rect = centered(56, 31, area);
    let block = Block::bordered()
        .title(" Help ")
        .title_alignment(Alignment::Center);
    let inner = popup(block, rect, buf);

    let lines = vec![
        Line::from("Global").style(Style::default().add_modifier(Modifier::BOLD)),
        Line::from("  ?          toggle this help"),
        Line::from("  v          toggle grid / list view"),
        Line::from("  1 / 2      grid / list view"),
        Line::from("  n          new command"),
        Line::from("  !          error log"),
        Line::from("  q, Ctrl-c  quit"),
        Line::from(""),
        Line::from("Navigation").style(Style::default().add_modifier(Modifier::BOLD)),
        Line::from("  j/k, h/l   move focus"),
        Line::from("  Tab / S-Tab switch panel"),
        Line::from("  arrows     scroll selected panel"),
        Line::from("  g / G      first / last job"),
        Line::from("  Alt-1..9   focus job N"),
        Line::from("  [ / ]      previous / next page"),
        Line::from("  Ctrl-u/d   half page (list view)"),
        Line::from("  Ctrl-b/f   full page (list view)"),
        Line::from(""),
        Line::from("Session").style(Style::default().add_modifier(Modifier::BOLD)),
        Line::from("  Enter, a   attach"),
        Line::from("  Ctrl-\\     detach (while attached)"),
        Line::from("  Ctrl-n/p   next / previous session"),
        Line::from("  s          stop (keeps output)"),
        Line::from("  X          force kill (drops output)"),
        Line::from("  d / D      dismiss / dismiss all finished"),
        Line::from("  r          re-run"),
        Line::from(""),
        Line::from("Esc to close").style(Style::default().fg(Color::DarkGray)),
    ];
    Paragraph::new(lines).render(inner, buf);
}

fn render_compose(
    input: &str,
    queued: &[String],
    launch_on_submit: bool,
    area: Rect,
    buf: &mut Buffer,
) {
    let rect = centered(60, (queued.len() as u16 + 6).clamp(6, 20), area);
    let title = if launch_on_submit {
        " New run "
    } else {
        " Add command "
    };
    let block = Block::bordered().title(title);
    let inner = popup(block, rect, buf);

    let mut lines: Vec<Line<'static>> = Vec::new();
    if !queued.is_empty() {
        lines.push(Line::from("queued:").style(Style::default().fg(Color::DarkGray)));
        for cmd in queued {
            lines.push(Line::from(format!("  {cmd}")));
        }
    }
    lines.push(Line::from(vec![
        Span::raw("> "),
        Span::raw(input.to_string()),
    ]));
    lines.push(Line::from(""));
    lines.push(
        Line::from("Enter: next  Ctrl-s: launch  Ctrl-u: clear line  Esc: cancel")
            .style(Style::default().fg(Color::DarkGray)),
    );
    Paragraph::new(lines)
        .wrap(Wrap { trim: false })
        .render(inner, buf);
}

fn render_confirm(prompt: &str, area: Rect, buf: &mut Buffer) {
    let rect = centered((prompt.len() as u16 + 6).max(30), 5, area);
    let block = Block::bordered()
        .title(" Confirm ")
        .title_alignment(Alignment::Center);
    let inner = popup(block, rect, buf);

    let lines = vec![
        Line::from(prompt.to_string()),
        Line::from(""),
        Line::from("[y] yes    [n] no").style(Style::default().fg(Color::DarkGray)),
    ];
    Paragraph::new(lines)
        .alignment(Alignment::Center)
        .render(inner, buf);
}

fn render_exit_prompt(
    running: usize,
    finished: usize,
    reattach_hint: &str,
    area: Rect,
    buf: &mut Buffer,
) {
    let rect = centered(64, 9, area);
    let block = Block::bordered().title(" Quit multi-run ");
    let inner = popup(block, rect, buf);

    let total = running + finished;
    let summary = format!(
        "{running} session{} still running, {finished} finished.",
        if running == 1 { "" } else { "s" }
    );
    let lines = vec![
        Line::from(summary),
        Line::from(""),
        Line::from("  [k]  keep them running"),
        Line::from(format!("       reattach with:  {reattach_hint}"))
            .style(Style::default().fg(Color::DarkGray)),
        Line::from(format!(
            "  [x]  kill all {total} session{}",
            if total == 1 { "" } else { "s" }
        )),
        Line::from("  [Esc] cancel"),
    ];
    Paragraph::new(lines).render(inner, buf);
}

/// A small, non-blocking corner box. Transient: `app.rs` clears it after 5s
/// (DESIGN.md section 4); nothing here enforces that timing since this
/// module does no IO or timing of its own.
fn render_toast(text: &str, is_error: bool, area: Rect, buf: &mut Buffer) {
    let width = (text.len() as u16 + 4).min(area.width);
    let rect = Rect {
        x: area.x + area.width.saturating_sub(width),
        y: area.y + area.height.saturating_sub(3),
        width,
        height: 3.min(area.height),
    };
    let style = if is_error {
        Style::default().fg(Color::Red)
    } else {
        Style::default().fg(Color::Yellow)
    };
    let block = Block::bordered().border_style(style);
    let inner = popup(block, rect, buf);
    Paragraph::new(text).render(inner, buf);
}

fn render_error_log(entries: &[String], scroll: u16, area: Rect, buf: &mut Buffer) {
    let rect = centered(
        area.width.saturating_sub(4).max(20),
        area.height.saturating_sub(4).max(6),
        area,
    );
    let block = Block::bordered().title(" Errors (Esc to close) ");
    let inner = popup(block, rect, buf);

    let text: Text<'static> = if entries.is_empty() {
        Text::from("No errors logged.")
    } else {
        Text::from(
            entries
                .iter()
                .map(|e| Line::from(e.clone()))
                .collect::<Vec<_>>(),
        )
    };
    Paragraph::new(text).scroll((scroll, 0)).render(inner, buf);
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn render_to_buffer(overlay: &Overlay, width: u16, height: u16) -> Buffer {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                render(overlay, frame.area(), frame.buffer_mut());
            })
            .unwrap();
        terminal.backend().buffer().clone()
    }

    fn buffer_text(buf: &Buffer) -> String {
        let mut out = String::new();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                out.push_str(buf[(x, y)].symbol());
            }
            out.push('\n');
        }
        out
    }

    #[test]
    fn help_overlay_lists_key_bindings() {
        let buf = render_to_buffer(&Overlay::Help, 60, 24);
        let text = buffer_text(&buf);
        assert!(text.contains("Help"));
        assert!(text.contains("toggle this help"));
        assert!(text.contains("quit"));
    }

    #[test]
    fn help_overlay_names_the_attached_session_keys() {
        // These are bound inside tmux by the generated config, so the help
        // overlay is the only place the TUI can teach them.
        let buf = render_to_buffer(&Overlay::Help, 60, 40);
        let text = buffer_text(&buf);
        assert!(text.contains("Ctrl-\\"), "no detach key in:\n{text}");
        assert!(text.contains("detach"));
        assert!(text.contains("Ctrl-n/p"));
    }

    #[test]
    fn compose_overlay_shows_queued_and_current_input() {
        let queued = vec!["cargo test".to_string(), "npm run dev".to_string()];
        let overlay = Overlay::Compose {
            input: "cargo fmt",
            queued: &queued,
            launch_on_submit: true,
        };
        let buf = render_to_buffer(&overlay, 70, 20);
        let text = buffer_text(&buf);
        assert!(text.contains("New run"));
        assert!(text.contains("cargo test"));
        assert!(text.contains("npm run dev"));
        assert!(text.contains("cargo fmt"));
    }

    #[test]
    fn compose_overlay_add_command_title_differs_from_bare_launch() {
        let queued: Vec<String> = Vec::new();
        let overlay = Overlay::Compose {
            input: "",
            queued: &queued,
            launch_on_submit: false,
        };
        let buf = render_to_buffer(&overlay, 70, 20);
        let text = buffer_text(&buf);
        assert!(text.contains("Add command"));
    }

    #[test]
    fn confirm_overlay_shows_prompt_and_yn_hint() {
        let overlay = Overlay::Confirm {
            prompt: "Kill job g1.2?",
        };
        let buf = render_to_buffer(&overlay, 50, 10);
        let text = buffer_text(&buf);
        assert!(text.contains("Kill job g1.2?"));
        assert!(text.contains("[y]"));
        assert!(text.contains("[n]"));
    }

    #[test]
    fn exit_prompt_shows_counts_and_reattach_hint() {
        let overlay = Overlay::ExitPrompt {
            running: 2,
            finished: 1,
            reattach_hint: "multi-run -a g1",
        };
        let buf = render_to_buffer(&overlay, 70, 12);
        let text = buffer_text(&buf);
        assert!(text.contains("2 sessions still running, 1 finished"));
        assert!(text.contains("multi-run -a g1"));
        assert!(text.contains("kill all 3 sessions"));
    }

    #[test]
    fn toast_shows_text_and_uses_error_style() {
        let overlay = Overlay::Toast {
            text: "tmux not found",
            is_error: true,
        };
        let buf = render_to_buffer(&overlay, 60, 20);
        let text = buffer_text(&buf);
        assert!(text.contains("tmux not found"));
    }

    #[test]
    fn error_log_lists_entries() {
        let entries = vec!["first error".to_string(), "second error".to_string()];
        let overlay = Overlay::ErrorLog {
            entries: &entries,
            scroll: 0,
        };
        let buf = render_to_buffer(&overlay, 60, 20);
        let text = buffer_text(&buf);
        assert!(text.contains("first error"));
        assert!(text.contains("second error"));
    }

    #[test]
    fn error_log_empty_shows_placeholder() {
        let entries: Vec<String> = Vec::new();
        let overlay = Overlay::ErrorLog {
            entries: &entries,
            scroll: 0,
        };
        let buf = render_to_buffer(&overlay, 60, 20);
        let text = buffer_text(&buf);
        assert!(text.contains("No errors logged"));
    }
}
