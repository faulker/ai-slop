# Phase 3: Stash Creator - Research

**Researched:** 2026-02-11
**Domain:** Git working directory status, selective file stashing, TUI checkbox/selection widgets
**Confidence:** HIGH

## Summary

Phase 3 implements the Create Stash tab, allowing users to see working directory changes and selectively stash chosen files. The research confirms that git2 provides comprehensive status inspection and supports selective stashing via pathspecs. For the UI, ratatui's stateful widgets combined with custom checkbox rendering or third-party widgets provide the interactive file selection experience.

**Key findings:**
- git2's `statuses()` with `StatusOptions` filters working directory changes (WT_MODIFIED, INDEX_MODIFIED flags)
- git2's `stash_save_ext()` with `StashSaveOptions::pathspec()` enables selective file stashing
- Ratatui requires custom checkbox implementation or using third-party widgets like `tui-checkbox` (v0.2.0)
- Input prompts can be implemented with ratatui's official pattern (character-based cursor tracking) or `tui-input` (v0.15.0)
- Modal popup pattern from Phase 2 applies to stash message prompts
- State synchronization between file list and selections is critical to avoid out-of-bounds errors

**Primary recommendation:** Use git2's status API with tracked-only filtering, implement custom stateful checkbox list widget (avoiding external dependencies), use Phase 2's popup pattern for message input, and leverage existing event handling patterns for space-to-toggle and arrow key navigation.

## Standard Stack

### Core
| Library | Version | Purpose | Why Standard |
|---------|---------|---------|--------------|
| git2 | 0.20.4 | Status inspection, stash creation | Comprehensive status API, pathspec support for selective stashing |
| ratatui | 0.29 | Checkbox list UI, input prompt | Stateful widgets for selection state, existing popup pattern |
| crossterm | 0.29 | Keyboard events | Space key toggle, arrow navigation already in use |

### Supporting
| Library | Version | Purpose | When to Use |
|---------|---------|---------|-------------|
| tui-checkbox | 0.2.0 | Pre-built checkbox widget | OPTIONAL: If custom implementation is too complex (note: requires external dependency) |
| tui-input | 0.15.0 | Text input handling | OPTIONAL: For complex input needs (current requirements fit custom solution) |

### Alternatives Considered
| Instead of | Could Use | Tradeoff |
|------------|-----------|----------|
| Custom checkbox list | tui-checkbox | tui-checkbox saves implementation time but adds dependency; custom gives full control and matches app patterns |
| Custom input field | tui-input | tui-input has more features but simple prompt fits ratatui's official pattern without extra dependency |
| Pathspec stashing | Index manipulation | Pathspec is simpler and directly supported; index manipulation is complex and error-prone |

**Installation:**
```bash
# No new dependencies required (use existing stack)
# Optional if choosing third-party widgets:
# cargo add tui-checkbox tui-input
```

## Architecture Patterns

### Pattern 1: Working Directory Status Inspection
**What:** Get modified and staged tracked files using git2's status API
**When to use:** On Create Stash tab activation and after stash creation
**Example:**
```rust
// Source: https://docs.rs/git2/latest/git2/struct.Repository.html
// Source: https://github.com/rust-lang/git2-rs/blob/master/examples/status.rs

fn load_working_directory_files(repo: &Repository) -> Vec<FileEntry> {
    let mut opts = StatusOptions::new();

    // Show tracked files only (no untracked)
    opts.include_untracked(false);
    opts.include_ignored(false);

    let statuses = repo.statuses(Some(&mut opts)).unwrap();
    let mut files = Vec::new();

    for entry in statuses.iter() {
        let status = entry.status();
        let path = entry.path().unwrap();

        // Check for working directory or index changes (tracked files only)
        if status.intersects(
            Status::WT_MODIFIED | Status::WT_DELETED |
            Status::INDEX_MODIFIED | Status::INDEX_NEW | Status::INDEX_DELETED
        ) {
            files.push(FileEntry {
                path: path.to_string(),
                status,
                selected: false,
            });
        }
    }

    files
}

struct FileEntry {
    path: String,
    status: git2::Status,
    selected: bool,
}
```

