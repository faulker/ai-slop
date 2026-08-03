# tmux Research for `multi-run`

All findings verified against **tmux 3.7b (Homebrew, macOS arm64, Darwin 25.5.0)** on 2026-07-28,
using isolated sockets (`tmux -L <name>`) so the user's default server was never touched.
Items marked UNVERIFIED were not reproduced locally.

## Headline architectural finding

Run everything on a **private socket with a private config file**:

```
tmux -L multi-run -f ~/.config/multi-run/tmux.conf <command...>
```

This one decision solves four problems at once: namespacing (item 2), the `remain-on-exit`
startup race (item 1), custom keybindings without touching `~/.tmux.conf` (item 5), and
config isolation. The user's `~/.tmux.conf` is never written to. `-f` is honoured only when
the server is first started; later invocations may omit it and still inherit the globals
(verified).

Recommended `tmux.conf` contents:

```tmux
# Optionally inherit the user's own settings first, so our overrides win.
source-file -q ~/.tmux.conf
source-file -q ~/.config/tmux/tmux.conf

set -g remain-on-exit on
set -g remain-on-exit-format ""      # requires tmux >= 3.3; see item 1
set -g exit-empty off
set -g detach-on-destroy on
set -g history-limit 50000
set -g default-shell /bin/sh         # see item 1 gotcha
set -g escape-time 10
bind -n M-d detach-client            # server-local, never touches user config
```

Since conf version 2 the shipped file also turns the status line back **on** (an attached client
renders it; a polled, detached window does not, so tiles are unaffected) and binds `C-\` to detach
plus `C-n`/`C-p` to `switch-client -n`/`-p`. See DESIGN.md section 5 for the current text.

---

## 1. Session creation and exit detection

### Verified behaviour

Without `remain-on-exit`, the pane, window, session and (if it was the only session) the
whole server disappear the instant the command exits. Verified: a `new-session` running
`exit 3` left `no server running` within 500ms.

`remain-on-exit` is resolved as a **pane option** that inherits from the window scope.
`show-options -g`, `-wg` and `-pg` all report it.

### Exact invocation (recommended)

```
tmux -L multi-run -f <conf> new-session -d -s <name> -c <cwd> -x <W> -y <H> '<shell command>'
```

With `set -g remain-on-exit on` in `<conf>` there is **no race at all**, because the option
is set at server start, before any pane exists.

### Comparison of approaches

| Approach | Verdict |
|---|---|
| `-f conf` with `set -g remain-on-exit on` | **Best.** Race-free, no user config mutation, exit status available as a first-class format. |
| Chain `\; set-option -p -t <sess> remain-on-exit on` after `new-session` | Works. Hammer-tested 50x with an instantly-exiting `exit 7` command: 50/50 survived with `pane_dead_status=7`. tmux runs the whole command queue before returning to the event loop, so the option lands before the SIGCHLD is processed. Fine as a fallback for tmux < 3.3, but strictly worse than `-f`. |
| `tmux start-server \; set-option -g ...` then create sessions | **Broken.** Fails with `server exited unexpectedly`, because a server with zero sessions exits immediately (`exit-empty` defaults to `on`). |
| Wrap in a shell that waits (`sh -c 'cmd; echo $?; read x'`) | **Not recommended.** Needs a live stdin, pollutes the pane output, requires scraping the exit code out of the screen, and the pane is still "alive" so you cannot distinguish finished from running. |

### Detecting completion and exit status

One call gets everything:

```
tmux -L multi-run list-panes -a -F '#{session_name}|#{pane_dead}|#{pane_dead_status}|#{pane_dead_signal}|#{pane_dead_time}|#{pane_pid}'
```

Verified results:
- Normal exit 42 → `pane_dead=1`, `pane_dead_status=42`, `pane_dead_signal=` (empty).
- `kill -TERM` → `pane_dead=1`, `pane_dead_status=` (empty), `pane_dead_signal=term`, `pane_dead_time=<unix ts>`.

So: `pane_dead=1` means finished; then exactly one of `pane_dead_status` / `pane_dead_signal`
is populated.

### GOTCHA A: the "Pane is dead" banner scrolls your output

When a pane dies with `remain-on-exit on`, tmux writes `remain-on-exit-format` into the pane
at the **bottom row**, which pushes the whole screen up by one line. Verified: a pane printing
`A` then `B` ended up with `history_size=1`, `cursor_y=23`, and a plain
`capture-pane -p` returned `B` as its first line. `A` had scrolled into the scrollback.

Fix (tmux >= 3.3):

```tmux
set -g remain-on-exit-format ""
```

Verified after this: `history_size=0`, `cursor_y=2`, capture returns `A`, `B`, and
`pane_dead_status` still works. On tmux < 3.3 you must either accept the banner or always
capture with `-S -1` to recover the scrolled line.

### GOTCHA B: tmux runs your command with `$SHELL`, not `/bin/sh`

Verified: `ps` inside the pane showed `zsh -c <command>`, because `default-shell` defaults to
the user's login shell. If the user's shell is fish or another non-POSIX shell, your command
string is interpreted with different syntax. Set `set -g default-shell /bin/sh` in the config
for predictable semantics. Exit codes propagate correctly through the shell (verified `exit 42`
→ `pane_dead_status=42`).

An argv form also works and avoids a shell entirely: `new-session -d -s x /bin/echo hello world`
(verified `rc=0`).

### `remain-on-exit failed`

tmux 3.2+ accepts `failed` as a value: the pane is kept only if the command exited non-zero.
Verified: `exit 0` session vanished, `exit 1` session stayed with `pane_dead=1 st=1`.
Not what `multi-run` wants (we want all output readable) but worth knowing.

**RECOMMENDATION:** Private socket + `-f` config with `set -g remain-on-exit on`,
`set -g remain-on-exit-format ""`, `set -g default-shell /bin/sh`. Detect completion by polling
`pane_dead` / `pane_dead_status` / `pane_dead_signal` in the same batched call that fetches
everything else. Require tmux >= 3.3 for the empty banner format; degrade gracefully below that.

---

## 2. Namespacing and discovery

### Verified behaviour

Two independent mechanisms, and you want both.

**Private socket** (`-L multi-run`) gives hard isolation: the user's sessions on the default
socket are simply not in the same server and cannot collide. Verified: after all experiments,
`tmux ls` on the default socket still reported `no server running`.

**User options** (`@`-prefixed) carry per-session metadata and survive fine. Set them:

```
tmux -L multi-run set-option -t <name> @mr_group   'run-1785271018'
tmux -L multi-run set-option -t <name> @mr_cmd     'cargo test --all'
tmux -L multi-run set-option -t <name> @mr_cwd     '/Users/sane/Dev'
tmux -L multi-run set-option -t <name> @mr_started '1785271018'
```

Read them back, **filtered server-side to only our sessions, in a single call**:

```
tmux -L multi-run list-sessions \
  -f '#{!=:#{@mr_group},}' \
  -F '#{session_name}|#{@mr_group}|#{@mr_cmd}|#{@mr_cwd}|#{@mr_started}|#{session_attached}'
