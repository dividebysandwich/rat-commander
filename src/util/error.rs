//! Crate-wide error type and `Result` alias.

use std::io;

/// The crate-wide error type. Backends and subsystems convert their own errors
/// into this so the UI layer has a single thing to render.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),

    #[error("operation not supported by this filesystem")]
    Unsupported,

    #[error("not found: {0}")]
    NotFound(String),

    #[error("invalid path: {0}")]
    InvalidPath(String),

    #[error("operation cancelled")]
    Cancelled,

    #[error("{0}")]
    Other(String),
}

impl Error {
    pub fn other(msg: impl Into<String>) -> Self {
        Error::Other(msg.into())
    }
}

pub type Result<T> = std::result::Result<T, Error>;
