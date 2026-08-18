//! Rendering. Owns `draw(frame, &DrawState)`, view-mode dispatch, the status
//! bar, and overlay dispatch. Contains no application state mutation and no
//! tmux calls: every function here is a pure function of its arguments plus
//! the `Frame`/`Buffer` it draws into. Renders from fixture data only, which
//! is exactly what the tests in this module and in `tile`/`overlay` do.
//!
//! # Contract with `app.rs`
//!
//! `app.rs` (not yet built) owns `App` (DESIGN.md section 4) and must build
//! a `DrawState` from it once per frame:
//!
//! - `group: &Group` — `App::group`.
//! - `view: ViewMode` — `App::view`.
//! - `focus: Focus` — `App::focus`.
//! - `page: usize` — `App::page`.
//! - `overlay: Option<overlay::Overlay>` — derived from `App::modal`
//!   (DESIGN.md section 4's `Modal` enum) each frame. `Modal::None` maps to
//!   `None`; every other `Modal` variant maps to the matching `overlay::Overlay`
//!   variant, translating `Modal`'s richer fields into `Overlay`'s render-only
//!   ones (e.g. `Modal::Compose { input, queued, launch_on_submit }` maps
//!   directly; `Modal::ExitPrompt` needs `App` to compute `running`/`finished`
//!   counts and the reattach hint string from `App::group`).
//!
//! Then call `draw(frame, &state)` once inside `Terminal::draw`'s closure.
//!
//! `keymap::map` needs a *different*, narrower view of the same modal state
//! (`keymap::ModalState`) to decide which keys are legal. `app.rs` derives
//! both `overlay::Overlay` and `keymap::ModalState` from its own `Modal` on
//! every frame/keystroke; neither is the source of truth, `Modal` is.

use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph, Widget};

use crate::model::Group;

#[allow(dead_code)]
pub mod layout;
#[allow(dead_code)]
pub mod overlay;
#[allow(dead_code)]
pub mod tile;

use layout::{Focus, Pane, ViewMode};
use overlay::Overlay;

/// Columns the status bar's left half needs before it is worth also drawing
/// the key hints on the right.
const MIN_STATUS_LEFT: u16 = 24;

/// Everything `draw` needs for one frame. See the module doc comment for
/// exactly what `app.rs` must supply.
pub struct DrawState<'a> {
    pub group: &'a Group,
    pub view: ViewMode,
    pub focus: Focus,
    pub page: usize,
    pub overlay: Option<Overlay<'a>>,
}

/// Draw one frame: the body (grid or list/detail, depending on `state.view`),
/// the status bar, and, if present, the topmost overlay on top of everything
/// else.
pub fn draw(frame: &mut Frame, state: &DrawState) {
    let area = frame.area();
    let rows = Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).split(area);
    let body = rows[0];
    let status = rows[1];

    let geo = layout::compute(state.group.jobs.len(), body, state.view, state.page);

    let buf = frame.buffer_mut();
    match state.view {
        ViewMode::Grid => draw_grid(buf, state, &geo, body),
        ViewMode::ListDetail => draw_list_detail(buf, state, body),
    }
    draw_status_bar(buf, state, &geo, status);

    if let Some(overlay) = &state.overlay {
        overlay::render(overlay, area, buf);
    }
}

/// Draw the current grid page: one tile per visible job, laid out by
/// `layout::compute`'s `tiles` rects. Empty groups get a centered hint
/// instead of an empty grid.
fn draw_grid(buf: &mut Buffer, state: &DrawState, geo: &layout::Geometry, area: Rect) {
    if state.group.jobs.is_empty() {
        draw_empty_hint(buf, area);
        return;
    }

    let start = layout::page_start(state.page, geo.per_page);
    let end = (start + geo.tiles.len()).min(state.group.jobs.len());
    for (i, job) in state.group.jobs[start..end].iter().enumerate() {
        let Some(rect) = geo.tiles.get(i) else {
            continue;
        };
        let focused = state.focus.job == start + i;
        tile::render(job, *rect, buf, focused);
    }
}

