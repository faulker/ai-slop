#[derive(Debug, thiserror::Error)]
#[allow(dead_code)]
pub enum Error {
    #[error("serial port error: {0}")]
    Serial(#[from] tokio_serial::Error),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("timeout waiting for response")]
    Timeout,

    #[error("ELM327 error: {0}")]
    Elm(String),

    #[error("UDS negative response: service 0x{service:02X}, NRC 0x{nrc:02X} ({nrc_name})")]
    UdsNegativeResponse {
        service: u8,
        nrc: u8,
        nrc_name: String,
    },

    #[error("protocol error: {0}")]
    Protocol(String),

    #[error("security access denied")]
    SecurityAccessDenied,

    #[error("config error: {0}")]
    Config(String),

    #[error("hex decode error: {0}")]
    Hex(#[from] hex::FromHexError),

    #[error("device not connected")]
    NotConnected,

    #[error("write verification failed: expected {expected}, got {actual}")]
    WriteVerificationFailed { expected: String, actual: String },

    #[error("rollback failed after verification error: {0}")]
    RollbackFailed(String),

    #[error("DID 0x{did:04X} not in whitelist or not marked writable")]
    DidNotWhitelisted { did: u16 },

    #[error("value out of range for DID 0x{did:04X}: {detail}")]
    ValueOutOfRange { did: u16, detail: String },
}

pub type Result<T> = std::result::Result<T, Error>;

/// Human-readable name for UDS Negative Response Codes
pub fn nrc_name(nrc: u8) -> &'static str {
    match nrc {
        0x10 => "generalReject",
        0x11 => "serviceNotSupported",
        0x12 => "subFunctionNotSupported",
        0x13 => "incorrectMessageLengthOrInvalidFormat",
        0x14 => "responseTooLong",
        0x21 => "busyRepeatRequest",
        0x22 => "conditionsNotCorrect",
        0x24 => "requestSequenceError",
        0x25 => "noResponseFromSubnetComponent",
        0x26 => "failurePreventsExecutionOfRequestedAction",
        0x31 => "requestOutOfRange",
        0x33 => "securityAccessDenied",
        0x35 => "invalidKey",
        0x36 => "exceededNumberOfAttempts",
        0x37 => "requiredTimeDelayNotExpired",
        0x70 => "uploadDownloadNotAccepted",
        0x71 => "transferDataSuspended",
        0x72 => "generalProgrammingFailure",
        0x73 => "wrongBlockSequenceCounter",
        0x78 => "requestCorrectlyReceivedResponsePending",
        0x7E => "subFunctionNotSupportedInActiveSession",
        0x7F => "serviceNotSupportedInActiveSession",
        _ => "unknown",
    }
}
