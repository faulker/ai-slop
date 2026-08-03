//! Tab-delimited `-F` record parsers and capture-marker splitting for the
//! tmux backend contract in DESIGN.md section 5. Pure functions only: no
//! `std::process`, no IO, no knowledge of sockets or config paths. Callers
//! in `tmux/mod.rs` run the actual `tmux` invocations and hand this module
//! the raw stdout.
//!
//! Two record shapes come out of the metadata poll (invocation 3):
//! `SessionRecord` from `list-sessions -F ...` and `PaneRecord` from
//! `list-panes -a -F ...`. One marker-delimited shape comes out of the
//! content poll (invocation 4): a `display-message` line immediately
//! before each `capture-pane`, so the batched output can be split back into
//! per-session screens.

/// A parse failure for one record or one field within it. Carries a message
/// good enough to show in the error log (`!` in the UI); callers that need
/// the typed `TmuxError::Parse` variant wrap `.0` into it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError(pub String);

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for ParseError {}

/// One row of `list-sessions -f '#{!=:#{@mr_group},}' -F
/// '#{session_name}TAB#{@mr_group}TAB#{@mr_index}TAB#{@mr_cwd}TAB#{@mr_started}TAB#{session_attached}TAB#{@mr_label}TAB#{@mr_cmd}'`
/// (DESIGN.md section 5, invocation 3). `@mr_cmd` is deliberately the last
/// field: a command string can itself contain a tab, and putting it last
/// plus parsing with `splitn` means any embedded tab lands harmlessly
/// inside `cmd` instead of shifting every field after it (DESIGN.md section
/// 14, open question 2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionRecord {
    pub session_name: String,
    pub group: String,
    pub index: u16,
    pub cwd: String,
    pub started: u64,
    pub attached: bool,
    /// `@mr_label` resolves to an empty string when unset; that maps to
    /// `None` rather than `Some(String::new())`.
    pub label: Option<String>,
    pub cmd: String,
}

/// One row of `list-panes -a -F
/// '#{session_name}TAB#{pane_dead}TAB#{pane_dead_status}TAB#{pane_dead_signal}TAB#{pane_dead_time}TAB#{pane_pid}TAB#{pane_width}TAB#{pane_height}TAB#{history_size}TAB#{cursor_y}'`
/// (DESIGN.md section 5, invocation 3). Every dead-pane field is empty
/// unless the pane is actually dead; exactly one of `dead_status` /
/// `dead_signal` is populated once `dead` is true (research item 1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaneRecord {
    pub session_name: String,
    pub dead: bool,
    pub dead_status: Option<i32>,
    pub dead_signal: Option<String>,
    pub dead_time: Option<u64>,
    pub pane_pid: Option<u32>,
    pub width: u16,
    pub height: u16,
    pub history_size: u32,
    pub cursor_y: u16,
}

/// Strip a trailing `\r` left over from a CRLF-terminated line. tmux itself
/// emits bare `\n`, but this guards against a pty or a wrapping shell that
/// introduces `\r` before we ever see the line.
fn strip_cr(line: &str) -> &str {
    line.strip_suffix('\r').unwrap_or(line)
}

/// Parse a `"0"`/`"1"` tmux boolean format field. Anything else is
/// malformed input rather than a value we should silently coerce.
fn parse_bool_field(field: &str, name: &str, line: &str) -> Result<bool, ParseError> {
    match field {
        "0" => Ok(false),
        "1" => Ok(true),
        other => Err(ParseError(format!(
            "field {name}: expected \"0\" or \"1\", got {other:?} in row {line:?}"
        ))),
    }
}

/// Parse a required numeric field, producing a `ParseError` that names both
/// the field and the offending row so a bad poll is debuggable from the log
/// alone.
fn parse_required<T: std::str::FromStr>(
    field: &str,
    name: &str,
    line: &str,
) -> Result<T, ParseError> {
    field.parse::<T>().map_err(|_| {
        ParseError(format!(
            "field {name}: could not parse {field:?} in row {line:?}"
        ))
    })
}

