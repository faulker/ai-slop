# multi-run

Run several shell commands in parallel, each in its own tmux session, and watch them all at once
in a terminal UI. Every job gets a tile; you can jump into any tile for real interactive access and
jump back out. Sessions outlive the tool, so you can quit, come back, and pick up where you left off.

```
┌ sc2q.1 ✓ cargo fmt --check ── exit 0 ┐┌ sc2q.2 ● cargo watch -x test running ┐
│                                      ││running 12 tests                      │
│                                      ││test model::advance ... ok            │
│                                      ││                                      │
└──────────────────────────────────────┘└──────────────────────────────────────┘
grid  sc2q.1 cargo fmt --check          ?:help  v:view  n:new  q:quit
```

## Requirements

- **tmux 3.3 or newer** for the full experience. 2.9 through 3.2a work in a degraded mode where a
  finished pane shows an extra banner line. Below 2.9 the tool refuses to start.
- **Rust** (2024 edition) to build.

multi-run checks for tmux at startup and prints the right install command for your platform if it
is missing (`brew install tmux`, `sudo apt install tmux`, and so on).

## Setup and build

```sh
cd multi-run
cargo build --release
./target/release/multi-run "cargo watch -x test" "npm run dev"
```

## Test

```sh
cargo test          # unit tests plus integration tests against a real tmux
cargo clippy --all-targets
```

The integration tests create throwaway tmux servers on unique socket names and kill them on drop,
including on panic. They never touch your own tmux sessions. If tmux is not installed they skip
rather than fail.

## Usage

```sh
multi-run "cargo watch -x test" "npm run dev" "cargo fmt --check"
multi-run                       # bare: opens the compose screen
multi-run --list
multi-run -a g1                 # reopen a group you left running
multi-run --kill g1
multi-run --kill all
multi-run --view list "make -j8" "tail -f /var/log/system.log"
```

### Commands that have their own flags

Three forms work:

```sh
# 1. Quote it. The command is one argument, so its flags are just characters.
multi-run "cargo test -- --nocapture" "npm run dev -- --port 3000"

# 2. Separator. Everything after `--` is a command, hyphens and all.
multi-run -- "-x weird-first-char-command" "cargo build --release"

# 3. Explicit flag. Repeatable, good for scripts and aliases.
multi-run -c "cargo watch -x test" -c "npm run dev"
```

Put every `-c` before any bare positional command. Once a positional appears, later `-c` flags are
treated as part of it.

### Options

| Flag | Meaning |
|---|---|
| `-c, --command <CMD>` | Add a command. Repeatable. |
| `-l, --list` | List active groups and exit. |
| `-a, --attach <ID>` | Open the TUI on an existing group. `g1`, or `g1.2` to jump straight into job 2. |
| `--kill <ID>` | Kill a group, a job, or `all`. Exits without opening the TUI. |
| `--view <MODE>` | Initial view, `grid` or `list`. Defaults to whatever fits. |
| `--label <TEXT>` | Human label for the group, shown in `--list`. |
| `--socket <NAME>` | tmux socket name, default `multi-run`. Lets you run isolated groups side by side. |
| `--tmux-conf <PATH>` | Override the generated tmux config path. |
| `--no-user-conf` | Do not source `~/.tmux.conf` into the private server. |
| `--restart-server` | Kill our tmux server first, forcing a config reload. Destroys existing groups. |

Exit codes: `0` success including "nothing running", `1` a real tmux failure, `2` a usage error,
`127` tmux not installed.

## Keys

**Global**

| Key | Action |
|---|---|
| `?` | Toggle help |
| `v` | Switch grid / list view |
| `1` / `2` | Grid view / list view |
| `n` | New command, joins the current group |
| `!` | Error log |
| `q`, `Ctrl-c` | Quit, prompting keep-or-kill if anything is live |
| `Esc` | Close the topmost overlay |

**Navigation**