### Pattern 2: Selective File Stashing with Pathspecs
**What:** Create stash with only selected files using StashSaveOptions
**When to use:** After user confirms stash message and has selected files
**Example:**
```rust
// Source: https://github.com/rust-lang/git2-rs/blob/master/src/stash.rs

fn stash_selected_files(
    repo: &Repository,
    selected_files: &[String],
    message: &str,
) -> Result<git2::Oid, git2::Error> {
    let signature = repo.signature()?;
    let mut opts = StashSaveOptions::new(signature);

    // Add each selected file as a pathspec
    for file_path in selected_files {
        opts.pathspec(file_path);
    }

    repo.stash_save_ext(Some(&mut opts))
}
```

### Pattern 3: Stateful Checkbox List Widget
**What:** Custom widget combining List with checkbox state management
**When to use:** For file selection in Create Stash tab
**Example:**
```rust
// Source: https://ratatui.rs/concepts/widgets/ (StatefulWidget pattern)
// Source: Phase 2 implementation (ListState usage)

struct FileListState {
    list_state: ListState,
    files: Vec<FileEntry>,
}

impl FileListState {
    fn new(files: Vec<FileEntry>) -> Self {
        let mut list_state = ListState::default();
        if !files.is_empty() {
            list_state.select(Some(0));
        }
        Self { list_state, files }
    }

    fn toggle_selected(&mut self) {
        if let Some(idx) = self.list_state.selected() {
            if let Some(file) = self.files.get_mut(idx) {
                file.selected = !file.selected;
            }
        }
    }

    fn select_next(&mut self) {
        self.list_state.select_next();
    }

    fn select_previous(&mut self) {
        self.list_state.select_previous();
    }

    fn get_selected_files(&self) -> Vec<String> {
        self.files
            .iter()
            .filter(|f| f.selected)
            .map(|f| f.path.clone())
            .collect()
    }
}

// Rendering with checkbox symbols
fn render_file_list(frame: &mut Frame, state: &mut FileListState, area: Rect) {
    let items: Vec<ListItem> = state.files
        .iter()
        .map(|file| {
            let checkbox = if file.selected { "[✓]" } else { "[ ]" };
            let status_str = format_status(file.status);
            ListItem::new(format!("{} {} ({})", checkbox, file.path, status_str))
        })
        .collect();

    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title("Files to Stash"))
        .highlight_style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))
        .highlight_symbol("> ");

    frame.render_stateful_widget(list, area, &mut state.list_state);
}

fn format_status(status: git2::Status) -> &'static str {
    if status.contains(Status::INDEX_MODIFIED) {
        "staged"
    } else if status.contains(Status::WT_MODIFIED) {
        "modified"
    } else if status.contains(Status::INDEX_NEW) {
        "staged new"
    } else if status.contains(Status::WT_DELETED) {
        "deleted"
    } else {
        "changed"
    }
}
```