```

Verified output:

```
mr-run-1785271018-1|run-1785271018|echo one; sleep 300|/Users/sane|1785271018|0
mr-run-1785271018-2|run-1785271018|seq 1 5; exit 2|/Users/sane|1785271018|0
```

A session without `@mr_group` (created separately) was correctly excluded. Session options do
not leak between sessions (verified: `show-options -t unrelated | grep @` was empty).

Single value read: `tmux -L multi-run show-options -v -t <name> @mr_cmd`.

### GOTCHA C: tmux target names are prefix-matched

`tmux has-session -t L` returned **rc=0** while the only session was named `L2`. Any partial
name matches. Use the `=` exact-match prefix for anything destructive:

```
tmux -L multi-run has-session   -t '=mr-run-123-1'      # rc 0 exact, rc 1 otherwise
tmux -L multi-run kill-session  -t '=mr-run-123-1'
```

Verified: `has-session -t '=L'` → `can't find session: L`, rc=1. Correct.

### GOTCHA D: `set-option -t '=name'` fails, `set-option -t '=name:'` works

Verified: `set-option -t '=mr-1' @x y` → `no such session: =mr-1`, because `set-option`
parses its target as a pane target by default. The trailing colon forces session parsing.

```
tmux -L multi-run set-option -t '=mr-1:' @mr_cmd '...'      # rc=0, works
tmux -L multi-run set-option -t 'mr-1'   @mr_cmd '...'      # rc=0, works (prefix-matched)
```

Since the field separator matters, use `#{l:\t}` in the format string to emit a literal tab,
or pick a separator that cannot occur in a command string. Command strings can contain `|`,
so tab is safer, and note that a command could theoretically contain a tab too. The
bulletproof option is one `show-options -v` call per field, but that costs round trips.

