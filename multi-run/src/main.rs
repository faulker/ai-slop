//! Entry point for multi-run. Owns preflight, CLI dispatch, terminal
//! setup/teardown, the panic hook, and the main event loop. Contains no tmux
//! argv construction, no drawing, and no key mapping; those live in `tmux/`,
//! `ui/`, and `keymap.rs`.

mod app;
#[allow(dead_code)]
mod attach;
mod cli;
mod error;
mod keymap;
mod model;
mod poll;
mod preflight;
mod tmux;
mod ui;

use std::collections::HashMap;
use std::io;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use clap::Parser;
use crossterm::event::{self, DisableMouseCapture, EnableMouseCapture, Event};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::Rect;

use crate::app::{App, Effect, Modal};
use crate::cli::{Cli, GroupId, JobId, Target, ViewArg};
use crate::error::TmuxError;
use crate::model::{Group, Job, JobState, PaneFacts};
use crate::poll::{TileKind, TileSpec, TmuxOp, ToMain, ToPoller};
use crate::preflight::Tier;
use crate::tmux::{CaptureMode, NewSessionSpec, ServerCwd, Tmux};
use crate::ui::layout::ViewMode;

/// Exit code when tmux is not installed (DESIGN.md section 2).
const EXIT_NO_TMUX: i32 = 127;

/// Main-loop tick. Short enough that a 10 Hz content poll never waits on us.
const TICK: Duration = Duration::from_millis(16);

fn main() {
    let cli = Cli::parse();
    match run(cli) {
        Ok(code) => std::process::exit(code),
        Err(e) => {
            eprintln!("multi-run: {e:#}");
            std::process::exit(1);
        }
    }
}

/// Preflight, then dispatch to a headless command or the TUI. Returns the
/// process exit code.
fn run(cli: Cli) -> Result<i32> {
    let tier = match preflight_tier() {
        Ok(t) => t,
        Err(msg) => {
            eprintln!("{msg}");
            return Ok(EXIT_NO_TMUX);
        }
    };

    let conf_path = tmux::conf::resolve_path(cli.tmux_conf.as_deref())
        .context("cannot determine a config path: neither XDG_CONFIG_HOME nor HOME is set")?;
    let conf_rewritten = tmux::conf::ensure(
        &conf_path,
        tmux::conf::ConfOptions {
            source_user_conf: !cli.no_user_conf,
            tmux_3_3_plus: tier == Tier::Full,
            ..tmux::conf::ConfOptions::default()
        },
    )
    .with_context(|| format!("cannot write tmux config at {}", conf_path.display()))?;

    let tmux = Tmux::new(cli.socket.clone(), conf_path);

    // A server left running by an older version never saw the new file: `-f`
    // is read once, at server start. Sourcing it now is what makes an
    // upgrade's key bindings and status bar reach sessions that are already
    // up, without killing them.
    if conf_rewritten && !cli.restart_server {
        let _ = tmux.source_conf();
    }

    if cli.restart_server {
        // Killing a stopped server is not an error worth reporting: the goal
        // is "no server running", which is already true.
        let _ = tmux.kill_server();
    } else {
        handle_stale_server(&tmux);
    }

    if cli.list {
        return cmd_list(&tmux);
    }
    if let Some(spec) = &cli.kill {
        return cmd_kill(&tmux, spec);
    }

    let commands = cli.all_commands();

    // `--attach` opens an existing group; `all` is only meaningful for
    // `--kill`, so reject it here rather than silently opening nothing.
    let attach_target = match &cli.attach {
        Some(s) => Some(parse_attach_target(s)?),
        None => None,
    };

    let view = cli.view.map(|v| match v {
        ViewArg::Grid => ViewMode::Grid,
        ViewArg::List => ViewMode::ListDetail,
    });

    tui(tmux, tier, commands, attach_target, view, cli.label.clone())
}

/// Deal with a server whose working directory has been deleted out from under
/// it (see `tmux::ServerCwd`). Every pane such a server creates is born broken,
/// and the only cure is a new server, so:
///
/// - holding nothing: replace it silently. There is nothing to lose, and this
///   is the common case, since `exit-empty off` keeps an idle server alive
///   forever waiting to poison the next launch.
/// - holding sessions: say so and carry on. Throwing away the user's jobs,
///   including finished ones they have not read yet, is their call to make with
///   `--restart-server`, not ours to make for them.
fn handle_stale_server(tmux: &Tmux) {
    match tmux.server_cwd_state() {
        ServerCwd::Gone { sessions: 0, .. } => {
            let _ = tmux.kill_server();
        }
        ServerCwd::Gone { cwd, sessions } => {
            let plural = if sessions == 1 { "" } else { "s" };
            eprintln!(
                "multi-run: the running tmux server started in {}, which no longer exists, so new \
                 jobs will report \"shell-init: error retrieving current directory\". Run \
                 `multi-run --restart-server` to replace it, discarding {sessions} existing \
                 session{plural}.",
                cwd.display()
            );
        }
        ServerCwd::NoServer | ServerCwd::Live | ServerCwd::Unknown => {}
    }
}

