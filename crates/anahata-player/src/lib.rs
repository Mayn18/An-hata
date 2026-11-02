//! Anāhata Player: Playback Manager, Queue, and State Machine

pub mod backend;
pub mod manager;
pub mod queue;
pub mod types;

pub use backend::AudioBackend;
pub use manager::{PlaybackManager, Player, PlayerObserver};
pub use queue::{Queue, RemoveOutcome};
pub use types::{
    PlaybackPosition, PlaybackStatus, PlayerCommand, PlayerEvent, PlayerState, QueueItem, RepeatMode,
};
