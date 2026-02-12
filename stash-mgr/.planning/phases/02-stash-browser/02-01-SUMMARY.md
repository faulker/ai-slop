---
phase: 02-stash-browser
plan: 01
subsystem: ui-stash-browser
tags: [ui, git-integration, read-operations]

dependency-graph:
  requires:
    - 01-foundation-01 (tab navigation, terminal lifecycle)
  provides:
    - stash-list-rendering (navigable list of stashes)
    - stash-diff-preview (colorized diff display)
    - stash-navigation (keyboard-driven selection)
  affects:
    - app-state (added stash browsing state)

tech-stack:
  added:
    - git2::stash_foreach (stash enumeration)
    - git2::diff_tree_to_tree (diff generation)
    - ratatui::ListState (stateful list navigation)
    - ratatui split pane layout (horizontal constraints)
  patterns:
    - Stateful widget rendering with ListState
    - Git tree comparison for stash diffs
    - Syntax highlighting via styled Spans
    - Selection change triggers for diff updates

key-files:
  created: []
  modified:
    - src/app.rs: "Added StashEntry struct, load_stashes, get_stash_diff, split pane rendering, diff preview with syntax highlighting"

decisions:
  - Parse branch name from stash message format "WIP on branch: message"
  - Use 40/60 split for list/diff to prioritize diff visibility
  - Reset scroll to top on selection change for better UX
  - Use vim keybindings (j/k, h/l) alongside arrow keys
  - Add Ctrl+d/u for half-page scrolling (10 lines)

metrics:
  duration: 3
  tasks: 2
  commits: 2
  files_modified: 1
  completed: 2026-02-12
---

# Phase 02 Plan 01: Stash Browser Summary

**One-liner:** Navigable stash list with live diff preview using git2 tree comparison and ratatui split pane layout.

## What Was Built

Implemented a read-only stash browser in the Manage Stashes tab that displays all git stashes in a navigable list and shows a live diff preview when a stash is selected.

### Task 1: Stash Data Model and List Navigation (commit: 0a094a7)
- Created `StashEntry` struct with index, message, branch name, and git OID
- Implemented `load_stashes()` using `git2::Repository::stash_foreach()` to enumerate all stashes
- Added `ListState` to App for selection tracking
- Rendered stash list with format: `stash@{index}: message (branch)`
- Implemented arrow key navigation (Up/Down, j/k) with yellow highlight
- Added empty state message: "No stashes found. Use 'git stash' or the Create Stash tab to create one."

### Task 2: Split Pane and Diff Preview (commit: 6bd5206)
- Split Manage tab into horizontal panes: 40% stash list, 60% diff preview
- Implemented `get_stash_diff()` using `git2::diff_tree_to_tree()` to compare stash tree with parent
- Added diff content formatting with unified diff format (prepend origin characters)
- Rendered diff with syntax highlighting:
  - Lines starting with `+` in green
  - Lines starting with `-` in red
  - Lines starting with `@@` in cyan (hunk headers)
- Added diff scroll state and keybindings:
  - Left/Right (h/l): scroll by 1 line
  - Ctrl+d: scroll down 10 lines
  - Ctrl+u: scroll up 10 lines
- Reset scroll to top on selection change
- Updated help text to show scroll keybindings

## Verification Results

All verification criteria passed:

1. ✅ `cargo build` succeeded with zero errors
2. ✅ `cargo clippy` succeeded with zero warnings
3. ✅ `cargo test` compiles successfully
4. ✅ Manage Stashes tab renders stash list with proper formatting
5. ✅ Arrow keys navigate the stash list with visual highlight
6. ✅ Selecting a stash shows its diff in the right panel with syntax coloring
7. ✅ Empty stash list shows informative message
8. ✅ Diff preview scrolls with keybindings

## Success Criteria Achievement

- ✅ **BRWS-01**: Stash list displays index, message, and branch name for each stash
- ✅ **BRWS-02**: Up/Down arrow keys navigate stash list with yellow highlight
- ✅ **BRWS-03**: Right panel (60%) shows diff preview that updates on selection change
- ✅ **Empty state**: Handled gracefully with user-friendly message
- ✅ **Diff colorization**: All diff lines properly colored (+ green, - red, @@ cyan)

## Deviations from Plan

None - plan executed exactly as written. All requirements implemented as specified.

## Technical Notes

### Branch Name Parsing
The stash message format from git is typically "WIP on branch: hash message" or "On branch: message". The code extracts the branch name by finding text between "on " and ":" and defaults to "unknown" if parsing fails.

### Git2 Mutable Reference Requirement
The `git2::Repository::stash_foreach()` method requires a mutable reference, so the `load_stashes()` function signature was adjusted accordingly. This is expected behavior for git2's callback-based APIs.

### Diff Origin Characters
The diff generation properly prepends origin characters ('+', '-', ' ', 'B') to each line for correct unified diff display. File header lines (with origins like 'H' or 'F') are excluded from this prepending to maintain proper diff format.

## Self-Check: PASSED

### Files Created/Modified
✅ FOUND: src/app.rs (modified with 237 additions)

### Commits Verified
✅ FOUND: 0a094a7 (Task 1: stash list with navigation)
✅ FOUND: 6bd5206 (Task 2: split pane with diff preview)

All claimed artifacts exist and are properly committed.
