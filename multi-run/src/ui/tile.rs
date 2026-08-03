//! One tile: border, title, status glyph, decoded content, padding.
//! DESIGN.md section 6 ("Capture to ratatui") is the source of truth for the
//! decode pipeline; tmux-research.md gotcha I is the source of truth for why
//! padding happens here rather than being trusted from `capture-pane`.
//!
//! `decode_and_pad` is the pure byte-to-`Line` pipeline described in
//! DESIGN.md section 6. It is exercised here with fixture bytes; the actual
//! call site, once `app.rs`/`poll.rs` exist, is wherever raw `capture-pane`
//! output arrives from the poller thread, on the assumption that its result
//! becomes `Job::content` (see `model.rs`'s doc comment on that field: "Last
//! successful decode ... already padded/truncated"). `render` then assumes
//! `Job::content` is already decoded and padded and only lays it out; it does
//! not re-decode.
//!
//! No tmux calls, no IO, no state mutation: everything here is either a pure
//! function of bytes/a `Job` reference, or a widget render into a caller-
//! supplied `Buffer`.

use ansi_to_tui::IntoText;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Paragraph, Widget};
use unicode_width::UnicodeWidthStr;

use crate::model::{Job, JobState, Outcome};

/// Decode one `capture-pane -p -e` byte capture into exactly `height` lines,
/// each padded to exactly `width` display columns. Implements the pipeline
/// in DESIGN.md section 6:
/// - prepend a reset SGR so decoding starts from a deterministic state
/// - decode with `ansi-to-tui`
/// - normalise line count to `height` (pad with blanks, or keep the last
///   `height` lines if there are more)
/// - pad each line to `width` columns (gotcha I: `capture-pane -N` does not
///   reliably do this itself)
///
/// On a decode error, the caller should keep the previous `Job::content` and
/// log the error rather than blank the tile (DESIGN.md section 6); this
/// function has no "previous content" to fall back to, so it just reports
/// the error.
pub fn decode_and_pad(
    bytes: &[u8],
    width: u16,
    height: u16,
) -> Result<Vec<Line<'static>>, ansi_to_tui::Error> {
    let mut framed = Vec::with_capacity(bytes.len() + 4);
    framed.extend_from_slice(b"\x1b[0m");
    framed.extend_from_slice(bytes);

    let text = framed.into_text()?;
    let mut lines = text.lines;

    let height = height as usize;
    if lines.len() < height {
        lines.resize(height, Line::default());
    } else if lines.len() > height {
        let drop = lines.len() - height;
        lines.drain(0..drop);
    }

    Ok(lines
        .into_iter()
        .map(|line| pad_line(line, width))
        .collect())
}

