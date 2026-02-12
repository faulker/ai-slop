# Phase 4: Integration & Polish - Research

**Researched:** 2026-02-12
**Domain:** Error handling, cross-platform compatibility, performance optimization, graceful degradation
**Confidence:** HIGH

## Summary

Phase 4 focuses on making the application production-ready through comprehensive error handling, cross-platform compatibility verification, and performance optimization for edge cases. The research confirms that git2 provides detailed error classification through ErrorClass and ErrorCode enums, enabling user-friendly error messages for common failure scenarios (locked index, detached HEAD, no repository). Color-eyre integration with ratatui's panic hook ensures errors display properly after terminal restoration. Crossterm provides cross-platform support but requires explicit KeyEventKind filtering on Windows to handle duplicate key events. For performance, ratatui's differential rendering and Paragraph scrolling handle moderately large diffs efficiently, though extremely large content (>65K lines) may require pagination or lazy loading strategies.

**Key findings:**
- git2::ErrorCode provides 28 specific error variants including Locked, BareRepo, UnbornBranch, NotFound for precise error detection
- git2::ErrorClass categorizes errors into 35 classes (Repository, Index, Stash, etc.) for contextual error messages
- color-eyre with ratatui requires panic hook installation BEFORE terminal initialization to restore terminal state before displaying errors
- Crossterm sends duplicate key events on Windows (Press + Release) requiring KeyEventKind::Press filtering
- Ratatui's Buffer has maximum scrollback height of 65,535 lines (u16::MAX) for TestBackend
- Diff printing via callback pattern allows streaming large diffs without loading entire content into memory
- Platform-specific pathspec behavior requires explicit case-sensitivity flags rather than relying on OS defaults

**Primary recommendation:** Implement user-friendly error messages by matching git2::ErrorCode variants (Locked → "Git index is locked", NotFound → "Not a git repository", etc.), filter KeyEventKind::Press on all platforms for consistency, paginate or truncate diff content beyond 10K lines to prevent UI freezes, and verify all operations work on Windows/macOS/Linux through manual testing or CI.

## Standard Stack

### Core
| Library | Version | Purpose | Why Standard |
|---------|---------|---------|--------------|
| git2 | 0.20 | Error detection and handling | Provides ErrorCode/ErrorClass enums for precise error classification |
| color-eyre | 0.6 | User-friendly error reports | Standard for TUI apps, integrates with panic hooks for terminal restoration |
| ratatui | 0.29 | Cross-platform TUI rendering | Differential rendering handles performance, crossterm backend supports all platforms |
| crossterm | 0.29 | Platform-independent terminal I/O | Supports Windows, macOS, Linux; Windows 7+ compatibility |

### Supporting
| Library | Version | Purpose | When to Use |
|---------|---------|---------|-------------|
| ratatui-testlib | 0.1+ | Integration testing with PTY | OPTIONAL: For automated testing of TUI behavior (new library, not production-critical) |

### Alternatives Considered
| Instead of | Could Use | Tradeoff |
|------------|-----------|----------|
| color-eyre | anyhow, thiserror | color-eyre provides better formatting for end users; anyhow/thiserror better for libraries |
| Crossterm backend | termion, termwiz | Crossterm has best Windows support and most active maintenance |
| Manual error messages | Generic Error::to_string() | Custom messages are user-friendly, generic are technical/confusing |

**Installation:**
```bash
# No new dependencies required (color-eyre already in Cargo.toml)
# Already have: color-eyre = "0.6"
```

## Architecture Patterns