/// Resolve tmux presence and version into a capability tier, or an error
/// message to print (DESIGN.md section 10).
fn preflight_tier() -> Result<Tier, String> {
    let raw = match preflight::tmux_version_string() {
        Ok(v) => v,
        Err(TmuxError::NotInstalled) => return Err(preflight::not_installed_message()),
        Err(e) => return Err(format!("cannot run tmux: {e}")),
    };

    match preflight::parse_version(&raw) {
        Ok(v) => {
            let tier = preflight::tier_for(&v);
            if tier == Tier::Unsupported {
                return Err(format!(
                    "multi-run needs tmux 2.9 or newer, found {}.\n{}",
                    v.raw,
                    preflight::install_command()
                ));
            }
            if tier == Tier::Degraded {
                eprintln!(
                    "multi-run: tmux {} is supported but predates 3.3; finished panes show an \
                     extra banner line.",
                    v.raw
                );
            }
            Ok(tier)
        }
        // An unrecognised version shape must not brick the tool: a future
        // tmux is far more likely to be newer than older.
        Err(_) => {
            eprintln!("multi-run: could not parse tmux version {raw:?}, assuming a recent tmux.");
            Ok(Tier::Full)
        }
    }
}

/// `--attach <id>`. `all` is rejected: it only makes sense for `--kill`.
fn parse_attach_target(s: &str) -> Result<Target> {
    match s.parse::<Target>() {
        Ok(Target::All) => {
            anyhow::bail!("--attach needs a group or job id; `all` is only valid for --kill")
        }
        Ok(t) => Ok(t),
        Err(e) => anyhow::bail!("invalid --attach id {s:?}: {e}"),
    }
}

/// Current unix time in seconds.
fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// One tracked group reconstructed from tmux, since tmux's `@mr_*` options
/// are the only persistence we have (DESIGN.md section 9).
struct DiscoveredGroup {
    id: GroupId,
    label: Option<String>,
    cwd: PathBuf,
    created_at: u64,
    jobs: Vec<Job>,
    next_index: u16,
}

/// Read every multi-run group off the server, newest job order preserved.
fn discover(tmux: &Tmux) -> Result<Vec<DiscoveredGroup>, TmuxError> {
    let meta = tmux.poll_metadata()?;
    let panes: HashMap<&str, &tmux::parse::PaneRecord> = meta
        .panes
        .iter()
        .map(|p| (p.session_name.as_str(), p))
        .collect();

    let mut groups: Vec<DiscoveredGroup> = Vec::new();
    let mut sessions = meta.sessions;
    sessions.sort_by(|a, b| a.group.cmp(&b.group).then(a.index.cmp(&b.index)));

    for rec in &sessions {
        let gid = GroupId::new(rec.group.clone());
        let id = JobId {
            group: gid.clone(),
            index: rec.index,
        };
        let mut job = Job::new(id, rec.cmd.clone(), PathBuf::from(&rec.cwd), rec.started);

        // Fold the pane facts in immediately so a discovered job renders with
        // its real state instead of flashing "starting".
        if let Some(p) = panes.get(rec.session_name.as_str()) {
            let facts = pane_facts(p, rec.attached);
            model::advance(&mut job, model::PollObservation::Present(facts), now_secs());
        }

        match groups.iter_mut().find(|g| g.id == gid) {
            Some(g) => {
                g.next_index = g.next_index.max(rec.index + 1);
                g.jobs.push(job);
            }
            None => groups.push(DiscoveredGroup {
                id: gid,
                label: rec.label.clone(),
                cwd: PathBuf::from(&rec.cwd),
                created_at: rec.started,
                jobs: vec![job],
                next_index: rec.index + 1,
            }),
        }
    }
    Ok(groups)
}

/// Convert a polled pane record into the model's fact struct.
fn pane_facts(p: &tmux::parse::PaneRecord, attached: bool) -> PaneFacts {
    PaneFacts {
        dead: p.dead,
        dead_status: p.dead_status,
        dead_signal: p.dead_signal.clone(),
        dead_time: p.dead_time,
        pane_pid: p.pane_pid,
        width: p.width,
        height: p.height,
        history_size: p.history_size,
        cursor_y: p.cursor_y,
        session_attached: attached,
    }
}