/// Parse an optional numeric field: an empty string is `None` (tmux emits
/// an empty string for a format like `#{pane_dead_status}` on a live
/// pane), anything else must parse or the row is malformed.
fn parse_optional<T: std::str::FromStr>(
    field: &str,
    name: &str,
    line: &str,
) -> Result<Option<T>, ParseError> {
    if field.is_empty() {
        Ok(None)
    } else {
        Ok(Some(parse_required(field, name, line)?))
    }
}

/// Parse one `list-sessions` row into a `SessionRecord`. `line` should be a
/// single record with no trailing newline, though a trailing `\r`/`\n` is
/// tolerated and stripped. An empty line (including one left over from
/// splitting on a trailing newline) is malformed, not an empty record: a
/// real empty poll result has zero lines, handled by
/// `parse_session_records` before this function ever runs.
pub fn parse_session_line(line: &str) -> Result<SessionRecord, ParseError> {
    let line = strip_cr(line.trim_end_matches('\n'));
    if line.is_empty() {
        return Err(ParseError("empty session record line".to_string()));
    }

    let fields: Vec<&str> = line.splitn(8, '\t').collect();
    if fields.len() != 8 {
        return Err(ParseError(format!(
            "expected 8 tab-separated fields, got {} in row {line:?}",
            fields.len()
        )));
    }
    let [
        session_name,
        group,
        index,
        cwd,
        started,
        attached,
        label,
        cmd,
    ] = fields[..]
    else {
        unreachable!("length checked above");
    };

    Ok(SessionRecord {
        session_name: session_name.to_string(),
        group: group.to_string(),
        index: parse_required(index, "@mr_index", line)?,
        cwd: cwd.to_string(),
        started: parse_required(started, "@mr_started", line)?,
        attached: parse_bool_field(attached, "session_attached", line)?,
        label: if label.is_empty() {
            None
        } else {
            Some(label.to_string())
        },
        cmd: cmd.to_string(),
    })
}

/// Parse one `list-panes` row into a `PaneRecord`. Same line-shape
/// tolerance as `parse_session_line`.
pub fn parse_pane_line(line: &str) -> Result<PaneRecord, ParseError> {
    let line = strip_cr(line.trim_end_matches('\n'));
    if line.is_empty() {
        return Err(ParseError("empty pane record line".to_string()));
    }

    let fields: Vec<&str> = line.splitn(10, '\t').collect();
    if fields.len() != 10 {
        return Err(ParseError(format!(
            "expected 10 tab-separated fields, got {} in row {line:?}",
            fields.len()
        )));
    }
    let [
        session_name,
        dead,
        dead_status,
        dead_signal,
        dead_time,
        pane_pid,
        width,
        height,
        history_size,
        cursor_y,
    ] = fields[..]
    else {
        unreachable!("length checked above");
    };

    Ok(PaneRecord {
        session_name: session_name.to_string(),
        dead: parse_bool_field(dead, "pane_dead", line)?,
        dead_status: parse_optional(dead_status, "pane_dead_status", line)?,
        dead_signal: if dead_signal.is_empty() {
            None
        } else {
            Some(dead_signal.to_string())
        },
        dead_time: parse_optional(dead_time, "pane_dead_time", line)?,
        pane_pid: parse_optional(pane_pid, "pane_pid", line)?,
        width: parse_required(width, "pane_width", line)?,
        height: parse_required(height, "pane_height", line)?,
        history_size: parse_required(history_size, "history_size", line)?,
        cursor_y: parse_required(cursor_y, "cursor_y", line)?,
    })
}

/// Parse every non-blank line of a `list-sessions` poll's stdout.
/// DESIGN.md section 5: "empty result is not an error", so raw output that
/// is empty or all-blank yields `Ok(vec![])`, not an error. The first
/// genuinely malformed line aborts the whole batch, since a metadata poll
/// that partially fails to parse indicates something is wrong with the
/// format string or the tmux version, not one bad row worth skipping.
pub fn parse_session_records(raw: &str) -> Result<Vec<SessionRecord>, ParseError> {
    raw.lines()
        .map(strip_cr)
        .filter(|l| !l.is_empty())
        .map(parse_session_line)
        .collect()
}

