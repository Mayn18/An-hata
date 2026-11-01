use std::path::PathBuf;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AudioError {
    #[error("File not found: {0}")]
    FileNotFound(PathBuf),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Unsupported audio format or codec: {0}")]
    UnsupportedFormat(String),

    #[error("Decoding failed: {0}")]
    DecodeFailed(String),

    #[error("Audio device unavailable: {0}")]
    DeviceUnavailable(String),

    #[error("Audio stream error: {0}")]
    StreamError(String),

    #[error("Seeking error: {0}")]
    SeekError(String),

    #[error("Audio engine channel closed")]
    ChannelClosed,

    #[error("Internal audio engine error: {0}")]
    Internal(String),
}

pub type AudioResult<T> = Result<T, AudioError>;
