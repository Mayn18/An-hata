use std::path::PathBuf;
use std::sync::Arc;
use serde::{Deserialize, Serialize};

use anahata_library::{Database, Queries, Scanner, TrackRow};
use anahata_player::{PlaybackManager, PlayerCommand, PlayerEvent, PlayerObserver, QueueItem};

struct AppObserver;

impl PlayerObserver for AppObserver {
    fn on_event(&self, event: &PlayerEvent) {
        log::info!("Player event: {:?}", event);
    }
}

pub struct AppState {
    pub player: Arc<PlaybackManager>,
    pub db: Arc<Database>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ScanResult {
    pub files_seen: usize,
    pub added: usize,
}

pub fn create_app_state(db_path: PathBuf) -> AppState {
    let db = Arc::new(Database::open(db_path).expect("Failed to open database"));
    let observer = Arc::new(AppObserver);
    let player = Arc::new(PlaybackManager::start(vec![observer]));

    AppState { player, db }
}

pub fn play_track(state: &AppState, track: TrackRow) {
    let item = QueueItem {
        id: track.id as u64,
        track_id: track.id,
        path: PathBuf::from(&track.folder_location).join(&track.rel_path),
        title: track.title,
        artist: track.artist,
    };
    state.player.send(PlayerCommand::LoadQueue {
        items: vec![item],
        start_index: Some(0),
    });
}

pub fn toggle_playback(state: &AppState) {
    state.player.send(PlayerCommand::Toggle);
}

pub fn next_track(state: &AppState) {
    state.player.send(PlayerCommand::Next);
}

pub fn previous_track(state: &AppState) {
    state.player.send(PlayerCommand::Previous);
}

pub fn seek_position(state: &AppState, pos_ms: u64) {
    state.player.send(PlayerCommand::Seek(pos_ms));
}

pub fn list_tracks(state: &AppState, limit: usize, offset: usize) -> Result<Vec<TrackRow>, String> {
    Queries::list_tracks(&state.db, limit, offset).map_err(|e| e.to_string())
}

pub fn search_tracks(state: &AppState, query: &str, limit: usize) -> Result<Vec<TrackRow>, String> {
    Queries::search_tracks(&state.db, query, limit).map_err(|e| e.to_string())
}

pub fn scan_directory(state: &AppState, path: &str) -> Result<ScanResult, String> {
    let summary = Scanner::scan_directory(&state.db, path).map_err(|e| e.to_string())?;
    Ok(ScanResult {
        files_seen: summary.files_seen,
        added: summary.added,
    })
}
