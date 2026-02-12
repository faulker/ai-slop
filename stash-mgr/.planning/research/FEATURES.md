# Feature Landscape

**Domain:** Git stash management TUI
**Researched:** 2025-02-11

## Table Stakes

Features users expect from a git stash manager. Missing these = product feels incomplete.

| Feature | Why Expected | Complexity | Notes |
|---------|--------------|------------|-------|
| List existing stashes | Core purpose of a stash manager | Low | `git stash list` equivalent. Show stash index, branch, description. |
| Preview stash contents | Need to see what's in a stash before applying | Medium | `git stash show -p` equivalent. Display unified diff with syntax highlighting. |
| Apply stash | Restore stashed changes to working directory | Low | `git stash apply stash@{n}`. Need conflict handling. |
| Pop stash | Apply and remove in one operation | Low | `git stash pop stash@{n}`. Same as apply but deletes after. |
| Drop stash | Delete unwanted stashes | Low | `git stash drop stash@{n}`. Needs confirmation prompt. |
| Create stash from all changes | Save current work | Low | `git stash push -m "message"`. Standard stash operation. |
| Navigate with keyboard | TUI expectation | Medium | Vim-like keys (j/k, /, n/p) or arrow keys. Tab switching. |
| Repository auto-detection | Don't make user specify repo path | Low | Find .git from current directory. Walk up tree if needed. |

## Differentiators

Features that set this tool apart. Not expected, but highly valued.

| Feature | Value Proposition | Complexity | Notes |
|---------|-------------------|------------|-------|
| **File-level stash creation** | Only stash specific files, not entire working directory | Medium | `git stash push -m "msg" -- file1 file2`. Checkbox selection UI. |
| **Hunk-level stash creation** | Stash specific changes within a file | High | `git stash push -p` equivalent. Interactive hunk selection UI. Split screen with diff preview. |
| **Two-tab interface** | Separate "create" vs "browse" mental models | Medium | Tab 1: Working directory changes (create stashes). Tab 2: Existing stashes (browse/apply/delete). |
| **Live diff preview** | See exactly what will be stashed before confirming | Medium | Real-time diff rendering as files/hunks are selected. |
| **Search/filter stashes** | Find specific stash by content or message | Medium | Fuzzy search on stash messages. Highlight matching stashes. |
| **Syntax-highlighted diffs** | Easier to read code changes | High | Use syntect or similar for language-aware highlighting. Optional - can ship without. |
| **Branch context display** | Show which branch each stash was created on | Low | Already in `git stash list` output. Just parse and display. |

## Anti-Features

Features to explicitly NOT build.

| Anti-Feature | Why Avoid | What to Do Instead |
|--------------|-----------|-------------------|
| Stash editing | Git doesn't support editing stashes in place. Would require apply -> modify -> recreate flow that's error-prone. | Let user apply, modify in their editor, then create new stash. |
| Stash merging | No clear UX for merging multiple stashes. Git doesn't have primitives for this. | Let user apply multiple stashes sequentially. |
| Remote stash sync | Stashes are local by design. Syncing would require custom git plumbing and conflict resolution. | Document that stashes are local. If user needs shared state, use branches. |
| GUI/web interface | Scope is TUI only. GUI requires different framework entirely. | Stay focused on terminal users. |
| Stash visualization graphs | Stashes are independent snapshots, not a graph. Visualization adds complexity without value. | Show stashes as a simple chronological list. |
| Undo/redo for stash operations | Git doesn't track stash operation history. Would require custom state management. | Prompt for confirmation on destructive operations (drop, pop). |

## Feature Dependencies

```
Basic stash listing → Stash preview (need list before showing details)
Repository detection → All features (can't operate without repo)
Working dir diff → File selection → Hunk selection (progressive enhancement)
```

## MVP Recommendation

Prioritize for initial release:

