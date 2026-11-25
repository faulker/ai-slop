//! Vehicle-agnostic, read-only reconnaissance: DID snapshots, passive CAN
//! captures, and diffs between them. Nothing in this module writes to an ECU.

pub mod diff;
pub mod monitor;
pub mod session;
pub mod snapshot;