### Pattern 1: Git Error Handling with User-Friendly Messages
**What:** Convert git2::Error to helpful messages using ErrorCode pattern matching
**When to use:** All git2 operations (repository discovery, stash operations, status checks)
**Example:**
```rust
// Source: https://docs.rs/git2/latest/git2/enum.ErrorCode.html
// Source: https://docs.rs/git2/latest/git2/enum.ErrorClass.html

use git2::{ErrorClass, ErrorCode};

fn friendly_error_message(err: &git2::Error) -> String {
    match err.code() {
        ErrorCode::NotFound => {
            if err.class() == ErrorClass::Repository {
                "Not a git repository (or any parent up to mount point)".to_string()
            } else {
                format!("Not found: {}", err.message())
            }
        }
        ErrorCode::Locked => {
            "Git index is locked. Another git process may be running. \
             Try: rm -f .git/index.lock".to_string()
        }
        ErrorCode::BareRepo => {
            "This is a bare repository. Working directory operations are not supported.".to_string()
        }
        ErrorCode::UnbornBranch => {
            "Repository has no commits yet (unborn HEAD). \
             Create an initial commit first.".to_string()
        }
        ErrorCode::Conflict | ErrorCode::MergeConflict => {
            "Cannot perform operation: merge conflicts present. \
             Resolve conflicts first.".to_string()
        }
        ErrorCode::IndexDirty => {
            "Unsaved changes in index would be overwritten".to_string()
        }
        ErrorCode::Modified => {
            "Working directory has uncommitted changes".to_string()
        }
        _ => {
            // Fallback to git2's message for unexpected errors
            format!("Git operation failed: {}", err.message())
        }
    }
}

// Usage in main.rs
fn main() -> color_eyre::Result<()> {
    color_eyre::install()?;
    tui::install_panic_hook();

    let repo = match git2::Repository::discover(".") {
        Ok(repo) => repo,
        Err(e) => {
            eprintln!("Error: {}", friendly_error_message(&e));
            std::process::exit(1);
        }
    };

    // ... rest of app
}
```

### Pattern 2: Color-Eyre Integration with Terminal Restoration
**What:** Install panic hook BEFORE terminal initialization to restore terminal on panic
**When to use:** Application startup sequence
**Example:**
```rust
// Source: https://ratatui.rs/recipes/apps/color-eyre/

// In main.rs
fn main() -> color_eyre::Result<()> {
    // Install color-eyre FIRST
    color_eyre::install()?;

    // Install panic hook BEFORE terminal init
    tui::install_panic_hook();

    // Then initialize terminal
    let mut terminal = tui::init()?;

    // Run app with proper error handling
    let result = run(&mut terminal);

    // Restore terminal even on error path
    if let Err(err) = tui::restore() {
        eprintln!("Failed to restore terminal: {}", err);
    }

    // Return result (color-eyre displays formatted errors)
    result
}

// In tui.rs
pub fn install_panic_hook() {
    let original_hook = std::panic::take_hook();

    std::panic::set_hook(Box::new(move |panic_info| {
        // Restore terminal BEFORE printing panic
        let _ = restore();
        // Call original hook (color-eyre's pretty printer)
        original_hook(panic_info);
    }));
}

pub fn restore() -> color_eyre::Result<()> {
    disable_raw_mode()?;
    execute!(io::stdout(), LeaveAlternateScreen)?;
    Ok(())
}
```

### Pattern 3: Cross-Platform Key Event Handling
**What:** Filter KeyEventKind::Press to handle Windows duplicate events consistently
**When to use:** All keyboard event handling
**Example:**
```rust
// Source: https://ratatui.rs/faq/
// Source: https://github.com/crossterm-rs/crossterm/issues/347

use crossterm::event::{Event, KeyEvent, KeyEventKind};

fn handle_events(&mut self) -> Result<()> {
    if event::poll(Duration::from_millis(100))?
        && let Event::Key(key) = event::read()?
    {
        self.handle_key_event(key);
    }
    Ok(())
}

fn handle_key_event(&mut self, key: KeyEvent) {
    // CRITICAL: Filter for Press events only
    // Windows sends both Press and Release, Linux/macOS only Press
    if key.kind != KeyEventKind::Press {
        return;
    }

    // Now handle key normally
    match key.code {
        KeyCode::Char('q') => self.should_quit = true,
        // ... rest of handlers
        _ => {}
    }
}
```

