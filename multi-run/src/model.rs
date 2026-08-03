//! `Job`, `Group`, `JobState`, `PaneFacts`, and the state machine that
//! advances a job's state from polled tmux facts. DESIGN.md section 4 is the
//! source of truth, in particular the "State machine" table.
//!
//! This module is pure: no tmux subprocess calls (those live in `tmux/`) and
//! no rendering (that lives in `ui/`). `GroupId`/`JobId` are owned by
//! `cli.rs`, not redefined here.

use std::path::PathBuf;
use std::time::Instant;

use ratatui::text::Line;

use crate::cli::{GroupId, JobId};

/// The observable facts about one pane, straight from a poll. Every field is
/// exactly what tmux reported; no interpretation happens here. Interpretation
/// (turning `dead_status`/`dead_signal` into an `Outcome`) happens in
/// `advance`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PaneFacts {
    pub dead: bool,                  // #{pane_dead}
    pub dead_status: Option<i32>,    // #{pane_dead_status}, empty when signalled
    pub dead_signal: Option<String>, // #{pane_dead_signal}, e.g. "term"
    pub dead_time: Option<u64>,      // #{pane_dead_time}
    pub pane_pid: Option<u32>,       // #{pane_pid}
    pub width: u16,                  // #{pane_width}
    pub height: u16,                 // #{pane_height}
    pub history_size: u32,           // #{history_size}, cheap activity heuristic
    pub cursor_y: u16,               // #{cursor_y}, ditto
    pub session_attached: bool,      // #{session_attached}
}

/// How a finished job ended.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Outcome {
    /// `pane_dead=1`, `pane_dead_status=N`, no signal.
    Exit(i32),
    /// `pane_dead=1`, `pane_dead_signal="term"/"kill"/...`, no status.
    Signal(String),
    /// `pane_dead=1` and both fields empty. Should not happen; surfaced, not
    /// hidden. Also used for the case where a poll somehow reports both a
    /// status and a signal at once, which tmux should never do; treating it
    /// as `Unknown` rather than guessing which field to trust keeps this
    /// function total without inventing a third meaning.
    Unknown,
}

impl Outcome {
    /// Classify death facts into an `Outcome`. Only meaningful once
    /// `facts.dead` is true; callers are expected to check that first (see
    /// `advance`).
    fn from_facts(facts: &PaneFacts) -> Outcome {
        match (&facts.dead_status, &facts.dead_signal) {
            (Some(status), None) => Outcome::Exit(*status),
            (None, Some(signal)) => Outcome::Signal(signal.clone()),
            _ => Outcome::Unknown,
        }
    }
}

/// A job's lifecycle state. See DESIGN.md section 4 for the full transition
/// table; `advance` implements the poll-driven edges and `dismiss`/
/// `force_kill` implement the user-driven ones.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum JobState {
    /// We issued `new-session` and have not yet seen this session in a poll.
    Starting,
    /// Present in `list-sessions`, `pane_dead = 0`.
    Running,
    /// Present in `list-sessions`, `pane_dead = 1`. Output is still readable.
    Finished { outcome: Outcome, at: Option<u64> },
    /// Present in our model but absent from `list-sessions`. Killed by
    /// something other than us. The tile keeps its last content and a "gone"
    /// marker until dismissed.
    Vanished,
    /// The user explicitly dismissed it. The tile is removed on the next
    /// frame via `Group::remove_dismissed`. This state exists only so
    /// removal is a normal state transition rather than a special case in
    /// the event handler.
    Dismissed,
}

/// One job inside a group: everything needed to track and render it.
pub struct Job {
    pub id: JobId,
    /// Cached `JobId::tmux_name()`.
    pub tmux_name: String,
    pub command: String,
    pub cwd: PathBuf,
    pub started_at: u64,
    pub state: JobState,
    pub facts: PaneFacts,

    /// Last successful decode of `capture-pane` output, already
    /// padded/truncated to exactly `tile_size.1` lines. Owned so rendering
    /// never borrows from IO.
    pub content: Vec<Line<'static>>,
    /// Bumped whenever `content` changes. Used to skip redraws and to drive
    /// the "new output" indicator for offscreen tiles.
    pub content_rev: u64,
    pub last_capture: Option<Instant>,

    /// The size we last told tmux this pane should be. Compared against the
    /// desired size so we only issue `resize-window` when it actually
    /// changed.
    pub tile_size: (u16, u16),

    /// Ratcheting stop escalation: 0 = none sent, 1 = C-c sent, 2 = TERM, 3 =
    /// KILL.
    pub stop_level: u8,
    pub last_stop: Option<Instant>,

