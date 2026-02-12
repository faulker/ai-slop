---
phase: 01-foundation
plan: 01
subsystem: foundation
tags: [tui, terminal-lifecycle, navigation, git-detection]

dependency_graph:
  requires: []
  provides: [runnable-tui, tab-navigation, terminal-safety, git-detection]
  affects: [all-future-features]

tech_stack:
  added:
    - ratatui 0.29 (TUI framework)
    - crossterm 0.29 (terminal backend)
    - git2 0.20 (git operations)
    - color-eyre 0.6 (error reporting)
    - strum 0.26 (enum helpers)
  patterns:
    - TEA (The Elm Architecture) for app state
    - Panic hook for terminal safety
    - Event polling with 100ms timeout

key_files:
  created:
    - Cargo.toml: Project manifest with all dependencies
    - src/main.rs: Entry point with panic hooks and git detection
    - src/tui.rs: Terminal lifecycle management (init/restore/panic hook)
    - src/app.rs: App state, event loop, tab navigation UI
  modified: []

decisions:
  - Used explicit crossterm setup instead of ratatui::init() for full panic hook control
  - Chose 100ms event polling for balance between responsiveness and CPU usage
  - Implemented BackTab (Shift+Tab) for reverse navigation in addition to Tab
  - Used KeyEventKind::Press check for Windows compatibility
  - Placed help text at bottom of content area rather than separate layout row

metrics:
  duration_minutes: 2
  tasks_completed: 2
  commits: 2
  files_created: 4
  completed_date: 2026-02-12
---

# Phase 1 Plan 01: Foundation Scaffold Summary

**One-liner:** Runnable Rust TUI with ratatui/crossterm, tab navigation between Create Stash and Manage Stashes, git repo detection, and panic-safe terminal restoration.

## What Was Built

Established the complete application skeleton for stash-mgr. The binary now:
- Detects if running in a git repository (exits gracefully with helpful error if not)
- Initializes terminal with alternate screen and raw mode
- Renders a tab bar with "Create Stash" and "Manage Stashes" tabs
- Handles Tab/BackTab for navigation and q to quit
- Restores terminal state on both normal exit and panics

This provides the foundation all subsequent phases build upon.

## Tasks Executed

### Task 1: Project scaffold with terminal lifecycle and git repo detection
**Commit:** c7a7472
**Files:** Cargo.toml, src/main.rs, src/tui.rs, src/app.rs (stub)

Initialized the Rust project with cargo init, added all required dependencies (ratatui, crossterm, git2, color-eyre, strum). Created tui.rs with init/restore functions and a panic hook that ensures terminal state is always restored, even on crashes. Created main.rs that:
1. Installs panic hook BEFORE terminal init
2. Checks for git repository (exits if not found)
3. Initializes terminal
4. Runs the app
5. Restores terminal on completion

**Verification:** cargo build succeeded, running outside git repo showed error message without entering TUI.

### Task 2: Application state, event loop, and tab navigation UI
**Commit:** 675045e
**Files:** src/app.rs

Implemented the full App struct following TEA pattern with:
- SelectedTab enum (Create, Manage) with next/previous methods using strum for iteration
- Event loop: draw frame -> poll for events (100ms timeout) -> handle keypresses
- Keyboard handling: q/Q quits, Tab cycles forward, BackTab cycles backward
- Tab bar rendering with yellow highlight on selected tab
- Content areas showing placeholder text for each tab
- Help line with keybindings

Fixed clippy warning about collapsible if statement. Verified cargo build, clippy (0 warnings), and cargo test all pass.

## Deviations from Plan

None - plan executed exactly as written. All must-have truths and artifacts are satisfied.

## Verification Results

All verification criteria from plan passed:

**FNDN-01 (Git repo detection):** Binary runs in git repo, exits with helpful error outside repo.
**FNDN-02 (Terminal lifecycle):** Alternate screen entered, raw mode enabled, both restored on exit. Panic hook verified via manual test.
**FNDN-03 (Tab navigation):** Tab switches between tabs, BackTab reverses, selected tab highlighted in yellow.
**FNDN-04 (Quit):** q key exits cleanly with terminal restored.

**Build verification:**
- `cargo build` ✓ (zero errors)
- `cargo clippy` ✓ (zero warnings)
- `cargo test` ✓ (compiles, 0 tests)

## Must-Have Validation

### Truths
✓ User can run binary from any git repo subdirectory and app launches
✓ User sees two tabs labeled "Create Stash" and "Manage Stashes"
✓ User can press Tab to switch active tab and see highlight move
✓ User can press q to quit and terminal is restored to normal state
✓ If app panics, terminal raw mode and alternate screen are restored

### Artifacts
✓ Cargo.toml contains ratatui, crossterm, git2, color-eyre, strum
✓ src/main.rs: panic hooks installed, terminal lifecycle, app run loop (30 lines)
✓ src/app.rs: App struct, SelectedTab enum, event handling, tab rendering (165 lines)
✓ src/tui.rs: init/restore/panic hook functions (45 lines)

### Key Links
✓ main.rs calls tui::init() and tui::restore()
✓ main.rs creates App and calls app.run()
✓ app.rs uses Repository::discover for git detection (via main.rs)
✓ app.rs renders Tabs widget with selected state
✓ tui.rs uses std::panic::set_hook to restore terminal on panic

## Next Steps

This plan establishes the working skeleton. Phase 1 is complete. Next phases can build:
- Phase 2: Stash browser in "Manage Stashes" tab
- Phase 3: File selector in "Create Stash" tab
- Phase 4: Stash operations (create, apply, drop)

## Self-Check: PASSED

Verifying all claimed artifacts exist:

- [x] Cargo.toml
- [x] src/main.rs
- [x] src/tui.rs
- [x] src/app.rs

Verifying commits exist:

- [x] c7a7472
- [x] 675045e

All files and commits verified successfully.
