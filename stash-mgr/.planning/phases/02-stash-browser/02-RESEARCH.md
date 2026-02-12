# Phase 2: Stash Browser - Research

**Researched:** 2026-02-11
**Domain:** Git stash browsing with interactive list UI and diff preview
**Confidence:** MEDIUM-HIGH

## Summary

Phase 2 implements an interactive stash browser using ratatui's List widget with stateful selection and a split-pane layout for diff preview. The research confirms that ratatui provides all necessary components: List + ListState for navigable stash lists, Layout constraints for side-by-side panels, Paragraph with scroll for diff display, and Clear widget for confirmation popups.

The git2 library provides complete stash enumeration via `stash_foreach()` callback, and stash operations (apply, pop, drop) are straightforward. The primary complexity is extracting diff content from stash commits, which requires navigating the commit graph to compare the stash commit's tree with its parent.

**Key findings:**
- ListState provides built-in navigation methods (select_next/previous/first/last) that handle edge cases
- Split pane layouts use nested Layout with Constraint::Percentage for responsive side panels
- Stashes are git commits, so diff generation uses standard tree-to-tree diff operations
- Confirmation dialogs use Clear widget + popup area calculation + boolean state flag
- Paragraph scroll takes (y_lines, x_chars) tuple applied after text wrapping

**Primary recommendation:** Use List with ListState for stash browser, horizontal Layout split (50/50 or 40/60) for list + diff preview, and simple Clear-based popup for delete confirmation. Extract stash diffs by comparing stash commit tree with first parent tree.

## Standard Stack

### Core
| Library | Version | Purpose | Why Standard |
|---------|---------|---------|--------------|
| ratatui | 0.29+ | TUI framework | Already in use from Phase 1, provides List/Layout/Paragraph/Clear widgets |
| crossterm | 0.29 | Terminal backend | Already in use from Phase 1 |
| git2 | 0.20.4 | Git operations | Already in use, provides stash_foreach, stash_apply, stash_pop, stash_drop |

### Supporting
| Library | Version | Purpose | When to Use |
|---------|---------|---------|-------------|
| color-eyre | latest | Error reporting | Already in use from Phase 1 |
| strum | latest | Enum helpers | Already in use from Phase 1 |

### Alternatives Considered
| Instead of | Could Use | Tradeoff |
|------------|-----------|----------|
| Built-in List | tui-widget-list crate | Built-in List is sufficient for simple vertical lists; tui-widget-list adds complexity for features we don't need |
| Built-in popup pattern | tui-popup crate | Built-in Clear widget + boolean flag is simpler for single confirmation dialog |
| Paragraph for diff | tui-scrollview | Paragraph.scroll() is sufficient for simple vertical scrolling; tui-scrollview adds features we don't need yet |

**Installation:**
No new dependencies required - all functionality available in existing stack.

## Architecture Patterns

### Recommended Project Structure
```
src/
├── main.rs          # Entry point, terminal lifecycle (Phase 1)
├── app.rs           # Application state (extend for stash browser)
├── tui.rs           # Terminal wrapper, panic hooks (Phase 1)
└── (future: components/ for modular UI components)
```

**Phase 2 extends App struct to add:**
- Stash list data (Vec of stash entries)
- ListState for selection tracking
- Diff preview state (current diff content, scroll position)
- Confirmation popup state (visible flag, pending action)

### Pattern 1: List Widget with StatefulWidget Rendering
**What:** Use List + ListState for interactive stash selection
**When to use:** Whenever you need selectable/navigable list (perfect for stash browser)
**Example:**
```rust
// Source: https://ratatui.rs/examples/widgets/list/
use ratatui::widgets::{List, ListItem, StatefulWidget};
use ratatui::widgets::ListState;

struct StashBrowser {
    items: Vec<StashEntry>,
    state: ListState,
}

// Navigation
impl StashBrowser {
    fn next(&mut self) {
        self.state.select_next();
    }

    fn previous(&mut self) {
        self.state.select_previous();
    }

    fn selected_index(&self) -> Option<usize> {
        self.state.selected()
    }
}

// Rendering
fn render_stash_list(f: &mut Frame, browser: &mut StashBrowser, area: Rect) {
    let items: Vec<ListItem> = browser.items
        .iter()
        .map(|entry| ListItem::new(entry.display_text()))
        .collect();

    let list = List::new(items)
        .highlight_style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))
        .highlight_symbol("> ");

    // Use StatefulWidget::render to disambiguate from Widget trait
    StatefulWidget::render(list, area, f.buffer_mut(), &mut browser.state);
}
```