/// Draw list/detail view: a session list on the left, the focused job's tile
/// filling the right.
fn draw_list_detail(buf: &mut Buffer, state: &DrawState, area: Rect) {
    if state.group.jobs.is_empty() {
        draw_empty_hint(buf, area);
        return;
    }

    // The same split `main.rs` resizes the focused tmux window to, so the
    // detail pane draws output that was captured at exactly this size.
    let (list, detail) = layout::list_detail_split(area);
    draw_session_list(buf, state, list);

    if let Some(job) = state.group.jobs.get(state.focus.job) {
        let focused = state.focus.pane == Pane::Detail;
        tile::render(job, detail, buf, focused);
    }
}

/// Draw the session list pane: one line per job, glyph + id + command,
/// highlighted (reverse video) when it is both the focused job and the List
/// pane owns the keyboard, bold when it is the focused job but the Detail
/// pane owns the keyboard.
fn draw_session_list(buf: &mut Buffer, state: &DrawState, area: Rect) {
    let focused_pane = state.focus.pane == Pane::List;
    let border_style = if focused_pane {
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
    };
    let block = Block::bordered()
        .title(" Sessions ")
        .border_style(border_style);
    let inner = block.inner(area);
    block.render(area, buf);

    for (i, job) in state.group.jobs.iter().enumerate() {
        if i as u16 >= inner.height {
            break;
        }
        let y = inner.y + i as u16;
        let selected = state.focus.job == i;
        let style = if selected && focused_pane {
            Style::default().add_modifier(Modifier::REVERSED)
        } else if selected {
            Style::default().add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        let (glyph, glyph_style) = tile::status_glyph(&job.state);
        let line = Line::from(vec![
            Span::styled(glyph, if selected { style } else { glyph_style }),
            Span::styled(format!(" {} {}", job.id, job.command), style),
        ]);
        buf.set_line(inner.x, y, &line, inner.width);
    }
}

/// Centered placeholder for a group with no jobs yet.
fn draw_empty_hint(buf: &mut Buffer, area: Rect) {
    let text = "No jobs yet. Press 'n' to add one.";
    Paragraph::new(text)
        .alignment(ratatui::layout::Alignment::Center)
        .render(area, buf);
}

/// Bottom status line: view mode and page indicator on the left, the
/// focused job's id and command in the middle, key hints on the right.
fn draw_status_bar(buf: &mut Buffer, state: &DrawState, geo: &layout::Geometry, area: Rect) {
    let mut left = match state.view {
        ViewMode::Grid => "grid".to_string(),
        ViewMode::ListDetail => "list".to_string(),
    };
    if state.view == ViewMode::Grid && geo.pages > 1 {
        left.push_str(&format!("  page {}/{}", state.page + 1, geo.pages));
    }
    if let Some(job) = state.group.jobs.get(state.focus.job) {
        left.push_str(&format!("  {} {}", job.id, job.command));
    }

    let hints = "?:help  v:view  n:new  q:quit";

    // The left side carries state the user cannot get anywhere else (page
    // number, focused job), so it wins when the bar is too narrow for both.
    // The hints are recoverable from the help overlay.
    let hints_width = hints.len() as u16;
    if area.width < hints_width + MIN_STATUS_LEFT {
        Paragraph::new(left).render(area, buf);
        return;
    }

    let cols =
        Layout::horizontal([Constraint::Min(0), Constraint::Length(hints_width)]).split(area);
    Paragraph::new(left).render(cols[0], buf);
    Paragraph::new(hints)
        .alignment(ratatui::layout::Alignment::Right)
        .style(Style::default().fg(Color::DarkGray))
        .render(cols[1], buf);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::{GroupId, JobId};
    use crate::model::{Job, JobState, Outcome};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::layout::Rect as LRect;
    use std::path::PathBuf;

    fn group_id() -> GroupId {
        GroupId::new("k3f9")
    }

    fn job_id(index: u16) -> JobId {
        JobId {
            group: group_id(),
            index,
        }
    }

    fn job(index: u16, command: &str, state: JobState) -> Job {
        let mut j = Job::new(
            job_id(index),
            command.to_string(),
            PathBuf::from("/tmp"),
            1000,
        );
        j.state = state;
        j.content = crate::ui::tile::decode_and_pad(b"", 10, 3).unwrap();
        j
    }

    fn group(jobs: Vec<Job>) -> Group {
        Group {
            id: group_id(),
            label: None,
            cwd: PathBuf::from("/tmp"),
            jobs,
            next_index: 1,
        }
    }

    fn draw_state(group: &Group, view: ViewMode) -> DrawState<'_> {
        DrawState {
            group,
            view,
            focus: Focus {
                job: 0,
                pane: Pane::List,
            },
            page: 0,
            overlay: None,
        }
    }

    fn render_to_buffer(state: &DrawState, width: u16, height: u16) -> Buffer {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, state)).unwrap();
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
    fn draws_one_tile_in_grid_view() {
        let g = group(vec![job(1, "cargo test", JobState::Running)]);
        let state = draw_state(&g, ViewMode::Grid);
        let buf = render_to_buffer(&state, 40, 12);
        let text = buffer_text(&buf);
        assert!(text.contains("k3f9.1"), "expected tile: {text}");
        assert!(text.contains("cargo test"), "expected tile: {text}");
        assert!(text.contains("grid"), "expected status bar: {text}");
    }

    #[test]
    fn draws_a_2x2_grid() {
        let g = group(vec![
            job(1, "a", JobState::Running),
            job(2, "b", JobState::Running),
            job(3, "c", JobState::Running),
            job(4, "d", JobState::Running),
        ]);
        let state = draw_state(&g, ViewMode::Grid);
        // 60x30 is chosen (as in layout.rs's own tests) to force a 2x2 grid
        // for 4 jobs.
        let geo = layout::compute(
            4,
            LRect {
                x: 0,
                y: 0,
                width: 60,
                height: 29,
            },
            ViewMode::Grid,
            0,
        );
        assert_eq!((geo.cols, geo.rows), (2, 2), "test assumes a 2x2 grid");

        let buf = render_to_buffer(&state, 60, 30);
        let text = buffer_text(&buf);
        for id in ["k3f9.1", "k3f9.2", "k3f9.3", "k3f9.4"] {
            assert!(text.contains(id), "expected {id} in 2x2 grid: {text}");
        }
    }

    #[test]
    fn finished_job_shows_exit_code_in_grid() {
        let g = group(vec![job(
            1,
            "cargo build",
            JobState::Finished {
                outcome: Outcome::Exit(1),
                at: Some(1),
            },
        )]);
        let state = draw_state(&g, ViewMode::Grid);
        let buf = render_to_buffer(&state, 40, 12);
        let text = buffer_text(&buf);
        assert!(text.contains("exit 1"), "expected exit code: {text}");
    }

    #[test]
    fn signal_killed_job_shows_signal_in_grid() {
        let g = group(vec![job(
            1,
            "sleep 100",
            JobState::Finished {
                outcome: Outcome::Signal("kill".to_string()),
                at: Some(1),
            },
        )]);
        let state = draw_state(&g, ViewMode::Grid);
        let buf = render_to_buffer(&state, 40, 12);
        let text = buffer_text(&buf);
        assert!(text.contains("sig kill"), "expected signal: {text}");
    }

    #[test]
    fn draws_list_detail_mode_with_session_list_and_detail_pane() {
        let g = group(vec![
            job(1, "cargo watch", JobState::Running),
            job(2, "npm run dev", JobState::Running),
        ]);
        let mut state = draw_state(&g, ViewMode::ListDetail);
        state.focus = Focus {
            job: 1,
            pane: Pane::Detail,
        };
        let buf = render_to_buffer(&state, 70, 20);
        let text = buffer_text(&buf);
        assert!(
            text.contains("Sessions"),
            "expected list pane header: {text}"
        );
        assert!(
            text.contains("cargo watch"),
            "expected first job in list: {text}"
        );
        assert!(
            text.contains("npm run dev"),
            "expected focused job in detail: {text}"
        );
    }

    #[test]
    fn empty_group_shows_hint_instead_of_grid() {
        let g = group(Vec::new());
        let state = draw_state(&g, ViewMode::Grid);
        let buf = render_to_buffer(&state, 40, 12);
        let text = buffer_text(&buf);
        assert!(text.contains("No jobs yet"), "expected empty hint: {text}");
    }

    #[test]
    fn status_bar_shows_page_indicator_when_paginated() {
        let jobs: Vec<Job> = (1..=20).map(|i| job(i, "x", JobState::Running)).collect();
        let g = group(jobs);
        let state = draw_state(&g, ViewMode::Grid);
        // A small area forces pagination (mirrors layout.rs's own tests).
        let buf = render_to_buffer(&state, 40, 12);
        let text = buffer_text(&buf);
        assert!(text.contains("page 1/"), "expected page indicator: {text}");
    }

    #[test]
    fn overlay_draws_on_top_of_the_grid() {
        let g = group(vec![job(1, "cargo test", JobState::Running)]);
        let mut state = draw_state(&g, ViewMode::Grid);
        state.overlay = Some(Overlay::Help);
        let buf = render_to_buffer(&state, 60, 24);
        let text = buffer_text(&buf);
        assert!(
            text.contains("Help"),
            "expected help overlay on top: {text}"
        );
    }

    #[test]
    fn confirm_overlay_draws_on_top() {
        let g = group(vec![job(1, "cargo test", JobState::Running)]);
        let mut state = draw_state(&g, ViewMode::Grid);
        state.overlay = Some(Overlay::Confirm {
            prompt: "Kill it?",
            selected: 0,
        });
        let buf = render_to_buffer(&state, 60, 24);
        let text = buffer_text(&buf);
        assert!(
            text.contains("Kill it?"),
            "expected confirm overlay: {text}"
        );
    }

    #[test]
    fn exit_prompt_overlay_draws_on_top() {
        let g = group(vec![job(1, "cargo test", JobState::Running)]);
        let mut state = draw_state(&g, ViewMode::Grid);
        state.overlay = Some(Overlay::ExitPrompt {
            running: 1,
            finished: 0,
            reattach_hint: "multi-run -a g1",
            selected: 0,
        });
        let buf = render_to_buffer(&state, 70, 24);
        let text = buffer_text(&buf);
        assert!(
            text.contains("Quit multi-run"),
            "expected exit prompt overlay: {text}"
        );
    }

    #[test]
    fn compose_overlay_draws_on_top() {
        let g = group(vec![job(1, "cargo test", JobState::Running)]);
        let mut state = draw_state(&g, ViewMode::Grid);
        let queued: Vec<String> = Vec::new();
        state.overlay = Some(Overlay::Compose {
            input: "npm test",
            queued: &queued,
            launch_on_submit: false,
        });
        let buf = render_to_buffer(&state, 70, 24);
        let text = buffer_text(&buf);
        assert!(
            text.contains("Add command"),
            "expected compose overlay: {text}"
        );
        assert!(text.contains("npm test"), "expected compose input: {text}");
    }

    #[test]
    fn toast_overlay_draws_on_top() {
        let g = group(vec![job(1, "cargo test", JobState::Running)]);
        let mut state = draw_state(&g, ViewMode::Grid);
        state.overlay = Some(Overlay::Toast {
            text: "session started",
            is_error: false,
        });
        let buf = render_to_buffer(&state, 60, 24);
        let text = buffer_text(&buf);
        assert!(
            text.contains("session started"),
            "expected toast overlay: {text}"
        );
    }

    #[test]
    fn error_log_overlay_draws_on_top() {
        let g = group(vec![job(1, "cargo test", JobState::Running)]);
        let mut state = draw_state(&g, ViewMode::Grid);
        let entries = vec!["kill-session failed".to_string()];
        state.overlay = Some(Overlay::ErrorLog {
            entries: &entries,
            scroll: 0,
        });
        let buf = render_to_buffer(&state, 60, 24);
        let text = buffer_text(&buf);
        assert!(
            text.contains("kill-session failed"),
            "expected error log overlay: {text}"
        );
    }
}