    /// Scrollback offset in lines, 0 = live bottom. Only non-zero in
    /// list/detail view.
    pub scroll: u32,

    /// Consecutive polls in which this job was `Starting` but absent from
    /// `list-sessions`. Reset to 0 the moment it is seen. Drives the
    /// `Starting -> Vanished` edge at 3 consecutive misses (DESIGN.md section
    /// 4). Not listed among `Job`'s fields in the design doc, which does not
    /// otherwise say where this bookkeeping lives; it is added here because
    /// it is per-job state needed to make `advance` a pure function of
    /// `(Job, one poll observation)` rather than requiring external
    /// bookkeeping in the caller.
    pub missed_polls: u16,
}

impl Job {
    /// Number of consecutive "not seen" polls after which a `Starting` job
    /// is declared `Vanished` (DESIGN.md section 4).
    const STARTING_MISS_LIMIT: u16 = 3;

    /// Build a fresh job in the `Starting` state, as it exists the instant
    /// `new-session` returns rc 0, before any poll has observed it.
    pub fn new(id: JobId, command: String, cwd: PathBuf, started_at: u64) -> Self {
        let tmux_name = id.tmux_name();
        Job {
            id,
            tmux_name,
            command,
            cwd,
            started_at,
            state: JobState::Starting,
            facts: PaneFacts::default(),
            content: Vec::new(),
            content_rev: 0,
            last_capture: None,
            tile_size: (0, 0),
            stop_level: 0,
            last_stop: None,
            scroll: 0,
            missed_polls: 0,
        }
    }

    /// Highest `scroll` offset that still has real content behind it: the
    /// number of lines tmux holds above the visible screen. Scrolling past it
    /// makes `capture-pane -S -<offset>` clamp to the top of the history while
    /// `-E` keeps descending, so tmux returns fewer lines than the tile is
    /// tall and `ui::tile::decode` pads the shortfall at the bottom, which
    /// looks like the output being eaten from the bottom up.
    pub fn max_scroll(&self) -> u32 {
        self.facts.history_size
    }
}

/// One poll's observation of a job: either it was not in `list-sessions` at
/// all, or it was, with the given pane facts.
pub enum PollObservation {
    Absent,
    Present(PaneFacts),
}

/// Advance a job's state given one poll's observation. Implements every
/// poll-driven row of the DESIGN.md section 4 state machine table. Rows not
/// listed there (e.g. `Dismissed` receiving another poll) are no-ops: once a
/// job leaves the tracked lifecycle by user action, further poll facts do
/// not move it.
pub fn advance(job: &mut Job, observation: PollObservation, now: u64) {
    match (&job.state, observation) {
        (JobState::Starting, PollObservation::Present(facts)) => {
            job.missed_polls = 0;
            if facts.dead {
                job.state = JobState::Finished {
                    outcome: Outcome::from_facts(&facts),
                    at: facts.dead_time.or(Some(now)),
                };
            } else {
                job.state = JobState::Running;
            }
            job.facts = facts;
        }
        (JobState::Starting, PollObservation::Absent) => {
            job.missed_polls += 1;
            if job.missed_polls >= Job::STARTING_MISS_LIMIT {
                job.state = JobState::Vanished;
            }
        }
        (JobState::Running, PollObservation::Present(facts)) => {
            if facts.dead {
                job.state = JobState::Finished {
                    outcome: Outcome::from_facts(&facts),
                    at: facts.dead_time.or(Some(now)),
                };
            }
            job.facts = facts;
        }
        (JobState::Running, PollObservation::Absent) => {
            job.state = JobState::Vanished;
        }
        (JobState::Finished { .. }, PollObservation::Present(facts)) => {
            // Still finished; a dead pane does not become alive again. Facts
            // are refreshed (e.g. a resize can update width/height) but the
            // state itself does not change.
            job.facts = facts;
        }
        (JobState::Finished { .. }, PollObservation::Absent) => {
            job.state = JobState::Vanished;
        }
        // Vanished and Dismissed are absorbing under further poll
        // observations; only user actions (dismiss/force_kill) move out of
        // them, and Dismissed jobs are removed from the group entirely
        // before their next poll.
        (JobState::Vanished, _) | (JobState::Dismissed, _) => {}
    }
}