### Pattern 2: Split Pane Layout (List + Preview)
**What:** Horizontal split for side-by-side list and diff preview
**When to use:** Two-panel interfaces where panels should resize together
**Example:**
```rust
// Source: https://ratatui.rs/concepts/layout/
use ratatui::layout::{Layout, Constraint, Direction};

fn render_browser(f: &mut Frame, browser: &mut StashBrowser) {
    // Split horizontally: 40% list, 60% preview
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(vec![
            Constraint::Percentage(40),
            Constraint::Percentage(60),
        ])
        .split(f.area());

    render_stash_list(f, browser, chunks[0]);
    render_diff_preview(f, browser, chunks[1]);
}
```

### Pattern 3: Scrollable Diff Preview with Paragraph
**What:** Use Paragraph.scroll() for navigating multi-line diff output
**When to use:** Displaying text content that exceeds visible area
**Example:**
```rust
// Source: https://docs.rs/ratatui/latest/ratatui/widgets/struct.Paragraph.html
use ratatui::widgets::Paragraph;

struct DiffPreview {
    content: String,
    scroll_offset: u16,
}

impl DiffPreview {
    fn scroll_down(&mut self) {
        self.scroll_offset = self.scroll_offset.saturating_add(1);
    }

    fn scroll_up(&mut self) {
        self.scroll_offset = self.scroll_offset.saturating_sub(1);
    }

    fn render(&self, f: &mut Frame, area: Rect) {
        let paragraph = Paragraph::new(self.content.as_str())
            .block(Block::default().borders(Borders::ALL).title("Diff Preview"))
            .scroll((self.scroll_offset, 0)); // (y_lines, x_chars)

        f.render_widget(paragraph, area);
    }
}
```

### Pattern 4: Git Stash Enumeration
**What:** Use stash_foreach callback to collect all stashes
**When to use:** Loading stash list on app start or refresh
**Example:**
```rust
// Source: https://github.com/rust-lang/git2-rs/blob/master/src/stash.rs
use git2::Repository;

struct StashEntry {
    index: usize,
    name: String,
    oid: git2::Oid,
}

fn load_stashes(repo: &Repository) -> Result<Vec<StashEntry>, git2::Error> {
    let mut stashes = Vec::new();

    repo.stash_foreach(|index, name, oid| {
        stashes.push(StashEntry {
            index,
            name: name.to_string(),
            oid: *oid,
        });
        true // continue iteration
    })?;

    Ok(stashes)
}
```

### Pattern 5: Stash Diff Extraction
**What:** Get diff by comparing stash commit tree with parent tree
**When to use:** Generating diff preview for selected stash
**Example:**
```rust
// Source: https://docs.rs/git2/latest/git2/struct.Commit.html
// Source: https://github.com/rust-lang/git2-rs/blob/master/examples/diff.rs
use git2::{Repository, Oid, DiffFormat};

fn get_stash_diff(repo: &Repository, stash_oid: Oid) -> Result<String, git2::Error> {
    // Stash is a commit - find it
    let stash_commit = repo.find_commit(stash_oid)?;

    // Get the stash commit's tree
    let stash_tree = stash_commit.tree()?;

    // Get the first parent (base commit before stash)
    let parent_tree = if stash_commit.parent_count() > 0 {
        stash_commit.parent(0)?.tree()?
    } else {
        // If no parent, compare to empty tree
        return Ok("(no parent)".to_string());
    };

    // Create diff between parent tree and stash tree
    let diff = repo.diff_tree_to_tree(Some(&parent_tree), Some(&stash_tree), None)?;

    // Format as unified diff patch
    let mut diff_text = String::new();
    diff.print(DiffFormat::Patch, |_delta, _hunk, line| {
        diff_text.push_str(std::str::from_utf8(line.content()).unwrap_or(""));
        true
    })?;

    Ok(diff_text)
}
```

