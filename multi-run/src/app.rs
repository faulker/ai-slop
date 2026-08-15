//! Application state and the single place where it mutates. `keymap.rs`
//! produces `Action`s, this module applies exactly one per keystroke
//! (DESIGN.md section 4). No drawing, no argv construction, no tmux calls:
//! side effects leave here as `Effect` values for `main.rs` to execute.
//!
//! `Modal` is the source of truth for modal state. `ui::overlay::Overlay` and
//! `keymap::ModalState` are both derived from it each frame; neither is
//! authoritative.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;

use crate::keymap::{Action, ModalState};
use crate::model::{self, Group, Job, JobState};
use crate::preflight::Tier;
use crate::ui::layout::{self, Focus, Geometry, Pane, ViewMode};
use crate::ui::overlay::Overlay;

/// Error-log ring capacity (DESIGN.md section 4).
const MAX_ERRORS: usize = 200;

/// How long a toast stays up before it auto-clears.
const TOAST_TTL: Duration = Duration::from_secs(5);

/// Lines the mouse wheel moves per notch.
const MOUSE_SCROLL_LINES: u16 = 3;

/// Width of the list pane in list/detail view. Must match `ui::mod`'s
/// `LIST_WIDTH`, which owns the drawing; this copy exists so hit-testing does
/// not pull a rendering constant into state handling.
const LIST_PANE_WIDTH: u16 = 28;

/// Does `rect` contain the cell at `(col, row)`?
fn contains(rect: &Rect, col: u16, row: u16) -> bool {
    col >= rect.x && col < rect.x + rect.width && row >= rect.y && row < rect.y + rect.height
}

/// How long a stop signal gets to work before pressing `s` again escalates to
/// a harder one. Stops an impatient double-tap from jumping to SIGKILL.
const STOP_ESCALATION_GRACE: Duration = Duration::from_millis(750);

/// How long after the last resize event before we tell tmux about the new
/// tile sizes (DESIGN.md section 6; research item 4 suggests roughly 100ms).
pub const RESIZE_DEBOUNCE: Duration = Duration::from_millis(120);

/// Which overlay, if any, owns the keyboard. Richer than both
/// `keymap::ModalState` and `ui::overlay::Overlay`, which are projections of
/// this.
pub enum Modal {
    None,
    Help,
    /// Bare-launch compose screen, and the mid-run "add command" prompt.
    /// `launch_on_submit` distinguishes them.
    Compose {
        input: String,
        queued: Vec<String>,
        launch_on_submit: bool,
    },
    Confirm {
        prompt: String,
        action: Pending,
        /// 0 = yes, 1 = no. Arrow keys move this; Enter activates it.
        selected: usize,
    },
    /// Quit was requested while live jobs exist.
    /// `selected`: 0 = keep, 1 = kill. Arrow keys move this; Enter activates it.
    ExitPrompt {
        selected: usize,
    },
    Toast {
        text: String,
        at: Instant,
        is_error: bool,
    },
    ErrorLog {
        scroll: u16,
    },
}

/// What a `Modal::Confirm` will do if the user says yes. Deliberately not a
/// `keymap::Action`: the action that *opens* a confirmation is not the action
/// that runs on confirm, and reusing one enum for both would make
/// `KillFocused` reopen its own prompt forever.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pending {
    KillFocused,
    DismissAllFinished,
}

/// A side effect `main.rs` must perform. Returned from `apply` rather than
/// executed here, so `App` stays pure enough to unit test without tmux or a
/// terminal.
#[derive(Debug, PartialEq, Eq)]
pub enum Effect {
    /// Hand the terminal to `tmux attach-session` for this session.
    Attach(String),
    /// Stop level 1: send `C-c` to this session.
    Stop(String),
    /// Stop level 2 and 3: signal the pane's process directly.
    Signal(String, &'static str),
    SendEof(String),
    /// Kill the session outright, dropping its output.
    Kill(String),
    /// Kill several sessions, for "dismiss all finished".
    KillMany(Vec<String>),
    /// Launch these command strings as new jobs in the current group.
    Launch(Vec<String>),
    /// Re-run this job's command as a new job, in its own directory.
    Rerun(String, PathBuf),
    /// Quit, leaving every session running.
    QuitKeep,
    /// Quit after killing every session in the group.
    QuitKill,
}

/// Everything the TUI needs to render and decide what to poll.
pub struct App {
    pub group: Group,
    pub view: ViewMode,
    pub focus: Focus,
    pub modal: Modal,
    pub page: usize,
    pub geometry: Geometry,
    pub tier: Tier,
    /// Bounded ring of error strings, newest last.
    pub errors: Vec<String>,
    pub pending_resize: Option<Instant>,
    pub should_quit: bool,
    pub redraw: bool,
    /// Body area of the last frame, excluding the status bar. Geometry is
    /// recomputed against this whenever the job count or view changes.
    pub body: Rect,
    /// Precomputed `multi-run -a <group>` line for the exit prompt.
    /// Stored rather than formatted on demand because `Overlay` borrows it.
    pub reattach_hint: String,
}

impl App {
    /// Build an app around an existing (possibly empty) group.
    pub fn new(group: Group, view: ViewMode, tier: Tier) -> Self {
        let reattach_hint = format!("multi-run -a {}", group.id);
        App {
            group,
            view,
            focus: Focus {
                job: 0,
                pane: Pane::List,
            },
            modal: Modal::None,
            page: 0,
            geometry: Geometry {
                view,
                cols: 0,
                rows: 0,
                per_page: 0,
                pages: 0,
                tiles: Vec::new(),
            },
            tier,
            errors: Vec::new(),
            pending_resize: None,
            should_quit: false,
            redraw: true,
            body: Rect::default(),
            reattach_hint,
        }
    }