**RECOMMENDATION:** Private socket `-L multi-run` for isolation, plus an `@mr_*` user-option
namespace for metadata, plus a `mr-<group>-<n>` name prefix purely for human readability in
`tmux ls`. Discovery is a single `list-sessions -f '#{!=:#{@mr_group},}' -F ...` call. Always
use `=name` (or `=name:` for `set-option`) when targeting a specific session.

---

## 3. Capturing output for rendering

### Exact invocation

```
tmux -L multi-run capture-pane -p -e -t <session>
```

- `-p` writes to stdout instead of a paste buffer.
- `-e` includes SGR escape sequences. Verified output for a red/bold-green line:
  `^[[31mRED^[[39m ^[[1m^[[32mBOLDGREEN^[[0m plain`. Note tmux emits `^[[39m` (default
  foreground) rather than a full reset when a colour ends.
- No `-S`/`-E` means "visible screen only", which is exactly what you want if the pane is
  already sized to the tile (item 4).

Scrollback variants:
- `-S -` start of history, `-E -` end of visible screen. `-S - -E -` = everything.
- Negative `-S` values are history lines: `-S -50` = 50 lines of scrollback plus the visible screen.
- Last N lines of an H-tall pane: `-S $((H-N)) -E -`.

### Flag semantics (from the 3.7b man page, verified)

- `-J` preserves trailing spaces **and joins wrapped lines** (implies `-T`). Verified: a
  25-char line in a 20-col pane came back as two physical lines without `-J` and one logical
  line with `-J`. **Do not use `-J` for tile rendering**, it destroys the physical layout you
  are trying to mirror. Use it for a "copy full output" feature.
- `-N` preserves trailing spaces. **Do not rely on it for padding.** Verified inconsistency in
  a 20-column pane: a full-width CJK line came back at 20 columns, but `AB` came back as
  `'AB   '` (5 columns) and `中文` as `'中文 '` (5 columns). It pads to an internal per-line
  extent, not the pane width.
- `-T` is accepted (rc=0) despite being missing from the usage flag list in the man page.
- `-a` captures the *alternate* screen. You do **not** need it for full-screen apps: a pane
  running vim was captured normally with plain `capture-pane -p` (verified, `alternate_on=1`).
- `-C` escapes non-printables as octal, `-L` prepends line numbers, `-F` prepends per-line flags.

### Unicode

Verified clean on 3.7b. `中文`, `🚀` and mixed lines round-trip as valid UTF-8, and tmux wraps
CJK correctly at the pane width (12 CJK chars in a 20-column pane split as 10 + 2). Column
accounting inside tmux uses display width, so a Rust renderer should use `unicode-width` to
agree with it.

### Cost benchmark

8 sessions, 100x40 panes, each running a busy `while :; do date; ls /usr/bin | head -20; done`
loop. 100 polls each, Python driver, same machine, `capture-pane -p -e` per pane:

| Strategy | ms per poll (8 panes) | ms per pane | Relative |
|---|---|---|---|
| 8 separate `tmux` processes | **39.19** | 4.90 | 1x |
| 1 `tmux` process, 8 `capture-pane` commands joined with `;` | **5.35** | 0.67 | **7.3x faster** |
| Persistent control-mode client (`tmux -C attach`) | **0.42** | 0.05 | **93x faster** |

CPU cost is negligible in all cases; the process-per-poll cost is almost entirely fork/exec
and socket-connect latency (~4.5ms per `tmux` invocation, measured independently at 4.2ms for
100 single captures).

Batching syntax (one process, N captures, output concatenated in order):

```
tmux -L multi-run capture-pane -p -e -t s1 \; capture-pane -p -e -t s2 \; ... 
```

Interleave `\; display-message -p '@@MARK s2'` between captures to get parseable delimiters,
or rely on the fact that each pane emits exactly `pane_height` lines when the pane size is
known (which it is, because you set it).

### Control mode notes

`tmux -C -L <sock>` with **no command defaults to `new-session`** and silently creates a stray
session (verified: a session literally named `8` appeared). To get a control client without
that side effect, create a dedicated hidden session and attach to it:

```
tmux -L multi-run new-session -d -s __mr_ctl -x 80 -y 24 'sh -c "while :; do sleep 3600; done"'
tmux -C -L multi-run attach-session -t __mr_ctl
```