/// Pad one line to exactly `width` display columns by appending a trailing
/// blank span. Never truncates: a pane is sized to match its tile, so an
/// overlong line indicates a resize race rather than something to hide.
fn pad_line(mut line: Line<'static>, width: u16) -> Line<'static> {
    let content_width: usize = line
        .spans
        .iter()
        .map(|s| UnicodeWidthStr::width(s.content.as_ref()))
        .sum();
    let width = width as usize;
    if content_width < width {
        line.spans
            .push(Span::raw(" ".repeat(width - content_width)));
    }
    line
}

/// Status glyph and its style for a job's current state. `pub(crate)` so
/// `ui/mod.rs` can reuse it for the list/detail session list without
/// duplicating the state-to-glyph mapping.
pub(crate) fn status_glyph(state: &JobState) -> (&'static str, Style) {
    match state {
        JobState::Starting => ("\u{22ef}", Style::default().fg(Color::DarkGray)), // ⋯
        JobState::Running => ("\u{25cf}", Style::default().fg(Color::Green)),     // ●
        JobState::Finished {
            outcome: Outcome::Exit(0),
            ..
        } => {
            ("\u{2713}", Style::default().fg(Color::Green)) // ✓
        }
        JobState::Finished {
            outcome: Outcome::Exit(_),
            ..
        } => {
            ("\u{2717}", Style::default().fg(Color::Red)) // ✗
        }
        JobState::Finished {
            outcome: Outcome::Signal(_),
            ..
        } => {
            ("\u{2717}", Style::default().fg(Color::Red)) // ✗
        }
        JobState::Finished {
            outcome: Outcome::Unknown,
            ..
        } => ("?", Style::default().fg(Color::Yellow)),
        JobState::Vanished => ("\u{26a0}", Style::default().fg(Color::Magenta)), // ⚠
        JobState::Dismissed => ("\u{00d7}", Style::default().fg(Color::DarkGray)), // ×
    }
}

/// Short human status text shown at the right of a tile's title and in the
/// list/detail session list.
pub(crate) fn status_text(state: &JobState) -> String {
    match state {
        JobState::Starting => "starting".to_string(),
        JobState::Running => "running".to_string(),
        JobState::Finished {
            outcome: Outcome::Exit(n),
            ..
        } => format!("exit {n}"),
        JobState::Finished {
            outcome: Outcome::Signal(s),
            ..
        } => format!("sig {s}"),
        JobState::Finished {
            outcome: Outcome::Unknown,
            ..
        } => "unknown".to_string(),
        JobState::Vanished => "gone".to_string(),
        JobState::Dismissed => "dismissed".to_string(),
    }
}

/// Border style for a tile that does not have keyboard focus, colour-coded
/// by job state so a finished/vanished job is visible at a glance across a
/// full grid page.
fn unfocused_border_style(state: &JobState) -> Style {
    match state {
        JobState::Starting | JobState::Running => Style::default(),
        JobState::Finished {
            outcome: Outcome::Exit(0),
            ..
        } => Style::default().fg(Color::Green),
        JobState::Finished { .. } => Style::default().fg(Color::Red),
        JobState::Vanished => Style::default().fg(Color::Magenta),
        JobState::Dismissed => Style::default().fg(Color::DarkGray),
    }
}

/// How long a still-running job has been going, as a compact `1m02s`. Returns
/// `None` for a job that is not running, or whose start time is unknown or in
/// the future (a clock change should not print nonsense).
fn elapsed_label(job: &Job) -> Option<String> {
    if !matches!(job.state, JobState::Running) {
        return None;
    }
    // 0 is the "no start time recorded" sentinel used by `Job::new` callers
    // that have nothing better; rendering it would claim the job started at
    // the epoch and print an absurd duration.
    if job.started_at == 0 {
        return None;
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_secs();
    let secs = now.checked_sub(job.started_at)?;
    Some(match secs {
        s if s < 60 => format!("{s}s"),
        s if s < 3600 => format!("{}m{:02}s", s / 60, s % 60),
        s => format!("{}h{:02}m", s / 3600, (s % 3600) / 60),
    })
}

/// Render one job's tile into `area` of `buf`: a bordered block titled with
/// the job id, command, and status glyph on the left, and elapsed time plus
/// status text on the right, with `Job::content` (already decoded and padded,
/// per `model.rs`) filling the inside. `focused` highlights the border to show
/// keyboard focus (DESIGN.md section 7 navigation).
pub fn render(job: &Job, area: Rect, buf: &mut Buffer, focused: bool) {
    let (glyph, glyph_style) = status_glyph(&job.state);
    let border_style = if focused {
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    } else {
        unfocused_border_style(&job.state)
    };

    let left_title = Line::from(vec![
        Span::raw(format!(" {} ", job.id)),
        Span::styled(glyph, glyph_style),
        Span::raw(" "),
        Span::raw(job.command.clone()),
        Span::raw(" "),
    ]);
    // A running job's most useful extra fact is how long it has been going,
    // so the right title carries elapsed time alongside the status. A
    // finished job's status already says what happened, so it stays bare.
    let right_title = match elapsed_label(job) {
        Some(e) => Line::from(format!(" {e}  {} ", status_text(&job.state))),
        None => Line::from(format!(" {} ", status_text(&job.state))),
    }
    .right_aligned();

    let block = Block::bordered()
        .border_style(border_style)
        .title(left_title)
        .title(right_title);
    let inner = block.inner(area);
    block.render(area, buf);

    let text = Text::from(job.content.clone());
    Paragraph::new(text).render(inner, buf);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::{GroupId, JobId};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use std::path::PathBuf;

    fn job_id(index: u16) -> JobId {
        JobId {
            group: GroupId::new("k3f9"),
            index,
        }
    }

    /// Seconds since the epoch, so fixtures can use a start time that is
    /// actually plausible. A hardcoded small value would render as tens of
    /// thousands of hours and crowd the title out of the border.
    fn now_secs() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
    }

    fn job(state: JobState, content: Vec<Line<'static>>) -> Job {
        let mut j = Job::new(
            job_id(1),
            "cargo test".to_string(),
            PathBuf::from("/tmp"),
            now_secs().saturating_sub(5),
        );
        j.state = state;
        j.content = content;
        j
    }

    #[test]
    fn elapsed_is_shown_for_a_running_job_only() {
        let mut j = job(JobState::Running, vec![]);
        j.started_at = now_secs().saturating_sub(65);
        assert_eq!(elapsed_label(&j).as_deref(), Some("1m05s"));

        j.state = JobState::Finished {
            outcome: Outcome::Exit(0),
            at: None,
        };
        assert_eq!(
            elapsed_label(&j),
            None,
            "a finished job shows its outcome, not a timer"
        );
    }

    #[test]
    fn elapsed_formats_seconds_minutes_and_hours() {
        let mut j = job(JobState::Running, vec![]);
        j.started_at = now_secs().saturating_sub(5);
        assert_eq!(elapsed_label(&j).as_deref(), Some("5s"));
        j.started_at = now_secs().saturating_sub(3600 + 120);
        assert_eq!(elapsed_label(&j).as_deref(), Some("1h02m"));
    }

    #[test]
    fn elapsed_is_hidden_when_the_start_time_is_unset() {
        let mut j = job(JobState::Running, vec![]);
        j.started_at = 0;
        assert_eq!(elapsed_label(&j), None, "epoch 0 means unknown, not 1970");
    }

    #[test]
    fn elapsed_is_hidden_when_the_start_time_is_in_the_future() {
        let mut j = job(JobState::Running, vec![]);
        j.started_at = now_secs() + 600;
        assert_eq!(elapsed_label(&j), None, "a clock skew must not underflow");
    }

    fn render_to_buffer(job: &Job, width: u16, height: u16, focused: bool) -> Buffer {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                render(job, frame.area(), frame.buffer_mut(), focused);
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

    // --- decode_and_pad ---------------------------------------------------

    #[test]
    fn decode_and_pad_normalises_short_output_to_height() {
        let lines = decode_and_pad(b"hello", 10, 3).unwrap();
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0].width(), 10);
        assert_eq!(lines[1].width(), 10);
    }

    #[test]
    fn decode_and_pad_keeps_last_height_lines_when_too_many() {
        let bytes = b"one\r\ntwo\r\nthree\r\nfour\r\n";
        let lines = decode_and_pad(bytes, 10, 2).unwrap();
        assert_eq!(lines.len(), 2);
        // The last two logical lines survive.
        let rendered: Vec<String> = lines
            .iter()
            .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
            .collect();
        assert!(rendered[0].contains("three"));
        assert!(rendered[1].contains("four"));
    }

    #[test]
    fn decode_and_pad_pads_short_lines_to_width() {
        let lines = decode_and_pad(b"hi", 5, 1).unwrap();
        assert_eq!(lines[0].width(), 5);
    }

    #[test]
    fn decode_and_pad_handles_sgr_color_codes() {
        let lines = decode_and_pad(b"\x1b[31mred\x1b[0m", 10, 1).unwrap();
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].width(), 10);
    }

    #[test]
    fn decode_and_pad_does_not_truncate_overlong_lines() {
        let lines = decode_and_pad(b"0123456789", 4, 1).unwrap();
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].width(), 10);
    }

    // --- render: one tile ---------------------------------------------

    #[test]
    fn renders_one_tile_with_border_id_command_and_content() {
        let content = decode_and_pad(b"line one", 30, 5).unwrap();
        let j = job(JobState::Running, content);
        let buf = render_to_buffer(&j, 40, 7, false);
        let text = buffer_text(&buf);
        assert!(text.contains("k3f9.1"), "expected job id in tile: {text}");
        assert!(
            text.contains("cargo test"),
            "expected command in tile: {text}"
        );
        assert!(
            text.contains("running"),
            "expected status text in tile: {text}"
        );
        assert!(
            text.contains("line one"),
            "expected decoded content in tile: {text}"
        );
    }

    #[test]
    fn finished_job_shows_its_exit_code() {
        let content = decode_and_pad(b"", 20, 5).unwrap();
        let j = job(
            JobState::Finished {
                outcome: Outcome::Exit(42),
                at: Some(1),
            },
            content,
        );
        let buf = render_to_buffer(&j, 30, 7, false);
        let text = buffer_text(&buf);
        assert!(
            text.contains("exit 42"),
            "expected exit code in tile: {text}"
        );
    }

    #[test]
    fn signal_killed_job_shows_its_signal() {
        let content = decode_and_pad(b"", 20, 5).unwrap();
        let j = job(
            JobState::Finished {
                outcome: Outcome::Signal("term".to_string()),
                at: Some(1),
            },
            content,
        );
        let buf = render_to_buffer(&j, 30, 7, false);
        let text = buffer_text(&buf);
        assert!(text.contains("sig term"), "expected signal in tile: {text}");
    }

    #[test]
    fn vanished_job_shows_gone() {
        let content = decode_and_pad(b"", 20, 5).unwrap();
        let j = job(JobState::Vanished, content);
        let buf = render_to_buffer(&j, 30, 7, false);
        let text = buffer_text(&buf);
        assert!(text.contains("gone"), "expected 'gone' in tile: {text}");
    }

    #[test]
    fn focused_tile_uses_a_distinct_border_style() {
        let content = decode_and_pad(b"", 20, 5).unwrap();
        let j = job(JobState::Running, content);
        let focused_buf = render_to_buffer(&j, 30, 7, true);
        let unfocused_buf = render_to_buffer(&j, 30, 7, false);
        // Top-left border cell's foreground colour should differ between
        // focused and not (focused is always cyan; running is unstyled).
        assert_ne!(focused_buf[(0, 0)].fg, unfocused_buf[(0, 0)].fg);
    }
}
