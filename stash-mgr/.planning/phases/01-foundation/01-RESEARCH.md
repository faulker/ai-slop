# Phase 1: Foundation - Research

**Researched:** 2026-02-11
**Domain:** Rust TUI development with Git integration
**Confidence:** MEDIUM-HIGH

## Summary

Phase 1 builds a Rust terminal user interface (TUI) using the ratatui + crossterm stack with git2 for repository operations. The research confirms this is the current standard approach for Rust TUI development as of early 2026.

**Key findings:**
- Ratatui 0.29+ is stable with modern workspace architecture (0.30 reorganized into modular crates)
- Crossterm 0.29 is the recommended terminal backend with simple alternate screen/raw mode API
- git2 0.20.4 (libgit2 bindings) provides comprehensive git operations including stash management
- The Elm Architecture (TEA) pattern is officially recommended for structuring TUI apps
- Terminal panic handling is CRITICAL and has built-in support in ratatui
- Immediate-mode rendering means reconstructing UI from state every frame

**Primary recommendation:** Use the Component Template structure with panic hooks, TEA pattern for state management, and ratatui's convenience functions (init/restore) for terminal lifecycle.

## Standard Stack

### Core
| Library | Version | Purpose | Why Standard |
|---------|---------|---------|--------------|
| ratatui | 0.29+ | TUI framework | Official successor to tui-rs, active development, 0.30 modular architecture |
| crossterm | 0.29 | Terminal backend | Cross-platform (Windows/Unix), clean command API, officially supported by ratatui |
| git2 | 0.20.4 | Git operations | Official Rust bindings to libgit2, threadsafe, memory-safe, mature |

### Supporting
| Library | Version | Purpose | When to Use |
|---------|---------|---------|-------------|
| color-eyre | latest | Error reporting | Better panic messages in TUI context (recommended by ratatui docs) |
| strum | latest | Enum helpers | Tab navigation (derive Display, FromRepr, EnumIter for tab enums) |

### Alternatives Considered
| Instead of | Could Use | Tradeoff |
|------------|-----------|----------|
| git2 | gitoxide (pure Rust) | Gitoxide is newer/pure Rust but git2 is battle-tested with complete libgit2 API |
| crossterm | termion, termwiz | Termion is Unix-only; termwiz less common; crossterm is ratatui's primary backend |

**Installation:**
```bash
cargo add ratatui crossterm git2
cargo add color-eyre strum --features strum/derive
```

## Architecture Patterns

### Recommended Project Structure (Component Template)
```
src/
├── main.rs          # Entry point, terminal lifecycle
├── app.rs           # Application state (Model)
├── tui.rs           # Terminal wrapper, panic hooks
├── action.rs        # User action enum (Message)
├── components/      # UI components (View)
│   ├── mod.rs
│   ├── create_stash.rs
│   └── manage_stashes.rs
├── config.rs        # Configuration
└── errors.rs        # Error types
```