### Pattern 4: Diff Streaming with Callback Pattern
**What:** Process diff line-by-line via callback to avoid loading entire diff into memory
**When to use:** Generating diff previews, especially for large stashes
**Example:**
```rust
// Source: https://github.com/rust-lang/git2-rs/blob/master/examples/diff.rs
// Source: https://docs.rs/git2/latest/git2/struct.Diff.html

fn get_stash_diff_streaming(
    repo: &Repository,
    stash_oid: git2::Oid,
    max_lines: usize,
) -> Result<String, git2::Error> {
    let stash_commit = repo.find_commit(stash_oid)?;
    let stash_tree = stash_commit.tree()?;
    let parent_tree = stash_commit.parent(0)?.tree()?;

    let diff = repo.diff_tree_to_tree(Some(&parent_tree), Some(&stash_tree), None)?;

    let mut diff_text = String::new();
    let mut line_count = 0;

    diff.print(DiffFormat::Patch, |_delta, _hunk, line| {
        // Check limit BEFORE processing line
        if line_count >= max_lines {
            diff_text.push_str("\n... (diff truncated, too large to display) ...\n");
            return false; // Stop iteration
        }

        // Add origin character
        let origin = line.origin();
        if matches!(origin, ' ' | '+' | '-' | 'B') {
            diff_text.push(origin);
        }

        // Convert content to UTF-8 (handle non-UTF8 gracefully)
        if let Ok(content) = std::str::from_utf8(line.content()) {
            diff_text.push_str(content);
            line_count += content.lines().count();
        } else {
            diff_text.push_str("<binary content>\n");
            line_count += 1;
        }

        true // Continue iteration
    })?;

    Ok(diff_text)
}
```

### Pattern 5: Graceful Degradation for Large Content
**What:** Detect and handle large diffs/files that would freeze UI
**When to use:** Diff loading, file list loading, any potentially unbounded data
**Example:**
```rust
// Based on ratatui buffer limits and performance research

const MAX_DIFF_LINES: usize = 10_000; // Well below u16::MAX (65,535)
const MAX_FILES_TO_DISPLAY: usize = 1_000;

impl App {
    fn update_diff_preview(&mut self) {
        self.diff_scroll = 0;
        if let Some(selected) = self.stash_list_state.selected()
            && let Some(stash) = self.stashes.get(selected)
        {
            // Try to load diff with limit
            self.diff_content = Self::get_stash_diff_with_limit(
                &self.repo,
                stash.oid,
                MAX_DIFF_LINES,
            );
        }
    }

    fn get_stash_diff_with_limit(
        repo: &Repository,
        stash_oid: git2::Oid,
        max_lines: usize,
    ) -> String {
        match Self::get_stash_diff_streaming(repo, stash_oid, max_lines) {
            Ok(diff) => diff,
            Err(e) => {
                // Graceful degradation on error
                format!("Failed to load diff: {}", friendly_error_message(&e))
            }
        }
    }

    fn load_working_files(repo: &Repository) -> Vec<FileEntry> {
        let mut opts = StatusOptions::new();
        opts.include_untracked(false);
        opts.include_ignored(false);

        let statuses = match repo.statuses(Some(&mut opts)) {
            Ok(s) => s,
            Err(_) => return Vec::new(), // Graceful failure
        };

        let mut files = Vec::new();
        for (idx, entry) in statuses.iter().enumerate() {
            // Limit number of files to prevent UI freeze
            if idx >= MAX_FILES_TO_DISPLAY {
                files.push(FileEntry {
                    path: format!("... ({} more files not shown)", statuses.len() - idx),
                    status: Status::empty(),
                    selected: false,
                });
                break;
            }

            // ... normal processing
        }

        files
    }
}
```

