# Architecture Research

**Domain:** Rust TUI git stash management tool
**Researched:** 2026-02-11
**Confidence:** HIGH (well-established patterns from ratatui ecosystem and gitui/lazygit architectures)

## Reference Architecture

### The Elm Architecture (TEA) for TUI
Used by: gitui, most modern ratatui apps

```
Event → Update(state) → View(state) → Terminal
  ↑                                       |
  └───────────────────────────────────────┘
```

- **Event loop** polls for terminal events (keys, resize, ticks)
- **Update** modifies app state based on events
- **View** renders entire UI from current state (stateless rendering)
- No retained mode — each frame is drawn fresh

## Component Architecture

```
┌─────────────────────────────────────────────────┐
│                    main.rs                       │
│  - Terminal setup/teardown                       │
│  - Panic hook                                    │
│  - Event loop                                    │
└──────────────┬──────────────────────────────────┘
               │
┌──────────────▼──────────────────────────────────┐
│                   App (state)                    │
│  - active_tab: Tab (Create | Manage)             │
│  - create_view: CreateStashState                 │
│  - manage_view: ManageStashState                 │
│  - input_mode: InputMode (Normal | TextInput)    │
│  - message: Option<String>                       │
└──────┬─────────────────────────┬────────────────┘
       │                         │
┌──────▼──────────┐    ┌────────▼─────────────────┐
│  CreateStashView │    │    ManageStashView        │
│                  │    │                           │
│  - file_list     │    │  - stash_list (left)      │
│  - selected      │    │  - diff_preview (right)   │
│  - checked[]     │    │  - selected_index         │
│  - expanded_file │    │                           │
│  - hunks[]       │    │                           │
│  - message_input │    │                           │
└──────┬──────────┘    └────────┬─────────────────┘
       │                         │
┌──────▼─────────────────────────▼────────────────┐
│                  Git Backend                     │
│  (git module — all git operations)               │
│                                                  │
│  - list_changed_files() → Vec<ChangedFile>       │
│  - get_file_hunks(path) → Vec<Hunk>              │
│  - create_stash(files, message) → Result         │
│  - list_stashes() → Vec<StashEntry>              │
│  - show_stash(index) → StashDiff                 │
│  - apply_stash(index) → Result                   │
│  - pop_stash(index) → Result                     │
│  - drop_stash(index) → Result                    │
└─────────────────────────────────────────────────┘
```

## Data Flow

### Create Stash Flow
```
1. User opens app → App::new() opens git repo
2. Create tab active → git.list_changed_files()
3. User navigates file list → update selected index
4. User toggles file checkbox → update checked set
5. User expands file → git.get_file_hunks(path) → show hunks
6. User toggles hunk checkbox → update checked hunks
7. User presses "stash" key → enter text input mode
8. User types message → App.message_input buffer
9. User confirms → git.create_stash(checked, message)
10. Success → refresh file list, show confirmation
```

### Manage Stashes Flow
```
1. Manage tab active → git.list_stashes()
2. User navigates stash list → update selected index
3. Selection changes → git.show_stash(index) → update diff preview
4. User presses apply → git.apply_stash(index) → refresh list
5. User presses pop → git.pop_stash(index) → refresh list
6. User presses delete → confirm → git.drop_stash(index) → refresh list
```

## Module Structure

```
src/
├── main.rs           # Entry point, terminal setup, event loop
├── app.rs            # App state struct, tab management
├── event.rs          # Event handling (key dispatch, input modes)
├── ui/
│   ├── mod.rs        # Main render function, tab bar
│   ├── create.rs     # Create stash view rendering
│   ├── manage.rs     # Manage stash view rendering
│   ├── diff.rs       # Diff/hunk rendering (shared)
│   └── input.rs      # Text input widget
├── git/
│   ├── mod.rs        # Public API (GitRepo struct)
│   ├── stash.rs      # Stash CRUD operations
│   ├── diff.rs       # Diff parsing, hunk extraction
│   └── status.rs     # Working directory status
└── types.rs          # Shared types (ChangedFile, Hunk, StashEntry)
```

## Key Design Decisions

### 1. Git Backend Abstraction
Wrap all git operations behind a `GitRepo` struct. Single place for error handling, easy to test.

### 2. Hybrid git2 + CLI Approach
- `git2`: mutations (create/apply/drop stash), repo status
- `git` CLI: diff display (simpler than libgit2 diff callbacks)

### 3. State Management
Single `App` struct owns all state. No RefCell, no Arc<Mutex>. TUI is single-threaded.

### 4. Event Handling
Match on (input_mode, key_event). Input mode prevents key conflicts.

## Component Boundaries

| Component | Owns | Talks To | Doesn't Touch |
|-----------|------|----------|---------------|
| main.rs | Terminal, event loop | App | Git directly |
| App | All state | GitRepo | Terminal |
| ui/* | Nothing (stateless) | Reads App state | Git, events |
| git/* | Repository handle | Filesystem, git CLI | UI, terminal |
| event.rs | Nothing | Mutates App state | Git, UI |

## Suggested Build Order

1. **Foundation:** Terminal setup, panic hook, event loop, tab switching
2. **Manage Tab:** List stashes, diff preview, apply/pop/drop
3. **Create Tab (file-level):** Status, file selection, stash creation
4. **Hunk Selection:** Diff parsing, hunk display, partial stash
5. **Polish:** Error handling, edge cases, cross-platform

**Rationale:** Read before write. Simple before complex. Foundation before features.