**Source:** [Ratatui Component Template](https://ratatui.rs/templates/component/project-structure/)

### Pattern 1: The Elm Architecture (TEA)
**What:** State management pattern with Model-Update-View separation
**When to use:** Any TUI with complex state (recommended for this phase)
**Example:**
```rust
// Model: Application state
struct App {
    selected_tab: SelectedTab,
    should_quit: bool,
}

// Update: Handle events and update state
impl App {
    fn update(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Tab => self.next_tab(),
            _ => {}
        }
    }
}

// View: Render UI from state
fn render(f: &mut Frame, app: &App) {
    let tabs = Tabs::new(vec!["Create Stash", "Manage Stashes"])
        .select(app.selected_tab as usize);
    f.render_widget(tabs, f.area());
}
```
**Source:** [The Elm Architecture | Ratatui](https://ratatui.rs/concepts/application-patterns/the-elm-architecture/)

### Pattern 2: Tab Navigation with Enum State
**What:** Use enums with derive macros for tab management
**When to use:** Fixed set of tabs (perfect for Create/Manage workflow)
**Example:**
```rust
// Source: https://ratatui.rs/examples/widgets/tabs/
#[derive(Default, Clone, Copy, Display, FromRepr, EnumIter)]
enum SelectedTab {
    #[default]
    #[strum(to_string = "Create Stash")]
    Create,
    #[strum(to_string = "Manage Stashes")]
    Manage,
}

impl SelectedTab {
    fn next(self) -> Self {
        let next = self as usize + 1;
        Self::from_repr(next).unwrap_or(self)
    }

    fn previous(self) -> Self {
        let prev = (self as usize).saturating_sub(1);
        Self::from_repr(prev).unwrap_or(self)
    }
}
```

### Pattern 3: Terminal Lifecycle Management
**What:** Proper initialization and cleanup with panic handling
**When to use:** ALWAYS (required to prevent terminal corruption)
**Example:**
```rust
// Source: https://ratatui.rs/recipes/apps/panic-hooks/
use std::panic::{set_hook, take_hook};
use crossterm::{execute, terminal::*};

pub fn init_panic_hook() {
    let original_hook = take_hook();
    set_hook(Box::new(move |panic_info| {
        let _ = restore_tui();
        original_hook(panic_info);
    }));
}

pub fn restore_tui() -> io::Result<()> {
    disable_raw_mode()?;
    execute!(stdout(), LeaveAlternateScreen)?;
    Ok(())
}

// Modern convenience functions (ratatui 0.28+)
fn main() -> Result<()> {
    init_panic_hook();
    let mut terminal = ratatui::init(); // Sets up terminal + panic hook
    // ... run app ...
    ratatui::restore(); // Cleanup
    Ok(())
}
```

### Pattern 4: Git Repository Discovery
**What:** Find git repo from current directory, searching up hierarchy
**When to use:** App needs to work from any subdirectory of a git repo
**Example:**
```rust
// Source: https://docs.rs/git2/latest/git2/struct.Repository.html
use git2::Repository;

fn find_repository() -> Result<Repository, git2::Error> {
    // Discovers repo starting from "." and searching parent directories
    Repository::discover(".")
}

// Alternative: open directly if you know the path
fn open_repository(path: &str) -> Result<Repository, git2::Error> {
    Repository::open(path)
}
```

### Pattern 5: Immediate Mode Rendering
**What:** Reconstruct entire UI every frame from application state
**When to use:** ALWAYS with ratatui (fundamental design principle)
**Example:**
```rust
// Source: https://ratatui.rs/concepts/rendering/
fn run_app(terminal: &mut Terminal<impl Backend>, app: &mut App) -> Result<()> {
    loop {
        // Render entire UI every frame
        terminal.draw(|f| {
            render_tabs(f, app);
            match app.selected_tab {
                SelectedTab::Create => render_create_stash(f, app),
                SelectedTab::Manage => render_manage_stashes(f, app),
            }
        })?;

        // Handle events
        if event::poll(Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                app.update(key);
            }
        }

        if app.should_quit {
            break;
        }
    }
    Ok(())
}
```

### Anti-Patterns to Avoid
- **Caching widgets between frames:** Widgets should be reconstructed each frame from state
- **Manual ANSI escape codes:** Use crossterm's command API instead
- **Forgetting panic hooks:** Terminal will be left in corrupted state on panic
- **Mixing terminal libraries:** Use only crossterm (or only termion), not both
- **Windows duplicate key events:** Filter for `KeyEventKind::Press` only on Windows

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| Terminal manipulation | Custom ANSI escape sequences | crossterm command API (`execute!`, `queue!`) | Cross-platform compatibility, handles edge cases, type-safe |
| Git operations | Subprocess calls to `git` CLI | git2 library bindings | Type-safe API, better error handling, no subprocess overhead |
| Layout calculations | Manual coordinate math | ratatui's constraint-based layouts | Automatic resizing, nested layouts, percentage/ratio support |
| Widget rendering | Custom buffer manipulation | Built-in ratatui widgets | Optimized rendering, accessibility, consistent styling |
| Event handling | Raw terminal input parsing | crossterm event API | Cross-platform key codes, mouse support, window resize detection |

**Key insight:** TUI development has deceptively complex edge cases (Windows vs Unix key events, terminal size changes, panic recovery). Using established libraries prevents 80% of potential bugs.

## Common Pitfalls

### Pitfall 1: Terminal State Not Restored on Panic
**What goes wrong:** App panics, terminal left in raw mode with alternate screen active, user's shell is unusable
**Why it happens:** Default panic handler doesn't know to restore terminal state
**How to avoid:** ALWAYS set up panic hooks before entering terminal mode (see Pattern 3)
**Warning signs:** Terminal shows garbled output after panic, no shell prompt visible
**Severity:** CRITICAL - app is unusable without this

### Pitfall 2: Windows Duplicate Key Events
**What goes wrong:** Every key press triggers action twice on Windows
**Why it happens:** Windows crossterm sends both Press and Release events; Unix only sends Press
**How to avoid:** Filter events: `if key.kind == KeyEventKind::Press { /* handle */ }`
**Warning signs:** Actions happening twice per key press, but only on Windows
**Source:** [Ratatui FAQ](https://ratatui.rs/faq/)

### Pitfall 3: Partial Frame Rendering
**What goes wrong:** UI flickers or shows artifacts from previous frames
**Why it happens:** Not redrawing entire frame in `terminal.draw()` closure
**How to avoid:** Render ALL widgets in every `draw()` call (immediate mode principle)
**Warning signs:** Old UI elements persist, layout corruption on resize
**Source:** [Ratatui FAQ](https://ratatui.rs/faq/)

### Pitfall 4: Buffer Out-of-Range Panics
**What goes wrong:** Panic when rendering widget outside terminal bounds
**Why it happens:** Not constraining render areas to valid buffer coordinates
**How to avoid:** Use `Rect::intersection()` or `u16::clamp()` before rendering
**Warning signs:** Panics on terminal resize, especially when shrinking window
**Source:** [Ratatui FAQ](https://ratatui.rs/faq/)

### Pitfall 5: Crossterm Version Conflicts
**What goes wrong:** Confusing type mismatch errors for crossterm types
**Why it happens:** Dependency tree has multiple crossterm versions
**How to avoid:** Pin crossterm version compatible with ratatui's version (0.29 for both as of research date)
**Warning signs:** Compiler errors about mismatched `KeyEvent` or `Event` types
**Source:** [Ratatui FAQ](https://ratatui.rs/faq/)

### Pitfall 6: Git Repository Not Found
**What goes wrong:** `Repository::open(".")` fails when run from subdirectory
**Why it happens:** `open()` requires exact path to `.git`, doesn't search
**How to avoid:** Use `Repository::discover(".")` instead - it searches parent directories
**Warning signs:** App works from repo root but fails from subdirectories
**Source:** [git2 Repository docs](https://docs.rs/git2/latest/git2/struct.Repository.html)

### Pitfall 7: Missing Font Characters
**What goes wrong:** Boxes or replacement symbols instead of expected characters
**Why it happens:** Terminal font doesn't support required glyphs (box-drawing, unicode)
**How to avoid:** Use ASCII fallbacks or document Nerd Fonts requirement
**Warning signs:** Tab dividers show as `?` or boxes, UI looks broken visually
**Source:** [Ratatui FAQ](https://ratatui.rs/faq/)

## Code Examples

Verified patterns from official sources:

### Terminal Initialization (Modern API)
```rust
// Source: https://docs.rs/crossterm/latest/crossterm/terminal/
use crossterm::{execute, terminal::*};
use std::io;

fn setup_terminal() -> io::Result<()> {
    execute!(io::stdout(), EnterAlternateScreen)?;
    enable_raw_mode()?;
    Ok(())
}

fn cleanup_terminal() -> io::Result<()> {
    disable_raw_mode()?;
    execute!(io::stdout(), LeaveAlternateScreen)?;
    Ok(())
}
```

### Event Loop with Tab Navigation
```rust
// Source: https://ratatui.rs/examples/widgets/tabs/
use crossterm::event::{self, Event, KeyCode, KeyEventKind};

fn handle_events(app: &mut App) -> io::Result<()> {
    if event::poll(Duration::from_millis(100))? {
        if let Event::Key(key) = event::read()? {
            // Windows compatibility: only handle Press events
            if key.kind == KeyEventKind::Press {
                match key.code {
                    KeyCode::Char('q') => app.should_quit = true,
                    KeyCode::Tab => app.next_tab(),
                    KeyCode::BackTab => app.previous_tab(),
                    _ => {}
                }
            }
        }
    }
    Ok(())
}
```

### Tabs Widget Rendering
```rust
// Source: https://ratatui.rs/examples/widgets/tabs/
use ratatui::widgets::{Tabs, Block, Borders};
use ratatui::style::{Style, Color, Modifier};

fn render_tabs(f: &mut Frame, app: &App) {
    let titles = vec!["Create Stash", "Manage Stashes"];
    let tabs = Tabs::new(titles)
        .block(Block::default().borders(Borders::ALL).title("Stash Manager"))
        .select(app.selected_tab as usize)
        .highlight_style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD)
        )
        .divider("|");

    f.render_widget(tabs, f.area());
}
```

### Git Stash Creation (Basic)
```rust
// Source: https://github.com/rust-lang/git2-rs/blob/master/src/stash.rs
use git2::{Repository, Signature, StashFlags};

fn create_stash(repo: &Repository, message: &str) -> Result<git2::Oid, git2::Error> {
    let sig = repo.signature()?; // Uses git config user.name/email

    // stash_save2: signature, message, flags
    repo.stash_save2(
        &sig,
        Some(message),
        Some(StashFlags::DEFAULT) // Tracked files only (matches git stash default)
    )
}
```

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| tui-rs | ratatui | 2023 fork | Active development resumed, ratatui is the successor |
| Monolithic ratatui crate | Workspace with ratatui-core | v0.30.0 (2025) | Widget devs use ratatui-core for stability; app devs use ratatui |
| Manual terminal setup | `ratatui::init()` / `restore()` | v0.28+ | One-line setup with automatic panic hooks |
| `Screen` API (crossterm 0.x) | Command API with `execute!`/`queue!` | crossterm 0.10+ (2019) | Type-safe, composable commands |
| `stash_save` | `stash_save2` | git2 evolution | More explicit signature and flags |

**Deprecated/outdated:**
- **tui-rs crate:** Unmaintained since 2023, use ratatui instead
- **crossterm Screen/RawScreen types:** Use `enable_raw_mode()` + `EnterAlternateScreen` command
- **AlternateScreen::to_alternate():** Old API, use `execute!(stdout(), EnterAlternateScreen)`

## Open Questions

1. **Stash message prompting UX**
   - What we know: Phase 1 requires prompting for stash message (per prior decisions)
   - What's unclear: Best ratatui pattern for text input widget (tui-textarea? custom component?)
   - Recommendation: Research text input patterns in Phase 2 planning; Phase 1 can scaffold with placeholder

2. **Git error handling strategy**
   - What we know: git2 returns Result types, color-eyre recommended for TUI errors
   - What's unclear: How to display git errors in TUI context (modal? status line? error tab?)
   - Recommendation: Start with simple error-to-string display, refine UX in later phases

3. **Terminal size constraints**
   - What we know: Must handle resize events gracefully
   - What's unclear: Minimum terminal size for usable UI (80x24? smaller?)
   - Recommendation: Test on 80x24, document minimum size, handle gracefully if smaller

## Sources

### Primary (HIGH confidence)
- [Ratatui Official Website](https://ratatui.rs/) - Main documentation, tutorials, patterns
- [Ratatui Component Template](https://ratatui.rs/templates/component/project-structure/) - Recommended project structure
- [The Elm Architecture | Ratatui](https://ratatui.rs/concepts/application-patterns/the-elm-architecture/) - TEA pattern guide
- [Panic Hooks Recipe | Ratatui](https://ratatui.rs/recipes/apps/panic-hooks/) - Terminal restoration on panic
- [Ratatui FAQ](https://ratatui.rs/faq/) - Common pitfalls and solutions
- [Tabs Widget Example](https://ratatui.rs/examples/widgets/tabs/) - Tab navigation implementation
- [crossterm terminal module docs](https://docs.rs/crossterm/latest/crossterm/terminal/) - Modern alternate screen/raw mode API
- [git2 Repository docs](https://docs.rs/git2/latest/git2/struct.Repository.html) - Repository discovery and operations
- [git2-rs stash.rs source](https://github.com/rust-lang/git2-rs/blob/master/src/stash.rs) - Stash API implementation details

### Secondary (MEDIUM confidence)
- [Ratatui v0.30.0 Highlights](https://ratatui.rs/highlights/v030/) - Recent breaking changes (modular workspace)
- [Ratatui GitHub](https://github.com/ratatui/ratatui) - Version history, issue discussions
- [crossterm GitHub](https://github.com/crossterm-rs/crossterm) - Cross-platform terminal library
- [git2-rs GitHub](https://github.com/rust-lang/git2-rs) - Official Rust git bindings

### Tertiary (LOW confidence - WebSearch only)
- crates.io version searches (versions verified but specific features may need docs.rs confirmation)

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH - Current stable versions verified, official ratatui documentation confirms crossterm as primary backend
- Architecture: HIGH - TEA pattern and Component Template officially documented by ratatui project
- Pitfalls: MEDIUM-HIGH - Most from official FAQ, some from issue discussions (panic hooks are CRITICAL and well-documented)
- Code examples: HIGH - All examples sourced from official docs or verified source code

**Research date:** 2026-02-11
**Valid until:** ~2026-04-11 (60 days - stack is mature and stable)

**Notes:**
- Ratatui ecosystem is actively developed but stable; minor version bumps unlikely to break patterns
- git2 0.20.x is mature; patch updates safe, minor updates may add features
- crossterm 0.29 is stable; coordinate with ratatui version for compatibility
- Windows duplicate key event issue is confirmed and documented workaround is simple
- Panic hook pattern is MANDATORY for production TUI apps
