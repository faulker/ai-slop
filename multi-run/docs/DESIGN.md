# multi-run: Design and Implementation Plan

Status: spec, ready to implement. Companion to [`tmux-research.md`](./tmux-research.md), which is the
authoritative source for every tmux mechanic referenced here. This document cites that research by
item number and gotcha letter rather than re-deriving it. Read this document to implement; read the
research only when you need the underlying evidence.

Target: Rust 2024 edition, ratatui 0.30.2, crossterm 0.29.0, ansi-to-tui 8.0.1.

---

## 1. Overview and scope

`multi-run` runs N shell commands in parallel, each in its own tmux session rooted at the current
working directory, and shows all of them at once in a single ratatui TUI. You can watch them all
live, jump into any one for real interactive control, come back out, stop them, and leave them
running after the tool exits.

The core shape:

- Our ratatui app owns the terminal. It draws borders, titles, status, overlays.
- Each command becomes one tmux session on a **private socket** (`-L multi-run`) started with a
  **private config file** (research headline finding). One session = one window = one pane. That
  invariant holds everywhere and simplifies all targeting.
- Tile content comes from batched `capture-pane -p -e` (research item 3), decoded to ratatui `Text`
  with `ansi-to-tui`.
- Each pane is sized to exactly its tile's inner rect via `resize-window` (research item 4), so
  wrapping matches by construction and a capture returns exactly the rows we need.
- "Attach" leaves the alternate screen and hands the real terminal to `tmux attach-session`
  (research item 5), then restores on detach.
- Sessions outlive the process for free (research item 7). On exit we ask keep or kill.

### What it does

| Capability | Notes |
|---|---|
| Run N commands in parallel | From CLI args or an in-TUI compose screen |
| Grid view | 4 commands = 4 tiles, auto-arranged, paginated when they do not fit |
| List/detail view | Sessions listed on the left, selected session rendered large on the right |
| True attach | Full interactivity, including full-screen apps, via real `tmux attach-session` |
| Stop without losing output | `remain-on-exit` keeps the dead pane readable until explicitly dismissed |
| Add commands mid-run | New session joins the current group |
| Survive exit | Keep-or-kill prompt, `--list`, `--attach <id>` |
| Preflight | Detect tmux, parse its version, degrade or fail with a per-platform install hint |

### Non-goals

These are explicitly out of scope. Do not build them.

- **Not a multiplexer replacement.** No splits, no nested layouts inside a session, no window
  management. One command, one pane.
- **No general keystroke proxying.** We do not forward arbitrary keys through `send-keys`. Research
  item 6 is clear that this produces a subtly broken terminal. We support exactly `C-c`, `C-d`, and
  an optional literal line, and require a real attach for anything else.
- **No mouse forwarding into panes from the TUI.** `send-keys` cannot express mouse events at all
  (item 6), so a wheel notch in the grid scrolls our own tile buffer and never reaches the pane.
  Attaching is a real terminal handoff rather than a proxy, so the mouse does work there; tmux's
  `mouse on` handles it (section 5).
- **No remote hosts, no ssh, no Windows.** tmux is a hard dependency.
- **No supervision.** We do not restart crashed commands, retry, or apply backoff. `r` re-runs on
  demand and that is the whole feature.
- **No dependency graph or ordering between commands.** Everything starts at once. If you need a
  DAG, that is `orcha-ai`.
- **No log persistence to disk.** Output lives in tmux's scrollback and dies with the session.
- **No scrollback search** in v1.
- **We never write to `~/.tmux.conf`.** Ever. Our config lives in our own file (research headline).

---

## 2. CLI surface

### Parsing approach

**clap 4.5 with the derive API.** Reasoning:

1. We need a `--` separator that stops flag parsing, and clap gets that exactly right for free.
   Hand-rolling `--` handling plus flag/positional interleaving is where hand-rolled parsers
   actually break.
2. `--help` and `--version` output, unknown-flag errors, and "did you mean" suggestions are the
   whole reason a user's first five minutes go well or badly.
3. The dependency cost is one crate we would otherwise partially reimplement worse.

We do **not** use clap for the tmux version string; that gets a hand-rolled parser (section 10),
because pulling `regex` for one grammar is not worth it.

### Spec

```
multi-run [OPTIONS] [COMMAND]...

Arguments:
  [COMMAND]...  Shell commands to run, one per argument. Each runs via the user's
                $SHELL with that shell's profile loaded (e.g. .zshrc) in the
                current directory. If omitted, the compose screen opens.

Options:
  -c, --command <CMD>    Add a command. Repeatable. Use this when the command
                         starts with a hyphen and you do not want to use `--`.
  -l, --list             List active multi-run groups and exit.
  -a, --attach <ID>      Open the TUI on an existing group. `g1` opens the group;
                         `g1.2` opens it and immediately attaches to job 2.
      --kill <ID>        Kill a group (`g1`), a single job (`g1.2`), or
                         everything (`all`). Exits without opening the TUI.
      --view <MODE>      Initial view: `grid` or `list`. Default adapts to fit.
      --label <TEXT>     Human label for this group, shown in --list.
      --socket <NAME>    tmux socket name. Default `multi-run`. For tests and
                         for running isolated groups side by side.
      --tmux-conf <PATH> Override the generated tmux config path.
      --no-user-conf     Do not source ~/.tmux.conf into our private server.
      --restart-server   Kill our tmux server before starting, forcing the current
                         config to take effect. Destroys all existing groups.
  -V, --version          Print version, including the detected tmux version and tier.
  -h, --help             Print help.
```

### Commands with their own flags

Three supported forms, in order of what users will reach for:

```sh
# 1. Quote it. The command is one argv element, so its flags are just characters.
multi-run "cargo test -- --nocapture" "npm run dev -- --port 3000"

# 2. Separator. Everything after `--` is a positional command, hyphens and all.
multi-run -- "-x weird-first-char-command" "cargo build --release"

# 3. Explicit flag. Repeatable, unambiguous, good for scripts and aliases.
multi-run -c "cargo watch -x test" -c "npm run dev"
```

Positionals are declared with `allow_hyphen_values = true` so that form 2 works without clap
trying to interpret a leading hyphen as a flag. Commands from `-c` and from positionals are
concatenated in the order `-c` values first, then positionals. Duplicate command strings are
allowed; they get separate tiles and separate ids.

### Example invocations

```sh
multi-run "cargo watch -x test" "npm run dev" "cargo fmt --check"
multi-run                       # bare: opens the compose screen
multi-run --list
multi-run -a g1
multi-run -a g1.2               # open the group, jump straight into job 2
multi-run --kill g1
multi-run --kill all
multi-run --view list "make -j8" "tail -f /var/log/system.log"
```

### `--list` output

```
$ multi-run --list
GROUP  JOBS  DIR                      AGE    STATUS
g1     3     ~/Dev/ai-slop/multi-run  12m    2 running, 1 finished
  g1.1  running    cargo watch -x test
  g1.2  running    npm run dev
  g1.3  exit 0     cargo fmt --check
g2     2     ~/Dev/orcha-ai           2h     1 running, 1 signalled
  g2.1  running    pnpm test --watch
  g2.2  sig term   node scripts/seed.js

2 groups, 5 sessions.
Reattach:
  multi-run -a g1
  multi-run -a g2
```

With nothing running:

```
$ multi-run --list
No active multi-run sessions.
```