| Key | Action |
|---|---|
| `j` / `k` | Next / previous job (crosses grid pages) |
| `h` / `l` | Grid: tile left / right. List: session list / detail pane |
| `Tab` / `Shift-Tab` | Grid: next / previous job. List: detail pane / session list |
| `↑` `↓` `←` `→` | Scroll the selected panel |
| `g` `g` / `G` | First / last job. With the detail pane focused and scrolled back, `G` returns to the live bottom |
| `Alt-1`..`Alt-9` | Focus job N |
| `[` / `]` | Previous / next grid page |
| `Ctrl-u` / `Ctrl-d` | Scroll the detail pane half a page |
| `Ctrl-b` / `Ctrl-f` | Scroll a full page |

**Session**

| Key | Action |
|---|---|
| `Enter`, `a` | Attach. Detach with `Ctrl-\` |
| `s` | Stop. Escalates on each press: `C-c`, then SIGTERM, then SIGKILL. Output stays readable at every level |
| `e` | Send EOF |
| `d` | Dismiss a finished job, removing its tile |
| `D` | Dismiss all finished |
| `X` | Force kill, with confirmation. Destroys the output |
| `r` | Re-run the command as a new job |

**Inside an attached session**

These are bound on multi-run's private tmux server only, with no prefix, and they match the
[ccsm](https://github.com/faulker/ccsm) session browser so the same fingers work in both. A status
bar along the bottom of the terminal names them while you are attached, next to the job's session
name and command.

| Key | Action |
|---|---|
| `Ctrl-\` | Detach, back to the grid |
| `Ctrl-n` / `Ctrl-p` | Next / previous session, without detaching |
| `M-d` | Detach. Still bound from before `Ctrl-\` existed |
| Wheel up | Scroll back through the job's history (50,000 lines) |
| `Esc` | Leave scroll mode, back to the live end of the output |

Your own tmux prefix and bindings keep working too, since your config is sourced first, so
splitting the pane or opening another window is still yours to do. The job's tile keeps showing the
job either way: multi-run addresses each job's own pane explicitly rather than following whatever
window you happen to be looking at.

Because these bindings have no prefix, they are taken from whatever is running in the pane.
`Ctrl-n` and `Ctrl-p` are next/previous line in readline and in many TUIs, so those keystrokes will
not reach your job while you are attached. Detach and use the grid if you need them.

**Mouse**

| Action | Effect |
|---|---|
| Left click a tile | Focus it |
| Wheel up / down | Scroll the focused job's output |

The wheel works in both places, but two different things handle it. In the grid and list views the
TUI does, scrolling the tile you have focused. Inside an attached session the TUI is not running at
all, so tmux does: the wheel drops the pane into tmux's scroll (copy) mode and walks back through
the pane's history. `Esc` or `q` returns you to the live end, and the status bar says so while you
are scrolled back.

Mouse mode is on for the private server, which is what makes that wheel scroll work. The tradeoff is
the usual tmux one: your terminal's own click-and-drag text selection needs `Shift` (or `Option` on
iTerm2 and Terminal.app) held down while you are attached.

**Compose prompt**

| Key | Action |
|---|---|
| `Enter` | Commit this line, start another |
| `Enter` on an empty line, `Ctrl-s` | Launch everything queued |
| `Ctrl-u` | Clear the line |
| `Esc` | Cancel |

**Confirm and quit dialogs**

| Key | Action |
|---|---|
| `←` `→` `↑` `↓` | Move the highlighted option |
| `Enter` | Choose the highlighted option |
| `y` / `n` | Yes / no (confirm dialogs) |
| `k` / `x` | Keep / kill all (quit dialog) |
| `Esc` | Cancel |

## How it works

Each command runs in its own detached tmux session on a **private socket** (`multi-run` by default),
so multi-run's sessions can never collide with your own. The TUI polls `capture-pane` and draws the
output itself, batching every visible tile into a single tmux invocation per poll. Attaching hands
the real terminal to `tmux attach-session`, which is why interaction inside a tile is genuinely
identical to using tmux directly.

Sessions are kept alive after their command exits (`remain-on-exit`), which is what lets you read
the output of something that already finished. That is also why "close" and "dismiss" are separate:
`s` stops the process but keeps its output, `d` throws the output away once you are done with it.

`s` escalates because `send-keys C-c` only reaches a process that reads its terminal. A job that
traps SIGINT, or never reads stdin at all, shrugs it off. The second press signals the pane's
process group with SIGTERM and the third with SIGKILL, so `s` always eventually works.

State lives entirely in tmux's own `@mr_*` session options. There is no database and no state file
to get out of sync, which is why `--list` and `--attach` keep working across reboots of the tool.

The private server sources your `~/.tmux.conf` first, then applies its own settings on top, so your
prefix and bindings still work while attached. Nothing is ever written to your tmux config.

Each job pane runs through your `$SHELL` with that shell's profile loaded (login and interactive
flags, so zsh picks up `.zshrc`, bash picks up `.bashrc`, and so on). That matches what a normal
terminal emulator does, including PATH and other env from your rc files. multi-run writes a small
`user-shell` wrapper next to its generated tmux.conf for this; it is regenerated when needed.

Those settings on top include the attached-session status bar, the no-prefix keys, and
`base-index 0` / `pane-base-index 0`. The bases matter: multi-run addresses each job as
`<session>:0.0` rather than "whatever pane that session is currently on", so a window or split you
open yourself while attached is never mistaken for the job by the tile, the exit status, or `s`/`e`.
Pinning the bases makes that address true even if your own config numbers windows from 1.

The private tmux server starts in the directory you ran multi-run in, and keeps it for its whole
life: it is a daemon that outlives the invocation that started it, and `exit-empty off` means ours
never exits even with nothing left to hold. Delete or rename that directory afterwards and the
server is left pointing at a path that no longer exists. From then on every pane it creates inherits
the broken directory no matter what `new-session -c` said, so each tile opens with `shell-init:
error retrieving current directory` and anything that calls `getcwd` at shell startup breaks (a
version-manager hook such as fnm's, for one).

Nothing can move a running process's working directory from outside, so multi-run checks for this at
startup instead. A stale server holding no sessions is replaced silently, which is the common case
because an idle server sticks around indefinitely. A stale server that still holds jobs is reported
rather than thrown away: replacing it would discard output you may not have read yet, so the message
points at `--restart-server` and leaves the decision to you.

The generated config carries a version header. When a new version of multi-run rewrites it,
multi-run also sources it into a server that is already running, so new keys and the status bar
show up without killing your jobs. Options that only apply when something is created, `base-index`
among them, still only affect sessions started after that point; `--restart-server` is the clean
slate.

### Known rough edges

- Resizing the terminal after a job has finished re-lays-out its historical output rather than
  re-wrapping it, so a finished tile can look slightly off until dismissed. This is a tmux
  limitation on dead panes.
- On tmux older than 3.3 a finished pane shows tmux's "pane is dead" banner, which pushes one line
  of output into scrollback. multi-run compensates by capturing an extra line on those versions.
- Switching sessions with `Ctrl-n` / `Ctrl-p` and then detaching leaves the grid's selection on the
  job you originally attached to. tmux does not tell the tool which session the client was on when
  it went away.
- The stale-server check reads the server process's working directory from `/proc` on Linux and
  through `lsof` elsewhere. Where neither is available the check is skipped rather than guessed at,
  and a stale server stays undiagnosed until you run `--restart-server`. It also cannot see the case
  where the old directory has been replaced by a *new* directory at the same path, since the path
  resolves again even though the server is still holding the old one.

## Documentation

- [`docs/DESIGN.md`](docs/DESIGN.md): the full architecture and implementation plan.
- [`docs/tmux-research.md`](docs/tmux-research.md): verified tmux behaviour this is built on,
  including the ten gotchas that shaped the design.