/// `--list`: print every active group and exit. An absent server is the
/// empty case, not an error (DESIGN.md section 2).
fn cmd_list(tmux: &Tmux) -> Result<i32> {
    let groups = match discover(tmux) {
        Ok(g) => g,
        Err(TmuxError::NoServer { .. }) => Vec::new(),
        Err(e) => return Err(e.into()),
    };

    if groups.is_empty() {
        println!("No active multi-run sessions.");
        return Ok(0);
    }

    println!(
        "{:<20} {:<5} {:<26} {:<6} STATUS",
        "GROUP", "JOBS", "DIR", "AGE"
    );
    let now = now_secs();
    let mut total = 0usize;
    for g in &groups {
        total += g.jobs.len();
        // `GroupId`/`JobId` write through `write_str`, which ignores format
        // width, so render them to a String before padding the column.
        // The label, when set, is what the user actually recognises the group
        // by, so it leads the id rather than hiding at the end of the line.
        let id_cell = match &g.label {
            Some(l) if !l.is_empty() => format!("{} ({l})", g.id),
            _ => g.id.to_string(),
        };
        println!(
            "{:<20} {:<5} {:<26} {:<6} {}",
            id_cell,
            g.jobs.len(),
            tilde(&g.cwd),
            age(now.saturating_sub(g.created_at)),
            summarize(&g.jobs)
        );
        for job in &g.jobs {
            println!(
                "  {:<7} {:<10} {}",
                job.id.to_string(),
                state_label(&job.state),
                job.command
            );
        }
    }
    println!(
        "\n{} group{}, {} session{}.",
        groups.len(),
        plural(groups.len()),
        total,
        plural(total)
    );
    println!("Reattach:");
    for g in &groups {
        println!("  multi-run -a {}", g.id);
    }
    Ok(0)
}

/// `--kill <id>`: kill a group, one job, or everything, then exit.
fn cmd_kill(tmux: &Tmux, spec: &str) -> Result<i32> {
    let target: Target = spec
        .parse()
        .map_err(|e| anyhow::anyhow!("invalid --kill id {spec:?}: {e}"))?;

    if target == Target::All {
        match tmux.kill_server() {
            Ok(()) | Err(TmuxError::NoServer { .. }) => {
                println!("Killed all multi-run sessions.");
                return Ok(0);
            }
            Err(e) => return Err(e.into()),
        }
    }

    let groups = match discover(tmux) {
        Ok(g) => g,
        Err(TmuxError::NoServer { .. }) => Vec::new(),
        Err(e) => return Err(e.into()),
    };

    let victims: Vec<&Job> = match &target {
        Target::Group(gid) => groups
            .iter()
            .filter(|g| &g.id == gid)
            .flat_map(|g| g.jobs.iter())
            .collect(),
        Target::Job(jid) => groups
            .iter()
            .flat_map(|g| g.jobs.iter())
            .filter(|j| &j.id == jid)
            .collect(),
        Target::All => unreachable!("handled above"),
    };

    if victims.is_empty() {
        eprintln!("No such multi-run session: {spec}");
        return Ok(1);
    }

    let mut killed = 0;
    for job in &victims {
        match tmux.kill_session(&job.tmux_name) {
            Ok(()) => killed += 1,
            Err(e) => eprintln!("multi-run: could not kill {}: {e}", job.id),
        }
    }
    println!("Killed {killed} session{}.", plural(killed));
    Ok(0)
}

/// `"s"` when `n != 1`, for plain-English counts.
fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}

/// Abbreviate `$HOME` to `~` for display.
fn tilde(p: &std::path::Path) -> String {
    let s = p.display().to_string();
    match std::env::var("HOME") {
        Ok(home) if !home.is_empty() && s.starts_with(&home) => s.replacen(&home, "~", 1),
        _ => s,
    }
}

/// Compact age string: `45s`, `12m`, `2h`, `3d`.
fn age(secs: u64) -> String {
    match secs {
        s if s < 60 => format!("{s}s"),
        s if s < 3600 => format!("{}m", s / 60),
        s if s < 86400 => format!("{}h", s / 3600),
        s => format!("{}d", s / 86400),
    }
}

/// Short per-job status for `--list`.
fn state_label(state: &JobState) -> String {
    match state {
        JobState::Starting => "starting".into(),
        JobState::Running => "running".into(),
        JobState::Finished { outcome, .. } => match outcome {
            model::Outcome::Exit(c) => format!("exit {c}"),
            model::Outcome::Signal(s) => format!("sig {s}"),
            model::Outcome::Unknown => "finished".into(),
        },
        JobState::Vanished => "gone".into(),
        JobState::Dismissed => "dismissed".into(),
    }
}

/// Group-level summary line, e.g. `2 running, 1 finished`.
fn summarize(jobs: &[Job]) -> String {
    let running = jobs
        .iter()
        .filter(|j| matches!(j.state, JobState::Starting | JobState::Running))
        .count();
    let signalled = jobs
        .iter()
        .filter(|j| {
            matches!(
                j.state,
                JobState::Finished {
                    outcome: model::Outcome::Signal(_),
                    ..
                }
            )
        })
        .count();
    let finished = jobs
        .iter()
        .filter(|j| matches!(j.state, JobState::Finished { .. }))
        .count()
        - signalled;

    let mut parts = Vec::new();
    if running > 0 {
        parts.push(format!("{running} running"));
    }
    if finished > 0 {
        parts.push(format!("{finished} finished"));
    }
    if signalled > 0 {
        parts.push(format!("{signalled} signalled"));
    }
    if parts.is_empty() {
        "idle".into()
    } else {
        parts.join(", ")
    }
}

/// Guard that restores the terminal on drop, so every exit path (including
/// `?` and panics) leaves the user's terminal usable.
struct TerminalGuard;

