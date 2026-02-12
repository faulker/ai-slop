# Domain Pitfalls

**Domain:** Rust TUI Git Stash Management Tool
**Researched:** 2026-02-11
**Confidence:** MEDIUM (based on training data and domain expertise; unable to verify with current sources)

> **Research Note:** This research was conducted without access to web search or current documentation tools. Findings are based on training data (knowledge cutoff January 2025) and established patterns in Rust TUI/git tool development. All pitfalls should be verified against current ratatui and git2-rs documentation during implementation.

## Critical Pitfalls

These mistakes cause rewrites, major performance issues, or broken functionality.

### Pitfall 1: Git Index Lock Contention
**What goes wrong:** Multiple git2-rs operations attempt to lock `.git/index` simultaneously, causing deadlocks or "index.lock already exists" errors. Particularly problematic during stash creation when reading index state while trying to write.

**Why it happens:**
- git2-rs Repository instances don't coordinate locks
- Async code spawns concurrent operations
- Not releasing locks properly before next operation

**Consequences:**
- Application hangs waiting for locks
- User loses work if crash occurs during partial lock state
- Corruption risk if process killed during write

**Prevention:**
- Use single Repository instance per operation sequence
- Never hold Repository across await points
- Implement explicit lock/unlock pattern for stash operations
- Add timeout for lock acquisition (fail fast, don't hang)
- Test with rapid user interactions (keyboard mashing)

**Detection:**
- App freezes during stash creation
- `.git/index.lock` files persist after crashes
- Race conditions in integration tests

**Phase mapping:** Phase 1 (core stash operations) must establish locking patterns

---

### Pitfall 2: Terminal State Not Restored on Panic
**What goes wrong:** Application panics without restoring terminal to canonical mode, leaving user's shell unusable (no echo, raw mode active, alternate screen stuck).

**Why it happens:**
- Panic occurs before cleanup Drop handlers run
- Signal handlers (SIGINT, SIGTERM) not properly configured
- Not using panic hooks to ensure cleanup

**Consequences:**
- User must close terminal window and restart shell
- Lost work if panic happens during stash creation
- Poor user experience, perceived as unstable

**Prevention:**
```rust
// Set panic hook during initialization
std::panic::set_hook(Box::new(|panic_info| {
    // Restore terminal before printing panic
    let _ = terminal::disable_raw_mode();
    let _ = execute!(io::stdout(), terminal::LeaveAlternateScreen);
    eprintln!("{}", panic_info);
}));
```
- Wrap main loop in proper error handling
- Test panic scenarios in development
- Use `crossterm::terminal::enable_raw_mode()` with matching disable in Drop
- Register signal handlers for graceful shutdown

**Detection:**
- Terminal becomes unresponsive after crash
- Can't see typed characters in shell after app exits
- `reset` command needed to restore terminal

**Phase mapping:** Phase 1 foundation must include terminal cleanup infrastructure

---

### Pitfall 3: Naive Diff Parsing Breaking on Binary Files
**What goes wrong:** Parser assumes all diffs are UTF-8 text, crashes or corrupts data when encountering binary files or non-UTF-8 content.

**Why it happens:**
- Using `String::from_utf8()` instead of `String::from_utf8_lossy()`
- Not checking `DiffBinary` variants in git2-rs
- Assuming `git diff` output is always valid UTF-8

**Consequences:**
- Panic when stashing repositories with images, PDFs, etc.
- Silent corruption of binary file diffs
- Can't stash legitimate binary changes

**Prevention:**
- Check `DiffLine::origin()` for binary indicators
- Use `from_utf8_lossy()` for display purposes
- Skip or specially handle binary files in hunk selection
- Test with repository containing: images, `.ico` files, compiled binaries
- Consider using git2's `Diff::print()` callback instead of parsing text

**Detection:**
- Crashes when selecting files with binary content
- Corrupted diffs in preview pane
- Test suite missing binary file cases

**Phase mapping:** Phase 2 (diff parsing) must handle binary files from start

---

### Pitfall 4: Unbounded Memory Growth from Large Diffs
**What goes wrong:** Loading entire diff of massive files into memory causes OOM crashes or unacceptable slowdowns.

**Why it happens:**
- Loading full file content for diff preview
- Not paginating/virtualizing large diffs in UI
- Storing entire stash history in memory

**Consequences:**
- App crashes on repos with large binary files or generated code
- UI freezes scrolling through huge diffs
- Poor performance on real-world repositories

**Prevention:**
- Stream diffs instead of loading entirely
- Implement virtual scrolling for diff view (render only visible lines)
- Set max size limits for preview (e.g., 10MB, show "[file too large]")
- Use git2's streaming APIs (`Repository::diff_index_to_workdir` with callbacks)
- Lazy-load stash details (only when user selects them)

**Detection:**
- Memory usage grows unbounded
- Scrolling performance degrades with file size
- Crashes on repositories with generated files (package-lock.json, etc.)

**Phase mapping:** Phase 2 (diff viewing) needs streaming/pagination architecture

---

### Pitfall 5: Race Conditions in Hunk Selection State
**What goes wrong:** Stash is created with wrong hunks because file modified between preview and stash creation, or UI state doesn't match actual file state.

**Why it happens:**
- Time gap between showing diff and creating stash
- File system watch not implemented
- Not validating hunk line numbers still match at stash time
- External processes modify files while TUI is running

**Consequences:**
- User thinks they stashed hunk A, actually stashed different code
- Silent data loss (hunks go missing)
- Confusing UX where preview doesn't match result

**Prevention:**
- Capture file content hash when showing diff
- Re-validate hunks match before executing stash
- Show warning if file changed: "File modified, refresh diff?"
- Consider optional file system watching (notify-rs)
- Make stash operation atomic: validate -> apply -> verify

**Detection:**
- Integration tests with file modifications between operations
- User reports "stashed wrong code"
- Diff preview shows different content than actual stash

**Phase mapping:** Phase 1 (create stash) needs validation before write

---

### Pitfall 6: Cross-Platform Path Separator Assumptions
**What goes wrong:** Hardcoding `/` or `\` in path handling breaks on Windows or Unix.

**Why it happens:**
- String manipulation instead of `std::path::Path`
- Assuming Unix paths everywhere
- git2-rs returns forward slashes but filesystem expects platform-native

**Consequences:**
- File selection broken on Windows
- Can't stash files in subdirectories on wrong platform
- Path display shows wrong separators

**Prevention:**
- Always use `std::path::PathBuf` and `Path` methods
- Use `join()`, `components()` instead of string concatenation
- Test on both Windows and Unix (CI matrix)
- Use git2's path helpers which normalize paths

**Detection:**
- Windows-specific bug reports
- Path-related panics only on one platform
- File tree rendering wrong on Windows

**Phase mapping:** Phase 1 foundation, but validate in Phase 2 (file tree UI)

---

## Moderate Pitfalls

These cause bugs or poor UX but don't require rewrites.

### Pitfall 7: Terminal Resize Not Handled
**What goes wrong:** UI layout breaks or doesn't redraw when terminal resized.

**Prevention:**
- Listen for `Event::Resize` in event loop
- Recalculate layout on every render
- Test by resizing terminal during operation
- Use ratatui's `Rect` properly (don't cache dimensions)

**Phase mapping:** Phase 2 (TUI layout) must handle resize from start

---

### Pitfall 8: Color/Unicode Assumptions Breaking Accessibility
**What goes wrong:** Relying only on color to convey state (selected vs unselected) makes app unusable for colorblind users or limited terminals.

**Prevention:**
- Use symbols + color: `[x]` checked, `[ ]` unchecked
- Provide high-contrast themes
- Test in 16-color mode, not just 256-color
- Use bold/italic/underline as redundant indicators

**Phase mapping:** Phase 2 (UI design) should include accessibility from start

---

### Pitfall 9: Ignoring Git Config and Hooks
**What goes wrong:** Tool bypasses user's git configuration (diff.algorithm, core.hooksPath) or pre-stash hooks, causing inconsistent behavior with CLI.

**Prevention:**
- Read relevant git config values via git2-rs
- Respect `$GIT_DIR/hooks` if user has custom hooks
- Document which configs are honored
- Consider `--no-verify` flag option

**Phase mapping:** Phase 3 (polish) should audit git config respect

---

### Pitfall 10: No Progress Indication for Long Operations
**What goes wrong:** App appears frozen during large stash operations, user can't tell if it crashed.

**Prevention:**
- Show spinner or progress bar for operations > 100ms
- Use git2's progress callbacks for fetch/clone (if added later)
- Allow cancellation (Ctrl+C) during long operations
- Display operation status in UI footer

**Phase mapping:** Phase 2 (UX) should include progress feedback

---

### Pitfall 11: Stash Message Encoding Issues
**What goes wrong:** Non-ASCII characters in stash messages get corrupted or cause display issues.

**Prevention:**
- Ensure stash messages stored as UTF-8
- Test with emoji, CJK characters, RTL text
- Use proper string width calculation for UI (unicode-width crate)
- Don't assume 1 char = 1 terminal column

**Phase mapping:** Phase 1 (stash creation) for storage, Phase 2 for display

---

### Pitfall 12: Input Handling Edge Cases
**What goes wrong:** Keyboard shortcuts conflict, paste events flood event queue, modifier keys not detected properly across platforms.

**Prevention:**
- Use crossterm's proper event parsing
- Debounce rapid events (paste protection)
- Test with: Ctrl+C, Ctrl+Z, Ctrl+D, Alt key combos
- Document keyboard shortcuts, check for conflicts
- Support both vi and emacs navigation patterns

**Phase mapping:** Phase 2 (input handling) needs comprehensive key mapping

---

## Minor Pitfalls

These cause annoyances but are easily fixed.

### Pitfall 13: Not Preserving Stash Index Info
**What goes wrong:** Losing track of whether stash includes staged changes separately.

**Prevention:**
- Use `git stash create` properly to preserve index
- Show visual distinction between stashes with/without index
- Test partial staging scenarios

---

### Pitfall 14: Poor Error Messages
**What goes wrong:** Showing raw git2-rs errors like "Error code -3" instead of actionable messages.

**Prevention:**
- Wrap git2::Error with user-friendly messages
- Provide suggestions: "Merge conflict detected. Resolve conflicts first."
- Include context: which file, which operation

---

### Pitfall 15: Startup Cost from Repository Scanning
**What goes wrong:** App takes 5+ seconds to start on large repos because scanning all files upfront.

**Prevention:**
- Lazy-load file tree (populate on demand)
- Show UI immediately, load data in background
- Cache repository state (invalidate on file changes)

---

### Pitfall 16: Testing Only on Clean Repositories
**What goes wrong:** Tool works in dev but breaks in real repos with conflicts, submodules, worktrees.

**Prevention:**
- Test scenarios: merge conflicts, rebase in progress, detached HEAD
- Test with submodules, git worktrees, sparse checkouts
- Create fixture repos with complex states

---

## Phase-Specific Warnings

| Phase Topic | Likely Pitfall | Mitigation |
|-------------|---------------|------------|
| Core stash operations | Git index lock contention (#1) | Single Repository pattern, explicit lock management |
| Core stash operations | Race conditions in hunk selection (#5) | Content validation before write |
| Foundation setup | Terminal cleanup on panic (#2) | Panic hook in initialization |
| Diff parsing | Binary file handling (#3) | Check DiffBinary, use from_utf8_lossy |
| Diff viewing | Unbounded memory from large files (#4) | Streaming/virtual scrolling from start |
| UI layout | Terminal resize (#7) | Event::Resize handling in event loop |
| File tree rendering | Path separator assumptions (#6) | Use std::path throughout |
| Input handling | Keyboard edge cases (#12) | Comprehensive key event testing |
| Stash message input | Encoding issues (#11) | UTF-8 validation, width calculation |
| Polish phase | Git config/hooks ignored (#9) | Audit config value reading |

---

## Rust + TUI Specific Gotchas

### Async in TUI Context
**Problem:** Mixing tokio async with blocking git2-rs and blocking terminal I/O causes executor starvation.

**Solution:** Keep TUI loop synchronous. If async needed (future network features), use separate runtime and channels to communicate with main loop.

---

### Borrow Checker vs Terminal Drawing
**Problem:** ratatui requires mutable borrow of terminal for each draw, conflicts with borrowing application state.

**Solution:** Structure state so terminal borrow is outermost, or use interior mutability (RefCell) carefully for UI state.

---

### git2-rs Thread Safety
**Problem:** `Repository` is not Send, can't use across threads naively.

**Solution:** Create Repository per thread, or use message passing with repository operations on dedicated thread.

---

## Validation Checklist (for implementer)

Before marking each phase complete:

- [ ] Test with repositories containing binary files
- [ ] Test terminal resize during operation
- [ ] Test Ctrl+C during long operation (graceful shutdown)
- [ ] Test on both Unix and Windows (path handling)
- [ ] Test with non-ASCII filenames and commit messages
- [ ] Test with 100MB+ files in diff
- [ ] Test rapid keyboard input (spam arrow keys)
- [ ] Test with repository in conflicted state
- [ ] Kill process during stash creation (verify no corruption)
- [ ] Verify terminal restored after panic (add test panic)

---

## Sources

**Confidence Note:** This research is based on training data and established patterns in Rust TUI development. Key sources from training data include:

- ratatui documentation and examples (knowledge up to Jan 2025)
- git2-rs API patterns and common issues (knowledge up to Jan 2025)
- Similar projects: gitui, lazygit (architecture patterns)
- Rust async/terminal interaction patterns

**Recommended verification:**
- Check ratatui 0.29+ changelog for breaking changes or new best practices
- Review git2-rs current issues for known pitfalls
- Search "rust tui terminal cleanup panic" for current solutions
- Review gitui source for real-world solutions to these problems

**Areas needing current validation:**
- ratatui API changes (training data from 2024)
- git2-rs thread safety updates
- crossterm vs termion current recommendations
- New Rust patterns for panic handling in TUI apps

This document provides architectural guidance but implementation details should be verified against current (2026) documentation.
