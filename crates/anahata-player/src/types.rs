use std::path::PathBuf;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PlaybackStatus {
    Stopped,
    Loading,
    Playing,
    Paused,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RepeatMode {
    Off,
    All,
    One,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueueItem {
    pub id: u64,
    pub track_id: i64,
    pub path: PathBuf,
    pub title: String,
    pub artist: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaybackPosition {
    pub position_ms: u64,
    pub duration_ms: Option<u64>,
    pub playing: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlayerState {
    pub status: PlaybackStatus,
    pub queue_revision: u64,
    pub current_queue_index: Option<usize>,
    pub current_item: Option<QueueItem>,
    pub position: PlaybackPosition,
    pub volume: f32,
    pub shuffle: bool,
    pub repeat: RepeatMode,
    pub error: Option<String>,
}

#[derive(Debug, Clone)]
pub enum PlayerCommand {
    Play,
    Pause,
    Toggle,
    Stop,
    Next,
    Previous,
    Seek(u64), // position in milliseconds
    SetVolume(f32),
    SetShuffle(bool),
    SetRepeat(RepeatMode),
    PlayItem(u64), // queue item id
    LoadQueue { items: Vec<QueueItem>, start_index: Option<usize> },
    ClearQueue,
    RemoveQueueItem(u64),
    Shutdown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PlayerEvent {
    StateChanged(PlayerState),
    TrackChanged(Option<QueueItem>),
    QueueChanged { revision: u64, length: usize },
    PositionAnchor(PlaybackPosition),
    Error(String),
}
