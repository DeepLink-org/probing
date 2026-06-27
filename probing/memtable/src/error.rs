//! Unified errors for the memtable crate (L1 — no dependency on probing-core).

use thiserror::Error;

/// Errors from mmap tables, ring buffers, and hash-table operations.
#[derive(Debug, Error)]
pub enum MemtableError {
    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error("invalid memtable buffer: {0}")]
    InvalidBuffer(&'static str),

    #[error("table is not file-backed")]
    NotFileBacked,

    #[error(transparent)]
    Memh(#[from] crate::memh::MemhValidateError),

    #[error(transparent)]
    MemhInit(#[from] crate::memh::MemhInitError),
}

pub type Result<T> = std::result::Result<T, MemtableError>;

impl From<MemtableError> for std::io::Error {
    fn from(e: MemtableError) -> Self {
        std::io::Error::new(std::io::ErrorKind::InvalidData, e)
    }
}
