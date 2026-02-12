# stash-mgr

A TUI tool for managing git stashes. Browse, preview, apply, and delete stashes in one tab — selectively create new stashes from specific files in the other.

Built with Rust using [ratatui](https://github.com/ratatui/ratatui) and [git2](https://github.com/rust-lang/git2-rs).

## Features

- **Two-tab interface** — switch between Create Stash and Manage Stashes with `Tab`
- **File-level selective stashing** — pick exactly which files to stash using checkboxes
- **Live diff preview** — syntax-colored, scrollable diff of the selected stash
- **Stash operations** — apply, pop, and drop with confirmation for destructive actions
- **Vim keybindings** — `j`/`k` for navigation, `h`/`l` for scrolling, `Ctrl+d`/`Ctrl+u` for half-page scroll
- **User-friendly errors** — plain English messages with actionable remedies
- **Performance safeguards** — diffs capped at 10K lines, file lists at 1K entries

## Building

Requires [Rust](https://rustup.rs/) (edition 2024).

```sh
cargo build --release
```

The binary will be at `target/release/stash-mgr`.

## Usage

Run from any directory inside a git repository:

```sh
stash-mgr
```

### Keybindings

#### Global

| Key | Action |
|-----|--------|
| `Tab` | Switch to next tab |
| `Shift+Tab` | Switch to previous tab |
| `q` | Quit |

#### Create Stash tab

| Key | Action |
|-----|--------|
| `Up` / `k` | Move selection up |
| `Down` / `j` | Move selection down |
| `Space` | Toggle file selection |
| `s` | Create stash from selected files |

When the message prompt appears:

| Key | Action |
|-----|--------|
| Type | Enter stash message |
| `Enter` | Confirm and create stash |
| `Esc` | Cancel |

#### Manage Stashes tab

| Key | Action |
|-----|--------|
| `Up` / `k` | Move selection up |
| `Down` / `j` | Move selection down |
| `Left` / `h` | Scroll diff up |
| `Right` / `l` | Scroll diff down |
| `Ctrl+u` | Scroll diff up half page |
| `Ctrl+d` | Scroll diff down half page |
| `a` | Apply selected stash (keep in list) |
| `p` | Pop selected stash (apply and remove) |
| `d` | Drop selected stash (with confirmation) |

## License

MIT