impl TerminalGuard {
    /// Enter raw mode, the alternate screen and mouse reporting, and install
    /// a panic hook that undoes all three before the default hook prints.
    fn enter() -> Result<Self> {
        let default_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            TerminalGuard::restore();
            default_hook(info);
        }));

        enable_raw_mode()?;
        enter_screen(&mut io::stdout())?;
        Ok(TerminalGuard)
    }

    /// Best-effort restore. Deliberately ignores errors: it runs on panic and
    /// on drop, where there is nothing useful to do with a failure.
    fn restore() {
        let _ = disable_raw_mode();
        leave_screen(&mut io::stdout());
    }
}

/// The private-mode setup, as bytes on `w`. Every mode turned on here must be
/// turned off by `leave_screen`, which `screen_setup_is_fully_undone` checks:
/// a mode left on outlives the process. Mouse reporting is the one that bites,
/// because the terminal then answers a scroll with `\e[<65;…M` printed into
/// whatever shell prompt the user landed back on.
fn enter_screen(w: &mut impl io::Write) -> io::Result<()> {
    execute!(w, EnterAlternateScreen, EnableMouseCapture)
}

/// Undo `enter_screen`, innermost mode first. Errors are dropped: this runs
/// on the panic path too, where reporting one is not useful.
fn leave_screen(w: &mut impl io::Write) {
    let _ = execute!(w, DisableMouseCapture, LeaveAlternateScreen);
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        TerminalGuard::restore();
    }
}

/// Launch the TUI: build or adopt a group, spawn the input and poller
/// threads, then run the event loop.
fn tui(
    tmux: Tmux,
    tier: Tier,
    commands: Vec<String>,
    attach_target: Option<Target>,
    view: Option<ViewMode>,
    label: Option<String>,
) -> Result<i32> {
    let cwd = std::env::current_dir().context("cannot read the current directory")?;

    // Adopt an existing group when asked, otherwise start a fresh one.
    let (group, mut pending_launch) = match &attach_target {
        Some(target) => {
            let groups = discover(&tmux)?;
            let wanted = match target {
                Target::Group(g) => g.clone(),
                Target::Job(j) => j.group.clone(),
                Target::All => unreachable!("rejected in parse_attach_target"),
            };
            let found = groups
                .into_iter()
                .find(|g| g.id == wanted)
                .with_context(|| format!("no such multi-run group: {wanted}"))?;
            let g = Group {
                id: found.id,
                label: found.label,
                cwd: found.cwd,
                jobs: found.jobs,
                next_index: found.next_index,
            };
            (g, Vec::new())
        }
        None => {
            let id = tmux.new_group_id()?;
            let g = Group {
                id,
                label,
                cwd: cwd.clone(),
                jobs: Vec::new(),
                next_index: 1,
            };
            (g, commands)
        }
    };

    let group_id = group.id.clone();
    let mut app = App::new(group, view.unwrap_or(ViewMode::Grid), tier);

    // `--attach g1.2` means "open the group and jump straight into job 2",
    // which is what `--help` promises. Focus it now and attach on the first
    // loop iteration, once the terminal exists.
    let mut attach_on_start = None;
    if let Some(Target::Job(jid)) = &attach_target {
        match app.group.jobs.iter().position(|j| &j.id == jid) {
            Some(idx) => {
                app.focus.job = idx;
                attach_on_start = Some(app.group.jobs[idx].tmux_name.clone());
            }
            None => anyhow::bail!("no such job in group {group_id}: {jid}"),
        }
    }

    // Launched bare with nothing to run: open the compose screen so the user
    // has somewhere to start (DESIGN.md section 2).
    if pending_launch.is_empty() && app.group.jobs.is_empty() {
        app.modal = Modal::Compose {
            input: String::new(),
            queued: Vec::new(),
            launch_on_submit: true,
        };
    }

    let (to_main_tx, to_main_rx) = mpsc::channel::<ToMain>();
    let (to_poller_tx, to_poller_rx) = mpsc::channel::<ToPoller>();

    let input_stop = Arc::new(AtomicBool::new(false));
    let input_handle = spawn_input(to_main_tx.clone(), Arc::clone(&input_stop));

    // The poller owns its `Tmux`; the attach round trip needs one too. `Tmux`
    // is just socket and config-path state, so a second handle is free.
    let attach_tmux = Tmux::new(tmux.socket().to_string(), tmux.conf_path().to_path_buf());
    let poller_handle = {
        let tx = to_main_tx.clone();
        std::thread::spawn(move || poll::run(tmux, to_poller_rx, tx))
    };

    let guard = TerminalGuard::enter()?;
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
    terminal.clear()?;

    let size = terminal.size()?;
    app.on_resize(body_rect(size.width, size.height));

    // Launch the initial commands now that geometry is known, so panes are
    // created at roughly the right size instead of 80x24.
    if !pending_launch.is_empty() {
        let cmds = std::mem::take(&mut pending_launch);
        launch(&mut app, &to_poller_tx, &cmds, &cwd);
    }

    let mut ctx = LoopCtx {
        rx: &to_main_rx,
        to_main: &to_main_tx,
        to_poller: &to_poller_tx,
        tmux: &attach_tmux,
        input_stop,
        input_handle: Some(input_handle),
    };
    let result = attach_on_start
        .map(|session| do_attach(&mut terminal, &mut app, &mut ctx, &session))
        .unwrap_or(Ok(()))
        .and_then(|()| event_loop(&mut terminal, &mut app, &mut ctx));

    let _ = to_poller_tx.send(ToPoller::Shutdown);
    ctx.input_stop.store(true, Ordering::Relaxed);
    drop(guard);
    let _ = poller_handle.join();
    if let Some(handle) = ctx.input_handle.take() {
        let _ = handle.join();
    }

    result?;

    // Sessions outlive us by default (research item 7), so tell the user how
    // to get back to them.
    if !app.group.jobs.is_empty() {
        let n = app.group.jobs.len();
        println!("multi-run: {n} session{} still running.", plural(n));
        println!("Reattach with: multi-run -a {group_id}");
    }
    Ok(0)
}