/// Dismiss a finished or vanished job (`d`). Per DESIGN.md section 4, `d` is
/// only valid on `Finished` or `Vanished`; the keymap is expected to gate
/// this, but the transition itself is safe to call in any state and is a
/// no-op otherwise. Returns whether the state changed. Issuing `kill-session`
/// (only needed for `Finished`, not `Vanished`, since a vanished session is
/// already gone) is the caller's responsibility; this function only advances
/// the model.
pub fn dismiss(job: &mut Job) -> bool {
    match job.state {
        JobState::Finished { .. } | JobState::Vanished => {
            job.state = JobState::Dismissed;
            true
        }
        _ => false,
    }
}

/// Force-kill (`X`): valid in any state, always transitions to `Dismissed`.
/// Issuing `kill-session` is the caller's responsibility.
pub fn force_kill(job: &mut Job) {
    job.state = JobState::Dismissed;
}

/// A run group: everything created by one `multi-run` invocation.
pub struct Group {
    pub id: GroupId,
    pub label: Option<String>,
    pub cwd: PathBuf,
    pub jobs: Vec<Job>,
    /// Monotonic. Never reuses an index, so a job id always identifies one
    /// command run.
    pub next_index: u16,
}

impl Group {
    /// Allocate the next monotonic job index for this group. Indices are
    /// never reused, even after a job is dismissed.
    pub fn next_job_index(&mut self) -> u16 {
        let idx = self.next_index;
        self.next_index += 1;
        idx
    }

