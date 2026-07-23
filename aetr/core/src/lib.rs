//! aetr-core: encrypted text + voice over analog FM radio audio.
//!
//! Everything protocol-related lives here: Argon2id key derivation and
//! per-chunk XChaCha20-Poly1305 AEAD (`crypto`), the 11-byte chunk header,
//! chunking and reassembly (`frame`), Reed-Solomon erasure coding across
//! text chunks plus the ARQ parity pool (`fec`), codec2 voice spans with
//! 48k/8k resampling (`voice`), and the COFDM modem wrapper over the C++
//! shim (`modem`). Platform apps only move PCM and pixels.

// UniFFI's derives expect the scaffolding (UniFfiTag) at the crate root.
#[cfg(feature = "ffi")]
uniffi::setup_scaffolding!();

#[cfg(feature = "ffi")]
pub mod api;
pub mod crypto;
pub mod fec;
pub mod frame;
pub mod modem;
pub mod voice;

#[cfg(test)]
mod channel;
#[cfg(test)]
mod tests;

use std::fmt;

/// Errors surfaced by the aetr core. Library paths never panic; every
/// fallible operation returns one of these. Crosses the FFI as a flat
/// error: bindings see the variant name plus the Display message.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Error), uniffi(flat_error))]
pub enum AetrError {
    /// Key derivation failed (bad Argon2 parameters or output length).
    Kdf(String),
    /// AEAD authentication failed: wrong passphrase or corrupted chunk.
    AuthFailed,
    /// A frame or payload had an impossible size or malformed header.
    Malformed(String),
    /// Reed-Solomon coding failed (shard count/size mismatch).
    Fec(String),
    /// The voice codec or resampler rejected the input.
    Voice(String),
    /// The C++ modem shim reported an error.
    Modem(String),
    /// Message too large for the protocol limits (chunk_index is one byte).
    TooLarge(String),
}

impl fmt::Display for AetrError {
    /// Human-readable error text for logs and platform UI.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AetrError::Kdf(m) => write!(f, "key derivation failed: {m}"),
            AetrError::AuthFailed => write!(f, "chunk failed authentication"),
            AetrError::Malformed(m) => write!(f, "malformed frame: {m}"),
            AetrError::Fec(m) => write!(f, "erasure coding failed: {m}"),
            AetrError::Voice(m) => write!(f, "voice codec failed: {m}"),
            AetrError::Modem(m) => write!(f, "modem failed: {m}"),
            AetrError::TooLarge(m) => write!(f, "message too large: {m}"),
        }
    }
}

impl std::error::Error for AetrError {}
