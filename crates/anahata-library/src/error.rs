use std::path::PathBuf;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum LibraryError {
    #[error("Database error: {0}")]
    Database(#[from] rusqlite::Error),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Metadata parse error: {0}")]
    Metadata(String),

    #[error("File unavailable: {0}")]
    FileUnavailable(PathBuf),

    #[error("Internal library error: {0}")]
    Internal(String),
}

pub type LibraryResult<T> = Result<T, LibraryError>;