1. **Repository detection** (blocker for everything)
2. **List existing stashes** (browse tab core)
3. **Preview stash contents** (browse tab core)
4. **Apply/pop/drop stash** (browse tab actions)
5. **Create stash from all changes** (creation tab basic mode)
6. **File-level stash creation** (creation tab differentiator)
7. **Keyboard navigation** (TUI table stakes)

Defer to v2:

- **Hunk-level selection:** High complexity, diminishing returns. Most users stash at file level.
- **Syntax highlighting:** Nice-to-have but not essential for usability.
- **Search/filter:** Valuable but can be manual scrolling initially.

## Feature Parity with Git CLI

| Git Command | Equivalent Feature | Status |
|-------------|-------------------|--------|
| `git stash list` | Browse tab stash list | MVP |
| `git stash show stash@{n}` | Stash preview pane | MVP |
| `git stash push` | Create stash (all changes) | MVP |
| `git stash push -- <files>` | File-level selection | MVP |
| `git stash push -p` | Hunk-level selection | Defer v2 |
| `git stash apply stash@{n}` | Apply action | MVP |
| `git stash pop stash@{n}` | Pop action | MVP |
| `git stash drop stash@{n}` | Drop action | MVP |
| `git stash clear` | Not implementing | Anti-feature (too destructive without undo) |
| `git stash branch` | Not implementing | Anti-feature (use git directly for branch ops) |

## User Workflow Mapping

### Workflow 1: Quick save current work
1. Launch stash-mgr
2. Tab to "Create" tab (default view: working directory changes)
3. Press 's' for "stash all"
4. Type message, confirm
5. Exit

**Equivalent CLI:** `git stash push -m "message"`
**Time saved:** Minimal, but TUI provides preview before committing.

### Workflow 2: Selectively stash files
1. Launch stash-mgr on "Create" tab
2. Use j/k to navigate files with changes
3. Press Space to toggle file selection
4. Press 's' to stash selected files
5. Type message, confirm

**Equivalent CLI:** `git stash push -m "message" -- file1 file2 file3`
**Time saved:** Significant - no need to type file paths, visual confirmation.

### Workflow 3: Find and apply old stash
1. Launch stash-mgr on "Browse" tab
2. Use j/k to navigate stash list
3. Preview pane shows diff for selected stash
4. Press 'a' to apply when found
5. Confirm

**Equivalent CLI:** `git stash list`, find index, `git stash apply stash@{n}`
**Time saved:** Moderate - visual browsing faster than reading list output.

### Workflow 4: Clean up old stashes
1. Browse tab
2. Navigate to unwanted stash
3. Press 'd' to drop
4. Confirm
5. Repeat

**Equivalent CLI:** `git stash list`, `git stash drop stash@{n}` (repeat)
**Time saved:** Significant - no index management, visual confirmation.

## Platform-Specific Considerations

All features are platform-agnostic (pure git operations + TUI rendering). No OS-specific code needed.

**Testing focus:**
- Windows: Path handling, git.exe detection
- Linux: Terminal compatibility (various emulators)
- macOS: Primary target, should "just work"

## Accessibility Considerations

- **Keyboard-only navigation:** Required (TUI has no mouse support by default)
- **Screen reader support:** Not feasible for TUI applications (terminal limitation)
- **Color blindness:** Use symbols + colors for state indication (not color alone)
- **Vim vs Arrow keys:** Support both navigation styles

## Performance Considerations

| Feature | Performance Impact | Mitigation |
|---------|-------------------|------------|
| Large stash list | Rendering lag if 100+ stashes | Paginate or virtual scrolling |
| Large diffs | Preview lag for huge files | Limit preview to first N lines |
| Large repos | git operations may be slow | Show loading indicator, use git2 async APIs if needed |
| Syntax highlighting | Expensive for large files | Make optional, cache results |

## Sources

Based on:
- Standard git stash CLI behavior (authoritative: git documentation)
- Common TUI application patterns (training data)
- User pain points with git CLI (ecosystem knowledge)

No external sources verified (tools unavailable during research).
