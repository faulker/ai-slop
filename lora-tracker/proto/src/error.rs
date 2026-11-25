//! Error type shared by every proto module.

/// Everything that can go wrong encoding, decoding, sealing, or opening.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    /// The output buffer is too small for the encoded result.
    BufferTooSmall,
    /// Input ended before a complete header or tag.
    Truncated,
    /// Unknown wire or config version byte.
    BadVersion(u8),
    /// Config blob does not start with the expected magic bytes.
    BadMagic,
    /// postcard failed to serialize.
    Encode,
    /// postcard failed to deserialize.
    Decode,
    /// AEAD tag did not verify: wrong key, or the packet was altered.
    AuthFailed,
    /// Sequence number is not newer than the last accepted one.
    Replay,
}

impl core::fmt::Display for Error {
    /// Human-readable description, used in firmware logs and host CLIs.
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Error::BufferTooSmall => write!(f, "output buffer too small"),
            Error::Truncated => write!(f, "input truncated"),
            Error::BadVersion(v) => write!(f, "unsupported version {v}"),
            Error::BadMagic => write!(f, "not a lora-tracker config blob"),
            Error::Encode => write!(f, "serialization failed"),
            Error::Decode => write!(f, "deserialization failed"),
            Error::AuthFailed => write!(f, "authentication failed (wrong key or tampered)"),
            Error::Replay => write!(f, "replayed or stale sequence number"),
        }
    }
}
