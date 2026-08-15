//! `anytone-core`: protocol, device, and codeplug layers for programming
//! amateur radios over their USB serial links.
//!
//! The crate is organised per radio model. Everything specific to one radio,
//! its wire protocol, its memory layout, and the fake radio its tests drive,
//! lives in that model's module. [`anytone`] is the AnyTone AT-D878UVII Plus;
//! [`qyt`] is the QYT KT-5000.
//!
//! Model-agnostic pieces sit at the top level: the serial [`transport`], the
//! byte-encoding helpers in [`bytes`], macOS serial port enumeration in
//! [`ports`], the shared [`error`] type, and the C ABI for the Swift app in
//! [`ffi`].
//!
//! The firmware UPDATE path is intentionally out of scope.

#![deny(clippy::all)]

pub mod anytone;
pub mod bytes;
pub mod document;
pub mod driver;
pub mod error;
pub mod ffi;
pub mod model;
pub mod ports;
pub mod qyt;
pub mod transport;

// The AnyTone types are re-exported flat at the crate root because they were
// the crate's entire public surface before it grew a per-model structure.
// Keeping them here means `cli` and the C ABI compile unchanged.
pub use anytone::codeplug::{
    apply_edits, CallType, Channel, Codeplug, Contact, GroupList, RadioId, Zone,
};
pub use anytone::device::{codeplug_size, is_supported_model, Radio, Region, REGIONS};
pub use anytone::mock::MockTransport;
pub use anytone::protocol::BLOCK_SIZE;
pub use error::{Error, Result};
pub use ports::{autodetect_radio, list_ports, PortInfo};
pub use transport::{SerialTransport, Transport};