Exit codes: `0` for success including the empty case, `1` for a real tmux failure, `2` for a usage
error (clap's default), `127` when tmux is not installed.

---

## 3. Module layout

Organised so an agent can open exactly one file to change exactly one concern. Pure logic is
separated from IO so it is unit-testable without tmux.

```
multi-run/
  Cargo.toml
  README.md
  docs/
    DESIGN.md            this file
    tmux-research.md     verified tmux mechanics, the source of truth for section 5
  src/
    main.rs              entry point: preflight, CLI dispatch, terminal setup/teardown, panic hook
    cli.rs               clap derive types; GroupId/SessionId parse and display
    preflight.rs         tmux presence, `tmux -V` parse, version tiers, install hints
    error.rs             MrError/TmuxError (thiserror) and stderr classification
    model.rs             Job, Group, JobState, PaneFacts, and the state machine
    app.rs               App state, Action handling, modal state. No drawing, no argv.
    keymap.rs            (view, modal, KeyEvent) -> Option<Action>. Pure table.
    poll.rs              poller thread, poll scheduling, PollRequest/PollResult messages
    attach.rs            the attach/detach round trip, input-thread stop/restart
    tmux/
      mod.rs             Tmux struct: socket, conf path, argv construction, every invocation
      conf.rs            generate the private tmux.conf; path resolution; conf versioning
      parse.rs           tab-delimited -F record parsers and capture-marker splitting. Pure.
    ui/
      mod.rs             draw(frame, &App): dispatch by view mode, status bar, overlays
      layout.rs          grid dimension math, tile rects, pagination. Pure, no ratatui IO.
      tile.rs            one tile: border, title, status glyph, decoded content, padding
      overlay.rs         help, compose/add-command input, confirm, exit prompt, error toast
  tests/
    tmux_integration.rs  integration tests against a real tmux on a throwaway socket
```

Sixteen source files. Each has one job and a doc comment at the top saying what it owns and what it
must not do.

Dependencies:

```
ratatui       0.30.2
crossterm     0.29.0
ansi-to-tui   8.0.1
clap          4.5   (features = ["derive"])
anyhow        1
thiserror     2
unicode-width 0.2   (gotcha I: pad capture output to pane width ourselves)
```

No `regex`, no `chrono`, no `dirs`, no `tmux_interface` (research item 9 is emphatic about the
last one). Config paths come from `$XDG_CONFIG_HOME`/`$HOME`; relative times are formatted by hand
from a unix timestamp.

---

## 4. Data model

### Identifiers

```rust
/// A run group: everything created by one `multi-run` invocation.
/// Sequential `gN` (`g1`, `g2`, …). Short to type and easy to read off `--list`.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct GroupId(String);

/// One job inside a group. Displays as "g1.2".
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct JobId {
    pub group: GroupId,
    pub index: u16, // 1-based, monotonic within a group, never reused
}

impl JobId {
    /// tmux session name, e.g. "mr-g1-2". Purely for human readability in `tmux ls`;
    /// all logic keys off @mr_group / @mr_index (research item 2).
    pub fn tmux_name(&self) -> String { format!("mr-{}-{}", self.group.0, self.index) }
}
```

`Tmux::new_group_id()` picks the smallest unused sequential `gN` among the `@mr_group`
values currently visible in `list-sessions`. If the server is empty, that is `g1`. Legacy
four-character ids (`k3f9`) are still accepted by the parser so older live groups remain
attachable, but new groups are never assigned that form.

Parsing accepts these forms, in `cli.rs`:

| Input | Resolves to |
|---|---|
| `g1` | group `g1`, no specific job |
| `g1.2` | group `g1`, job 2 |
| `mr-g1-2` | group `g1`, job 2 (the raw tmux name, for copy-paste from `tmux ls`) |
| `k3f9` / `k3f9.2` / `mr-k3f9-2` | legacy four-character ids, same shapes |

### Job lifecycle

```rust
/// The observable facts about one pane, straight from a poll. Every field is
/// exactly what tmux reported; no interpretation happens here.
#[derive(Clone, Debug, Default)]
pub struct PaneFacts {
    pub dead: bool,             // #{pane_dead}
    pub dead_status: Option<i32>, // #{pane_dead_status}, empty when signalled
    pub dead_signal: Option<String>, // #{pane_dead_signal}, e.g. "term"
    pub dead_time: Option<u64>, // #{pane_dead_time}
    pub pane_pid: Option<u32>,  // #{pane_pid}
    pub width: u16,             // #{pane_width}
    pub height: u16,            // #{pane_height}
    pub history_size: u32,      // #{history_size}, cheap activity heuristic
    pub cursor_y: u16,          // #{cursor_y}, ditto
    pub session_attached: bool, // #{session_attached}
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Outcome {
    Exit(i32),        // pane_dead=1, pane_dead_status=N, no signal
    Signal(String),   // pane_dead=1, pane_dead_signal="term"/"kill"/..., no status
    Unknown,          // pane_dead=1 and both fields empty. Should not happen; surfaced, not hidden.
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum JobState {
    /// We issued new-session and have not yet seen this session in a poll.
    Starting,
    /// Present in list-sessions, pane_dead = 0.
    Running,
    /// Present in list-sessions, pane_dead = 1. Output is still readable.
    Finished { outcome: Outcome, at: Option<u64> },
    /// Present in our model but absent from list-sessions. Killed by something
    /// other than us. Tile stays with its last content and a "gone" marker until dismissed.
    Vanished,
    /// The user explicitly dismissed it. We ran kill-session; the tile is removed
    /// on the next frame. This state exists only so the removal is a normal
    /// state transition rather than a special case in the event handler.
    Dismissed,
}
```

#### State machine

| From | Event | To |
|---|---|---|
| (none) | `new-session` returned rc 0 | `Starting` |
| (none) | `new-session` returned rc != 0 | not added; error toast |
| `Starting` | seen in poll, `pane_dead=0` | `Running` |
| `Starting` | seen in poll, `pane_dead=1` | `Finished` (fast-exiting command; see below) |
| `Starting` | not seen for 3 consecutive polls | `Vanished` |
| `Running` | `pane_dead=1`, `pane_dead_status=N`, signal empty | `Finished { Exit(N) }` |
| `Running` | `pane_dead=1`, status empty, `pane_dead_signal=S` | `Finished { Signal(S) }` |
| `Running` | `pane_dead=1`, both empty | `Finished { Unknown }` |
| `Running` | absent from `list-sessions` | `Vanished` |
| `Finished` | absent from `list-sessions` | `Vanished` |
| `Finished` | user presses `d` | `Dismissed`, then `kill-session`, then removed |
| `Vanished` | user presses `d` | removed (no tmux call; it is already gone) |
| any | user presses `X` and confirms | `Dismissed`, `kill-session`, removed |

The `Starting -> Finished` edge matters and is safe: because `remain-on-exit on` is set globally in
the config *before any session exists* (research item 1), there is no race. A command that exits
instantly is observed as `pane_dead=1 pane_dead_status=N` on the very first poll, not as a missing
session. This is the single most important reason we use the `-f` config approach rather than
chaining `set-option -p` after `new-session`.

`Vanished` is deliberately not the same as `Finished`. Losing a session to an external
`tmux kill-server` is a different thing from a command exiting, and the UI says so.

### Jobs, groups, app state

```rust
pub struct Job {
    pub id: JobId,
    pub tmux_name: String,      // cached JobId::tmux_name()
    pub command: String,
    pub cwd: PathBuf,
    pub started_at: u64,        // unix seconds
    pub state: JobState,
    pub facts: PaneFacts,

    /// Last successful decode of capture-pane output, already padded/truncated
    /// to exactly `tile_size.1` lines. Owned so rendering never borrows from IO.
    pub content: Vec<ratatui::text::Line<'static>>,
    /// Bumped whenever `content` changes. Used to skip redraws and to drive the
    /// "new output" indicator for offscreen tiles.
    pub content_rev: u64,
    pub last_capture: Option<std::time::Instant>,

    /// The size we last told tmux this pane should be. Compared against the
    /// desired size so we only issue resize-window when it actually changed
    /// (each resize-window sets window-size manual; see gotcha E).
    pub tile_size: (u16, u16),

    /// Ratcheting stop escalation: 0 = none sent, 1 = C-c sent, 2 = TERM, 3 = KILL.
    pub stop_level: u8,
    pub last_stop: Option<std::time::Instant>,

    /// Scrollback offset in lines, 0 = live bottom. Only non-zero in list/detail view.
    pub scroll: u32,
}

pub struct Group {
    pub id: GroupId,
    pub label: Option<String>,
    pub cwd: PathBuf,
    pub created_at: u64,
    pub jobs: Vec<Job>,
    /// Monotonic. Never reuses an index, so a job id always identifies one command run.
    pub next_index: u16,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ViewMode { Grid, ListDetail }

/// Which region has keyboard focus. In Grid there is only one region.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Pane { List, Detail }

#[derive(Clone, Copy)]
pub struct Focus {
    pub job: usize,   // index into Group::jobs
    pub pane: Pane,   // meaningful only in ListDetail
}

pub enum Modal {
    None,
    Help,
    /// Bare-launch compose screen, and the mid-run "add command" prompt.
    /// `launch_on_submit` distinguishes them.
    Compose { input: String, queued: Vec<String>, launch_on_submit: bool },
    Confirm { prompt: String, action: Box<Action>, selected: usize },
    /// Quit was requested and live jobs exist. `selected`: 0 = keep, 1 = kill.
    ExitPrompt { selected: usize },
    /// Transient, auto-clears after 5s. Full history reachable with `!`.
    Toast { text: String, at: std::time::Instant, is_error: bool },
    ErrorLog { scroll: u16 },
}

pub struct App {
    pub group: Group,
    pub view: ViewMode,
    pub focus: Focus,
    pub modal: Modal,
    pub page: usize,              // grid pagination
    pub geometry: Geometry,       // from ui::layout, recomputed on resize
    pub tier: preflight::Tier,    // tmux capability tier
    pub errors: Vec<String>,      // bounded ring, 200 entries
    pub pending_resize: Option<std::time::Instant>,
    pub should_quit: bool,
    pub redraw: bool,
}

/// Every state change goes through one of these. keymap.rs produces them,
/// app.rs consumes them. Nothing else mutates App.
pub enum Action {
    FocusNext, FocusPrev, FocusLeft, FocusRight, FocusFirst, FocusLast, FocusIndex(usize),
    NextPage, PrevPage,
    ToggleView, SetView(ViewMode),
    Attach,
    Stop,            // escalating C-c / TERM / KILL
    SendEof,
    Dismiss,         // finished or vanished job only
    DismissAllFinished,
    KillFocused,     // any state, with confirmation
    Rerun,
    ScrollUp(u16), ScrollDown(u16), ScrollBottom,
    OpenCompose, OpenHelp, OpenErrorLog, CloseModal,
    ComposeChar(char), ComposeBackspace, ComposeClear, ComposeSubmit, ComposeCommit,
    ConfirmYes, ConfirmNo,
    SelectPrev, SelectNext, SelectActivate,
    Quit, QuitKeepAll, QuitKillAll,
}
```

---

## 5. tmux backend contract

Everything in this section lives in `src/tmux/mod.rs`. No other module constructs a tmux argv.

### Base invocation

```rust
/// Every call is:  tmux -L <socket> -f <conf> <subcommand...>
///
/// -f is honoured only when the server is first started (research headline finding),
/// so passing it unconditionally is harmless and removes a "have we bootstrapped yet"
/// branch. Integration test `f_flag_does_not_reset_running_options` pins this behaviour.
fn base(&self) -> Vec<String> {
    vec!["-L".into(), self.socket.clone(), "-f".into(), self.conf_path.display().to_string()]
}
```

### IMPLEMENTATION NOTE: chained commands

The research document shows chained commands in shell syntax, `cmd1 \; cmd2`. That backslash is
shell escaping. When building a `std::process::Command`, the separator is a **bare `;` as its own
argv element**:

```rust
// CORRECT
Command::new("tmux").args(["-L","multi-run","-f",conf,
    "capture-pane","-p","-e","-t","=mr-k3f9-1",
    ";",
    "capture-pane","-p","-e","-t","=mr-k3f9-2"]);

// WRONG: passes a literal backslash-semicolon to tmux
.args([..., "\\;", ...])
```

Getting this wrong produces `unknown command: \;`. It is the single most likely transcription bug
from the research doc.

### The config file

Location, in priority order:

1. `--tmux-conf <PATH>`
2. `$XDG_CONFIG_HOME/multi-run/tmux.conf`
3. `$HOME/.config/multi-run/tmux.conf`

This is the **only** file `multi-run` ever writes.

Generated content (`tmux/conf.rs`):

```tmux
# multi-run generated config. Do not edit; regenerated when the version below changes.
# multi-run-conf-version: 5

source-file -q ~/.tmux.conf          # omitted when --no-user-conf
source-file -q ~/.config/tmux/tmux.conf

set -g remain-on-exit on
set -g remain-on-exit-format ""      # omitted below tmux 3.3 (gotcha A, section 10)
set -g exit-empty off
set -g detach-on-destroy on
set -g history-limit 50000
set -g mouse on                      # wheel scrolls an attached session
set -g default-shell <config-dir>/user-shell   # wrapper: exec $SHELL -il -c …
set -g escape-time 10
set -g base-index 0                  # job pane is always <session>:0.0
set -g pane-base-index 0

set -g status on                     # only an attached client renders it
set -g status-position bottom
set -g status-interval 5
set -g status-style "bg=#1e1e2e,fg=#cdd6f4"
set -g status-format[0] "... session, command, key legend ..."

bind-key -n 'C-\' detach-client
bind-key -n M-d detach-client
bind-key -n C-n switch-client -n
bind-key -n C-p switch-client -p
bind-key -T copy-mode Escape send-keys -X cancel
bind-key -T copy-mode-vi Escape send-keys -X cancel
```

**Attached-session keys.** `C-\`, `C-n` and `C-p` match the ccsm session browser; `M-d` stays bound
from version 1. The status bar exists to make them discoverable, since none of them are visible
from the TUI's own keymap. Nothing here creates windows for the user: a job session is one pane,
and only the user's own prefix bindings can change that.

**Attached-session scrollback.** Two different things handle the wheel, and only one of them is
ours. In the grid and list views the TUI owns mouse capture and scrolls the focused tile
(section 7's `ScrollUp`/`ScrollDown`). Inside an attached session the TUI is not running: `attach.rs`
step 4 tears mouse reporting down before handing the terminal over, so a wheel notch is tmux's to
interpret. Without `mouse on` tmux has no root-table `WheelUpPane` binding to fire and the notch
does nothing, which is why the single-session view was unscrollable while the grid was not. With it
on, the stock binding runs `copy-mode -e` and the pane walks back through `history-limit`.

`mouse on` is set after the `source-file` lines, like every other `set -g` here, so a user config
with `set -g mouse off` cannot take the attached view's scrollback away. The cost is the usual tmux
one: terminal-native selection needs `Shift` held while attached.

The two `Escape` bindings back the status bar's "ESC exits scroll mode" hint. tmux only cancels copy
mode on `Escape` under `mode-keys emacs`; with `mode-keys vi` (which tmux also infers from a vi-ish
`$EDITOR`) the stock binding is `clear-selection`, which leaves the pane in copy mode and the hint
lying. Binding both tables makes it true either way.

**Job-pane addressing.** Because the user *can* split or add a window with their own bindings,
"the session's current pane" is not reliably the job. Everything that reads or acts on a job targets `=<S>:0.0` (`capture-pane`, `send-keys`,
the `#{pane_pid}` read behind `signal_pane`) or `=<S>:0` (`resize-window`, `set-option -uw
window-size`), and the metadata poll filters `list-panes -a` to window 0 pane 0. The two
`*-base-index` lines are what make that address unconditional; without them a user config setting
either base to 1 would silently point every one of those calls at nothing.

The user's config is sourced **first** so their prefix and bindings work during an attach, and our
`set -g` lines come after so ours win (research item 5). `--no-user-conf` skips both `source-file`
lines for users whose config runs plugin managers they do not want firing on a second server.

**Regeneration policy.** On every startup, generate the text and rewrite the file unless it already
matches byte for byte. As built this is a content comparison rather than the header check this
document originally specified: parts of the text depend on the environment rather than the template
(`--no-user-conf` drops the `source-file` lines), and a header check cannot see those change. `# multi-run-conf-version:` is still written, and
bumping `CONF_VERSION` still forces a rewrite, but it is now a marker for humans rather than the
decision itself. Because
`-f` only applies at server start, a rewrite does not affect an already-running server. As built,
a rewrite is therefore followed by `source-file -q <conf>` against the socket (invocation 15), which
does reach a running server: an upgrade's key bindings and status bar apply to sessions that are
already up, without killing them. Options only consulted at creation time (`base-index`,
`pane-base-index`, `default-shell`) still only affect what is created afterwards, so
`--restart-server` remains the clean slate.

### Invocation catalogue

Throughout, `S` is a tmux session name and `TAB` is a literal 0x09.

| # | When | argv (after `-L <sock> -f <conf>`) | Parses back | Guards |
|---|---|---|---|---|
| 1 | Preflight, once | `-V` (no `-L`/`-f`) | version string | gotcha G |
| 2 | Create a job | see below | rc only | items 1, 2; gotchas B, C, D |
| 3 | Metadata poll, 4 Hz | see below | session + pane records | item 2; gotcha C |
| 4 | Content poll, up to 10 Hz | see below | marker-delimited screens | item 3 |
| 5 | Resize, debounced | `resize-window -t =S:0 -x W -y H` (chained per tile) | rc only | item 4; gotcha E |
| 6 | Pre-attach | `set-option -uw -t =S:0 window-size` | rc only | gotchas D, E |
| 7 | Attach | `attach-session -t =S` (inherited stdio) | exit status | item 5; gotchas C, E, F |
| 8 | Post-detach | call 5 over **every** tile | rc only | item 4, SIGWINCH note |
| 9 | Stop level 1 | `send-keys -t =S:0.0 C-c` | rc only | item 6; gotcha C |
| 10 | Send EOF | `send-keys -t =S:0.0 C-d` | rc only | item 6 |
| 11 | Dismiss / kill | `kill-session -t =S` | rc, stderr | item 7; gotcha C |
| 12 | Kill everything | `kill-server` | rc, stderr | item 7 |
| 13 | Validate before attach | `has-session -t =S` | rc 0/1 | item 7; gotcha C |
| 14 | `--list`, startup orphan scan | call 3 | same | item 2 |
| 15 | Conf file was just rewritten | `source-file -q <conf>` | rc only | `-f` is read only at server start |

Stop levels 2 and 3 do not go through tmux at all: they are
`/bin/kill -TERM <pane_pid>` and `/bin/kill -KILL <pane_pid>`, using the `#{pane_pid}` from the
metadata poll, which reads `#{pane_pid}` from `=S:0.0`. tmux has no command that kills a pane's
process while keeping the pane, and
`kill-pane` would destroy the session and its output, which is exactly what we must not do.

#### 2. Create a job

One process, creation and all metadata in one command queue:

```
new-session -d -s mr-k3f9-1 -c /abs/cwd -x 78 -y 18 cargo test --all
;
set-option -t =mr-k3f9-1: @mr_group   k3f9
;
set-option -t =mr-k3f9-1: @mr_index   1
;
set-option -t =mr-k3f9-1: @mr_cmd     cargo test --all
;
set-option -t =mr-k3f9-1: @mr_cwd     /abs/cwd
;
set-option -t =mr-k3f9-1: @mr_started 1785271018
;
set-option -t =mr-k3f9-1: @mr_label   backend checks
```

Notes:

- The command string is the **last** argument to `new-session`. Our config sets
  `default-shell` to a generated `user-shell` wrapper that re-execs the user's
  real `$SHELL` with login/interactive flags (e.g. `zsh -il -c '<cmd>'`), so
  each job pane loads the same profile files a normal terminal session would
  (`.zshrc`, `.bashrc`, fish login config, …). Command syntax therefore matches
  the user's shell. The wrapper bakes the shell path in; it must not read
  `$SHELL` at runtime, because tmux sets `$SHELL` to `default-shell` itself.
- `-x`/`-y` are the tile's inner size, computed before creation (research item 4). If the layout is
  not known yet, use 80x24 and let the first debounced resize correct it.
- Targets use `=name:` on `set-option`, with the trailing colon, per **gotcha D**. Without the
  colon, `set-option` parses the target as a pane target and fails with `no such session`.
- `@mr_label` is only emitted when a label exists.

#### 3. Metadata poll

Two subcommands in one process, with a marker between them:

```
list-sessions -f '#{!=:#{@mr_group},}'
  -F '#{session_name}TAB#{@mr_group}TAB#{@mr_index}TAB#{@mr_cwd}TAB#{@mr_started}TAB#{session_attached}TAB#{@mr_label}TAB#{@mr_cmd}'
;
display-message -p '@@MR-PANES'
;
list-panes -a -f '#{&&:#{==:#{window_index},0},#{==:#{pane_index},0}}'
  -F '#{session_name}TAB#{pane_dead}TAB#{pane_dead_status}TAB#{pane_dead_signal}TAB#{pane_dead_time}TAB#{pane_pid}TAB#{pane_width}TAB#{pane_height}TAB#{history_size}TAB#{cursor_y}'
```

The two record sets are joined on `session_name` in Rust. The pane half is filtered server-side,
but on *pane* formats only (`window_index`, `pane_index`), not on `@mr_group`: research item 2
verified the `@mr_group` filter on `list-sessions` and did not verify that a session-scope user
option resolves in a pane format context, so rows from sessions that are not ours are still
discarded in Rust against the session set. The window/pane filter exists because a user can add a
second window or a split to a job's session with their own prefix bindings; without it that pane
arrives as a second row for the same session and overwrites the job's liveness and exit status.

**Field separator.** Emit tabs with `#{l:\t}` in the format string. Because a user command could
itself contain a tab, `@mr_cmd` is the **last** field and parsing uses `splitn(N, '\t')`, so any
embedded separator lands harmlessly inside the final field. An integration test asserts the
separator arrives as a real 0x09 byte; if a tmux version renders `#{l:\t}` literally, fall back to
embedding an actual tab character in the argv string.

**Empty result is not an error.** rc != 0 with stderr containing `no server running` or
`error connecting to` means zero sessions (research item 7). With `exit-empty off` set, the more
common shape is rc 0 with empty stdout. Both are handled.

#### 4. Content poll

One process, a marker before each capture so the output splits cleanly:

```
display-message -p '@@MR:mr-k3f9-1'
;
capture-pane -p -e -t =mr-k3f9-1:0.0
;
display-message -p '@@MR:mr-k3f9-2'
;
capture-pane -p -e -t =mr-k3f9-2:0.0
```

- No `-S`/`-E`: the pane is exactly the tile size, so a plain capture returns exactly the visible
  screen, which is exactly what we draw (research items 3 and 4).
- `-e` includes SGR sequences. tmux emits `ESC[39m` for "colour ends", not a full reset; ansi-to-tui
  handles that as default-foreground.
- **Never `-J`.** It joins wrapped lines and destroys the physical layout we are mirroring
  (research item 3). Reserved for a future "copy full output" feature.
- **Never rely on `-N` for padding** (gotcha I). We pad in Rust with `unicode-width`.
- The target is the job's own pane, `=S:0.0`, not the session's current pane: a window or split the
  user opens while attached leaves the session pointing somewhere that is not the job.
- On tmux < 3.3, dead panes are captured as `capture-pane -p -e -S -1 -t =S:0.0` to recover the line
  that the unsuppressible dead-pane banner scrolled off (gotcha A, section 10).
- Scrollback (list/detail view, `scroll = o > 0`) uses
  `capture-pane -p -e -S -<o> -E <H-1-o> -t =S`, which yields exactly `H` lines ending `o` lines
  above the live bottom. `H-1-o` is negative once the view is more than one screen back; that is a
  valid history line number and must not be clamped to 0. `o` itself is clamped to the pane's
  `#{history_size}` on the way in (`Job::max_scroll`), because past that tmux clamps `-S` to the
  oldest history line while `-E` keeps descending and returns a short capture, which the tile pads
  at the bottom.

We deliberately do **not** use control mode (`tmux -C`). Research item 3 measured it at 93x faster
per pane, which only starts to matter past roughly 30 tiles, and gotcha H (a bare `tmux -C` silently
creates a stray session) plus the need for a persistent protocol state machine is real complexity.
The batched form is 5.35 ms for 8 panes, comfortably under 6% of one core at 10 Hz. Control mode is
documented as the escape hatch, not the v1 design.

---

## 6. Poll loop and rendering

### Concurrency model: threads

Three OS threads and `std::sync::mpsc`. **No async runtime.**

Justification: there are exactly three blocking concerns (tty input, tmux subprocess IO, a timer),
no IO fan-out that benefits from a scheduler, and the tmux calls are already batched into one
subprocess per poll. Tokio would add a large dependency, a colour split through the codebase, and a
harder story for the attach round trip (which needs a plain blocking `Command::status()` with
inherited stdio). Sibling projects in this repo are synchronous. Threads win on every axis here.

```
                       ┌─────────────────────────────┐
   crossterm::read ───►│ input thread                │──Msg::Key/Resize──┐
                       │ event::poll(100ms) + flag   │                   │
                       └─────────────────────────────┘                   ▼
                                                                 ┌───────────────┐
                       ┌─────────────────────────────┐           │  main thread  │
   tmux subprocesses ─►│ poller thread               │──Msg::────►  App + draw   │
                       │ owns Tmux, schedules polls  │  Poll     │               │
                       └──────────▲──────────────────┘           └───────┬───────┘
                                  │                                      │
                                  └──────── PollRequest ─────────────────┘
                                    (visible set, sizes, resize batch,
                                     one-shot commands, pause/resume)
```

- **main** owns the `Terminal`, drains the channel with a `recv_timeout(16ms)`, applies messages to
  `App`, and draws when `redraw` is set. It never blocks on tmux.
- **input** loops `event::poll(100ms)` then `event::read()`, forwarding key, resize, and mouse
  events. It checks an `Arc<AtomicBool>` stop flag each iteration, so it exits within 100 ms of
  being asked. This is what makes gotcha F tractable.
- **poller** owns the `Tmux` handle and every subprocess. It receives a `PollRequest` describing
  which sessions are visible and at what size, plus one-shot commands (create, kill, send-keys,
  resize batch). All tmux latency lives here.

Message types:

```rust
enum ToMain {
    Key(crossterm::event::KeyEvent),
    Resize(u16, u16),
    Mouse(crossterm::event::MouseEvent),
    Meta { sessions: Vec<SessionRecord>, panes: Vec<PaneRecord> },
    Content { updates: Vec<(String, Vec<u8>)> },   // raw ANSI, decoded on main
    TmuxError(String),
    PollerPaused,       // ack for the attach handshake
    InputStopped,       // ack for the attach handshake
}

enum ToPoller {
    SetVisible { names: Vec<String>, sizes: Vec<(String, u16, u16)> },
    ResizeBatch(Vec<(String, u16, u16)>),
    OneShot(TmuxOp),    // Create, KillSession, SendKeys, UnsetWindowSize, ...
    Pause,
    Resume,
    Shutdown,
}
```

ANSI decoding happens on **main**, not the poller, because `ansi-to-tui` produces
`ratatui::text::Text` which is cheap to build and belongs next to the buffer that consumes it.
Sending raw bytes over the channel keeps the poller free of ratatui types.

### Poll rates

| Tile condition | Metadata | Content |
|---|---|---|
| Any tracked job | 4 Hz (single batched call for all) | n/a |
| Visible and `Running` | | 10 Hz |
| Visible and `Finished`, content unchanged for 3 polls | | 0.2 Hz |
| Visible and `Vanished` | | never (content is frozen) |
| Offscreen (other grid page, or non-selected in list view) | | 1 Hz, and 0 Hz once tile count exceeds 12 |
| Focused detail pane in list view | | 10 Hz |

Research item 3 recommends 10 Hz visible and 1 to 2 Hz offscreen; this is that plus two cheap
optimisations. Dead panes do not change unless resized, so backing them off to 0.2 Hz is free. The
`history_size` and `cursor_y` fields in the 4 Hz metadata poll give offscreen tiles a "new output"
dot with no capture cost at all, which is what lets us drop to 0 Hz past 12 tiles.

### Capture to ratatui

```
raw bytes from capture-pane -p -e
  -> prepend b"\x1b[0m"                 // deterministic starting SGR state
  -> ansi_to_tui::IntoText::into_text() // -> ratatui::text::Text<'static>
  -> normalise to exactly H lines:
       if fewer, push blank lines (tmux trims trailing blanks)
       if more,  keep the last H (a resize raced the capture)
  -> per line, pad to W display columns using unicode_width::UnicodeWidthStr  // gotcha I
  -> store as Job::content, bump content_rev
```

If `into_text()` errors, keep the previous content, push the error to the log, and retry next poll.
A malformed escape sequence must never blank a tile or kill the app.

### Tile geometry

`ui/layout.rs`, pure functions, no ratatui IO:

```rust
pub const MIN_TILE_W: u16 = 24;        // 22 inner + 2 border
pub const MIN_TILE_H: u16 = 7;         //  5 inner + 2 border
pub const PREFERRED_TILE_H: u16 = 12;  // 10 inner + 2 border

/// Horizontal by default: the fewest columns (so the most stacking) that
/// still give every tile PREFERRED_TILE_H rows. One full-width row per job
/// on a tall terminal, vertical columns side by side on a short one, and a
/// grid in between. Falls back to MIN_TILE_H, then to the 2:1 aspect ideal
/// when nothing fits and the caller is about to paginate anyway.
pub fn grid_dims(n: usize, area: Rect) -> (u16, u16) {
    let cols = fewest_cols(n, area, PREFERRED_TILE_H)
        .or_else(|| fewest_cols(n, area, MIN_TILE_H))
        .unwrap_or_else(|| ideal_cols(n, area));
    (cols, (n as u16).div_ceil(cols))
}

pub struct Geometry {
    pub view: ViewMode,
    pub cols: u16,
    pub rows: u16,
    pub per_page: usize,
    pub pages: usize,
    /// Inner rect per visible tile, in the same order as the page's job slice.
    pub tiles: Vec<Rect>,
}

pub fn compute(n: usize, area: Rect, view: ViewMode, page: usize) -> Geometry;

/// The session list / detail split, shared by rendering and the resize
/// sweep so both agree on how big the detail pane is.
pub fn list_detail_split(area: Rect) -> (Rect, Rect);
```

`compute` reduces `cols`/`rows` until every tile meets the minimum, and whatever count then fits
becomes `per_page`. `per_page` is clamped to at least 1, so a hopelessly small terminal still shows
one tile rather than dividing by zero.

`ViewMode::ListDetail` produces no tile rects at all: the single visible job owns the whole detail
pane. The resize sweep in `main.rs` sizes that job against `list_detail_split`'s detail rect rather
than iterating `tiles`, and moving the selection (or switching views) arms the resize debounce, so
the pane the user is looking at always wraps its output to the space it is drawn in.

### More sessions than fit

Grid view paginates. The status bar shows `page 2/3 ([ ] to change)`, and `[`/`]` move between
pages. Focus moving past the last tile on a page advances the page and lands on its first tile, so
`j` walks the whole group without the user thinking about pages.

If the terminal cannot fit even one tile at the minimum (a very short terminal), the app switches to
`ListDetail` automatically and shows a hint. `ListDetail` never paginates; the left list scrolls.

The default view at launch is `Grid` if all commands fit on one page at the current terminal size,
otherwise `ListDetail`. `--view` overrides and pins the choice.

### Resize debounce

1. `Event::Resize(w, h)` sets `App::pending_resize = Some(Instant::now())` and marks `redraw`. The
   TUI immediately re-lays out and redraws with stale content; it does not wait.
2. On each 16 ms tick, if `pending_resize` is at least **120 ms** old (research item 4 suggests
   roughly 100 ms), recompute `Geometry`, diff each job's desired inner size against `Job::tile_size`,
   and send a single `ToPoller::ResizeBatch` containing **only the tiles that changed**.
3. The poller issues one chained `resize-window` process for the whole batch.
4. `Job::tile_size` is updated on ack.

Only sending changed tiles matters because every `resize-window` silently sets that window's
`window-size` to `manual` (**gotcha E**), and re-setting it needlessly is churn we then have to
undo before an attach.

Resizing a dead pane works but re-lays-out historical content rather than re-wrapping the original
byte stream, so expect cosmetic imperfection there (research item 4). Accepted, documented in the
README.

---

## 7. Keybindings

Vim-flavoured, matching `stash-mgr` (`j`/`k` navigate, `h`/`l` move across, `Ctrl-d`/`Ctrl-u`
half-page, `q` quits, confirmations are `y`/`n`).

### Global

| Key | Action |
|---|---|
| `?` | Toggle help overlay |
| `v` | Toggle view mode (grid <-> list/detail) |
| `1` | Grid view |
| `2` | List/detail view |
| `n` | New command prompt; the new job joins the current group |
| `!` | Open the error log |
| `q` | Quit. Prompts keep-or-kill if any job is live |
| `Ctrl-c` | Same as `q`. Never kills anything without the prompt |
| `Esc` | Close the topmost overlay, else no-op |

### Navigation

| Key | Action |
|---|---|
| `j` | Focus next job (advances the page at the end of a grid page) |
| `k` | Focus previous job |
| `h` | Grid: focus the tile to the left. List: move focus to the session list |
| `l` | Grid: focus the tile to the right. List: move focus to the detail pane |
| `Tab` | Grid: focus next job. List: move focus to the detail pane |
| `Shift-Tab` | Grid: focus previous job. List: move focus to the session list |
| `Up` / `Down` | List view with the session list focused: previous / next job. Otherwise scroll the focused job's output one line |
| `Left` / `Right` | List view with the session list focused: same as `h` / `l`. Otherwise scroll the focused job's output one line |
| `g` `g` | Focus the first job |
| `G` | Focus the last job |
| `Alt-1` .. `Alt-9` | Focus job N directly |
| `[` / `]` | Previous / next grid page |
| `Ctrl-u` / `Ctrl-d` | Scroll the focused detail pane half a page (list view) |
| `Ctrl-b` / `Ctrl-f` | Scroll the focused detail pane a full page |
| `G` (detail focused) | Jump the detail pane back to the live bottom |

`1` and `2` select view modes, so direct job selection uses `Alt-<n>`. This avoids the ambiguity of
overloading digits.

### Session actions (operate on the focused job)

| Key | Action | Notes |
|---|---|---|
| `Enter` or `a` | Attach | Hands over the terminal. Detach with `C-\` or `M-d` (or the user's own prefix + `d`) |
| `s` | Stop | Escalates: `C-c`, then TERM, then KILL. Output is preserved either way |
| `e` | Send EOF | `send-keys C-d` |
| `d` | Dismiss | Only valid on `Finished` or `Vanished`. Kills the session and removes the tile |
| `D` | Dismiss all finished | Confirms first |
| `X` | Force kill | Any state. Confirms. Destroys the session and its output immediately |
| `r` | Re-run | Starts the same command as a new job appended to the group. The old tile stays |

`s` versus `X` is the important distinction and the help overlay says so explicitly: **`s` stops the
command and keeps the output; `X` throws the output away.** Because `remain-on-exit on` is global, a
stopped command leaves a readable dead pane (research item 1), which is precisely the "output stays
until the user dismisses it" requirement.

### Compose / add-command prompt

| Key | Action |
|---|---|
| type | Edit the current line |
| `Enter` | Commit this line and start another |
| `Ctrl-s` | Launch everything queued |
| `Enter` on an empty line | Same as `Ctrl-s` |
| `Ctrl-u` | Clear the current line |
| `Backspace` on an empty line | Remove the last queued command |
| `Esc` | Cancel. On the bare-launch compose screen, this quits the tool |

### Confirmation and exit prompts

| Key | Action |
|---|---|
| `y` | Confirm (confirm dialog only) |
| `n` / `Esc` | Cancel (confirm: cancel; exit: Esc cancels) |
| `←` `→` `↑` `↓` / Tab / Shift-Tab | Move the highlighted option |
| `Enter` | Activate the highlighted option |

Exit prompt only:

| Key | Action |
|---|---|
| `k` | Keep all sessions running and exit |
| `x` | Kill all sessions and exit |
| `Esc` | Cancel, stay in the tool |

The exit prompt uses `k`/`x` rather than `y`/`n` because "yes" is genuinely ambiguous when the
question is "keep or kill". Arrow keys and Enter work the same way on both dialogs so the highlight
is usable without memorizing the letter shortcuts.

`keymap.rs` implements this as one pure function,
`fn map(view: ViewMode, pane: Pane, modal: &Modal, key: KeyEvent) -> Option<Action>`, with modal
maps checked first. `pane` is what makes the arrows mean "move the highlight" in the session list
and "scroll" everywhere else. That makes the entire table unit-testable with no terminal and no tmux.

---

## 8. Attach / detach round trip

Implemented in `attach.rs` as one function. Order is load-bearing; every step cites its reason.

```
 1. has-session -t '=<S>'                      gotcha C: exact match, no prefix surprise
       rc 1  -> mark Vanished, toast, return. Do not tear down the terminal.

 2. tx_poller.send(Pause); wait for ToMain::PollerPaused
       No tmux subprocess should be in flight while the user owns the terminal.

 3. input_stop.store(true); input_handle.join()     GOTCHA F
       tmux attach shares our stdin. A crossterm reader blocked in read(0,..) will
       steal keystrokes from it. The thread's 100ms event::poll makes this bounded.

 4. crossterm::terminal::disable_raw_mode()

 5. execute!(stdout, DisableMouseCapture, LeaveAlternateScreen); stdout.flush()
       ITEM 5, FACT 2. tmux unconditionally enters alt screen (?1049h) on attach and
       unconditionally leaves it (?1049l) on detach. If we are still in the alt screen
       when tmux exits it, we end up drawing into a buffer nobody is looking at.

 6. tmux set-option -uw -t '=<S>:' window-size      GOTCHA E (+ gotcha D's trailing colon)
       resize-window left this window at 'manual'. Without unsetting it, attaching a
       120x40 client leaves the pane marooned at the old tile size (verified in research).
       -uw restores inheritance from the global 'latest'.

 7. Command::new(tmux)
        .args(["-L", sock, "-f", conf, "attach-session", "-t", "=<S>"])
        .status()                                   inherited stdio, blocking

    Print NOTHING between steps 5 and 8. tmux writes "[detached (from session S)]" to
    the primary screen on the way out (item 5, fact 4); step 8 wipes it.

 8. enable_raw_mode()
 9. execute!(stdout, EnterAlternateScreen, EnableMouseCapture, Clear(ClearType::All))
10. terminal.clear()
       ITEM 5. ratatui diffs against its previous buffer, and that buffer is garbage
       after another program owned the screen. Without this you get partial redraws.

11. let (w, h) = terminal.size(); recompute Geometry; send ResizeBatch for EVERY tile.
       ITEM 4 + the SIGWINCH note. A detached session keeps whatever size it last had
       and does not revert, and we may have received SIGWINCH while suspended. Sweep
       all tiles, not just the one that was attached.

12. Respawn the input thread with a fresh stop flag.
13. tx_poller.send(Resume); force one immediate metadata + content poll; set redraw.
```

Error handling at step 7: stderr is inherited, so we cannot capture it. If `status()` is non-zero,
re-run `has-session` to decide between "the session died while attached" (mark `Vanished`) and "the
attach itself failed" (toast the exit code). Steps 8 through 13 run either way; the terminal is
always restored.

Detach ergonomics: our private config binds `C-\` (and, still, `M-d`) to `detach-client` with `-n`
(no prefix needed), verified in research item 5. `C-n`/`C-p` switch sessions under the same no-prefix
scheme; the status bar names all three. Because the user's `~/.tmux.conf` is sourced first, their own prefix
and bindings also work during the attach. Both are advertised in the tile footer while attached and
in the help overlay.

The same restore sequence (steps 8 to 10) is what the panic hook runs, so a panic inside the attach
window still leaves a usable shell.

---

## 9. Persistence and reattach

### Nothing is stored on disk

The only file `multi-run` writes is the generated tmux config. All group and job metadata lives in
tmux `@mr_*` session options (research item 2).

Justification. tmux's option store is already durable, server-side, and authoritative for exactly
the lifetime we care about. A sidecar JSON file would be a second source of truth that drifts the
moment someone runs `tmux -L multi-run kill-session` by hand, and it would need file locking once
two `multi-run` processes run at once (which `--attach` makes routine). Reading it back is one
`list-sessions` call we already make every 250 ms. There is no state we need that outlives the tmux
server, because when the server dies the sessions are gone and the metadata is meaningless.

Options written per session, at creation:

| Option | Contents |
|---|---|
| `@mr_group` | group id, `g1` |
| `@mr_index` | job index within the group |
| `@mr_cmd` | the command string |
| `@mr_cwd` | absolute cwd |
| `@mr_started` | unix seconds |
| `@mr_label` | optional group label |

Plus one server-scope option, `@mr_conf_version`, for the config-drift notice (section 5).

### Discovery

`--list`, `--attach`, `--kill`, and the startup orphan scan all use the same metadata poll
(invocation 3), group rows by `@mr_group`, sort groups by `@mr_started` descending and jobs by
`@mr_index` ascending. The presence of `@mr_group` is what makes a session ours; a session created
by anything else on the same socket is filtered out server-side and ignored.

### What the user types

`g1` for a group, `g1.2` for one job, and the raw `mr-g1-2` is also accepted. Groups are
sequential (`g1`, `g2`, …) so they are short to type and easy to pick out of `--list`. Legacy
four-character ids remain parseable for older live groups.

- `--attach g1` (or `-a g1`) opens the TUI on that existing group. No new sessions are created.
- `--attach g1.2` opens the group and immediately runs the attach round trip on job 2.
- `--attach` on an unknown id prints the id, the closest matches, and `multi-run --list`, exit 1.

### Exit prompt flow

`q` with at least one job not in `Dismissed`:

```
┌ Quit multi-run ────────────────────────────────────────────┐
│  2 sessions still running, 1 finished.                     │
│                                                            │
│    [k]  keep them running                                  │
│         reattach with:  multi-run -a g1                    │
│    [x]  kill all 3 sessions                                │
│    [Esc] cancel                                            │
└────────────────────────────────────────────────────────────┘
```

- **keep**: restore the terminal, then print the reattach line to the primary screen so it survives
  in scrollback. Exit 0. The tmux server keeps running (`exit-empty off`), so `--list` and
  `--attach` behave identically whether or not sessions remain.
- **kill**: one chained process of `kill-session -t '=<S>'` per job, then restore and exit 0.
  Failures are reported but do not block the exit.
- If every job is already `Dismissed`, quit immediately with no prompt.
- `Q` (shift-q) is a shortcut for keep-and-exit with no prompt.

### Orphaned groups

A group from a previous run that nobody is attached to is not special and is never adopted
automatically. It appears in `--list`, and a bare `multi-run` launch shows a line on the compose
screen: `2 earlier groups still running (multi-run --list)`. Explicit reattach is the only way in.
This avoids the failure mode where a stale group silently merges into a fresh run and the user kills
work they wanted.

`--kill all` runs `kill-server`, which is the only clean way to guarantee nothing is left over.

---

## 10. Preflight

Runs before any other work, in `preflight.rs`. Sequence:

**Step 1. Presence.** `Command::new("tmux").arg("-V").output()`. An `io::ErrorKind::NotFound` means
tmux is not installed. We do not shell out to `which` (research item 8).

**Step 2. Not installed.** Print a per-platform install command and exit 127:

```
multi-run needs tmux, and it is not installed.

  Install it with:

      brew install tmux

  Then run multi-run again.
```

Platform detection: `cfg!(target_os = "macos")` selects Homebrew. On Linux, read `ID` and `ID_LIKE`
from `/etc/os-release` and map through a small table; unknown falls back to printing all of them.

| Platform | Command |
|---|---|
| macOS | `brew install tmux` |
| Debian, Ubuntu | `sudo apt install tmux` |
| Fedora, RHEL | `sudo dnf install tmux` |
| Arch | `sudo pacman -S tmux` |
| Alpine | `sudo apk add tmux` |

**We print the command. We do not run it.** Every platform except Homebrew needs `sudo`, and
prompting for a root password from inside a TUI is both a security smell and an interaction we
cannot render correctly. Package manager output is unbounded and would need its own pager. Printing
one copy-pasteable line and exiting is honest, auditable, and takes the user two seconds. This is a
deliberate decision, not a shortcut.

**Step 3. Version parse.** Hand-rolled, implementing exactly the grammar from research item 8
(**gotcha G**: tmux versions are not semver):

```
^(?:tmux\s+)?(next-)?(\d+)\.(\d+)([a-z]?)(-rc\d*)?$
```

```rust
pub struct TmuxVersion { pub major: u32, pub minor: u32, pub letter: Option<char> }
// Ord is (major, minor, letter), where None sorts before Some('a').
// `next-X.Y` and `X.Y-rc[N]` both compare as X.Y: they contain X.Y's features.
```

Must parse correctly: `tmux 3.7b`, `3.7`, `3.0a`, `2.9`, `next-3.8`, `3.4-rc`, `3.4-rc1`. Must
reject: `""`, `tmux`, `3`, `x.y`. Never parse a package version string like Arch's `3.7_b`; only
ever parse `tmux -V` output.

**Step 4. Tier.**

| Detected | Tier | Behaviour |
|---|---|---|
| `< 2.9` | `Unsupported` | **Hard fail**, exit 1. No `resize-window` or `window-size` option, so the entire tile-sizing model is impossible. Print the version, the requirement, and the install hint. |
| `2.9` to `3.2a` | `Degraded` | Omit `set -g remain-on-exit-format ""` from the config (it does not exist before 3.3). Capture dead panes with `-S -1` to recover the line the banner scrolls off (**gotcha A**). Show a one-line status-bar warning naming the version. Ubuntu 22.04 ships 3.2a, which is why this tier exists rather than hard-failing at 3.3. |
| `>= 3.3` | `Full` | Everything as designed. |
| unparseable | `Full` + warning | Assume modern, warn once, print the raw string. A future tmux with a new version shape should not brick the tool. |

**Step 5. Config.** Ensure the config directory exists and the file matches `CONF_VERSION`; write it
if not, with the tier baked in (the `remain-on-exit-format` line is conditional).

**Step 6. Server probe.** `tmux -L <sock> -f <conf> list-sessions -F '#{session_name}'`. Classify
per section 11: `no server running` and `error connecting to` are the benign empty case (research
item 7), not errors.

**Step 7. Config drift.** If a server already exists and its `@mr_conf_version` is older than
`CONF_VERSION`, add a status-bar notice with the `--restart-server` remedy. Do not restart
automatically; that would destroy the user's running jobs.

`--version` output includes all of this so bug reports are tractable:

```
$ multi-run --version
multi-run 0.1.0
tmux 3.7b (tier: full)
socket: multi-run
config: /Users/sane/.config/multi-run/tmux.conf
```

---

## 11. Error handling

**Both `thiserror` and `anyhow`, with a clear split.**

`thiserror` for `TmuxError`, because it is the one error class we actually branch on. Everything
else gets `anyhow` with `.context()`, because nothing else needs matching and typed errors there
would be ceremony.

```rust
#[derive(Debug, thiserror::Error)]
pub enum TmuxError {
    #[error("tmux is not installed")]
    NotInstalled,

    /// Benign in almost every context: it means zero sessions.
    #[error("no tmux server running on socket {socket}")]
    NoServer { socket: String },

    #[error("no such tmux session: {0}")]
    NoSuchSession(String),

    #[error("tmux {subcommand} failed (exit {code}): {stderr}")]
    CommandFailed { subcommand: String, code: i32, stderr: String, argv: Vec<String> },

    #[error("could not parse tmux output: {0}")]
    Parse(String),

    #[error("tmux io error: {0}")]
    Io(#[from] std::io::Error),
}
```

Classification lives in exactly one function so the rules cannot drift:

```rust
/// Map a finished tmux process into a typed error. Research item 7: there is no
/// dedicated exit code for "no server", so we must match stderr text.
pub fn classify(subcommand: &str, argv: &[String], out: &std::process::Output) -> TmuxError
```

| stderr contains (case-insensitive) | Maps to |
|---|---|
| `no server running` | `NoServer` |
| `error connecting to` | `NoServer` |
| `can't find session` | `NoSuchSession` |
| `no such session` | `NoSuchSession` |
| `session not found` | `NoSuchSession` |
| anything else, rc != 0 | `CommandFailed` |

Rules that follow from this:

- **A bare non-zero exit is never fatal.** Research item 7 is explicit. Read stderr first.
- `NoServer` from a poll means "the group is gone": every tracked job goes `Vanished`, the UI says
  so, and the tiles keep their last content until dismissed. It is not an error toast.
- `NoServer` from `--list` prints `No active multi-run sessions.` and exits 0.
- `NoSuchSession` from a poll targeting one session means that job vanished; same handling.
- `CommandFailed` from a poll retries with exponential backoff, 250 ms doubling to 5 s, and posts a
  toast after the third consecutive failure so a transient hiccup stays quiet.
- `CommandFailed` from a user-initiated action (kill, send-keys, resize) toasts immediately.

**Nothing in the TUI panics on a tmux error.** Errors become a 5-second toast in the status bar and
an entry in a bounded 200-line ring buffer, viewable with `!`.

**Panic hook.** `main` installs a `std::panic::set_hook` that runs the terminal restore sequence
(`disable_raw_mode`, `LeaveAlternateScreen`, `DisableMouseCapture`, `Show` cursor) before delegating
to the default hook. Without it, a panic inside the alt screen leaves the user's shell wrecked. We
do this by hand rather than pulling `color-eyre` (which `stash-mgr` uses) because we already have
`anyhow` and the hook is fifteen lines.

`main` returns `anyhow::Result<()>`; the top level prints `{:#}` (the full context chain) to stderr
and picks the exit code from the error type.

---

## 12. Test strategy

`cargo test`, per repo convention. Two layers.

### Unit tests, no tmux required

In-module `#[cfg(test)]`. These must pass on a machine with no tmux installed at all.

| Module | What is tested |
|---|---|
| `tmux/parse.rs` | Tab-delimited record parsing: normal rows, empty `pane_dead_status`, empty `pane_dead_signal`, missing `@mr_label`, a command containing a tab (must land wholly in the last field via `splitn`), a command containing `\|`, trailing newline, CRLF, completely empty input, a row with too few fields |
| `tmux/parse.rs` | Capture-marker splitting: 3 markers 3 screens, a screen containing a line that looks like a marker, zero markers, marker with no following content |
| `preflight.rs` | Version parse over `tmux 3.7b`, `3.7`, `3.0a`, `2.9`, `next-3.8`, `3.4-rc`, `3.4-rc1`, `2.8`, `""`, `tmux`, `banana`. Ordering: `3.7 < 3.7a < 3.7b < 3.8`, `2.9 < 3.0`. Tier mapping at each boundary (2.8, 2.9, 3.2a, 3.3, 3.7b) |
| `preflight.rs` | os-release ID to install command mapping, including unknown |
| `cli.rs` | `GroupId`/`JobId` round trip; parsing `g1`, `g1.2`, `mr-g1-2`, legacy `k3f9`, `all`, and rejects `g1.`, `g1.0`, `G1`, `g0`, `abcde`, `g1.abc` |
| `cli.rs` | clap parsing of every example invocation in section 2, especially `-- "-x foo"` and repeated `-c` |
| `ui/layout.rs` | `grid_dims` for n in 1..=24 across 80x24, 120x40, 200x60, 40x12. Minimum tile size enforced. `per_page` and `pages` arithmetic. Degenerate 10x3 area returns exactly one tile |
| `ui/layout.rs` | Focus movement: next/prev wrap, left/right at row edges, page advance at page boundary, single-job group |
| `keymap.rs` | The whole table as data. Every `(view, modal, key)` triple in section 7 maps to the documented `Action`. Modal maps shadow global maps. Unmapped keys return `None` |
| `model.rs` | Every row of the state machine table in section 4, driven by synthetic `PaneFacts` |
| `ui/tile.rs` | ANSI decode of a fixture containing tmux's `ESC[39m` idiom; padding a CJK line to exact display width (**gotcha I**); truncating an over-long capture to the last H lines; a malformed escape sequence leaves prior content intact |
| `ui/mod.rs` | Full-frame render into `ratatui::backend::TestBackend` from fixture jobs, asserting the buffer, for grid and list modes |

### Integration tests, real tmux

`tests/tmux_integration.rs`.

**Skip guard.** Every test starts with `let Some(v) = tmux_or_skip() else { return; };`, which runs
`tmux -V` and returns `None` (printing a skip line) if tmux is missing or below 2.9. `cargo test`
must succeed on a machine without tmux.

**Isolation.** Each test gets a unique socket name, `format!("mrtest-{}-{}", std::process::id(),
nanos)`, and a unique config written under `std::env::temp_dir()`. Never the default socket, never
the name `multi-run`. Because socket names are unique per test, tests run in parallel safely and
`--test-threads=1` is not needed.

**Cleanup.** An RAII guard:

```rust
struct TestServer { socket: String, conf: PathBuf }
impl Drop for TestServer {
    /// Runs even when an assertion panics, which is exactly why this is Drop
    /// and not a teardown call at the end of each test.
    fn drop(&mut self) {
        let _ = Command::new("tmux").args(["-L", &self.socket, "kill-server"]).output();
        let _ = std::fs::remove_file(&self.conf);
    }
}
```

**Cases**, each pinning a specific research finding so a regression points straight at the cause:

| Test | Pins |
|---|---|
| `create_and_read_back_metadata` | item 2: `@mr_*` round trip through `list-sessions -f -F` |
| `unrelated_session_is_filtered_out` | item 2: a session without `@mr_group` does not appear |
| `exit_42_reports_dead_status` | item 1: `pane_dead=1`, `pane_dead_status=42`, empty signal |
| `sigterm_reports_dead_signal` | item 1: empty status, `pane_dead_signal=term` |
| `instant_exit_is_not_a_race` | item 1: a session running `exit 7` created via `-f` config is observed dead with status 7, 20 iterations |
| `banner_suppression_keeps_first_line` | gotcha A: `history_size=0` and the first captured line is the first printed line (3.3+ only) |
| `has_session_is_exact` | gotcha C: create `L2`, assert `has-session -t '=L'` is rc 1 and `-t '=L2'` is rc 0 |
| `set_option_needs_trailing_colon` | gotcha D: `-t '=n:'` succeeds, `-t '=n'` fails |
| `resize_window_sets_manual_and_unset_restores` | gotcha E: `window-size` is `manual` after resize, inherits after `-uw` |
| `resize_changes_pane_dimensions` | item 4: `pane_width`/`pane_height` follow `resize-window` |
| `batched_capture_splits_on_markers` | item 3: 3 sessions, one process, correct per-session screens in order |
| `capture_returns_exact_pane_height` | items 3 and 4: a pane sized W x H yields H lines |
| `tab_separator_is_a_real_tab` | section 5: `#{l:\t}` emits 0x09 |
| `no_server_stderr_is_classified_benign` | item 7: `classify` returns `NoServer` for both message forms |
| `f_flag_does_not_reset_running_options` | section 5: change an option at runtime, make another call with `-f`, assert the option survived |
| `command_with_tab_survives_round_trip` | section 5: `@mr_cmd` as the last field, `splitn` recovery |

**Not covered by automation:** the attach round trip needs a real tty. It is a `#[ignore]`d test that
can be driven under a nested tmux (`tmux -L outer new-session -d '...'`), plus a manual checklist in
the README: attach, resize the terminal while attached, `C-\` to detach, confirm every tile is
re-sized correctly and the border art is not corrupted, then attach to a *different* tile and repeat.

---

## 13. Build order

Sixteen milestones, each independently verifiable. Milestone 1 must come first; after that the
parallel groups noted below touch disjoint files.

| # | Milestone | Verify by | Parallel with |
|---|---|---|---|
| 1 | Scaffold: `Cargo.toml` (deps from section 3), `main.rs` stub, `error.rs` with `TmuxError` and `classify` | `cargo build`, unit tests for `classify` pass | none |
| 2 | `preflight.rs`: presence check, version parser, tier mapping, install hint table | `cargo test`; `multi-run --version` shows the real tmux version and tier | 3, 4 |
| 3 | `cli.rs`: clap derive, `GroupId`/`JobId` types and parsing | `cargo test`; `--help` matches section 2; every example invocation in section 2 parses | 2, 4 |
| 4 | `tmux/conf.rs`: config generation, path resolution, `CONF_VERSION` header, tier-conditional lines | unit test on the generated string; config path honours `$XDG_CONFIG_HOME` | 2, 3 |
| 5 | `tmux/parse.rs`: record and marker parsers | `cargo test`, including the tab-in-command case | 9 |
| 6 | `tmux/mod.rs`: `Tmux` struct and every invocation from the section 5 catalogue. Remember `;` as its own argv element | compiles; exercised by milestone 7 | 9 |
| 7 | `tests/tmux_integration.rs`: `TestServer` guard plus every case in section 12 | `cargo test` green with tmux, skips cleanly without | 8, 9 |
| 8 | `model.rs`: types and the state machine | `cargo test` over the section 4 table | 7, 9 |
| 9 | `ui/layout.rs`: grid math, pagination, focus movement. Touches no other file | `cargo test` | 5, 6, 7, 8 |
| 10 | Headless paths: `--list`, `--kill`, `--restart-server` end to end | manual against a real tmux; output matches section 2 | 11 |
| 11 | `ui/mod.rs` + `ui/tile.rs`: render a grid from fixture jobs, no tmux involved | `TestBackend` buffer assertions; a `--demo` hidden flag renders fake tiles | 10 |
| 12 | `poll.rs` + wire the poller thread: a **live, read-only** grid of running commands. First genuinely useful build | `multi-run "seq 1 100" "sleep 30"` shows live output | none |
| 13 | `keymap.rs` + `app.rs`: focus movement, view toggle, pagination, help overlay, list/detail mode | `cargo test` on the keymap table; manual navigation | none |
| 14 | `attach.rs`: the full section 8 sequence | manual checklist from section 12; resize while attached and confirm the post-detach sweep | none |
| 15 | Lifecycle actions: stop escalation, dismiss, force kill, re-run, confirm modals, error toasts and log | manual; a command killed with `s` still shows its output | 16 |
| 16 | Compose screen, add-command prompt, exit prompt, `--attach`, orphan notice, degraded-tier handling (`-S -1` on dead panes), `README.md` | manual; run under a tmux 3.2 build if available | 15 |

Milestone 12 is the useful-demo checkpoint and the natural place to stop and reassess. Milestones 2,
3, 4 are three agents in parallel right after scaffolding; 5 through 9 are another parallel band; 10
and 11 pair up.

---

## 14. Open questions and risks

**1. Does `list-panes -a -f '#{!=:#{@mr_group},}'` see session-scope user options?**
Unverified. If it does, the metadata poll collapses from two subcommands to one and drops a join.
*Default: use the two-subcommand chained form from section 5.* Add an integration test that probes
the single-call form; if it passes on 3.3 through 3.7, switch and keep the fallback behind the tier
check.

**2. A command containing a tab breaks the field separator.**
*Default: put `@mr_cmd` last and parse with `splitn`,* so an embedded tab lands harmlessly inside the
final field. Residual risk is nil for the fields we have, but any future field added after `@mr_cmd`
reintroduces it. `tmux/parse.rs` carries a comment saying so.

**3. Dead panes re-layout badly on resize.**
Research item 4: resizing a dead pane re-lays-out historical content rather than re-wrapping from
the original bytes, so it looks slightly wrong. *Default: accept it and document it.* The
alternative (freeze dead panes at their last size and letterbox them inside the tile) is more code
and looks stranger. Revisit if it turns out to be visually offensive in practice.

**4. Do we keep capturing dead panes at all?**
*Default: capture once when death is detected, then only on resize and at 0.2 Hz.* A dead pane
cannot produce new output.

**5. Passing `-f` on every invocation.**
Research says `-f` is honoured only at server start. If a future tmux re-sourced it per client, we
would reset options on every poll. *Default: pass it always* for the simpler code path, pinned by
the `f_flag_does_not_reset_running_options` integration test. If that test ever fails, switch to
passing `-f` only on the bootstrap call.

**6. Scaling past ~30 tiles.**
Batched capture is 5.35 ms for 8 panes; control mode is 93x faster per pane (research item 3).
*Default: cap live capture at 12 visible tiles, paginate the rest, and stop offscreen capture
entirely past 12.* Control mode is the documented escape hatch, with gotcha H (a bare `tmux -C`
creates a stray session) as its known trap.

**7. Mouse support.**
*Default: enable mouse capture for click-to-focus and double-click-to-attach on a tile only.* No
forwarding of mouse events into panes, because `send-keys` cannot express them at all (item 6). If
mouse capture proves to break the user's terminal selection, make it opt-in with `--mouse`.

**8. `r` (re-run) semantics.**
*Default: append a new job to the group; the old tile stays until dismissed.* Reusing the tile would
destroy output the user may still want, which contradicts the core requirement.

**9. Sourcing `~/.tmux.conf` into our private server.**
Good for ergonomics (the user's prefix and bindings work during attach) but it also runs their
plugin manager on a second server, which can spawn background processes they did not expect.
*Default: source it, with `--no-user-conf` to opt out,* and say so in the README. Our `set -g`
overrides come after theirs, so a user config that turns `remain-on-exit` off cannot break us.

**10. Groups spanning multiple working directories.**
*Default: a group has exactly one cwd, fixed at creation.* Commands added mid-run use the group's
cwd, not the shell's current directory (which may have changed). `--attach` from anywhere still
works and displays the original cwd.

**11. Terminal emulators that mishandle the alt-screen handoff.**
Research item 5 verified the byte sequences on this machine's terminal. Some emulators, and some
nested-tmux setups, restore scroll regions differently. *Default: follow the section 8 sequence
exactly, including `terminal.clear()`,* and add "run `multi-run` inside your own tmux" to the manual
test checklist, since that is where this will show up first.