### Pattern 6: Repository State Validation
**What:** Check repository state before operations to provide better error messages
**When to use:** Before potentially failing operations (stash, apply, pop)
**Example:**
```rust
// Source: https://docs.rs/git2/latest/git2/struct.Repository.html

impl App {
    fn validate_repository_state(&self) -> Result<(), String> {
        // Check if HEAD is detached
        if self.repo.head_detached().unwrap_or(false) {
            return Err(
                "Repository is in detached HEAD state. \
                 Some operations may not work as expected.".to_string()
            );
        }

        // Check if repository is bare
        if self.repo.is_bare() {
            return Err(
                "This is a bare repository. Working directory operations are not available.".to_string()
            );
        }

        // Check if in the middle of a merge/rebase/etc
        let repo_state = self.repo.state();
        if repo_state != git2::RepositoryState::Clean {
            return Err(
                format!("Repository is in {:?} state. Complete or abort that operation first.", repo_state)
            );
        }

        Ok(())
    }

    fn apply_stash(&mut self) {
        // Validate state first
        if let Err(msg) = self.validate_repository_state() {
            self.status_message = Some(msg);
            return;
        }

        // ... proceed with apply
    }
}
```

### Anti-Patterns to Avoid
- **Using Error::to_string() for user messages:** Git error messages are technical; users need plain English explanations
- **Not checking KeyEventKind on Windows:** Results in double-registration of keypresses, confusing behavior
- **Loading unbounded diff content into String:** Large diffs (>10K lines) cause memory pressure and UI freezes
- **Forgetting to restore terminal before exit:** Panics or early returns leave terminal in broken state
- **Assuming pathspec behavior is consistent across platforms:** Windows may have different case-sensitivity; set explicit flags

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| Error message formatting | Custom error formatting, panic unwrapping | color-eyre with custom git2 error wrapper | color-eyre handles backtraces, span traces, pretty printing automatically |
| Cross-platform key event handling | Platform detection and conditional logic | KeyEventKind::Press filter for all platforms | Crossterm already handles platform differences; just filter consistently |
| Terminal restoration on panic | Manual signal handlers, atexit hooks | std::panic::set_hook before terminal init | Rust's panic hook is reliable and works across all exit paths |
| Diff truncation and pagination | Manual string manipulation, character counting | Diff callback with line counter + early return | Callback pattern is zero-copy, allows streaming, handles errors cleanly |

**Key insight:** Error handling and cross-platform compatibility have subtle edge cases (terminal state, key event timing, error context). Use battle-tested libraries (color-eyre, crossterm) rather than implementing platform-specific logic.

## Common Pitfalls

### Pitfall 1: Terminal Left in Raw Mode After Panic
**What goes wrong:** Application panics, terminal stays in raw mode (no echo, weird key behavior), user has to close terminal
**Why it happens:** Panic occurs after terminal initialization but before restoration; default panic handler doesn't know about terminal state
**How to avoid:** Install panic hook with `std::panic::set_hook` BEFORE calling `tui::init()`, hook must call `tui::restore()` before printing panic
**Warning signs:** After panic, terminal doesn't echo typed characters or displays garbage

### Pitfall 2: Duplicate Key Events on Windows
**What goes wrong:** On Windows, every key press triggers action twice (user sees double navigation, double character input)
**Why it happens:** Windows sends both KeyEventKind::Press and KeyEventKind::Release events; macOS/Linux only send Press
**How to avoid:** Filter for `key.kind == KeyEventKind::Press` at start of key handler; apply to ALL platforms for consistency
**Warning signs:** Windows users report "keys trigger twice" or "cursor jumps two items"

### Pitfall 3: Generic Error Messages Confuse Users
**What goes wrong:** User sees "Error: reference 'refs/heads/main' not found" when they meant to run in a git repo but were in wrong directory
**Why it happens:** Using git2::Error::to_string() or unwrap() exposes technical libgit2 error messages
**How to avoid:** Match on err.code() and err.class() to provide context-specific user-friendly messages
**Warning signs:** Bug reports with "error message doesn't make sense" or "what does that mean?"

### Pitfall 4: UI Freezes on Large Diffs
**What goes wrong:** User selects stash with 50K line diff, UI freezes for 5+ seconds or runs out of memory
**Why it happens:** Loading entire diff into String, rendering all lines even though only ~50 visible at once
**How to avoid:** Use callback pattern with line limit (e.g., 10K lines), truncate with message, or paginate
**Warning signs:** Performance testing reveals high memory usage or slow diff rendering