    /// Recompute geometry for the current job count, body area, view and
    /// page. Called after anything that can change tile layout.
    pub fn recompute(&mut self) {
        self.geometry = layout::compute(self.group.jobs.len(), self.body, self.view, self.page);
        // A page can disappear when jobs are dismissed; clamp rather than
        // rendering an empty page.
        if self.geometry.pages > 0 && self.page >= self.geometry.pages {
            self.page = self.geometry.pages - 1;
            self.geometry = layout::compute(self.group.jobs.len(), self.body, self.view, self.page);
        }
        let n = self.group.jobs.len();
        if self.focus.job >= n {
            self.focus.job = n.saturating_sub(1);
        }
    }

    /// Record the new terminal body area and start the resize debounce.
    pub fn on_resize(&mut self, body: Rect) {
        self.body = body;
        self.pending_resize = Some(Instant::now());
        self.recompute();
        self.redraw = true;
    }

    /// Click a tile to focus it, and scroll the wheel to move through its
    /// output. Any other mouse event is ignored: this is a keyboard-first
    /// tool, and swallowing drags or motion would fight terminal selection.
    pub fn on_mouse(&mut self, m: MouseEvent) {
        match m.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                if let Some(job) = self.job_at(m.column, m.row) {
                    self.focus.job = job;
                    if self.view == ViewMode::ListDetail {
                        self.focus.pane = Pane::List;
                    }
                    self.redraw = true;
                }
            }
            MouseEventKind::ScrollUp => {
                self.apply(Action::ScrollUp(MOUSE_SCROLL_LINES));
            }
            MouseEventKind::ScrollDown => {
                self.apply(Action::ScrollDown(MOUSE_SCROLL_LINES));
            }
            _ => {}
        }
    }

    /// Which job index, if any, is drawn at this cell. Grid view hit-tests
    /// the tile rects; list view maps a click in the left column to its row.
    fn job_at(&self, col: u16, row: u16) -> Option<usize> {
        match self.view {
            ViewMode::Grid => {
                let start = layout::page_start(self.page, self.geometry.per_page);
                let hit = self
                    .geometry
                    .tiles
                    .iter()
                    .position(|r| self.body.intersects(*r) && contains(r, col, row))?;
                let idx = start + hit;
                (idx < self.group.jobs.len()).then_some(idx)
            }
            ViewMode::ListDetail => {
                // The list pane is bordered, so its first row is one below the
                // body's top edge, and its rows start one column in.
                if col >= LIST_PANE_WIDTH || row <= self.body.y {
                    return None;
                }
                let idx = (row - self.body.y - 1) as usize;
                (idx < self.group.jobs.len()).then_some(idx)
            }
        }
    }

    /// Push an error onto the bounded ring and show it as a toast.
    pub fn push_error(&mut self, msg: impl Into<String>) {
        let msg = msg.into();
        if self.errors.len() >= MAX_ERRORS {
            self.errors.remove(0);
        }
        self.errors.push(msg.clone());
        // A modal the user opened deliberately outranks a transient toast.
        if matches!(self.modal, Modal::None | Modal::Toast { .. }) {
            self.modal = Modal::Toast {
                text: msg,
                at: Instant::now(),
                is_error: true,
            };
        }
        self.redraw = true;
    }

    /// Clear a toast once it has outlived `TOAST_TTL`. Returns whether
    /// anything changed.
    pub fn expire_toast(&mut self, now: Instant) -> bool {
        if let Modal::Toast { at, .. } = &self.modal
            && now.duration_since(*at) >= TOAST_TTL
        {
            self.modal = Modal::None;
            self.redraw = true;
            return true;
        }
        false
    }

    /// The modal state `keymap::map` needs to route the next key.
    pub fn modal_state(&self) -> ModalState {
        match &self.modal {
            Modal::None => ModalState::None,
            Modal::Help => ModalState::Help,
            Modal::Compose {
                input,
                launch_on_submit,
                ..
            } => ModalState::Compose {
                line_empty: input.is_empty(),
                launch_on_submit: *launch_on_submit,
            },
            Modal::Confirm { .. } => ModalState::Confirm,
            Modal::ExitPrompt { .. } => ModalState::ExitPrompt,
            Modal::Toast { .. } => ModalState::Toast,
            Modal::ErrorLog { .. } => ModalState::ErrorLog,
        }
    }

    /// The overlay to draw this frame, derived from `Modal`.
    pub fn overlay(&self) -> Option<Overlay<'_>> {
        match &self.modal {
            Modal::None => None,
            Modal::Help => Some(Overlay::Help),
            Modal::Compose {
                input,
                queued,
                launch_on_submit,
            } => Some(Overlay::Compose {
                input,
                queued,
                launch_on_submit: *launch_on_submit,
            }),
            Modal::Confirm {
                prompt,
                selected,
                ..
            } => Some(Overlay::Confirm {
                prompt,
                selected: *selected,
            }),
            Modal::ExitPrompt { selected } => Some(Overlay::ExitPrompt {
                running: self.count_running(),
                finished: self.count_finished(),
                reattach_hint: &self.reattach_hint,
                selected: *selected,
            }),
            Modal::Toast { text, is_error, .. } => Some(Overlay::Toast {
                text,
                is_error: *is_error,
            }),
            Modal::ErrorLog { scroll } => Some(Overlay::ErrorLog {
                entries: &self.errors,
                scroll: *scroll,
            }),
        }
    }

    /// Jobs still running or still starting up.
    pub fn count_running(&self) -> usize {
        self.group
            .jobs
            .iter()
            .filter(|j| matches!(j.state, JobState::Starting | JobState::Running))
            .count()
    }

    /// Jobs that have exited but whose output is still on screen.
    pub fn count_finished(&self) -> usize {
        self.group
            .jobs
            .iter()
            .filter(|j| matches!(j.state, JobState::Finished { .. }))
            .count()
    }

    /// The focused job, if the group is non-empty.
    pub fn focused(&self) -> Option<&Job> {
        self.group.jobs.get(self.focus.job)
    }

    /// Apply one action, returning any side effect for `main.rs` to run.
    /// This is the only function that mutates `App` in response to input.
    pub fn apply(&mut self, action: Action) -> Option<Effect> {
        self.redraw = true;
        let n = self.group.jobs.len();

        match action {
            Action::FocusNext => {
                let (job, page) = layout::focus_next(self.focus.job, self.geometry.per_page, n);
                self.focus.job = job;
                self.set_page(page);
            }
            Action::FocusPrev => {
                let (job, page) = layout::focus_prev(self.focus.job, self.geometry.per_page, n);
                self.focus.job = job;
                self.set_page(page);
            }
            Action::FocusLeft => {
                // In list view, left moves keyboard focus to the list pane
                // rather than between tiles; there is only one tile.
                if self.view == ViewMode::ListDetail {
                    self.focus.pane = Pane::List;
                } else {
                    self.focus.job = layout::focus_left(self.focus.job, self.page, &self.geometry);
                }
            }
            Action::FocusRight => {
                if self.view == ViewMode::ListDetail {
                    self.focus.pane = Pane::Detail;
                } else {
                    self.focus.job = layout::focus_right(self.focus.job, self.page, &self.geometry);
                }
            }
            Action::FocusFirst => {
                let (job, page) = layout::focus_first();
                self.focus.job = job;
                self.set_page(page);
            }
            Action::FocusLast => {
                // `G` means "go to the end of the thing you are looking at".
                // With the detail pane focused and scrolled back, that is the
                // live bottom of the output, not the last job (DESIGN.md
                // section 7). `keymap` cannot make this call because it does
                // not know which pane has focus.
                if self.view == ViewMode::ListDetail
                    && self.focus.pane == Pane::Detail
                    && self.focused().is_some_and(|j| j.scroll > 0)
                {
                    return self.apply(Action::ScrollBottom);
                }
                let (job, page) = layout::focus_last(n, self.geometry.per_page);
                self.focus.job = job;
                self.set_page(page);
            }
            Action::FocusIndex(idx) => {
                let (job, page) = layout::focus_index(idx, n, self.geometry.per_page);
                self.focus.job = job;
                self.set_page(page);
            }
            Action::NextPage => {
                let page = layout::next_page(self.page, self.geometry.pages);
                self.set_page(page);
                self.focus_into_page();
            }
            Action::PrevPage => {
                let page = layout::prev_page(self.page, self.geometry.pages);
                self.set_page(page);
                self.focus_into_page();
            }
            Action::ToggleView => {
                self.view = match self.view {
                    ViewMode::Grid => ViewMode::ListDetail,
                    ViewMode::ListDetail => ViewMode::Grid,
                };
                self.recompute();
            }
            Action::SetView(v) => {
                self.view = v;
                self.recompute();
            }

            Action::Attach => {
                return self.focused().map(|j| Effect::Attach(j.tmux_name.clone()));
            }
            Action::Stop => {
                return self.escalate_stop();
            }
            Action::SendEof => {
                return self.focused().map(|j| Effect::SendEof(j.tmux_name.clone()));
            }
            Action::Rerun => {
                // Re-run in the job's own directory, which is not necessarily
                // the group's: a job adopted from a previous session carries
                // the cwd it was originally started in.
                return self
                    .focused()
                    .map(|j| Effect::Rerun(j.command.clone(), j.cwd.clone()));
            }

            Action::Dismiss => {
                // Dismissing is only legal once a job has finished; the
                // output stays until the user says otherwise (PLAN.md).
                if let Some(job) = self.group.jobs.get_mut(self.focus.job)
                    && model::dismiss(job)
                {
                    let name = job.tmux_name.clone();
                    self.group.remove_dismissed();
                    self.recompute();
                    return Some(Effect::Kill(name));
                }
            }
            Action::DismissAllFinished => {
                // Dismissing throws output away, so confirm rather than
                // clearing the whole screen on one keystroke.
                let n = self.count_dismissable();
                if n > 0 {
                    let prompt = format!(
                        "Dismiss {n} finished job{}? Their output is lost.",
                        if n == 1 { "" } else { "s" }
                    );
                    self.modal = Modal::Confirm {
                        prompt,
                        action: Pending::DismissAllFinished,
                        selected: 0,
                    };
                }
            }
            Action::KillFocused => {
                if let Some(job) = self.focused() {
                    let prompt = format!("Kill {} ({})? Output is lost.", job.id, job.command);
                    self.modal = Modal::Confirm {
                        prompt,
                        action: Pending::KillFocused,
                        selected: 0,
                    };
                }
            }

            Action::ScrollUp(lines) => {
                if let Some(job) = self.group.jobs.get_mut(self.focus.job) {
                    job.scroll = job
                        .max_scroll()
                        .min(job.scroll.saturating_add(lines as u32));
                }
            }
            Action::ScrollDown(lines) => {
                if let Some(job) = self.group.jobs.get_mut(self.focus.job) {
                    job.scroll = job.scroll.saturating_sub(lines as u32);
                }
            }
            Action::ScrollBottom => {
                if let Some(job) = self.group.jobs.get_mut(self.focus.job) {
                    job.scroll = 0;
                }
            }

            Action::OpenCompose => {
                self.modal = Modal::Compose {
                    input: String::new(),
                    queued: Vec::new(),
                    launch_on_submit: false,
                };
            }
            Action::OpenHelp => {
                self.modal = match self.modal {
                    Modal::Help => Modal::None,
                    _ => Modal::Help,
                };
            }
            Action::OpenErrorLog => {
                self.modal = match self.modal {
                    Modal::ErrorLog { .. } => Modal::None,
                    _ => Modal::ErrorLog { scroll: 0 },
                };
            }
            Action::CloseModal => {
                self.modal = Modal::None;
            }

            Action::ComposeChar(c) => {
                if let Modal::Compose { input, .. } = &mut self.modal {
                    input.push(c);
                }
            }
            Action::ComposeBackspace => {
                if let Modal::Compose { input, .. } = &mut self.modal {
                    input.pop();
                }
            }
            Action::ComposeClear => {
                if let Modal::Compose { input, .. } = &mut self.modal {
                    input.clear();
                }
            }
            Action::ComposeCommit => {
                if let Modal::Compose { input, queued, .. } = &mut self.modal
                    && !input.trim().is_empty()
                {
                    queued.push(std::mem::take(input));
                }
            }
            Action::ComposeSubmit => {
                if let Modal::Compose { input, queued, .. } = &mut self.modal {
                    let mut cmds = std::mem::take(queued);
                    if !input.trim().is_empty() {
                        cmds.push(std::mem::take(input));
                    }
                    self.modal = Modal::None;
                    if !cmds.is_empty() {
                        return Some(Effect::Launch(cmds));
                    }
                }
            }

            Action::ConfirmYes => {
                if let Modal::Confirm { action, .. } = &self.modal {
                    let pending = *action;
                    self.modal = Modal::None;
                    return self.run_pending(pending);
                }
            }
            Action::ConfirmNo => {
                self.modal = Modal::None;
            }

            Action::SelectPrev => match &mut self.modal {
                Modal::Confirm { selected, .. } | Modal::ExitPrompt { selected } => {
                    *selected = selected.saturating_sub(1);
                }
                _ => {}
            },
            Action::SelectNext => match &mut self.modal {
                Modal::Confirm { selected, .. } | Modal::ExitPrompt { selected } => {
                    *selected = (*selected + 1).min(1);
                }
                _ => {}
            },
            Action::SelectActivate => match &self.modal {
                Modal::Confirm {
                    selected, action, ..
                } => {
                    let pending = *action;
                    let yes = *selected == 0;
                    self.modal = Modal::None;
                    if yes {
                        return self.run_pending(pending);
                    }
                }
                Modal::ExitPrompt { selected } => {
                    let keep = *selected == 0;
                    self.should_quit = true;
                    return Some(if keep {
                        Effect::QuitKeep
                    } else {
                        Effect::QuitKill
                    });
                }
                _ => {}
            },

            Action::Quit => {
                // Live jobs mean the user has something to lose, so ask
                // rather than assuming (PLAN.md, DESIGN.md section 9).
                if self.count_running() > 0 || self.count_finished() > 0 {
                    self.modal = Modal::ExitPrompt { selected: 0 };
                } else {
                    self.should_quit = true;
                    return Some(Effect::QuitKeep);
                }
            }
            Action::QuitKeepAll => {
                self.should_quit = true;
                return Some(Effect::QuitKeep);
            }
            Action::QuitKillAll => {
                self.should_quit = true;
                return Some(Effect::QuitKill);
            }
        }
        None
    }

    /// One press of `s`. Ratchets: `C-c`, then SIGTERM, then SIGKILL, so a
    /// process that ignores an interrupt can still be stopped by pressing the
    /// same key again. Never rewinds, and never escalates twice from one
    /// press. All three levels leave the pane alive, so output stays readable
    /// (`remain-on-exit`); only `X` destroys output.
    fn escalate_stop(&mut self) -> Option<Effect> {
        let job = self.group.jobs.get_mut(self.focus.job)?;

        // A job that already exited has nothing to signal.
        if !matches!(job.state, JobState::Starting | JobState::Running) {
            return None;
        }
        // Give the previous signal a moment to work before hitting harder, so
        // an impatient double-tap does not jump straight to KILL.
        if let Some(last) = job.last_stop
            && last.elapsed() < STOP_ESCALATION_GRACE
        {
            return None;
        }

        job.stop_level = job.stop_level.saturating_add(1).min(3);
        job.last_stop = Some(Instant::now());

        let name = job.tmux_name.clone();
        Some(match job.stop_level {
            1 => Effect::Stop(name),
            2 => Effect::Signal(name, "TERM"),
            _ => Effect::Signal(name, "KILL"),
        })
    }

    /// Perform a confirmed action. Separate from `apply` so the action that
    /// opens a prompt and the action that runs on yes cannot be confused.
    fn run_pending(&mut self, pending: Pending) -> Option<Effect> {
        match pending {
            Pending::KillFocused => {
                let job = self.group.jobs.get_mut(self.focus.job)?;
                model::force_kill(job);
                let name = job.tmux_name.clone();
                self.group.remove_dismissed();
                self.recompute();
                Some(Effect::Kill(name))
            }
            Pending::DismissAllFinished => {
                let mut killed = Vec::new();
                for job in &mut self.group.jobs {
                    if model::dismiss(job) {
                        killed.push(job.tmux_name.clone());
                    }
                }
                if killed.is_empty() {
                    return None;
                }
                self.group.remove_dismissed();
                self.recompute();
                Some(Effect::KillMany(killed))
            }
        }
    }

    /// How many jobs `d` / `D` would accept right now: finished or vanished,
    /// never running.
    fn count_dismissable(&self) -> usize {
        self.group
            .jobs
            .iter()
            .filter(|j| matches!(j.state, JobState::Finished { .. } | JobState::Vanished))
            .count()
    }

    /// Set the current page and recompute geometry for it.
    fn set_page(&mut self, page: usize) {
        if self.page != page {
            self.page = page;
            self.recompute();
        }
    }

    /// After a page change, move focus onto that page's first tile so the
    /// focused tile is always visible.
    fn focus_into_page(&mut self) {
        let start = layout::page_start(self.page, self.geometry.per_page);
        if self.focus.job < start || self.focus.job >= start + self.geometry.per_page {
            self.focus.job = start.min(self.group.jobs.len().saturating_sub(1));
        }
    }

    /// The group's working directory, used when launching added commands.
    pub fn cwd(&self) -> PathBuf {
        self.group.cwd.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::{GroupId, JobId};
    use crate::model::Outcome;

    fn job(i: u16, state: JobState) -> Job {
        let id = JobId {
            group: GroupId::new("k3f9"),
            index: i,
        };
        let mut j = Job::new(id, format!("cmd{i}"), PathBuf::from("/tmp"), 0);
        j.state = state;
        j
    }

    fn app(jobs: Vec<Job>) -> App {
        let group = Group {
            id: GroupId::new("k3f9"),
            label: None,
            cwd: PathBuf::from("/tmp"),
            jobs,
            next_index: 1,
        };
        let mut a = App::new(group, ViewMode::Grid, Tier::Full);
        a.body = Rect {
            x: 0,
            y: 0,
            width: 120,
            height: 40,
        };
        a.recompute();
        a
    }

    #[test]
    fn reattach_hint_uses_short_attach_flag() {
        let a = app(vec![]);
        assert_eq!(a.reattach_hint, "multi-run -a k3f9");
    }

    #[test]
    fn attach_targets_the_focused_job() {
        let mut a = app(vec![job(1, JobState::Running), job(2, JobState::Running)]);
        a.focus.job = 1;
        assert_eq!(
            a.apply(Action::Attach),
            Some(Effect::Attach("mr-k3f9-2".into()))
        );
    }

    #[test]
    fn attach_on_an_empty_group_does_nothing() {
        let mut a = app(vec![]);
        assert_eq!(a.apply(Action::Attach), None);
    }

    #[test]
    fn dismiss_is_refused_while_a_job_is_running() {
        let mut a = app(vec![job(1, JobState::Running)]);
        assert_eq!(a.apply(Action::Dismiss), None);
        assert_eq!(a.group.jobs.len(), 1, "a running job must not be dismissed");
    }

    #[test]
    fn dismiss_removes_a_finished_job_and_kills_its_session() {
        let mut a = app(vec![job(
            1,
            JobState::Finished {
                outcome: Outcome::Exit(0),
                at: None,
            },
        )]);
        assert_eq!(
            a.apply(Action::Dismiss),
            Some(Effect::Kill("mr-k3f9-1".into()))
        );
        assert!(a.group.jobs.is_empty());
    }

    #[test]
    fn dismiss_all_asks_first_then_removes_every_finished_job() {
        let fin = || JobState::Finished {
            outcome: Outcome::Exit(0),
            at: None,
        };
        let mut a = app(vec![
            job(1, fin()),
            job(2, JobState::Running),
            job(3, fin()),
        ]);

        assert_eq!(
            a.apply(Action::DismissAllFinished),
            None,
            "must confirm first"
        );
        assert!(matches!(a.modal, Modal::Confirm { .. }));
        assert_eq!(a.group.jobs.len(), 3, "nothing goes before confirmation");

        let effect = a.apply(Action::ConfirmYes);
        assert_eq!(
            effect,
            Some(Effect::KillMany(vec![
                "mr-k3f9-1".into(),
                "mr-k3f9-3".into()
            ]))
        );
        assert_eq!(a.group.jobs.len(), 1, "the running job survives");
    }

    #[test]
    fn dismiss_all_with_nothing_finished_does_not_prompt() {
        let mut a = app(vec![job(1, JobState::Running)]);
        assert_eq!(a.apply(Action::DismissAllFinished), None);
        assert!(
            matches!(a.modal, Modal::None),
            "no prompt when there is nothing to dismiss"
        );
    }

    #[test]
    fn stop_escalates_c_c_then_term_then_kill() {
        let mut a = app(vec![job(1, JobState::Running)]);

        assert_eq!(
            a.apply(Action::Stop),
            Some(Effect::Stop("mr-k3f9-1".into()))
        );
        // Each further press must clear the grace window, otherwise an
        // impatient double-tap would jump straight to KILL.
        a.group.jobs[0].last_stop = Some(Instant::now() - STOP_ESCALATION_GRACE);
        assert_eq!(
            a.apply(Action::Stop),
            Some(Effect::Signal("mr-k3f9-1".into(), "TERM"))
        );

        a.group.jobs[0].last_stop = Some(Instant::now() - STOP_ESCALATION_GRACE);
        assert_eq!(
            a.apply(Action::Stop),
            Some(Effect::Signal("mr-k3f9-1".into(), "KILL"))
        );

        // KILL is the top of the ratchet; it never rewinds or goes further.
        a.group.jobs[0].last_stop = Some(Instant::now() - STOP_ESCALATION_GRACE);
        assert_eq!(
            a.apply(Action::Stop),
            Some(Effect::Signal("mr-k3f9-1".into(), "KILL"))
        );
    }

    #[test]
    fn stop_ignores_a_rapid_second_press() {
        let mut a = app(vec![job(1, JobState::Running)]);
        assert_eq!(
            a.apply(Action::Stop),
            Some(Effect::Stop("mr-k3f9-1".into()))
        );
        assert_eq!(a.apply(Action::Stop), None, "within the grace window");
        assert_eq!(
            a.group.jobs[0].stop_level, 1,
            "level must not have advanced"
        );
    }

    #[test]
    fn stop_does_nothing_to_a_job_that_already_exited() {
        let mut a = app(vec![job(
            1,
            JobState::Finished {
                outcome: Outcome::Exit(0),
                at: None,
            },
        )]);
        assert_eq!(a.apply(Action::Stop), None);
    }

    #[test]
    fn clicking_a_grid_tile_focuses_it() {
        let mut a = app(vec![job(1, JobState::Running), job(2, JobState::Running)]);
        let second = a.geometry.tiles[1];
        a.on_mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: second.x + 1,
            row: second.y + 1,
            modifiers: crossterm::event::KeyModifiers::NONE,
        });
        assert_eq!(a.focus.job, 1);
    }

    #[test]
    fn clicking_empty_space_leaves_focus_alone() {
        let mut a = app(vec![job(1, JobState::Running)]);
        a.on_mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 5000,
            row: 5000,
            modifiers: crossterm::event::KeyModifiers::NONE,
        });
        assert_eq!(a.focus.job, 0);
    }

    #[test]
    fn g_jumps_the_detail_pane_to_the_live_bottom_when_scrolled() {
        let mut a = app(vec![job(1, JobState::Running), job(2, JobState::Running)]);
        a.view = ViewMode::ListDetail;
        a.focus.pane = Pane::Detail;
        a.focus.job = 0;
        a.group.jobs[0].scroll = 40;

        a.apply(Action::FocusLast);
        assert_eq!(a.group.jobs[0].scroll, 0, "G returns to the live bottom");
        assert_eq!(a.focus.job, 0, "and does not move to the last job");
    }

    #[test]
    fn scroll_up_stops_at_the_top_of_the_pane_history() {
        // Regression: an unbounded offset made tmux clamp `-S` to the oldest
        // history line while `-E` kept descending, so the capture came back
        // short and the tile padded the shortfall at the bottom, eating the
        // output from the bottom up.
        let mut a = app(vec![job(1, JobState::Running)]);
        a.view = ViewMode::ListDetail;
        a.focus.pane = Pane::Detail;
        a.group.jobs[0].facts.history_size = 10;

        for _ in 0..5 {
            a.apply(Action::ScrollUp(4));
        }
        assert_eq!(a.group.jobs[0].scroll, 10, "clamped to history_size");
    }

    #[test]
    fn scroll_up_does_nothing_when_there_is_no_history() {
        let mut a = app(vec![job(1, JobState::Running)]);
        a.view = ViewMode::ListDetail;
        a.focus.pane = Pane::Detail;
        a.group.jobs[0].facts.history_size = 0;

        a.apply(Action::ScrollUp(3));
        assert_eq!(a.group.jobs[0].scroll, 0, "stays on the live bottom");
    }

    #[test]
    fn g_still_focuses_the_last_job_when_not_scrolled() {
        let mut a = app(vec![job(1, JobState::Running), job(2, JobState::Running)]);
        a.view = ViewMode::ListDetail;
        a.focus.pane = Pane::Detail;
        a.apply(Action::FocusLast);
        assert_eq!(a.focus.job, 1);
    }

    #[test]
    fn kill_focused_asks_before_destroying_output() {
        let mut a = app(vec![job(1, JobState::Running)]);
        assert_eq!(a.apply(Action::KillFocused), None);
        assert!(matches!(a.modal, Modal::Confirm { .. }));
        assert_eq!(a.group.jobs.len(), 1, "nothing dies before confirmation");
    }

    #[test]
    fn confirming_a_kill_runs_the_wrapped_action() {
        let mut a = app(vec![job(1, JobState::Running)]);
        a.apply(Action::KillFocused);
        assert_eq!(
            a.apply(Action::ConfirmYes),
            Some(Effect::Kill("mr-k3f9-1".into()))
        );
        assert!(a.group.jobs.is_empty());
        assert!(matches!(a.modal, Modal::None));
    }

    #[test]
    fn declining_a_kill_leaves_the_job_alone() {
        let mut a = app(vec![job(1, JobState::Running)]);
        a.apply(Action::KillFocused);
        assert_eq!(a.apply(Action::ConfirmNo), None);
        assert_eq!(a.group.jobs.len(), 1);
        assert!(matches!(a.modal, Modal::None));
    }

    #[test]
    fn quitting_with_live_jobs_opens_the_exit_prompt() {
        let mut a = app(vec![job(1, JobState::Running)]);
        assert_eq!(a.apply(Action::Quit), None);
        assert!(matches!(a.modal, Modal::ExitPrompt { .. }));
        assert!(!a.should_quit, "quit must not be immediate while jobs live");
    }

    #[test]
    fn quitting_an_empty_group_exits_immediately() {
        let mut a = app(vec![]);
        assert_eq!(a.apply(Action::Quit), Some(Effect::QuitKeep));
        assert!(a.should_quit);
    }

    #[test]
    fn exit_prompt_keep_and_kill_are_distinct_effects() {
        let mut a = app(vec![job(1, JobState::Running)]);
        assert_eq!(a.apply(Action::QuitKeepAll), Some(Effect::QuitKeep));
        let mut b = app(vec![job(1, JobState::Running)]);
        assert_eq!(b.apply(Action::QuitKillAll), Some(Effect::QuitKill));
    }

    #[test]
    fn exit_prompt_arrows_move_selection_and_enter_activates() {
        let mut a = app(vec![job(1, JobState::Running)]);
        a.apply(Action::Quit);
        assert!(matches!(
            a.modal,
            Modal::ExitPrompt { selected: 0 }
        ));

        a.apply(Action::SelectNext);
        assert!(matches!(
            a.modal,
            Modal::ExitPrompt { selected: 1 }
        ));
        // Does not wrap past the last option.
        a.apply(Action::SelectNext);
        assert!(matches!(
            a.modal,
            Modal::ExitPrompt { selected: 1 }
        ));

        a.apply(Action::SelectPrev);
        assert!(matches!(
            a.modal,
            Modal::ExitPrompt { selected: 0 }
        ));

        // Enter on keep exits without killing.
        assert_eq!(a.apply(Action::SelectActivate), Some(Effect::QuitKeep));
        assert!(a.should_quit);
    }

    #[test]
    fn exit_prompt_enter_on_kill_emits_quit_kill() {
        let mut a = app(vec![job(1, JobState::Running)]);
        a.apply(Action::Quit);
        a.apply(Action::SelectNext);
        assert_eq!(a.apply(Action::SelectActivate), Some(Effect::QuitKill));
        assert!(a.should_quit);
    }

    #[test]
    fn confirm_arrows_move_selection_and_enter_activates() {
        let mut a = app(vec![job(1, JobState::Running)]);
        a.apply(Action::KillFocused);
        assert!(matches!(
            a.modal,
            Modal::Confirm { selected: 0, .. }
        ));

        // Move to no, Enter cancels without killing.
        a.apply(Action::SelectNext);
        assert_eq!(a.apply(Action::SelectActivate), None);
        assert_eq!(a.group.jobs.len(), 1);
        assert!(matches!(a.modal, Modal::None));

        // Re-open and Enter on yes (default) kills.
        a.apply(Action::KillFocused);
        assert_eq!(
            a.apply(Action::SelectActivate),
            Some(Effect::Kill("mr-k3f9-1".into()))
        );
    }

    #[test]
    fn compose_commits_lines_then_submits_them_together() {
        let mut a = app(vec![]);
        a.apply(Action::OpenCompose);
        for c in "one".chars() {
            a.apply(Action::ComposeChar(c));
        }
        a.apply(Action::ComposeCommit);
        for c in "two".chars() {
            a.apply(Action::ComposeChar(c));
        }
        let effect = a.apply(Action::ComposeSubmit);
        assert_eq!(
            effect,
            Some(Effect::Launch(vec!["one".into(), "two".into()]))
        );
        assert!(matches!(a.modal, Modal::None));
    }

    #[test]
    fn compose_submitting_nothing_launches_nothing() {
        let mut a = app(vec![]);
        a.apply(Action::OpenCompose);
        assert_eq!(a.apply(Action::ComposeSubmit), None);
    }

    #[test]
    fn compose_ignores_a_blank_commit() {
        let mut a = app(vec![]);
        a.apply(Action::OpenCompose);
        a.apply(Action::ComposeChar(' '));
        a.apply(Action::ComposeCommit);
        if let Modal::Compose { queued, .. } = &a.modal {
            assert!(queued.is_empty(), "whitespace must not become a command");
        } else {
            panic!("compose modal should still be open");
        }
    }

    #[test]
    fn toggling_view_switches_and_returns() {
        let mut a = app(vec![job(1, JobState::Running)]);
        a.apply(Action::ToggleView);
        assert_eq!(a.view, ViewMode::ListDetail);
        a.apply(Action::ToggleView);
        assert_eq!(a.view, ViewMode::Grid);
    }

    #[test]
    fn focus_next_wraps_at_the_end() {
        let mut a = app(vec![job(1, JobState::Running), job(2, JobState::Running)]);
        a.apply(Action::FocusNext);
        assert_eq!(a.focus.job, 1);
        a.apply(Action::FocusNext);
        assert_eq!(a.focus.job, 0, "focus should wrap to the first job");
    }

    #[test]
    fn help_toggles_open_and_closed() {
        let mut a = app(vec![]);
        a.apply(Action::OpenHelp);
        assert!(matches!(a.modal, Modal::Help));
        a.apply(Action::OpenHelp);
        assert!(matches!(a.modal, Modal::None));
    }

    #[test]
    fn errors_are_capped_and_keep_the_newest() {
        let mut a = app(vec![]);
        for i in 0..MAX_ERRORS + 10 {
            a.push_error(format!("e{i}"));
        }
        assert_eq!(a.errors.len(), MAX_ERRORS);
        assert_eq!(a.errors.last().unwrap(), &format!("e{}", MAX_ERRORS + 9));
    }

    #[test]
    fn an_error_does_not_clobber_a_deliberate_modal() {
        let mut a = app(vec![]);
        a.apply(Action::OpenHelp);
        a.push_error("boom");
        assert!(
            matches!(a.modal, Modal::Help),
            "help must survive a background error"
        );
        assert_eq!(a.errors.len(), 1, "the error is still logged");
    }

    #[test]
    fn toasts_expire_on_their_own() {
        let mut a = app(vec![]);
        a.push_error("boom");
        assert!(matches!(a.modal, Modal::Toast { .. }));
        assert!(a.expire_toast(Instant::now() + TOAST_TTL));
        assert!(matches!(a.modal, Modal::None));
    }

    #[test]
    fn focus_is_clamped_when_jobs_disappear() {
        let mut a = app(vec![job(1, JobState::Running), job(2, JobState::Running)]);
        a.focus.job = 1;
        a.group.jobs.pop();
        a.recompute();
        assert_eq!(a.focus.job, 0);
    }

    #[test]
    fn modal_state_mirrors_the_modal() {
        let mut a = app(vec![]);
        assert_eq!(a.modal_state(), ModalState::None);
        a.apply(Action::OpenHelp);
        assert_eq!(a.modal_state(), ModalState::Help);
        a.apply(Action::CloseModal);
        a.apply(Action::OpenCompose);
        assert_eq!(
            a.modal_state(),
            ModalState::Compose {
                line_empty: true,
                launch_on_submit: false
            }
        );
    }
}
