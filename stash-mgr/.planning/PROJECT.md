# stash-mgr

## What This Is

A Rust TUI tool for managing git stashes. It provides two tabbed views: one for selectively creating stashes from working changes (file-level selection with checkboxes), and one for browsing, previewing diffs, applying, and deleting existing stashes. Built for developers who want a better interactive experience than the git stash CLI.

## Core Value

Users can selectively stash specific files from their working tree with a clear visual interface — the thing `git stash` makes awkward becomes fast and obvious.

## Requirements

### Validated

- ✓ Two-tab TUI: "Create Stash" and "Manage Stashes" with Tab key switching — v1.0
- ✓ Create Stash view shows modified and staged tracked files (no untracked) — v1.0
- ✓ File-level selection with checkboxes to choose which files to stash — v1.0
- ✓ Always prompt for a stash message before saving — v1.0
- ✓ Manage Stashes view shows stash list alongside a diff preview of the selected stash — v1.0
- ✓ Apply (unstash) a selected stash from the manage view — v1.0
- ✓ Pop (apply and remove) a selected stash from the manage view — v1.0
- ✓ Delete a selected stash with confirmation from the manage view — v1.0
- ✓ Vim keybindings (j/k, h/l) alongside arrow keys — v1.0 (delivered early from v2 backlog)
- ✓ User-friendly error messages for common failures — v1.0
- ✓ Performance safeguards for large diffs and file lists — v1.0

### Active

- [ ] Drill-down into individual hunks within a file for partial stashing
- [ ] Search/filter stashes by message text
- [ ] Syntax highlighting in diff preview
- [ ] Loading indicator for long git operations

### Out of Scope

- Untracked files in create view — matches default git stash behavior, keeps scope tight
- Editing file contents — this is a stash tool, not an editor
- Remote operations — purely local git stash management
- Merge conflict resolution — if unstash conflicts, defer to user's normal workflow
- Full git workflow (commit, push, pull) — scope creep, lazygit/gitui handle this
- GUI/web interface — TUI-only tool
- Configuration file — sensible defaults work well
- `git stash clear` (drop all) — too destructive without undo
- Stash branching (`git stash branch`) — niche, easy via CLI

## Context

Shipped v1.0 with 1,196 LOC Rust across 4 source files.
Tech stack: ratatui 0.29, crossterm 0.29, git2 0.20, color-eyre 0.6.
Application follows TEA (The Elm Architecture) pattern for state management.
Vim keybindings were delivered in v1.0 (originally v2 backlog).
Performance safeguards cap diffs at 10K lines and file lists at 1K entries.

## Constraints

- **Language**: Rust — user's choice, non-negotiable
- **Interface**: TUI only — no GUI, no web
- **Git interaction**: Should work with any standard git repository
- **Platform**: Cross-platform (macOS primary, Linux/Windows compatible)

## Key Decisions

| Decision | Rationale | Outcome |
|----------|-----------|---------|
| Rust TUI with ratatui + crossterm | User preference, performance, single binary distribution | ✓ Good — fast build, 1,196 LOC for full feature set |
| Tab-based navigation | Two distinct workflows in one tool, quick switching | ✓ Good — clean separation of concerns |
| Always prompt for stash message | Better stash hygiene, easier to find stashes later | ✓ Good — prevents unnamed stashes |
| Tracked files only | Matches git stash default, reduces noise | ✓ Good — simple and predictable |
| git2 crate for git operations | Native bindings, no subprocess overhead | ✓ Good — fast, reliable, cross-platform |
| 40/60 split for list/diff pane | Prioritize diff visibility over list | ✓ Good — diffs are the primary information |
| Explicit crossterm setup (not ratatui::init) | Full control over panic hook ordering | ✓ Good — guaranteed terminal restoration |
| 100ms event polling | Balance responsiveness and CPU usage | ✓ Good — responsive without spinning |
| Confirmation popup only for drop | Drop is destructive, pop preserves changes | ✓ Good — right balance of safety vs speed |
| 10K line diff cap, 1K file cap | Prevent UI freezes in large repos | ✓ Good — covers real-world cases |

---
*Last updated: 2026-02-12 after v1.0 milestone*