### Pitfall 5: Locked Index Error Without Explanation
**What goes wrong:** User sees "locked" error after Ctrl+C during git operation, doesn't know how to fix
**Why it happens:** Git leaves .git/index.lock file when interrupted; error code is Locked but message doesn't explain remedy
**How to avoid:** Detect ErrorCode::Locked specifically, provide message with solution: "Git index is locked. Try: rm -f .git/index.lock"
**Warning signs:** User reports "app says locked, what do I do?"

### Pitfall 6: Detached HEAD State Not Detected
**What goes wrong:** User in detached HEAD state tries to create stash, gets confusing error about "reference not found"
**Why it happens:** Stash messages include branch names, but detached HEAD has no branch
**How to avoid:** Check `repo.head_detached()` and provide clear message: "You are in detached HEAD state. Stashing still works, but stash will be saved without a branch reference."
**Warning signs:** Errors when user is in detached HEAD but other git operations work fine

## Code Examples

Verified patterns from official sources:

### Error Code Matching for User-Friendly Messages
```rust
// Source: https://docs.rs/git2/latest/git2/enum.ErrorCode.html
use git2::{ErrorCode, ErrorClass};

match err.code() {
    ErrorCode::NotFound if err.class() == ErrorClass::Repository => {
        eprintln!("Error: not a git repository (or any parent up to mount point)");
    }
    ErrorCode::Locked => {
        eprintln!("Error: git index is locked by another process");
        eprintln!("Try: rm -f .git/index.lock");
    }
    ErrorCode::BareRepo => {
        eprintln!("Error: this is a bare repository, working directory operations not supported");
    }
    _ => {
        eprintln!("Error: {}", err.message());
    }
}
```

### Panic Hook Installation for Terminal Restoration
```rust
// Source: https://ratatui.rs/recipes/apps/color-eyre/
pub fn install_panic_hook() {
    let original_hook = std::panic::take_hook();

    std::panic::set_hook(Box::new(move |panic_info| {
        // CRITICAL: restore terminal FIRST
        let _ = restore();
        // Then print panic (via color-eyre)
        original_hook(panic_info);
    }));
}
```

### KeyEventKind Filtering for Cross-Platform Consistency
```rust
// Source: https://ratatui.rs/faq/
use crossterm::event::{KeyEvent, KeyEventKind};

fn handle_key_event(&mut self, key: KeyEvent) {
    // Only handle Press events (Windows sends Press + Release)
    if key.kind != KeyEventKind::Press {
        return;
    }

    // ... rest of key handling
}
```

### Diff Print Callback with Line Limit
```rust
// Source: https://github.com/rust-lang/git2-rs/blob/master/examples/diff.rs
let mut line_count = 0;
const MAX_LINES: usize = 10_000;

diff.print(DiffFormat::Patch, |_delta, _hunk, line| {
    if line_count >= MAX_LINES {
        return false; // Stop iteration
    }

    let origin = line.origin();
    if matches!(origin, ' ' | '+' | '-') {
        print!("{}", origin);
    }
    print!("{}", str::from_utf8(line.content()).unwrap());

    line_count += 1;
    true // Continue
})?;
```

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| Generic panic messages | color-eyre with custom hooks | 2023-2024 | Better error context, backtraces, terminal-aware formatting |
| Ignore KeyEventKind | Filter for Press only | Crossterm 0.26+ (2023) | Windows compatibility without platform-specific code |
| Load all diff lines into String | Callback with line limit | Always best practice | Prevents OOM and UI freezes on large diffs |
| Technical error messages | User-friendly error messages | Always best practice | Users understand errors, can self-resolve common issues |
| Platform-specific code paths | Crossterm abstractions | ratatui 0.20+ (2023) | Single codebase works across platforms |