### Pattern 4: Text Input Popup for Stash Message
**What:** Modal popup with text input using ratatui's official pattern
**When to use:** After user presses 's' (stash) key with selected files
**Example:**
```rust
// Source: https://ratatui.rs/examples/apps/user_input/
// Source: Phase 2 popup implementation (layout, Clear widget)

struct MessageInputState {
    input: String,
    cursor_position: usize,
}

impl MessageInputState {
    fn enter_char(&mut self, c: char) {
        let idx = self.byte_index();
        self.input.insert(idx, c);
        self.move_cursor_right();
    }

    fn delete_char(&mut self) {
        if self.cursor_position == 0 {
            return;
        }
        let before = self.input.chars().take(self.cursor_position - 1);
        let after = self.input.chars().skip(self.cursor_position);
        self.input = before.chain(after).collect();
        self.move_cursor_left();
    }

    fn byte_index(&self) -> usize {
        self.input
            .char_indices()
            .map(|(i, _)| i)
            .nth(self.cursor_position)
            .unwrap_or(self.input.len())
    }

    fn move_cursor_left(&mut self) {
        self.cursor_position = self.cursor_position.saturating_sub(1);
    }

    fn move_cursor_right(&mut self) {
        self.cursor_position = self.clamp_cursor(self.cursor_position + 1);
    }

    fn clamp_cursor(&self, pos: usize) -> usize {
        pos.min(self.input.chars().count())
    }
}

// Rendering with cursor
fn render_message_input_popup(
    frame: &mut Frame,
    state: &MessageInputState,
    area: Rect,
) {
    // Center popup (60% width, 20% height) - reuse Phase 2 pattern
    let popup_area = popup_area(area, 60, 20);

    // Clear background
    frame.render_widget(Clear, popup_area);

    // Render input field
    let input = Paragraph::new(state.input.as_str())
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("Enter Stash Message")
        );
    frame.render_widget(input, popup_area);

    // Set cursor position
    frame.set_cursor_position(Position::new(
        popup_area.x + state.cursor_position as u16 + 1,
        popup_area.y + 1,
    ));
}

// Event handling (intercept all keys when popup visible, like Phase 2)
if self.show_message_input {
    match key.code {
        KeyCode::Char(c) => {
            self.message_input.enter_char(c);
        }
        KeyCode::Backspace => {
            self.message_input.delete_char();
        }
        KeyCode::Enter => {
            self.create_stash_with_message();
        }
        KeyCode::Esc => {
            self.cancel_message_input();
        }
        _ => {}
    }
    return; // Don't process other keys
}
```

### Pattern 5: State Management for Create Tab
**What:** Manage file list, selection state, and popup state
**When to use:** Throughout Create Stash tab lifecycle
**Example:**
```rust
// Extends existing App struct
pub struct App {
    // ... existing fields from Phase 1 & 2

    // Create Stash tab state
    file_list_state: Option<FileListState>,
    show_message_input: bool,
    message_input: MessageInputState,
}

impl App {
    fn activate_create_tab(&mut self) {
        // Load files when tab becomes active
        if self.file_list_state.is_none() {
            self.refresh_file_list();
        }
    }

    fn refresh_file_list(&mut self) {
        let files = load_working_directory_files(&self.repo);
        self.file_list_state = Some(FileListState::new(files));
    }

    fn initiate_stash_creation(&mut self) {
        if let Some(state) = &self.file_list_state {
            let selected = state.get_selected_files();
            if selected.is_empty() {
                self.status_message = Some("No files selected".to_string());
                return;
            }

            // Show message input popup
            self.show_message_input = true;
            self.message_input = MessageInputState::default();
        }
    }

    fn create_stash_with_message(&mut self) {
        if let Some(state) = &self.file_list_state {
            let selected = state.get_selected_files();
            let message = &self.message_input.input;

            match stash_selected_files(&self.repo, &selected, message) {
                Ok(_) => {
                    self.status_message = Some(format!(
                        "Stashed {} files successfully",
                        selected.len()
                    ));
                    self.refresh_file_list(); // Reload to show updated state
                }
                Err(e) => {
                    self.status_message = Some(format!("Stash failed: {}", e));
                }
            }

            // Close popup
            self.show_message_input = false;
        }
    }
}
```

### Anti-Patterns to Avoid
- **Out-of-bounds selection after file list refresh:** Always reset ListState selection when file list changes, or clamp to valid range
- **Forgetting to intercept all keys during popup:** Modal popup must capture ALL keyboard input to prevent unintended background actions (learned from Phase 2)
- **Using WT_NEW flag for tracked files:** WT_NEW is for untracked files; tracked modifications use WT_MODIFIED or INDEX_MODIFIED
- **Not reloading file list after stash creation:** Files may change state or disappear after stashing, stale list causes confusion

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| Status flag combinations | Custom status parsing logic | git2::Status bitflags with `contains()`/`intersects()` | Edge cases (renames, typechanges, conflicts) are complex |
| Cursor position in UTF-8 strings | Byte-based string indexing | Character index + byte_index() helper pattern | Multi-byte UTF-8 chars break naive indexing |
| Modal popup centering | Manual rectangle arithmetic | Ratatui's `Layout::flex(Flex::Center)` pattern | Handles edge cases (small terminals, odd dimensions) |
| Pathspec matching for stash | Manual file filtering before stash | `StashSaveOptions::pathspec()` | libgit2 handles gitignore rules, glob patterns, edge cases |

