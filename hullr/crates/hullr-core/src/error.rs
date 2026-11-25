use serde::{Deserialize, Serialize};

pub type Result<T, E = Error> = std::result::Result<T, E>;

/// One error type crosses every crate boundary and the wire, so it stays
/// simple: a kind plus a human message.
#[derive(Debug, Clone, thiserror::Error, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", content = "message", rename_all = "snake_case")]
pub enum Error {
    #[error("not found: {0}")]
    NotFound(String),
    #[error("already exists: {0}")]
    Conflict(String),
    #[error("invalid input: {0}")]
    Invalid(String),
    #[error("unsupported: {0}")]
    Unsupported(String),
    #[error("registry error: {0}")]
    Registry(String),
    #[error("runtime error: {0}")]
    Runtime(String),
    #[error("build failed: {0}")]
    Build(String),
    #[error("io error: {0}")]
    Io(String),
    #[error("{0}")]
    Other(String),
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        if e.kind() == std::io::ErrorKind::NotFound {
            Error::NotFound(e.to_string())
        } else {
            Error::Io(e.to_string())
        }
    }
}

impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Error::Invalid(format!("json: {e}"))
    }
}

/// Attach context to any error, keeping its kind.
pub trait Context<T> {
    fn context(self, msg: impl std::fmt::Display) -> Result<T>;
}

impl<T, E: Into<Error>> Context<T> for std::result::Result<T, E> {
    fn context(self, msg: impl std::fmt::Display) -> Result<T> {
        self.map_err(|e| {
            let e: Error = e.into();
            let wrap = |m: String| format!("{msg}: {m}");
            match e {
                Error::NotFound(m) => Error::NotFound(wrap(m)),
                Error::Conflict(m) => Error::Conflict(wrap(m)),
                Error::Invalid(m) => Error::Invalid(wrap(m)),
                Error::Unsupported(m) => Error::Unsupported(wrap(m)),
                Error::Registry(m) => Error::Registry(wrap(m)),
                Error::Runtime(m) => Error::Runtime(wrap(m)),
                Error::Build(m) => Error::Build(wrap(m)),
                Error::Io(m) => Error::Io(wrap(m)),
                Error::Other(m) => Error::Other(wrap(m)),
            }
        })
    }
}