/// The body area, i.e. the frame minus the one-row status bar.
fn body_rect(width: u16, height: u16) -> Rect {
    Rect {
        x: 0,
        y: 0,
        width,
        height: height.saturating_sub(1),
    }
}

/// Spawn the input thread. It polls with a timeout so it can notice the stop
/// flag and exit within 100ms of being asked (gotcha F, DESIGN.md section 6).
fn spawn_input(tx: Sender<ToMain>, stop: Arc<AtomicBool>) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        while !stop.load(Ordering::Relaxed) {
            match event::poll(Duration::from_millis(100)) {
                Ok(true) => match event::read() {
                    Ok(Event::Key(k)) => {
                        if tx.send(ToMain::Key(k)).is_err() {
                            return;
                        }
                    }
                    Ok(Event::Resize(w, h)) => {
                        if tx.send(ToMain::Resize(w, h)).is_err() {
                            return;
                        }
                    }
                    Ok(Event::Mouse(m)) => {
                        if tx.send(ToMain::Mouse(m)).is_err() {
                            return;
                        }
                    }
                    Ok(_) => {}
                    Err(_) => return,
                },
                Ok(false) => {}
                Err(_) => return,
            }
        }
    })
}

/// Create jobs for `commands` and ask the poller to start their sessions.
fn launch(app: &mut App, to_poller: &Sender<ToPoller>, commands: &[String], cwd: &std::path::Path) {
    // Size new panes to the tile they will occupy once they exist, so their
    // first output already wraps correctly.
    let projected = app.group.jobs.len() + commands.len();
    let geo = ui::layout::compute(projected, app.body, app.view, app.page);
    let (w, h) = geo
        .tiles
        .first()
        .map(|r| {
            (
                r.width.saturating_sub(2).max(1),
                r.height.saturating_sub(2).max(1),
            )
        })
        .unwrap_or((80, 24));

    for cmd in commands {
        let index = app.group.next_job_index();
        let id = JobId {
            group: app.group.id.clone(),
            index,
        };
        let started = now_secs();
        let spec = NewSessionSpec {
            session_name: id.tmux_name(),
            cwd: cwd.to_path_buf(),
            width: w,
            height: h,
            command: cmd.clone(),
            group: app.group.id.clone(),
            index,
            started,
            // `@mr_label` is per session, but the label is a property of the
            // group, so every session in the group carries the same one. That
            // is what lets `--list` show it after a restart, since tmux's own
            // options are the only state we keep.
            label: app.group.label.clone(),
        };
        app.group
            .jobs
            .push(Job::new(id, cmd.clone(), cwd.to_path_buf(), started));
        let _ = to_poller.send(ToPoller::OneShot(TmuxOp::Create(Box::new(spec))));
    }
    app.recompute();
    app.redraw = true;
}

/// Everything the event loop needs beyond `App` itself.
struct LoopCtx<'a> {
    rx: &'a Receiver<ToMain>,
    to_main: &'a Sender<ToMain>,
    to_poller: &'a Sender<ToPoller>,
    /// A second handle to the same socket, for the attach round trip. `Tmux`
    /// holds no OS resources, so this is just argv construction state.
    tmux: &'a Tmux,
    /// Stop flag for the live input thread, replaced on each attach.
    input_stop: Arc<AtomicBool>,
    input_handle: Option<std::thread::JoinHandle<()>>,
}