**Key insight:** Git status and stash operations have numerous edge cases (submodules, symlinks, renames, ignored files, etc.). libgit2 is battle-tested; custom implementations miss subtle behaviors.

## Common Pitfalls

### Pitfall 1: State Desync Between List and Selection
**What goes wrong:** File list changes (reload after stash) but ListState selection index becomes invalid, causing panic or wrong file selection
**Why it happens:** ListState holds numeric index but doesn't validate against list length
**How to avoid:** Always call `list_state.select(Some(0))` or `select(None)` after changing the file list; alternatively, clamp selection to `min(current, files.len() - 1)`
**Warning signs:** Panics when navigating after file operations, selection jumps unexpectedly

### Pitfall 2: Including Untracked Files Despite Requirements
**What goes wrong:** StatusOptions defaults to showing untracked files, violating Phase 3 requirement
**Why it happens:** git2's default behavior differs from requirement; forgetting to set `include_untracked(false)`
**How to avoid:** Explicitly set `opts.include_untracked(false)` in status configuration; filter by status flags (WT_MODIFIED, INDEX_MODIFIED) not WT_NEW
**Warning signs:** File list shows files that shouldn't be stashable, user confusion

### Pitfall 3: Empty Pathspec Stashing Entire Working Directory
**What goes wrong:** Calling `stash_save_ext()` with empty pathspec list stashes everything, ignoring user selection
**Why it happens:** libgit2 interprets empty pathspec as "match all files"
**How to avoid:** Validate selection is non-empty before calling stash; provide clear feedback if no files selected
**Warning signs:** User expects partial stash but gets full working directory stashed

### Pitfall 4: Input Field Cursor Position Off-by-One
**What goes wrong:** Cursor appears one character to the left or right of actual position
**Why it happens:** Forgetting to account for border width when setting cursor, or mixing character vs byte positions
**How to avoid:** Use `popup_area.x + cursor_position + 1` (the +1 accounts for left border); maintain character-based position, convert to bytes only for string operations
**Warning signs:** Cursor doesn't align with typed characters, looks unprofessional

### Pitfall 5: Popup Not Clearing Background
**What goes wrong:** Popup renders with previous UI elements visible underneath, creating visual noise
**Why it happens:** Forgetting to render `Clear` widget before popup content
**How to avoid:** Always render `Clear` first: `frame.render_widget(Clear, popup_area)` then render popup
**Warning signs:** Text from background bleeds through popup, hard to read

### Pitfall 6: Space Key Conflicts with Input
**What goes wrong:** Space key toggles checkbox when user is typing message in input field
**Why it happens:** Not checking popup visibility state before handling space key
**How to avoid:** Check `if self.show_message_input { return; }` early in key handler to intercept ALL keys during input
**Warning signs:** Typing space in message field causes unexpected behavior

## Code Examples

Verified patterns from official sources:

### Status Inspection with Tracked Files Only
```rust
// Source: https://docs.rs/git2/latest/git2/struct.StatusOptions.html
let mut opts = StatusOptions::new();
opts.include_untracked(false); // Key: exclude untracked files
opts.include_ignored(false);

let statuses = repo.statuses(Some(&mut opts))?;

for entry in statuses.iter() {
    let status = entry.status();

    // Filter for tracked file changes only
    if status.intersects(
        Status::WT_MODIFIED | Status::WT_DELETED |
        Status::INDEX_MODIFIED | Status::INDEX_NEW | Status::INDEX_DELETED
    ) {
        let path = entry.path().unwrap();
        println!("{}: {:?}", path, status);
    }
}
```

### Selective Stashing with Multiple Files
```rust
// Source: https://github.com/rust-lang/git2-rs/blob/master/src/stash.rs (test)
let signature = repo.signature()?;
let mut opts = StashSaveOptions::new(signature);

// Add multiple pathspecs
for file_path in selected_files {
    opts.pathspec(file_path);
}

let stash_oid = repo.stash_save_ext(Some(&mut opts))?;
println!("Created stash: {}", stash_oid);
```