Then write commands to its stdin and parse `%begin <ts> <cmdid> <flags>` / output /
`%end <ts> <cmdid> <flags>` (or `%error`) blocks. The command id lets you match responses to
requests. Verified working.

tmux 3.2+ also supports format subscriptions, which push changes instead of you polling:

```
refresh-client -B 'jobstate:%*:#{pane_dead},#{pane_dead_status}'
```

These arrive as `%subscription-changed` notifications, rate limited to **at most once per
second**, and `%*` only covers panes in the *attached* session. That rate limit and scoping
make subscriptions a poor fit for tile content but a reasonable fit for job-completion
notifications if you attach a control client per session. UNVERIFIED: I did not exercise `-B`
end to end.

**RECOMMENDATION:** Start with the **batched single-process** strategy: one `tmux` invocation
per poll containing one `capture-pane -p -e` per visible tile, with `display-message` markers
between them. At 5.35ms for 8 panes it comfortably supports a 10Hz redraw at under 6% of one
core, it is trivial to implement and debug, and it has no protocol state. Poll at **10Hz for
visible/focused tiles and 1-2Hz for offscreen ones**, and only capture tiles that are actually
on screen. Keep control mode as a documented optimisation for when tile counts exceed roughly
30, where the 0.05ms/pane figure starts to matter. Parse the captured ANSI with `ansi-to-tui`.

---

## 4. Sizing

This is the highest-risk area and the behaviour is non-obvious.

### Verified behaviour

Initial size comes from `new-session -x W -y H`, and it is honoured for a detached session
(verified `-x 40 -y 10` → `pane_width=40 pane_height=10`).

To resize later:

```
tmux -L multi-run resize-window -t <session> -x <W> -y <H>
```

Verified: works on a fully detached session, sticks, and the child process sees it. A shell
running `stty size` in a loop reported `11 37` then `15 55` after a resize, so tmux does
deliver SIGWINCH and update the pty winsize.

Defaults on 3.7b: `window-size latest` (global), `aggressive-resize off`.

### GOTCHA E: `resize-window` silently sets `window-size manual`

The man page states it outright: "This command will automatically set window-size to manual in
the window options." Verified: after `resize-window -x 60 -y 20`, `show-options -w -t a
window-size` returned `manual`.

Consequence, verified with a nested tmux acting as a real 120x40 terminal:
- With `window-size manual`, attaching a 120x40 client left the pane at **60x20**. The user
  gets a small pane marooned in a big terminal.
- After `set-option -uw -t <session> window-size` (unset the per-window override, falling back
  to the global `latest`), attaching the same client gave **120x40**. Correct.
- After detaching, the pane **stayed at 120x40**. A detached session keeps whatever size it
  last had; it does not revert.
- `resize-window -t a -x 44 -y 12` then restored the tile size.

So the size does not manage itself across an attach round trip. You must drive it explicitly.

### The sequence

```
# steady state, per tile, whenever the tile geometry changes
tmux -L multi-run resize-window -t '=<sess>' -x <tileW> -y <tileH>

# before handing the terminal over
tmux -L multi-run set-option -uw -t '=<sess>:' window-size     # back to 'latest'
tmux -L multi-run attach-session -t '=<sess>'