### Pattern 6: Stash Operations (Apply/Pop/Drop)
**What:** Use git2's stash methods with proper error handling
**When to use:** Implementing apply, pop, drop actions
**Example:**
```rust
// Source: https://github.com/rust-lang/git2-rs/blob/master/src/stash.rs
use git2::{Repository, StashApplyOptions};

fn apply_stash(repo: &Repository, index: usize) -> Result<(), git2::Error> {
    let mut opts = StashApplyOptions::new();
    repo.stash_apply(index, Some(&mut opts))?;
    Ok(())
}

fn pop_stash(repo: &Repository, index: usize) -> Result<(), git2::Error> {
    let mut opts = StashApplyOptions::new();
    repo.stash_pop(index, Some(&mut opts))?;
    Ok(())
}

fn drop_stash(repo: &Repository, index: usize) -> Result<(), git2::Error> {
    repo.stash_drop(index)?;
    Ok(())
}
```

### Pattern 7: Confirmation Popup Modal
**What:** Use Clear widget + centered popup area for confirmation dialog
**When to use:** Preventing accidental destructive actions (drop stash)
**Example:**
```rust
// Source: https://ratatui.rs/examples/apps/popup/
use ratatui::widgets::Clear;
use ratatui::layout::{Rect, Layout, Constraint, Flex};

struct ConfirmPopup {
    visible: bool,
    message: String,
}

fn popup_area(area: Rect, percent_x: u16, percent_y: u16) -> Rect {
    let vertical = Layout::vertical([Constraint::Percentage(percent_y)])
        .flex(Flex::Center)
        .split(area);
    Layout::horizontal([Constraint::Percentage(percent_x)])
        .flex(Flex::Center)
        .split(vertical[0])[0]
}

fn render_confirm_popup(f: &mut Frame, popup: &ConfirmPopup, area: Rect) {
    if !popup.visible {
        return;
    }

    let popup_area = popup_area(area, 60, 20);

    // Clear the background
    f.render_widget(Clear, popup_area);

    // Render popup content
    let block = Block::default()
        .title("Confirm")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Red));

    let text = Paragraph::new(format!("{}\n\nPress 'y' to confirm, 'n' to cancel", popup.message))
        .block(block)
        .centered();

    f.render_widget(text, popup_area);
}
```

### Anti-Patterns to Avoid
- **Rebuilding stash list every frame:** Cache stash list in app state, only refresh when stash operations complete
- **Not handling empty stash list:** ListState.select() can be None, always check before using selected index
- **Ignoring apply/pop conflicts:** git2 operations can fail if conflicts occur; display error to user
- **Hardcoded split percentages:** Consider making pane sizes configurable or resizable later
- **Unbounded diff preview:** Very large diffs can freeze UI; consider truncation or pagination
- **Stale diff after stash operation:** Refresh stash list and clear diff preview after apply/pop/drop

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| List selection state | Manual index tracking with bounds checking | ListState.select_next/previous | Handles wrapping, edge cases, offset calculation automatically |
| Centered popup positioning | Manual rect calculation | Layout with Flex::Center | Handles terminal resize, maintains centering, cleaner code |
| Diff formatting | Custom line-by-line parsing | git2 Diff.print() callback | Handles hunks, context lines, binary files, encoding issues |
| Stash iteration | Manual reflog parsing | Repository.stash_foreach() | Type-safe, handles stash list format changes, error handling |
| Scrolling logic | Manual offset + visible line calculation | Paragraph.scroll() | Handles wrapping, terminal resize, bounds checking |

**Key insight:** Ratatui and git2 have solved the hard problems (selection state management, diff generation, layout constraints). Use their abstractions rather than reimplementing.

## Common Pitfalls

### Pitfall 1: Stash Index Invalidation After Operations
**What goes wrong:** Stash indices shift after pop/drop operations, causing wrong stash to be selected
**Why it happens:** Git renumbers stashes sequentially (stash@{0}, stash@{1}, etc.) when one is removed
**How to avoid:** After pop/drop, reload entire stash list and reset selection to None or adjust index
**Warning signs:** Selecting stash@{0} but seeing different content, operations affecting wrong stash
**Severity:** HIGH - can cause data loss if user drops wrong stash

