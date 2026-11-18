//! Pure simulation library for Neon Run.
//!
//! No window or graphics dependency. All gameplay rules live here so
//! `cargo test` runs without a display.

pub mod bindings;
pub mod course;
pub mod input;
pub mod level;
pub mod prng;
pub mod tuning;

pub use bindings::{Action, BindingsError, KeyBindings, KeyName, DEFAULT_BINDINGS_PATH};
pub use course::{
    clamp_hardness, course_from_seed, course_from_seed_with, hardness_keeps, Course, Entity,
    EntityKind, FlankPhase, GenConfig, Segment, DEFAULT_PLAY_HARDNESS, HARDNESS_MAX, HARDNESS_MIN,
};
pub use input::Input;
pub use level::{
    chase_lateral, Explosion, Level, Player, RunStatus, SimConfig, CHASE_CURVE_RESPONSE,
    CHASE_SEG_LEN, EXPLOSION_LIFE,
};
pub use tuning::{default_tints, TintRgb, Tuning, TuningError, DEFAULT_TUNING_PATH};
