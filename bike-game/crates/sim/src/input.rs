//! Per-frame player action levels / edges for the simulation step.

/// Input snapshot for one simulation step.
///
/// Boolean fields are "held this frame" levels. Pipe and laser fire while
/// held, but cooldowns in the sim prevent machine-gunning.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Input {
    pub accelerate: bool,
    pub brake: bool,
    pub steer_left: bool,
    pub steer_right: bool,
    pub pipe_left: bool,
    pub pipe_right: bool,
    pub laser: bool,
}