**Deprecated/outdated:**
- **tui-rs:** Replaced by ratatui (actively maintained fork)
- **Ignoring KeyEventKind:** Required as of crossterm 0.26+ for Windows compatibility
- **Unwrapping git2 errors:** Should always provide user-friendly context

## Open Questions

1. **Should we add telemetry to detect which errors occur most frequently?**
   - What we know: color-eyre provides error capture and formatting
   - What's unclear: Whether tracking error frequency helps prioritize UX improvements
   - Recommendation: Not for Phase 4 (no external dependencies, privacy concerns); manual testing sufficient

2. **What's the practical diff size limit before UX degrades?**
   - What we know: ratatui Buffer limited to 65K lines, rendering is fast for <10K lines
   - What's unclear: Real-world user tolerance for diff loading time
   - Recommendation: Set MAX_DIFF_LINES to 10,000 with "diff truncated" message; allow scrolling in truncated view

3. **Should detached HEAD be an error or just a warning?**
   - What we know: Git stash works in detached HEAD, just doesn't include branch name
   - What's unclear: Whether users find this confusing or acceptable
   - Recommendation: Show warning in status message but allow operation; matches git CLI behavior

4. **Is automated cross-platform testing feasible for this phase?**
   - What we know: ratatui-testlib enables PTY-based testing, but it's very new (v0.1.0)
   - What's unclear: Whether setting up CI for Windows/macOS/Linux testing is worth effort for this project
   - Recommendation: Manual testing on all three platforms for Phase 4; CI can be added later if project grows

## Sources

### Primary (HIGH confidence)
- [git2::ErrorCode - Rust Docs](https://docs.rs/git2/latest/git2/enum.ErrorCode.html) - All 28 error code variants
- [git2::ErrorClass - Rust Docs](https://docs.rs/git2/latest/git2/enum.ErrorClass.html) - All 35 error class categories
- [git2::Repository - Rust Docs](https://docs.rs/git2/latest/git2/struct.Repository.html) - Repository state methods
- [git2-rs diff.rs example](https://github.com/rust-lang/git2-rs/blob/master/examples/diff.rs) - Diff callback pattern
- [color-eyre with Ratatui](https://ratatui.rs/recipes/apps/color-eyre/) - Panic hook integration pattern
- [Ratatui FAQ](https://ratatui.rs/faq/) - KeyEventKind filtering for Windows
- [Crossterm GitHub](https://github.com/crossterm-rs/crossterm) - Cross-platform terminal library
- [Paragraph Widget Docs](https://docs.rs/ratatui/latest/ratatui/widgets/struct.Paragraph.html) - Scrolling behavior
- [Ratatui TestBackend Docs](https://docs.rs/ratatui/latest/ratatui/backend/struct.TestBackend.html) - Buffer limits (u16::MAX)

### Secondary (MEDIUM confidence)
- [Integration testing TUI applications in Rust](https://quantonganh.com/2024/01/21/integration-testing-tui-app-in-rust.md) - Testing patterns
- [ratatui-testlib Docs](https://docs.rs/ratatui-testlib/latest/ratatui_testlib/) - PTY-based testing library (v0.1.0, very new)
- [Crossterm Issue #347](https://github.com/ratatui/ratatui/issues/347) - Duplicate key events on Windows
- [git2-rs Issue #340](https://github.com/rust-lang/git2-rs/issues/340) - Windows path handling issues
- [Pathspec Docs](https://docs.rs/git2/latest/git2/struct.Pathspec.html) - Platform case-sensitivity behavior

### Tertiary (LOW confidence)
- None (all findings verified with primary sources)

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH - All libraries verified in use, versions confirmed from Cargo.toml and docs.rs
- Architecture: HIGH - Patterns verified from official examples (git2-rs examples, ratatui recipes) and documentation
- Pitfalls: MEDIUM-HIGH - Error handling issues derived from official error enum docs; Windows key event issue confirmed in ratatui FAQ; performance limits from TestBackend docs

**Research date:** 2026-02-12
**Valid until:** 90 days (error handling and cross-platform APIs are stable, slow-moving domain)