### Pitfall 2: Empty List Selection
**What goes wrong:** Panic or wrong behavior when ListState.selected() returns None
**Why it happens:** No stashes exist, or user deselected with 'h' key in vim-style navigation
**How to avoid:** Always check `if let Some(index) = state.selected()` before using index
**Warning signs:** Crashes when stash list is empty, unwrap() panics
**Severity:** MEDIUM - crashes app but no data loss

### Pitfall 3: Diff Generation Performance
**What goes wrong:** UI freezes when generating diff for large stash
**Why it happens:** Synchronous diff generation blocks event loop
**How to avoid:** Initially accept blocking; optimize later with async or diff truncation if needed
**Warning signs:** App unresponsive when selecting certain stashes, large file changes
**Severity:** LOW for Phase 2 - document as known limitation, address if becomes problem

### Pitfall 4: Apply/Pop Conflict Handling
**What goes wrong:** git2 operations fail silently or panic when conflicts occur
**Why it happens:** Stash content conflicts with current working directory state
**How to avoid:** Check Result from stash_apply/pop, display error message to user
**Warning signs:** "Operation failed" with no explanation, terminal state corrupted on error
**Source:** [git2 Stash API](https://github.com/rust-lang/git2-rs/blob/master/src/stash.rs)
**Severity:** MEDIUM - confusing UX but no data loss

### Pitfall 5: Confirmation Popup State Leak
**What goes wrong:** Popup stays visible after action completes, or wrong stash is targeted
**Why it happens:** Forgetting to reset popup.visible = false, or storing stale stash index
**How to avoid:** Always reset popup state after action (confirm OR cancel)
**Warning signs:** Can't dismiss popup, popup shows wrong stash message
**Severity:** LOW - annoying but easy to fix

### Pitfall 6: Diff Preview Scroll Out of Bounds
**What goes wrong:** Scroll offset exceeds diff content length, showing blank screen
**Why it happens:** User scrolls down past end, or diff changes to shorter content
**How to avoid:** Clamp scroll offset to max(0, line_count - visible_height)
**Warning signs:** Blank diff preview when scrolling, can't scroll back to content
**Severity:** LOW - confusing UX but easy to recover

## Code Examples

Verified patterns from official sources:

### ListState Navigation Methods
```rust
// Source: https://docs.rs/ratatui/latest/ratatui/widgets/struct.ListState.html
let mut state = ListState::default();

// Selection methods
state.select(Some(0));              // Select specific index
state.select(None);                 // Deselect all
state.select_next();                // Next item (wraps to first if at end)
state.select_previous();            // Previous item (wraps to last if at start)
state.select_first();               // Jump to first item
state.select_last();                // Jump to last item

// Querying state
let index: Option<usize> = state.selected();
let offset: usize = *state.offset();

// Scrolling (for large lists)
state.scroll_down_by(5);
state.scroll_up_by(5);
```

### Horizontal Split Layout
```rust
// Source: https://ratatui.rs/concepts/layout/
use ratatui::layout::{Layout, Constraint, Direction};

let chunks = Layout::default()
    .direction(Direction::Horizontal)
    .constraints(vec![
        Constraint::Percentage(40),  // Left panel: stash list
        Constraint::Percentage(60),  // Right panel: diff preview
    ])
    .split(frame.area());

render_stash_list(frame, &mut app.stash_browser, chunks[0]);
render_diff_preview(frame, &app.diff_preview, chunks[1]);
```

### Git Stash Apply vs Pop
```rust
// Source: https://github.com/rust-lang/git2-rs/blob/master/src/stash.rs
use git2::{Repository, StashApplyOptions};

// Apply: keeps stash in list
fn apply_stash(repo: &Repository, index: usize) -> Result<(), git2::Error> {
    let mut opts = StashApplyOptions::new();
    repo.stash_apply(index, Some(&mut opts))
}

// Pop: applies AND removes from list (atomic)
fn pop_stash(repo: &Repository, index: usize) -> Result<(), git2::Error> {
    let mut opts = StashApplyOptions::new();
    repo.stash_pop(index, Some(&mut opts))
}

// Drop: removes without applying
fn drop_stash(repo: &Repository, index: usize) -> Result<(), git2::Error> {
    repo.stash_drop(index)
}
```

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| Manual list state | ListState with select_next/previous | ratatui 0.20+ | Simpler state management, fewer bugs |
| Flex enum in constraints | Flex::Center in Layout | ratatui 0.26+ | Easier popup centering |
| HighlightSpacing::WhenSelected | HighlightSpacing::Always | ratatui 0.23+ | Better visual consistency for lists |
| Manual diff parsing | Diff.print() callback | git2 always | Type-safe, handles edge cases |

**Deprecated/outdated:**
- **Manual wrapping list indices:** Use ListState.select_next/previous instead
- **Rect arithmetic for centering:** Use Layout with Flex::Center
- **git stash show subprocess:** Use git2 diff API

## Open Questions

1. **Diff preview truncation strategy**
   - What we know: Very large diffs could freeze UI during generation
   - What's unclear: What size threshold? How to indicate truncation?
   - Recommendation: Start without truncation, add "Diff too large (showing first 1000 lines)" if becomes problem

2. **Stash list refresh timing**
   - What we know: Need to reload after pop/drop operations
   - What's unclear: Should we also refresh periodically (external stash changes)?
   - Recommendation: Start with manual refresh after operations only, add periodic refresh if users request

3. **Error display strategy**
   - What we know: git2 operations return Result types
   - What's unclear: Modal error popup vs status line vs inline message?
   - Recommendation: Status line at bottom of screen for simple errors (matches Phase 1 help text location)

4. **Vim-style scroll bindings**
   - What we know: Users may expect j/k for list, Ctrl+d/u for diff preview
   - What's unclear: Does this conflict with future features? Worth the keybinding complexity?
   - Recommendation: Start with arrow keys only, add vim bindings if users request (easy to add later)

## Sources

### Primary (HIGH confidence)
- [Ratatui List Example](https://ratatui.rs/examples/widgets/list/) - List widget with selection
- [ListState API docs](https://docs.rs/ratatui/latest/ratatui/widgets/struct.ListState.html) - Navigation methods
- [Ratatui Layout Concepts](https://ratatui.rs/concepts/layout/) - Split pane patterns
- [Ratatui Popup Example](https://ratatui.rs/examples/apps/popup/) - Confirmation dialog pattern
- [Paragraph API docs](https://docs.rs/ratatui/latest/ratatui/widgets/struct.Paragraph.html) - Scroll method
- [git2-rs stash.rs source](https://github.com/rust-lang/git2-rs/blob/master/src/stash.rs) - Stash operations
- [git2 Commit API](https://docs.rs/git2/latest/git2/struct.Commit.html) - Commit tree/parent access
- [git2 Repository API](https://docs.rs/git2/latest/git2/struct.Repository.html) - find_commit, diff_tree_to_tree
- [git2 diff.rs example](https://github.com/rust-lang/git2-rs/blob/master/examples/diff.rs) - Diff formatting
- [libgit2 Stash API](https://libgit2.org/docs/reference/main/stash/index.html) - Underlying C API reference

### Secondary (MEDIUM confidence)
- [Git Stash Apply vs Pop](https://www.baeldung.com/ops/git-stash-pop-vs-stash-apply) - Semantic differences
- [Ratatui List vs Table](https://docs.rs/ratatui/latest/ratatui/widgets/struct.List.html) - Widget selection guidance

### Tertiary (LOW confidence - verified through multiple sources)
- WebSearch results on ratatui widgets cross-verified with official docs

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH - No new dependencies, all features available in Phase 1 stack
- Architecture: MEDIUM-HIGH - List/Layout/Paragraph patterns well-documented, stash diff extraction requires commit graph navigation
- Pitfalls: MEDIUM - Stash index invalidation is well-known git behavior, other issues inferred from API design
- Code examples: HIGH - All examples sourced from official docs or verified source code

**Research date:** 2026-02-11
**Valid until:** ~2026-04-11 (60 days - stack is mature and stable, stash API unlikely to change)

**Notes:**
- No new dependencies required - excellent for minimal complexity
- Stash as commit pattern is fundamental to git, won't change
- ListState API is stable, unlikely to have breaking changes
- Main uncertainty is UX decisions (error display, truncation) which are implementation details
- Diff preview performance is acceptable for typical use (defer optimization until proven necessary)
