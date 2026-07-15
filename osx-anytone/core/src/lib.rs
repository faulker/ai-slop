//! `anytone-core`: protocol and device layer for programming the AnyTone
//! AT-D878UVII Plus over its USB CDC-ACM serial link.
//!
//! Scope: serial transport (with an in-memory mock for tests), the byte-level
//! programming protocol, high-level codeplug backup/restore with read-back
//! verification, macOS serial port enumeration, the codeplug binary model
//! (channels, zones, DMR contacts/talk groups, RX group lists, radio IDs — see
//! [`codeplug`]) with JSON batch editing, and the C ABI for the AnyToneMac Swift
//! app (see [`ffi`]). The firmware UPDATE path is intentionally out of scope.

#![deny(clippy::all)]

pub mod codeplug;
pub mod device;
pub mod error;
pub mod ffi;
pub mod ports;
pub mod protocol;
pub mod transport;

pub use codeplug::{apply_edits, CallType, Channel, Codeplug, Contact, GroupList, RadioId, Zone};
pub use device::{codeplug_size, is_supported_model, Radio, Region, REGIONS};
pub use error::{Error, Result};
pub use ports::{autodetect_radio, list_ports, PortInfo};
pub use protocol::BLOCK_SIZE;
pub use transport::{MockTransport, SerialTransport, Transport};