# after the user detaches
tmux -L multi-run resize-window -t '=<sess>' -x <tileW> -y <tileH>
```

Note `-uw` unsets a window option, restoring inheritance from the global value.

`aggressive-resize` is irrelevant here: it only affects windows shown in multiple sessions,
and each of our sessions has exactly one window.

Alternative for control-mode users: a control client can declare a per-window size with
`refresh-client -C '@<window-id>:<W>x<H>'` (documented in the 3.7b man page). UNVERIFIED end
to end; `resize-window` is simpler and works from any invocation.

Also note that resizing a **dead** pane works (verified 38x9 → 30x6) but tmux obviously cannot
re-run the command, so the historical content is re-laid-out rather than re-wrapped from the
original byte stream. Expect cosmetic imperfection when the user resizes after a job finished.

**RECOMMENDATION:** Create each session with `-x`/`-y` at the exact tile inner size, and call
`resize-window -x -y` on every tile whenever the layout changes (debounce it, roughly 100ms
after the last SIGWINCH). Because the pane is exactly the tile size, `capture-pane -p -e` with
no `-S`/`-E` returns exactly the rows you need and wrapping matches by construction. Always
`set-option -uw ... window-size` before attach and `resize-window` after detach.

---

## 5. Attach / detach round trip

### What tmux actually emits (verified byte-level via a pty probe)

On **attach**, the first bytes tmux writes to the terminal are:

```
\x1b[?1049h  \x1b[22;0;0t  \x1b[?1h  \x1b=  \x1b[H  \x1b[2J  \x1b[?25h
\x1b[?1000l \x1b[?1002l \x1b[?1003l \x1b[?1006l \x1b[?1005l  \x1b[?2004h  \x1b[?2031h ...
```

On **detach**:

```
\x1b[?25l \x1b[?12l \x1b[?25h \x1b[1;40r \x1b(B \x1b[m \x1b[?1l \x1b> \x1b[H \x1b[2J
\x1b[?1000l \x1b[?1002l \x1b[?1003l \x1b[?1006l \x1b[?1005l \x1b[?2004l \x1b[?7727l
\x1b[?1004l \x1b[?1049l \x1b[23;0;0t \x1b[?2031l  [detached (from session A)]\r\n
```

The load-bearing facts:
1. tmux **unconditionally enters the alternate screen** (`\x1b[?1049h`) on attach and
   **unconditionally leaves it** (`\x1b[?1049l`) on detach. It uses 1049, not the old 47.
2. Therefore, if your ratatui app is still in the alternate screen when tmux detaches, tmux's
   `?1049l` drops the terminal back to the **primary** screen and your TUI is left drawing into
   a buffer nobody is looking at. You must leave the alt screen before attaching.
3. tmux resets mouse modes, bracketed paste and cursor visibility on the way out, so it does
   not leave those enabled behind it. It does **not** restore raw mode; it restores the termios
   state it saved at attach time, which is the state you handed it.
4. It prints `[detached (from session A)]` to the primary screen on the way out. Harmless if
   you immediately re-enter the alt screen. There is no documented flag to suppress it.

Also verified: the session survived detach (`attached=0`), and while attached from a 120x40
client the pane was 120x40.

### Recommended sequence from a crossterm/ratatui app

```
# suspend
stop the input reader / EventStream and join it        <- see gotcha F
disable_raw_mode()
execute!(stdout, DisableMouseCapture, LeaveAlternateScreen)
stdout.flush()

# hand over: inherit stdin/stdout/stderr, wait for it
Command::new("tmux")
    .args(["-L", "multi-run", "attach-session", "-t", "=<sess>"])
    .status()

# resume
enable_raw_mode()
execute!(stdout, EnterAlternateScreen, EnableMouseCapture, Clear(ClearType::All))
terminal.clear()            # discard ratatui's stale diff buffer
re-read terminal size, re-apply resize-window to every tile
restart the input reader
force a full redraw
```

`terminal.clear()` matters: ratatui diffs against its previous buffer, and that buffer is
garbage after another program owned the screen.

### GOTCHA F: two readers on one stdin

`tmux attach` runs in your process group and shares your stdin. If a crossterm event thread in
your process is blocked in `read(0, ...)` at the same time, the two will race and steal each
other's keystrokes. You must stop your input thread before spawning tmux and restart it after.
This is an **inference** from POSIX foreground-process-group semantics plus crossterm's
blocking-read design; I verified that tmux and the parent share the tty, but did not reproduce
the race itself. UNVERIFIED as a reproduced failure, but treat it as real.

### SIGWINCH

The tmux client installs its own SIGWINCH handler and resizes the session live. Your process
may also receive SIGWINCH while suspended; that is harmless as long as you do not draw. Because
a detached session keeps its last size (item 4), you must re-apply `resize-window` to every
tile after resuming, not just to the one that was attached.

### Making detach ergonomic without touching the user's config

Because we run our own server started with our own `-f` file, **every keybinding on that server
is ours**. Verified working: `bind -n M-d detach-client` in the config file, then sending
`ESC d` to the pty detached the client immediately. `-n` means no prefix key is needed. The same
mechanism carries the shipped binding, `bind-key -n 'C-\' detach-client`; the key spec is
single-quoted so the backslash survives config parsing.

Add for good measure:

```tmux
set -g detach-on-destroy on    # verified present and default 'on' in 3.7b
```

so that killing the session while attached detaches the client rather than switching it to some
other session.

If you want the user's own prefix and bindings available during attach, `source-file -q
~/.tmux.conf` **first** in your config and put your `set -g` overrides after it, so yours win.
`-q` suppresses the error when the file does not exist (verified rc=0 with the file absent).
Nothing is ever written back to the user's file.

**RECOMMENDATION:** Leave alt screen and raw mode, stop the input reader, run
`tmux -L multi-run attach-session -t '=<sess>'` as an inherited-stdio foreground child, then
restore in reverse order plus `terminal.clear()` and a full `resize-window` sweep. Advertise the
detach key (`C-\` as shipped, `M-d` also bound) via a `-n` binding in our own config file, and put
it on the status line so it is visible while attached. Never write to `~/.tmux.conf`.

---

## 6. Interacting without attaching

### Verified behaviour

`send-keys` works well, including for full-screen curses applications.

```
tmux -L multi-run send-keys -t '=<sess>' 'echo hello' Enter    # key names interpreted
tmux -L multi-run send-keys -t '=<sess>' -l 'literal text'     # no key-name interpretation
tmux -L multi-run send-keys -t '=<sess>' C-c                   # control characters
tmux -L multi-run send-keys -t '=<sess>' -H 03                 # raw hex bytes
```

All four verified. Specifically:
- `'echo hello-from-sendkeys' Enter` produced the expected output in an `sh` pane.
- `C-c` interrupted a running `sleep 60` (pane showed `^C` and returned to the prompt).
- `-l 'echo Enter-is-literal-here'` correctly did **not** interpret `Enter` as a key name.
- `-H 03` accepted (rc=0), giving byte-level control.
- A full-screen app works: launched `vi /etc/hosts`, pane reported `alternate_on=1` and
  `pane_current_command=vim`, sending `G` moved the cursor to `cursor_y=9`, and
  `Escape :q! Enter` quit cleanly back to `alternate_on=0`.

### Limits

- Key names are tmux's own vocabulary (`Enter`, `Escape`, `C-c`, `M-x`, `Up`, `F1`, ...). You
  must translate crossterm `KeyEvent`s into that vocabulary, and the mapping is lossy at the
  edges: modifier combinations, kitty-protocol keys, focus events and paste framing do not
  translate cleanly.
- Mouse input is not expressible through `send-keys` at all.
- Each `send-keys` is a separate `tmux` invocation at roughly 4.5ms, so fast typing means one
  process per keystroke. Acceptable for occasional input, wasteful as a primary interaction
  model (control mode fixes this if you go that route).
- There is no echo/latency guarantee: you send, then wait for the next poll to see the result,
  so typing feels laggy compared to a real attach.

**RECOMMENDATION:** Support `send-keys` for a deliberately narrow set of actions rather than as
a general input path. Concretely: a "send interrupt" key that issues `C-c`, a "send EOF" that
issues `C-d`, and optionally a small prompt that sends a literal line plus `Enter` via `-l`.
For anything genuinely interactive, require a full attach. Trying to proxy arbitrary keystrokes
through `send-keys` will produce a subtly broken terminal experience and is not worth the
complexity.

---

## 7. Lifecycle

### Sessions outlive our process: confirmed

`ps -o pid,ppid` showed every experiment's tmux server with **PPID 1**. The client that runs
`new-session` forks the server, which daemonises and reparents to launchd/init. Sessions
created in one shell invocation were still present in later, completely separate invocations
throughout this research. Nothing extra is needed for sessions to survive `multi-run` exiting.

### Killing

```
tmux -L multi-run kill-session -t '=<sess>'      # one session
tmux -L multi-run kill-server                    # everything on our socket
```

Verified: killing `L1` left `L2` running. Killing a missing session gives rc=1 and
`can't find session: ghost` on stderr.

### Detecting a session that died externally

`has-session` is the cheap check:

```
tmux -L multi-run has-session -t '=<sess>'    # rc 0 alive, rc 1 gone
```

Verified rc=0 for a live session and rc=1 for a missing one. In practice you get this for free:
the batched `list-sessions -f ... -F ...` poll from item 2 returns the authoritative set, so any
session missing from that output died externally. Diff it against your in-memory list each poll.

### Detecting "no server running" cleanly

Two distinct messages, both rc=1, both on **stderr**:

- `error connecting to /private/tmp/tmux-501/<sock> (No such file or directory)` when the socket
  file has never existed.
- `no server running on /private/tmp/tmux-501/<sock>` when the socket exists but the server is
  gone.

There is no dedicated exit code. Treat rc != 0 **plus** stderr containing either
`no server running` or `error connecting to` as the benign "zero sessions" case, and anything
else as a real error. Do not treat a bare non-zero exit as fatal.

`exit-empty off` changes this: with it set, the server persists with zero sessions and
`list-sessions` returns **rc=0 with empty stdout** (verified). That is arguably a cleaner state
machine, at the cost of leaving an idle server process around. `exit-empty` has existed since
tmux 2.7.

Note `#{version}` via `display-message` requires a running server; use `tmux -V` for preflight
(item 8).

**RECOMMENDATION:** Do nothing special for survival, it is the default. Use one batched
`list-sessions` poll as the single source of truth for liveness. Classify no-server as an empty
result, not an error, by matching both stderr strings. Set `exit-empty off` so the server
survives the user killing every job, which keeps `--list` and `--attach` behaviour uniform.

---

## 8. Preflight

### Detect presence and version

```
tmux -V
```

Verified output on this machine: `tmux 3.7b`. Detect presence by the spawn succeeding
(`ErrorKind::NotFound` means not installed); do not shell out to `which`.

### GOTCHA G: tmux versions are not semver

Per the [tmux FAQ](https://github.com/tmux/tmux/wiki/FAQ), version strings take these forms:

| Form | Example | Meaning |
|---|---|---|
| Main release | `3.0`, `2.9`, `3.7` | one digit after the period |
| Patch release | `3.0a`, `3.7b` | letter suffix, ordered `3.7 < 3.7a < 3.7b` |
| Development | `next-3.8` | git master heading toward 3.8 |
| Release candidate | `3.4-rc`, `3.4-rc1` | pre-release |

A naive semver parse breaks on all of these except the first. Suggested parse:

```
^(?:tmux\s+)?(next-)?(\d+)\.(\d+)([a-z]?)(-rc\d*)?$
```

Order by `(major, minor, letter_or_empty)` where empty sorts before `a`. Treat `next-X.Y` as
approximately `X.Y` for feature gating (it contains X.Y's features in progress) and
`X.Y-rc` likewise. Note Arch's *package* version renders as `3.7_b`, but `tmux -V` still
prints `3.7b`, so parse the command output, never a package string.

### Minimum version for our feature set

Determined from the upstream [CHANGES](https://raw.githubusercontent.com/tmux/tmux/master/CHANGES) file:

| Feature we use | First appeared |
|---|---|
| `exit-empty` | 2.7 |
| `resize-window`, `window-size` option | 2.9 |
| `remain-on-exit failed` value | 3.2 |
| control-mode format subscriptions (`refresh-client -B`) | 3.2 |
| **`remain-on-exit-format`** (banner suppression) | **3.3** |
| `remain-on-exit` itself, `capture-pane -e`, user options `@foo` | much older (1.x / 2.3) |

So: **minimum 3.3**, which is the `remain-on-exit-format` requirement. On 3.0 to 3.2a
everything else works but you inherit gotcha A (one line of output scrolls off when a job
finishes); you can compensate by capturing with `-S -1` on dead panes. Below 2.9 you lose
`resize-window` and the whole tile-sizing model, so hard-fail there.

### Distro reality check (from [repology](https://repology.org/project/tmux/versions))

| Platform | tmux version |
|---|---|
| Homebrew (macOS) | 3.7b |
| Arch | 3.7b |
| Fedora 43 | 3.7b |
| Fedora 41 / 42 | 3.5a |
| Debian 13 | 3.5a |
| Debian 12 | 3.3a |
| Ubuntu 26.04 | 3.6a |
| Ubuntu 24.04 | 3.4 |
| **Ubuntu 22.04** | **3.2a** (below our 3.3 target) |
| Alpine 3.21 | 3.5a |

Ubuntu 22.04 is the one common platform that falls short, which is a good argument for
degrading gracefully at 3.0-3.2 rather than hard-failing.

### Install advice

```
macOS (Homebrew)   brew install tmux
Debian / Ubuntu    sudo apt install tmux
Fedora             sudo dnf install tmux
Arch               sudo pacman -S tmux
Alpine             sudo apk add tmux
```

**RECOMMENDATION:** Run `tmux -V` once at startup. Parse with the regex above, not a semver
crate. Hard-fail below 2.9 with a per-platform install hint, warn and disable banner
suppression between 2.9 and 3.2a, and take the full path at 3.3+. Print the detected version in
`--version` output to make bug reports tractable.

---

## 9. Rust crates

### Current versions (crates.io API, 2026-07-28)

| Crate | Latest | Last published | Notes |
|---|---|---|---|
| `ratatui` | **0.30.2** | 2026-06-19 | now split into `ratatui-core` 0.1 / `ratatui-widgets` 0.3 |
| `crossterm` | **0.29.0** | 2025-04-05 | stable, what ratatui 0.30 targets |
| `ansi-to-tui` | **8.0.1** | 2026-01-10 | depends on `ratatui-core ^0.1`, so aligned with ratatui 0.30 |
| `tmux_interface` | 0.4.0 | 2026-03-10 | see below |
| `vt100` | 0.16.2 | 2025-07-12 | terminal emulator, if you ever parse raw output yourself |
| `vte` | 0.15.0 | 2025-02-02 | lower-level ANSI parser (alacritty) |
| `tui-term` | 0.3.4 | 2026-04-07 | ratatui widget wrapping `vt100` + `portable-pty` |
| `portable-pty` | 0.9.0 | 2025-02-11 | wezterm's pty crate |

### On `tmux_interface`

`tmux_interface` (also indexed as `tmux-interface`, same crate) is a typed builder API over the
tmux CLI. Real but shaky:

- Version 0.4.0 shipped 2026-03-10, but the previous release 0.3.2 was 2024-03-04. Two-year gaps.
- Its own README says the library "is still in experimental development stage (unstable)",
  that "many features are unimplemented or not well tested", and that APIs may change.
- It gates behaviour behind per-tmux-version cargo features (`tmux_3_4`, `tmux_3_5a`,
  `tmux_3_6`, `tmux_3_6a`, `tmux_X_X`, ...). The newest concrete release it knows about is
  **3.6a**, so **there is no `tmux_3_7` feature** for the version you are actually developing
  against.
- The version-feature model is compile-time, which is a poor fit for a tool that must adapt to
  whatever tmux the user has at runtime.
- ~70k recent downloads, versus ~15M for ratatui, so very little field testing.

Against that, the surface area you need is tiny and entirely stable across tmux versions:
`new-session`, `list-sessions`, `list-panes`, `capture-pane`, `resize-window`, `set-option`,
`show-options`, `send-keys`, `has-session`, `kill-session`, `attach-session`. That is eleven
subcommands, all invoked as `std::process::Command` with `-F` format strings you control.
Wrapping them yourself is maybe 200 lines, gives you exact control over batching (which item 3
showed is worth 7x), and lets you do runtime version detection instead of compile-time features.

`tui-term` is worth a look but solves a different problem: it pairs `portable-pty` with `vt100`
to run a process in an in-process pty and render it. That is the "no tmux at all" architecture.
It is incompatible with the requirement that sessions outlive the tool, so it is not a fit here,
though its `PseudoTerminal` widget is a useful reference for how to render a terminal grid in
ratatui.

**RECOMMENDATION:** **Shell out with `std::process::Command`.** Do not take a dependency on
`tmux_interface`: it has no tmux 3.7 support, self-describes as experimental, has a compile-time
version-feature model that fights runtime detection, and would obstruct the command batching
that gives the biggest performance win. Write a thin internal `tmux.rs` module that owns the
socket name, the `-f` config path, argument construction and the `-F` format parsing. Depend on
`ratatui` 0.30.2, `crossterm` 0.29.0, and `ansi-to-tui` 8.0.1 for turning captured escape
sequences into ratatui `Text`.

---

## Summary of gotchas

| # | Gotcha | Fix |
|---|---|---|
| A | Dead-pane banner scrolls output up one line | `set -g remain-on-exit-format ""` (tmux >= 3.3) |
| B | tmux runs commands with `$SHELL`, not `/bin/sh` | `set -g default-shell /bin/sh` |
| C | Session targets are prefix-matched (`-t L` matches `L2`) | use `=name` |
| D | `set-option -t '=name'` fails | use `=name:` or the bare name |
| E | `resize-window` silently sets `window-size manual`, overriding attached clients | `set-option -uw <sess> window-size` before attach, `resize-window` after detach |
| F | tmux attach and your crossterm reader race on stdin | stop the input thread before attaching |
| G | tmux versions are not semver (`3.7b`, `next-3.8`, `3.4-rc1`) | custom regex, order by (major, minor, letter) |
| H | `tmux -C` with no command creates a stray session | `tmux -C attach-session -t __mr_ctl` |
| I | `capture-pane -N` does not pad to pane width | pad in Rust with `unicode-width` |
| J | `start-server` alone exits immediately | `set -g exit-empty off`, or just create a session |