    /// Remove every job whose state is `Dismissed`. Called once per frame
    /// after a dismiss/kill action so removal is a normal consequence of the
    /// state machine rather than a special-cased `Vec::remove` at the call
    /// site (DESIGN.md section 4).
    pub fn remove_dismissed(&mut self) {
        self.jobs
            .retain(|j| !matches!(j.state, JobState::Dismissed));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn group_id() -> GroupId {
        "k3f9".parse().unwrap()
    }

    fn job_id(index: u16) -> JobId {
        JobId {
            group: group_id(),
            index,
        }
    }

    fn starting_job() -> Job {
        Job::new(
            job_id(1),
            "cargo test".to_string(),
            PathBuf::from("/tmp"),
            1000,
        )
    }

    fn alive_facts() -> PaneFacts {
        PaneFacts {
            dead: false,
            ..Default::default()
        }
    }

    fn exit_facts(status: i32) -> PaneFacts {
        PaneFacts {
            dead: true,
            dead_status: Some(status),
            ..Default::default()
        }
    }

    fn signal_facts(signal: &str) -> PaneFacts {
        PaneFacts {
            dead: true,
            dead_signal: Some(signal.to_string()),
            ..Default::default()
        }
    }

    fn unknown_dead_facts() -> PaneFacts {
        PaneFacts {
            dead: true,
            ..Default::default()
        }
    }

    // --- Outcome::from_facts --------------------------------------------

    #[test]
    fn outcome_exit_when_status_present_no_signal() {
        assert_eq!(Outcome::from_facts(&exit_facts(42)), Outcome::Exit(42));
    }

    #[test]
    fn outcome_signal_when_signal_present_no_status() {
        assert_eq!(
            Outcome::from_facts(&signal_facts("term")),
            Outcome::Signal("term".to_string())
        );
    }

    #[test]
    fn outcome_unknown_when_both_empty() {
        assert_eq!(Outcome::from_facts(&unknown_dead_facts()), Outcome::Unknown);
    }

    #[test]
    fn outcome_unknown_when_both_present() {
        let facts = PaneFacts {
            dead: true,
            dead_status: Some(0),
            dead_signal: Some("term".to_string()),
            ..Default::default()
        };
        assert_eq!(Outcome::from_facts(&facts), Outcome::Unknown);
    }

    // --- new-session outcome (not modelled as an advance() edge) --------

    #[test]
    fn new_job_starts_in_starting_state() {
        let job = starting_job();
        assert_eq!(job.state, JobState::Starting);
        assert_eq!(job.missed_polls, 0);
    }

    // --- Starting -> Running / Finished / Vanished -----------------------

    #[test]
    fn starting_seen_alive_becomes_running() {
        let mut job = starting_job();
        advance(&mut job, PollObservation::Present(alive_facts()), 1000);
        assert_eq!(job.state, JobState::Running);
    }

    #[test]
    fn starting_seen_dead_becomes_finished_fast_exit() {
        let mut job = starting_job();
        advance(&mut job, PollObservation::Present(exit_facts(0)), 1000);
        assert_eq!(
            job.state,
            JobState::Finished {
                outcome: Outcome::Exit(0),
                at: Some(1000)
            }
        );
    }

    #[test]
    fn starting_not_seen_once_stays_starting() {
        let mut job = starting_job();
        advance(&mut job, PollObservation::Absent, 1000);
        assert_eq!(job.state, JobState::Starting);
        assert_eq!(job.missed_polls, 1);
    }

    #[test]
    fn starting_not_seen_twice_stays_starting() {
        let mut job = starting_job();
        advance(&mut job, PollObservation::Absent, 1000);
        advance(&mut job, PollObservation::Absent, 1001);
        assert_eq!(job.state, JobState::Starting);
        assert_eq!(job.missed_polls, 2);
    }

    #[test]
    fn starting_not_seen_three_times_becomes_vanished() {
        let mut job = starting_job();
        for t in 0..3 {
            advance(&mut job, PollObservation::Absent, 1000 + t);
        }
        assert_eq!(job.state, JobState::Vanished);
    }

    #[test]
    fn starting_missed_poll_count_resets_on_sighting() {
        let mut job = starting_job();
        advance(&mut job, PollObservation::Absent, 1000);
        advance(&mut job, PollObservation::Absent, 1001);
        advance(&mut job, PollObservation::Present(alive_facts()), 1002);
        assert_eq!(job.state, JobState::Running);
        assert_eq!(job.missed_polls, 0);
    }

    // --- Running transitions ---------------------------------------------

    #[test]
    fn running_seen_alive_stays_running() {
        let mut job = starting_job();
        job.state = JobState::Running;
        advance(&mut job, PollObservation::Present(alive_facts()), 1000);
        assert_eq!(job.state, JobState::Running);
    }

    #[test]
    fn running_dead_with_status_becomes_finished_exit() {
        let mut job = starting_job();
        job.state = JobState::Running;
        advance(&mut job, PollObservation::Present(exit_facts(7)), 2000);
        assert_eq!(
            job.state,
            JobState::Finished {
                outcome: Outcome::Exit(7),
                at: Some(2000)
            }
        );
    }

    #[test]
    fn running_dead_with_signal_becomes_finished_signal() {
        let mut job = starting_job();
        job.state = JobState::Running;
        advance(
            &mut job,
            PollObservation::Present(signal_facts("kill")),
            2000,
        );
        assert_eq!(
            job.state,
            JobState::Finished {
                outcome: Outcome::Signal("kill".to_string()),
                at: Some(2000)
            }
        );
    }

    #[test]
    fn running_dead_with_neither_becomes_finished_unknown() {
        let mut job = starting_job();
        job.state = JobState::Running;
        advance(
            &mut job,
            PollObservation::Present(unknown_dead_facts()),
            2000,
        );
        assert_eq!(
            job.state,
            JobState::Finished {
                outcome: Outcome::Unknown,
                at: Some(2000)
            }
        );
    }

    #[test]
    fn running_absent_becomes_vanished() {
        let mut job = starting_job();
        job.state = JobState::Running;
        advance(&mut job, PollObservation::Absent, 2000);
        assert_eq!(job.state, JobState::Vanished);
    }

    #[test]
    fn finished_uses_dead_time_when_present() {
        let mut job = starting_job();
        job.state = JobState::Running;
        let facts = PaneFacts {
            dead: true,
            dead_status: Some(1),
            dead_time: Some(555),
            ..Default::default()
        };
        advance(&mut job, PollObservation::Present(facts), 2000);
        assert_eq!(
            job.state,
            JobState::Finished {
                outcome: Outcome::Exit(1),
                at: Some(555)
            }
        );
    }

    // --- Finished transitions ---------------------------------------------

    #[test]
    fn finished_stays_finished_when_seen_again() {
        let mut job = starting_job();
        job.state = JobState::Finished {
            outcome: Outcome::Exit(0),
            at: Some(1000),
        };
        advance(&mut job, PollObservation::Present(exit_facts(0)), 2000);
        assert_eq!(
            job.state,
            JobState::Finished {
                outcome: Outcome::Exit(0),
                at: Some(1000)
            }
        );
    }

    #[test]
    fn finished_refreshes_facts_without_changing_state() {
        let mut job = starting_job();
        job.state = JobState::Finished {
            outcome: Outcome::Exit(0),
            at: Some(1000),
        };
        let facts = PaneFacts {
            dead: true,
            dead_status: Some(0),
            width: 99,
            ..Default::default()
        };
        advance(&mut job, PollObservation::Present(facts.clone()), 2000);
        assert_eq!(job.facts, facts);
    }

    #[test]
    fn finished_absent_becomes_vanished() {
        let mut job = starting_job();
        job.state = JobState::Finished {
            outcome: Outcome::Exit(0),
            at: Some(1000),
        };
        advance(&mut job, PollObservation::Absent, 2000);
        assert_eq!(job.state, JobState::Vanished);
    }

    // --- Vanished / Dismissed are absorbing under polls -------------------

    #[test]
    fn vanished_present_again_is_a_no_op() {
        let mut job = starting_job();
        job.state = JobState::Vanished;
        advance(&mut job, PollObservation::Present(alive_facts()), 2000);
        assert_eq!(job.state, JobState::Vanished);
    }

    #[test]
    fn vanished_absent_is_a_no_op() {
        let mut job = starting_job();
        job.state = JobState::Vanished;
        advance(&mut job, PollObservation::Absent, 2000);
        assert_eq!(job.state, JobState::Vanished);
    }

    #[test]
    fn dismissed_present_is_a_no_op() {
        let mut job = starting_job();
        job.state = JobState::Dismissed;
        advance(&mut job, PollObservation::Present(alive_facts()), 2000);
        assert_eq!(job.state, JobState::Dismissed);
    }

    #[test]
    fn dismissed_absent_is_a_no_op() {
        let mut job = starting_job();
        job.state = JobState::Dismissed;
        advance(&mut job, PollObservation::Absent, 2000);
        assert_eq!(job.state, JobState::Dismissed);
    }

    // --- dismiss() ----------------------------------------------------

    #[test]
    fn dismiss_finished_becomes_dismissed() {
        let mut job = starting_job();
        job.state = JobState::Finished {
            outcome: Outcome::Exit(0),
            at: Some(1000),
        };
        assert!(dismiss(&mut job));
        assert_eq!(job.state, JobState::Dismissed);
    }

    #[test]
    fn dismiss_vanished_becomes_dismissed() {
        let mut job = starting_job();
        job.state = JobState::Vanished;
        assert!(dismiss(&mut job));
        assert_eq!(job.state, JobState::Dismissed);
    }

    #[test]
    fn dismiss_running_is_a_no_op() {
        let mut job = starting_job();
        job.state = JobState::Running;
        assert!(!dismiss(&mut job));
        assert_eq!(job.state, JobState::Running);
    }

    #[test]
    fn dismiss_starting_is_a_no_op() {
        let mut job = starting_job();
        assert!(!dismiss(&mut job));
        assert_eq!(job.state, JobState::Starting);
    }

    // --- force_kill() ---------------------------------------------------

    #[test]
    fn force_kill_from_any_state_becomes_dismissed() {
        for state in [
            JobState::Starting,
            JobState::Running,
            JobState::Finished {
                outcome: Outcome::Exit(0),
                at: None,
            },
            JobState::Vanished,
        ] {
            let mut job = starting_job();
            job.state = state;
            force_kill(&mut job);
            assert_eq!(job.state, JobState::Dismissed);
        }
    }

    // --- Group ------------------------------------------------------------

    fn group_with(jobs: Vec<Job>) -> Group {
        Group {
            id: group_id(),
            label: None,
            cwd: PathBuf::from("/tmp"),
            jobs,
            next_index: 1,
        }
    }

    #[test]
    fn next_job_index_is_monotonic_and_never_reused() {
        let mut group = group_with(Vec::new());
        assert_eq!(group.next_job_index(), 1);
        assert_eq!(group.next_job_index(), 2);
        assert_eq!(group.next_job_index(), 3);
    }

    #[test]
    fn remove_dismissed_drops_only_dismissed_jobs() {
        let mut running = starting_job();
        running.state = JobState::Running;
        let mut dismissed = Job::new(
            job_id(2),
            "npm run dev".to_string(),
            PathBuf::from("/tmp"),
            1000,
        );
        dismissed.state = JobState::Dismissed;
        let mut vanished = Job::new(
            job_id(3),
            "sleep 5".to_string(),
            PathBuf::from("/tmp"),
            1000,
        );
        vanished.state = JobState::Vanished;

        let mut group = group_with(vec![running, dismissed, vanished]);
        group.remove_dismissed();

        assert_eq!(group.jobs.len(), 2);
        assert!(group.jobs.iter().all(|j| j.state != JobState::Dismissed));
    }

    #[test]
    fn remove_dismissed_is_a_no_op_when_none_dismissed() {
        let mut running = starting_job();
        running.state = JobState::Running;
        let mut group = group_with(vec![running]);
        group.remove_dismissed();
        assert_eq!(group.jobs.len(), 1);
    }
}