/// The main event loop: drain messages, apply them, redraw when dirty.
fn event_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
    ctx: &mut LoopCtx<'_>,
) -> Result<()> {
    let cwd = app.cwd();
    let mut last_specs: Vec<TileSpec> = Vec::new();

    loop {
        match ctx.rx.recv_timeout(TICK) {
            Ok(msg) => {
                if let Some(session) = handle_message(app, msg, ctx.to_poller, &cwd)? {
                    do_attach(terminal, app, ctx, &session)?;
                    last_specs.clear();
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }

        if app.should_quit {
            break;
        }

        let now = Instant::now();
        app.expire_toast(now);

        // Debounced resize: tell tmux only about tiles whose size actually
        // changed, because every resize-window sets window-size to manual
        // (gotcha E) and needless churn has to be undone before an attach.
        if let Some(at) = app.pending_resize
            && now.duration_since(at) >= app::RESIZE_DEBOUNCE
        {
            app.pending_resize = None;
            let batch = resize_batch(app);
            if !batch.is_empty() {
                let _ = ctx.to_poller.send(ToPoller::ResizeBatch(batch));
            }
        }

        let specs = tile_specs(app);
        if specs != last_specs {
            last_specs.clone_from(&specs);
            let _ = ctx.to_poller.send(ToPoller::SetVisible(specs));
        }

        if app.redraw {
            app.redraw = false;
            terminal.draw(|frame| {
                let state = ui::DrawState {
                    group: &app.group,
                    view: app.view,
                    focus: app.focus,
                    page: app.page,
                    overlay: app.overlay(),
                };
                ui::draw(frame, &state);
            })?;
        }
    }
    Ok(())
}

/// Hand the terminal to tmux, then restore the TUI around it. Steps 1 to 10
/// live in `attach.rs`; steps 11 to 13 (resize sweep, input respawn, resume)
/// need `App` and the `Terminal`, so they are here (DESIGN.md section 8).
fn do_attach(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
    ctx: &mut LoopCtx<'_>,
    session: &str,
) -> Result<()> {
    let sessions: Vec<String> = app.group.jobs.iter().map(|j| j.tmux_name.clone()).collect();
    let outcome = attach::round_trip(attach::Handoff {
        tmux: ctx.tmux,
        session,
        sessions: &sessions,
        to_poller: ctx.to_poller,
        from_threads: ctx.rx,
        input_stop: &ctx.input_stop,
    });

    // The old input thread was told to stop before the handoff; reap it
    // before starting its replacement so only one reader ever owns stdin.
    if let Some(handle) = ctx.input_handle.take() {
        let _ = handle.join();
    }

    if outcome == attach::Outcome::Vanished {
        if let Some(job) = app.group.jobs.iter_mut().find(|j| j.tmux_name == session) {
            job.state = JobState::Vanished;
        }
        app.push_error(format!("session {session} is gone"));
    } else if let attach::Outcome::Failed(msg) = &outcome {
        app.push_error(msg.clone());
    }

    // Step 10: ratatui diffs against its previous buffer, which is garbage
    // after another program owned the screen.
    terminal.clear()?;

    // Step 11: a detached session keeps whatever size it last had, and we may
    // have been resized while suspended, so sweep every tile rather than only
    // the one that was attached.
    let size = terminal.size()?;
    app.on_resize(body_rect(size.width, size.height));
    for job in &mut app.group.jobs {
        job.tile_size = (0, 0);
    }
    let batch = resize_batch(app);
    if !batch.is_empty() {
        let _ = ctx.to_poller.send(ToPoller::ResizeBatch(batch));
    }

    // Step 12: fresh flag, because the old one is latched true forever.
    let stop = Arc::new(AtomicBool::new(false));
    ctx.input_stop = Arc::clone(&stop);
    ctx.input_handle = Some(spawn_input(ctx.to_main.clone(), stop));

    // Step 13.
    let _ = ctx.to_poller.send(ToPoller::Resume);
    app.redraw = true;
    Ok(())
}

/// Apply one inbound message to `App`, running any effect it produces.
/// Returns a session name when the user asked to attach to it.
fn handle_message(
    app: &mut App,
    msg: ToMain,
    to_poller: &Sender<ToPoller>,
    cwd: &std::path::Path,
) -> Result<Option<String>> {
    match msg {
        ToMain::Key(key) => {
            if let Some(action) = keymap::map(app.view, &app.modal_state(), key)
                && let Some(effect) = app.apply(action)
            {
                return run_effect(app, effect, to_poller, cwd);
            }
        }
        ToMain::Resize(w, h) => app.on_resize(body_rect(w, h)),
        ToMain::Mouse(m) => app.on_mouse(m),
        ToMain::Meta { sessions, panes } => apply_meta(app, sessions, panes),
        ToMain::Content { updates } => apply_content(app, updates),
        ToMain::TmuxError(e) => app.push_error(e),
        ToMain::PollerPaused => {}
    }
    Ok(None)
}

/// Fold one metadata poll into the model, advancing every job's state.
fn apply_meta(
    app: &mut App,
    sessions: Vec<tmux::parse::SessionRecord>,
    panes: Vec<tmux::parse::PaneRecord>,
) {
    let by_name: HashMap<&str, &tmux::parse::PaneRecord> =
        panes.iter().map(|p| (p.session_name.as_str(), p)).collect();
    let attached: HashMap<&str, bool> = sessions
        .iter()
        .map(|s| (s.session_name.as_str(), s.attached))
        .collect();
    let now = now_secs();

    for job in &mut app.group.jobs {
        let observation = match by_name.get(job.tmux_name.as_str()) {
            Some(p) => {
                let is_attached = attached
                    .get(job.tmux_name.as_str())
                    .copied()
                    .unwrap_or(false);
                model::PollObservation::Present(pane_facts(p, is_attached))
            }
            None => model::PollObservation::Absent,
        };
        model::advance(job, observation, now);
    }
    app.redraw = true;
}

/// Decode captured ANSI into tile content (DESIGN.md section 6).
fn apply_content(app: &mut App, updates: Vec<(String, Vec<u8>)>) {
    use ansi_to_tui::IntoText;

    for (name, raw) in updates {
        let Some(job) = app.group.jobs.iter_mut().find(|j| j.tmux_name == name) else {
            continue;
        };

        // Prepend a reset so decoding always starts from a known SGR state
        // rather than inheriting whatever the previous capture left set.
        let mut bytes = Vec::with_capacity(raw.len() + 4);
        bytes.extend_from_slice(b"\x1b[0m");
        bytes.extend_from_slice(&raw);

        match bytes.into_text() {
            Ok(text) => {
                let lines: Vec<_> = text.lines;
                if lines != job.content {
                    job.content = lines;
                    job.content_rev = job.content_rev.wrapping_add(1);
                }
                job.last_capture = Some(Instant::now());
            }
            // A malformed escape must never blank a tile: keep the previous
            // content and try again on the next poll.
            Err(e) => app.push_error(format!("decode {name}: {e}")),
        }
    }
    app.redraw = true;
}

/// Perform a side effect produced by an action. Returns a session name when
/// the effect is an attach, which only the event loop can carry out.
fn run_effect(
    app: &mut App,
    effect: Effect,
    to_poller: &Sender<ToPoller>,
    cwd: &std::path::Path,
) -> Result<Option<String>> {
    match effect {
        Effect::Launch(cmds) => launch(app, to_poller, &cmds, cwd),
        Effect::Rerun(cmd, job_cwd) => launch(app, to_poller, &[cmd], &job_cwd),
        Effect::Kill(name) => {
            let _ = to_poller.send(ToPoller::OneShot(TmuxOp::KillSession(name)));
        }
        Effect::KillMany(names) => {
            for name in names {
                let _ = to_poller.send(ToPoller::OneShot(TmuxOp::KillSession(name)));
            }
        }
        Effect::Stop(name) => {
            let _ = to_poller.send(ToPoller::OneShot(TmuxOp::SendInterrupt(name)));
        }
        Effect::Signal(name, signal) => {
            app.push_error(format!("sent SIG{signal} to {name}"));
            let _ = to_poller.send(ToPoller::OneShot(TmuxOp::Signal {
                session: name,
                signal,
            }));
        }
        Effect::SendEof(name) => {
            let _ = to_poller.send(ToPoller::OneShot(TmuxOp::SendEof(name)));
        }
        // The terminal handoff needs the `Terminal` and the input thread, so
        // it is performed by the event loop rather than here.
        Effect::Attach(name) => return Ok(Some(name)),
        Effect::QuitKeep => {}
        Effect::QuitKill => {
            for job in &app.group.jobs {
                let _ = to_poller.send(ToPoller::OneShot(TmuxOp::KillSession(
                    job.tmux_name.clone(),
                )));
            }
            app.group.jobs.clear();
        }
    }
    Ok(None)
}

/// Which tiles are visible, and at what capture mode, for the poller.
fn tile_specs(app: &App) -> Vec<TileSpec> {
    let start = ui::layout::page_start(app.page, app.geometry.per_page);
    let end = start + app.geometry.per_page.max(1);

    app.group
        .jobs
        .iter()
        .enumerate()
        .map(|(i, job)| {
            let visible = match app.view {
                ViewMode::Grid => i >= start && i < end,
                ViewMode::ListDetail => i == app.focus.job,
            };
            let kind = match &job.state {
                JobState::Vanished | JobState::Dismissed => TileKind::Vanished,
                JobState::Finished { .. } => TileKind::Finished { stale: 0 },
                _ => TileKind::Running,
            };
            let dead = matches!(job.state, JobState::Finished { .. });
            let mode = if job.scroll > 0 {
                // Scrolled back: ask for a window `scroll` lines above the
                // live bottom, still exactly one tile tall.
                CaptureMode::Scrollback {
                    offset: job.scroll,
                    height: job.tile_size.1.max(1),
                }
            } else if dead && app.tier == Tier::Degraded {
                // Below tmux 3.3 a dead pane's first line is pushed into
                // scrollback by the banner, so recover it (gotcha A).
                CaptureMode::DeadDegraded
            } else {
                CaptureMode::Live
            };
            TileSpec {
                session_name: job.tmux_name.clone(),
                visible,
                kind,
                mode,
            }
        })
        .collect()
}

/// Tiles whose desired inner size differs from what tmux was last told.
fn resize_batch(app: &mut App) -> Vec<(String, u16, u16)> {
    let geo = &app.geometry;
    let start = ui::layout::page_start(app.page, geo.per_page);
    let mut batch = Vec::new();

    for (i, rect) in geo.tiles.iter().enumerate() {
        let Some(job) = app.group.jobs.get_mut(start + i) else {
            continue;
        };
        let want = (
            rect.width.saturating_sub(2).max(1),
            rect.height.saturating_sub(2).max(1),
        );
        if job.tile_size != want {
            job.tile_size = want;
            batch.push((job.tmux_name.clone(), want.0, want.1));
        }
    }
    batch
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Collect the DEC private modes a byte stream turns on (`CSI ? Ps h`)
    /// or off (`CSI ? Ps l`). crossterm emits several per command and the
    /// exact set moves between versions, so the tests below compare the two
    /// sides rather than hard-coding sequences.
    fn private_modes(bytes: &[u8], final_byte: u8) -> Vec<String> {
        let text = String::from_utf8_lossy(bytes);
        let mut modes = Vec::new();
        for part in text.split("\u{1b}[?").skip(1) {
            if let Some(end) = part.find(final_byte as char) {
                let params = &part[..end];
                // Only a run of digits and `;` is a mode list; anything else
                // is an unrelated sequence that happens to follow.
                if !params.is_empty() && params.chars().all(|c| c.is_ascii_digit() || c == ';') {
                    modes.extend(params.split(';').map(str::to_string));
                }
            }
        }
        modes.sort();
        modes.dedup();
        modes
    }

    #[test]
    fn screen_setup_is_fully_undone() {
        // The bug this pins: mouse reporting stayed on after the process
        // exited, so the next scroll in the user's shell printed `\e[<65;…M`
        // instead of scrolling. Anything `enter_screen` switches on has to be
        // switched off again, not just the mode we happened to notice.
        let mut entered = Vec::new();
        enter_screen(&mut entered).unwrap();
        let mut left = Vec::new();
        leave_screen(&mut left);

        let on = private_modes(&entered, b'h');
        let off = private_modes(&left, b'l');
        assert!(!on.is_empty(), "expected some private modes to be set");
        for mode in &on {
            assert!(off.contains(mode), "mode {mode} is set but never reset");
        }
    }

    #[test]
    fn screen_setup_turns_mouse_reporting_on() {
        // The TUI's click-to-focus and wheel-scroll need it, and before this
        // it only ever came on as a side effect of returning from an attach.
        let mut entered = Vec::new();
        enter_screen(&mut entered).unwrap();
        // 1006 is SGR mouse mode, the one every current terminal uses.
        assert!(private_modes(&entered, b'h').contains(&"1006".to_string()));
    }

    #[test]
    fn attach_rejects_all() {
        assert!(
            parse_attach_target("all").is_err(),
            "`all` is only valid for --kill"
        );
    }

    #[test]
    fn attach_accepts_a_group_and_a_job() {
        assert!(matches!(parse_attach_target("g1"), Ok(Target::Group(_))));
        assert!(matches!(parse_attach_target("g1.2"), Ok(Target::Job(_))));
    }

    #[test]
    fn attach_rejects_nonsense() {
        assert!(parse_attach_target("not a real id").is_err());
    }

    #[test]
    fn age_is_compact_at_every_scale() {
        assert_eq!(age(45), "45s");
        assert_eq!(age(12 * 60), "12m");
        assert_eq!(age(2 * 3600), "2h");
        assert_eq!(age(3 * 86400), "3d");
    }

    #[test]
    fn body_reserves_exactly_one_row_for_the_status_bar() {
        assert_eq!(
            body_rect(80, 24),
            Rect {
                x: 0,
                y: 0,
                width: 80,
                height: 23
            }
        );
    }

    #[test]
    fn body_survives_a_zero_height_terminal() {
        assert_eq!(body_rect(80, 0).height, 0, "must not underflow");
    }

    #[test]
    fn state_labels_distinguish_exit_from_signal() {
        assert_eq!(state_label(&JobState::Running), "running");
        assert_eq!(
            state_label(&JobState::Finished {
                outcome: model::Outcome::Exit(2),
                at: None
            }),
            "exit 2"
        );
        assert_eq!(
            state_label(&JobState::Finished {
                outcome: model::Outcome::Signal("term".into()),
                at: None
            }),
            "sig term"
        );
    }

    #[test]
    fn summary_counts_each_category_separately() {
        let mk = |i: u16, state: JobState| {
            let id = JobId {
                group: GroupId::new("k3f9"),
                index: i,
            };
            let mut j = Job::new(id, "x".into(), PathBuf::from("/tmp"), 0);
            j.state = state;
            j
        };
        let jobs = vec![
            mk(1, JobState::Running),
            mk(
                2,
                JobState::Finished {
                    outcome: model::Outcome::Exit(0),
                    at: None,
                },
            ),
            mk(
                3,
                JobState::Finished {
                    outcome: model::Outcome::Signal("term".into()),
                    at: None,
                },
            ),
        ];
        assert_eq!(summarize(&jobs), "1 running, 1 finished, 1 signalled");
    }

    #[test]
    fn summary_of_nothing_is_idle() {
        assert_eq!(summarize(&[]), "idle");
    }
}