/// Parse every non-blank line of a `list-panes -a` poll's stdout. See
/// `parse_session_records` for the empty-input and error-aggregation
/// policy.
pub fn parse_pane_records(raw: &str) -> Result<Vec<PaneRecord>, ParseError> {
    raw.lines()
        .map(strip_cr)
        .filter(|l| !l.is_empty())
        .map(parse_pane_line)
        .collect()
}

/// One job's captured pane output, recovered from a marker-delimited
/// content-poll batch (DESIGN.md section 5, invocation 4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureBlock {
    pub session_name: String,
    pub content: String,
}

/// The exact `display-message -p` payload emitted immediately before each
/// session's `capture-pane` in the batched content-poll argv. Must match
/// what `tmux/mod.rs` builds into the command queue.
fn marker_for(session_name: &str) -> String {
    format!("@@MR:{session_name}")
}

/// Split raw content-poll stdout into per-session blocks.
///
/// `session_order` must be the exact sequence of session names the caller
/// requested captures for, in argv order (one `display-message -p
/// '@@MR:<name>'` followed by one `capture-pane` per entry). Markers are
/// recognised as whole lines matched against this known, ordered list,
/// never as a generic `@@MR:` prefix scan. That means pane output which
/// happens to contain a line shaped like a marker is *not* mistaken for one
/// unless it exactly reproduces the literal marker text of the next
/// session still being searched for, an accepted residual risk of the same
/// shape as the embedded-tab case in record parsing (DESIGN.md section 14,
/// open question 2).
///
/// Lines before the first recognised marker are discarded; there is no
/// prior block to attribute them to (this can only happen if the raw
/// output is itself malformed, since the real argv always starts with a
/// marker).
///
/// Iterates with `str::lines()` rather than `split('\n')` so a trailing
/// newline (which every `capture-pane`/`display-message` line has) is
/// treated as a line terminator, not a separator that would otherwise
/// manufacture a spurious blank final line. Each block's `content` is
/// therefore exactly its captured lines joined by `\n`, with no trailing
/// newline of its own; internal blank lines are preserved.
pub fn split_captures(raw: &str, session_order: &[String]) -> Vec<CaptureBlock> {
    let markers: Vec<String> = session_order.iter().map(|n| marker_for(n)).collect();

    let mut blocks = Vec::with_capacity(session_order.len());
    let mut want = 0usize;
    let mut current: Option<Vec<&str>> = None;

    for line in raw.lines() {
        if want < markers.len() && line == markers[want] {
            if let Some(prev_lines) = current.take() {
                blocks.push(CaptureBlock {
                    session_name: session_order[want - 1].clone(),
                    content: prev_lines.join("\n"),
                });
            }
            current = Some(Vec::new());
            want += 1;
            continue;
        }
        if let Some(buf) = current.as_mut() {
            buf.push(line);
        }
    }
    if let Some(buf) = current.take() {
        blocks.push(CaptureBlock {
            session_name: session_order[want - 1].clone(),
            content: buf.join("\n"),
        });
    }
    blocks
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- session records ----------------------------------------------

    #[test]
    fn parses_normal_session_record() {
        let line =
            "mr-k3f9-1\tk3f9\t1\t/Users/sane\t1785271018\t0\tbackend checks\tcargo test --all";
        let rec = parse_session_line(line).unwrap();
        assert_eq!(
            rec,
            SessionRecord {
                session_name: "mr-k3f9-1".to_string(),
                group: "k3f9".to_string(),
                index: 1,
                cwd: "/Users/sane".to_string(),
                started: 1785271018,
                attached: false,
                label: Some("backend checks".to_string()),
                cmd: "cargo test --all".to_string(),
            }
        );
    }

    #[test]
    fn empty_label_is_none() {
        let line = "mr-k3f9-2\tk3f9\t2\t/Users/sane\t1785271018\t1\t\tnpm run dev";
        let rec = parse_session_line(line).unwrap();
        assert_eq!(rec.label, None);
        assert!(rec.attached);
    }

    #[test]
    fn command_containing_a_tab_lands_wholly_in_last_field() {
        // splitn(8, '\t') stops splitting once it has 8 pieces, so an
        // embedded tab in @mr_cmd (the 8th/last field) is preserved verbatim.
        let line = "mr-k3f9-1\tk3f9\t1\t/cwd\t1785271018\t0\t\tprintf 'a\tb'";
        let rec = parse_session_line(line).unwrap();
        assert_eq!(rec.cmd, "printf 'a\tb'");
    }

    #[test]
    fn command_containing_a_pipe_is_untouched() {
        let line = "mr-k3f9-1\tk3f9\t1\t/cwd\t1785271018\t0\t\tls | wc -l";
        let rec = parse_session_line(line).unwrap();
        assert_eq!(rec.cmd, "ls | wc -l");
    }

    #[test]
    fn trailing_newline_on_a_single_line_is_stripped() {
        let line = "mr-k3f9-1\tk3f9\t1\t/cwd\t1785271018\t0\t\techo hi\n";
        let rec = parse_session_line(line).unwrap();
        assert_eq!(rec.cmd, "echo hi");
    }

    #[test]
    fn crlf_line_ending_is_stripped() {
        let line = "mr-k3f9-1\tk3f9\t1\t/cwd\t1785271018\t0\t\techo hi\r";
        let rec = parse_session_line(line).unwrap();
        assert_eq!(rec.cmd, "echo hi");
    }

    #[test]
    fn completely_empty_input_is_an_error() {
        assert!(parse_session_line("").is_err());
    }

    #[test]
    fn row_with_too_few_fields_is_an_error() {
        let line = "mr-k3f9-1\tk3f9\t1\t/cwd";
        let err = parse_session_line(line).unwrap_err();
        assert!(err.0.contains("8 tab-separated fields"));
    }

    #[test]
    fn non_numeric_index_is_an_error() {
        let line = "mr-k3f9-1\tk3f9\tnot-a-number\t/cwd\t1785271018\t0\t\techo hi";
        assert!(parse_session_line(line).is_err());
    }

    #[test]
    fn parse_session_records_skips_blank_lines_and_empty_input_is_ok() {
        assert_eq!(parse_session_records("").unwrap(), vec![]);
        assert_eq!(parse_session_records("\n\n").unwrap(), vec![]);

        let raw = "mr-k3f9-1\tk3f9\t1\t/cwd\t1785271018\t0\t\tcmd one\n\
                    mr-k3f9-2\tk3f9\t2\t/cwd\t1785271018\t0\t\tcmd two\n";
        let recs = parse_session_records(raw).unwrap();
        assert_eq!(recs.len(), 2);
        assert_eq!(recs[0].session_name, "mr-k3f9-1");
        assert_eq!(recs[1].session_name, "mr-k3f9-2");
    }

    #[test]
    fn parse_session_records_propagates_first_malformed_row() {
        let raw = "mr-k3f9-1\tk3f9\t1\t/cwd\t1785271018\t0\t\tcmd one\n\
                    garbage-row\n";
        assert!(parse_session_records(raw).is_err());
    }

    // ---- pane records ----------------------------------------------------

    #[test]
    fn parses_running_pane_record() {
        let line = "mr-k3f9-1\t0\t\t\t\t54321\t78\t18\t0\t0";
        let rec = parse_pane_line(line).unwrap();
        assert_eq!(
            rec,
            PaneRecord {
                session_name: "mr-k3f9-1".to_string(),
                dead: false,
                dead_status: None,
                dead_signal: None,
                dead_time: None,
                pane_pid: Some(54321),
                width: 78,
                height: 18,
                history_size: 0,
                cursor_y: 0,
            }
        );
    }

    #[test]
    fn dead_pane_with_exit_status() {
        // research item 1: normal exit 42 -> pane_dead_status=42, signal empty.
        let line = "mr-k3f9-1\t1\t42\t\t1785271200\t54321\t78\t18\t5\t17";
        let rec = parse_pane_line(line).unwrap();
        assert!(rec.dead);
        assert_eq!(rec.dead_status, Some(42));
        assert_eq!(rec.dead_signal, None);
        assert_eq!(rec.dead_time, Some(1785271200));
    }

    #[test]
    fn dead_pane_with_signal_instead_of_status() {
        // research item 1: kill -TERM -> status empty, signal="term".
        let line = "mr-k3f9-1\t1\t\tterm\t1785271200\t54321\t78\t18\t5\t17";
        let rec = parse_pane_line(line).unwrap();
        assert!(rec.dead);
        assert_eq!(rec.dead_status, None);
        assert_eq!(rec.dead_signal, Some("term".to_string()));
    }

    #[test]
    fn pane_record_malformed_boolean_is_an_error() {
        let line = "mr-k3f9-1\tmaybe\t\t\t\t54321\t78\t18\t0\t0";
        assert!(parse_pane_line(line).is_err());
    }

    #[test]
    fn pane_record_too_few_fields_is_an_error() {
        let line = "mr-k3f9-1\t0\t\t\t\t54321";
        assert!(parse_pane_line(line).is_err());
    }

    #[test]
    fn parse_pane_records_empty_input_is_ok() {
        assert_eq!(parse_pane_records("").unwrap(), vec![]);
    }

    // ---- capture-marker splitting -----------------------------------------

    #[test]
    fn three_markers_three_screens() {
        let names = vec![
            "mr-k3f9-1".to_string(),
            "mr-k3f9-2".to_string(),
            "mr-k3f9-3".to_string(),
        ];
        let raw = "@@MR:mr-k3f9-1\nfirst pane line 1\nfirst pane line 2\n\
                    @@MR:mr-k3f9-2\nsecond pane line 1\n\
                    @@MR:mr-k3f9-3\nthird pane line 1\nthird pane line 2\n";
        let blocks = split_captures(raw, &names);
        assert_eq!(blocks.len(), 3);
        assert_eq!(blocks[0].session_name, "mr-k3f9-1");
        assert_eq!(blocks[0].content, "first pane line 1\nfirst pane line 2");
        assert_eq!(blocks[1].session_name, "mr-k3f9-2");
        assert_eq!(blocks[1].content, "second pane line 1");
        assert_eq!(blocks[2].session_name, "mr-k3f9-3");
        assert_eq!(blocks[2].content, "third pane line 1\nthird pane line 2");
    }

    #[test]
    fn screen_containing_a_line_that_looks_like_a_marker_is_not_split() {
        let names = vec!["mr-k3f9-1".to_string(), "mr-k3f9-2".to_string()];
        // The pane for job 1 happens to print a line that looks like a
        // marker for a session we are not expecting yet ("notreal"); it
        // must stay inside job 1's content rather than starting a new block.
        let raw = "@@MR:mr-k3f9-1\nsome output\n@@MR:notreal\nmore output\n@@MR:mr-k3f9-2\nfinal output\n";
        let blocks = split_captures(raw, &names);
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].session_name, "mr-k3f9-1");
        assert_eq!(blocks[0].content, "some output\n@@MR:notreal\nmore output");
        assert_eq!(blocks[1].session_name, "mr-k3f9-2");
        assert_eq!(blocks[1].content, "final output");
    }

    #[test]
    fn zero_markers_yields_no_blocks() {
        let blocks = split_captures("stray output with no markers at all", &[]);
        assert_eq!(blocks, vec![]);
    }

    #[test]
    fn marker_with_no_following_content() {
        let names = vec!["mr-k3f9-1".to_string()];
        let blocks = split_captures("@@MR:mr-k3f9-1", &names);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].session_name, "mr-k3f9-1");
        assert_eq!(blocks[0].content, "");
    }

    #[test]
    fn text_before_the_first_marker_is_discarded() {
        let names = vec!["mr-k3f9-1".to_string()];
        let raw = "leftover noise\n@@MR:mr-k3f9-1\nreal content\n";
        let blocks = split_captures(raw, &names);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].content, "real content");
    }
}