### Checkbox Toggle with Space Key
```rust
// Source: Phase 2 patterns + ratatui ListState docs
KeyCode::Char(' ') if self.selected_tab == SelectedTab::Create => {
    if let Some(file_list) = &mut self.file_list_state {
        file_list.toggle_selected();
    }
}
```

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| tui-rs | ratatui 0.29+ | 2023 | ratatui is actively maintained, tui-rs is abandoned |
| Manual checkbox rendering | Custom StatefulWidget or tui-checkbox | 2025 | tui-checkbox (0.2.0) provides ready-made solution, but custom fits better for this app |
| stash_save() | stash_save_ext() with StashSaveOptions | git2 0.11+ | Pathspec support enables selective stashing |
| Byte-based string editing | Character-index with byte conversion | Always | Required for correct UTF-8 handling |

**Deprecated/outdated:**
- `stash_save()` without options: Use `stash_save_ext()` for flexibility and pathspec support
- Rendering without `Clear` widget for popups: Required as of ratatui 0.26+ for proper overlay behavior

## Open Questions

1. **Should we show status hints for each file (modified vs staged)?**
   - What we know: git2::Status bitflags distinguish WT_MODIFIED, INDEX_MODIFIED, etc.
   - What's unclear: Whether showing this detail improves UX or adds clutter
   - Recommendation: Show simple labels like "(modified)", "(staged)", "(deleted)" to help users understand what they're stashing

2. **What if all files become clean after a stash (empty list)?**
   - What we know: File list can become empty after successful stash
   - What's unclear: Best UX for empty state feedback
   - Recommendation: Show "No modified files — working directory clean" message, similar to Phase 2 empty stash list

3. **Should selecting files across staged and unstaged work?**
   - What we know: git2 pathspec stashing handles mixed staged/unstaged files
   - What's unclear: Whether this matches user expectations
   - Recommendation: Allow it (matches `git stash push <pathspec>` behavior), document in help text

## Sources

### Primary (HIGH confidence)
- [git2::Repository - Rust Docs](https://docs.rs/git2/latest/git2/struct.Repository.html) - statuses(), stash_save_ext() methods
- [git2::Status - Rust Docs](https://docs.rs/git2/latest/git2/struct.Status.html) - Status bitflags documentation
- [git2::StatusOptions - Rust Docs](https://docs.rs/git2/latest/git2/struct.StatusOptions.html) - Configuration methods
- [git2-rs/examples/status.rs](https://github.com/rust-lang/git2-rs/blob/master/examples/status.rs) - Official status example
- [git2-rs/src/stash.rs](https://github.com/rust-lang/git2-rs/blob/master/src/stash.rs) - StashSaveOptions implementation and tests
- [Ratatui User Input Example](https://ratatui.rs/examples/apps/user_input/) - Text input pattern
- [Ratatui Popup Example](https://ratatui.rs/examples/apps/popup/) - Modal popup pattern
- [StatefulWidget Documentation](https://docs.rs/ratatui/latest/ratatui/widgets/trait.StatefulWidget.html) - Stateful widget pattern
- [ListState Documentation](https://docs.rs/ratatui/latest/ratatui/widgets/struct.ListState.html) - Selection state management

### Secondary (MEDIUM confidence)
- [tui-checkbox GitHub](https://github.com/sorinirimies/tui-checkbox) - Third-party checkbox widget (v0.2.0, created Oct 2025)
- [tui-input GitHub](https://github.com/sayanarijit/tui-input) - Third-party input library (v0.15.0, Dec 2025)
- [Baeldung: Stashing Selected Files](https://www.baeldung.com/ops/git-stash-selected-files-changes) - Git stash pathspec behavior

### Tertiary (LOW confidence)
- None (all findings verified with primary sources)

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH - git2 and ratatui are confirmed in use from Phase 1 & 2, status and stash APIs verified in official docs
- Architecture: HIGH - Patterns verified from official examples and existing Phase 2 implementation
- Pitfalls: MEDIUM-HIGH - Most derived from official docs warnings and Phase 2 learnings, state sync issues are common TUI patterns

**Research date:** 2026-02-11
**Valid until:** 60 days (stable domain, slow-moving APIs)
