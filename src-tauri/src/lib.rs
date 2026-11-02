//! Anāhata app shell: wires the library, player and audio crates to the React UI.
//!
//! The UI never passes file paths. Folders are chosen with a native dialog opened from Rust,
//! scanning is by folder id, and playback is requested by track id; paths are resolved here
//! from the database.

mod bridge;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use anahata_library::{
    AlbumRow, Database, FolderRow, Queries, ScanOptions, ScanSummary, Scanner, TrackRow,
};
use anahata_player::{
    PlaybackManager, PlayerCommand, PlayerEvent, PlayerObserver, PlayerState, RepeatMode,
};
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_dialog::DialogExt;

use bridge::{clamp_limit, queue_from_rows};

/// Forwards player events to the webview.
struct TauriObserver {
    app: AppHandle,
}

impl PlayerObserver for TauriObserver {
    fn on_event(&self, event: &PlayerEvent) {
        let result = match event {
            PlayerEvent::StateChanged(state) => self.app.emit("player://state", state),
            PlayerEvent::TrackChanged(item) => self.app.emit("player://track", item),
            PlayerEvent::QueueChanged { revision, length } => self
                .app
                .emit("player://queue", serde_json::json!({ "revision": revision, "length": length })),
            PlayerEvent::PositionAnchor(pos) => self.app.emit("player://position", pos),
            PlayerEvent::Error(message) => self.app.emit("player://error", message),
        };
        if let Err(e) = result {
            log::warn!("Failed to emit player event: {e}");
        }
    }
}

pub struct AppState {
    player: PlaybackManager,
    db: Arc<Database>,
    /// Cancel flag of the scan in progress, if any. Only one scan runs at a time.
    active_scan: Mutex<Option<Arc<AtomicBool>>>,
}

type CmdResult<T> = Result<T, String>;

fn err<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

// ---------------------------------------------------------------- library

#[tauri::command]
fn library_list_tracks(state: State<'_, AppState>, limit: usize, offset: usize) -> CmdResult<Vec<TrackRow>> {
    Queries::list_tracks(&state.db, clamp_limit(limit), offset).map_err(err)
}

#[tauri::command]
fn library_count_tracks(state: State<'_, AppState>) -> CmdResult<u64> {
    Queries::count_tracks(&state.db).map_err(err)
}

#[tauri::command]
fn library_search_tracks(state: State<'_, AppState>, query: String, limit: usize) -> CmdResult<Vec<TrackRow>> {
    Queries::search_tracks(&state.db, &query, clamp_limit(limit)).map_err(err)
}

#[tauri::command]
fn library_list_albums(state: State<'_, AppState>, limit: usize, offset: usize) -> CmdResult<Vec<AlbumRow>> {
    Queries::list_albums(&state.db, clamp_limit(limit), offset).map_err(err)
}

#[tauri::command]
fn library_list_folders(state: State<'_, AppState>) -> CmdResult<Vec<FolderRow>> {
    Queries::list_folders(&state.db).map_err(err)
}

/// Opens the native folder picker and registers the chosen folder. Returns `None` if cancelled.
#[tauri::command]
async fn library_add_folder(app: AppHandle, state: State<'_, AppState>) -> CmdResult<Option<FolderRow>> {
    let Some(picked) = app.dialog().file().blocking_pick_folder() else {
        return Ok(None);
    };
    let path = picked.into_path().map_err(err)?;
    let id = Scanner::add_folder(&state.db, &path).map_err(err)?;
    let folders = Queries::list_folders(&state.db).map_err(err)?;
    Ok(folders.into_iter().find(|f| f.id == id))
}

/// Scans a registered folder. Emits `library://scan-progress` while running.
#[tauri::command]
async fn library_scan_folder(app: AppHandle, state: State<'_, AppState>, folder_id: i64) -> CmdResult<ScanSummary> {
    let cancel = Arc::new(AtomicBool::new(false));
    {
        let mut slot = state.active_scan.lock().map_err(err)?;
        if slot.is_some() {
            return Err("A scan is already running".to_string());
        }
        *slot = Some(Arc::clone(&cancel));
    }

    let db = Arc::clone(&state.db);
    let progress_app = app.clone();
    let opts = ScanOptions {
        cancel: Some(cancel),
        on_progress: Some(Arc::new(move |p| {
            let _ = progress_app.emit("library://scan-progress", p);
        })),
    };

    let result = tauri::async_runtime::spawn_blocking(move || Scanner::scan_folder(&db, folder_id, &opts)).await;

    if let Ok(mut slot) = state.active_scan.lock() {
        *slot = None;
    }
    let summary = result.map_err(err)?.map_err(err)?;
    let _ = app.emit("library://scan-finished", &summary);
    Ok(summary)
}

#[tauri::command]
fn library_cancel_scan(state: State<'_, AppState>) -> CmdResult<bool> {
    let slot = state.active_scan.lock().map_err(err)?;
    match slot.as_ref() {
        Some(flag) => {
            flag.store(true, Ordering::Relaxed);
            Ok(true)
        }
        None => Ok(false),
    }
}

// ----------------------------------------------------------------- player

/// Replaces the queue with the given tracks (by id) and starts `start_track_id`.
#[tauri::command]
fn player_play_tracks(state: State<'_, AppState>, track_ids: Vec<i64>, start_track_id: Option<i64>) -> CmdResult<()> {
    if track_ids.len() > 10_000 {
        return Err("Too many tracks requested".to_string());
    }
    let rows = Queries::tracks_by_ids(&state.db, &track_ids).map_err(err)?;
    if rows.is_empty() {
        return Err("None of the requested tracks exist in the library".to_string());
    }
    let (items, start_index) = queue_from_rows(rows, start_track_id);
    state.player.send(PlayerCommand::LoadQueue { items, start_index });
    Ok(())
}

#[tauri::command]
fn player_play(state: State<'_, AppState>) {
    state.player.send(PlayerCommand::Play);
}

#[tauri::command]
fn player_pause(state: State<'_, AppState>) {
    state.player.send(PlayerCommand::Pause);
}

#[tauri::command]
fn player_toggle(state: State<'_, AppState>) {
    state.player.send(PlayerCommand::Toggle);
}

#[tauri::command]
fn player_stop(state: State<'_, AppState>) {
    state.player.send(PlayerCommand::Stop);
}

#[tauri::command]
fn player_next(state: State<'_, AppState>) {
    state.player.send(PlayerCommand::Next);
}

#[tauri::command]
fn player_previous(state: State<'_, AppState>) {
    state.player.send(PlayerCommand::Previous);
}

#[tauri::command]
fn player_seek(state: State<'_, AppState>, position_ms: u64) {
    state.player.send(PlayerCommand::Seek(position_ms));
}

#[tauri::command]
fn player_set_volume(state: State<'_, AppState>, volume: f32) {
    state.player.send(PlayerCommand::SetVolume(volume));
}

#[tauri::command]
fn player_set_shuffle(state: State<'_, AppState>, enabled: bool) {
    state.player.send(PlayerCommand::SetShuffle(enabled));
}

#[tauri::command]
fn player_set_repeat(state: State<'_, AppState>, mode: RepeatMode) {
    state.player.send(PlayerCommand::SetRepeat(mode));
}

#[tauri::command]
fn player_play_item(state: State<'_, AppState>, queue_item_id: u64) {
    state.player.send(PlayerCommand::PlayItem(queue_item_id));
}

#[tauri::command]
fn player_remove_queue_item(state: State<'_, AppState>, queue_item_id: u64) {
    state.player.send(PlayerCommand::RemoveQueueItem(queue_item_id));
}

#[tauri::command]
fn player_clear_queue(state: State<'_, AppState>) {
    state.player.send(PlayerCommand::ClearQueue);
}

#[tauri::command]
fn player_get_state(state: State<'_, AppState>) -> PlayerState {
    state.player.state()
}

// -------------------------------------------------------------------- app

pub fn run() {
    let _ = env_logger::try_init();

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            let db = Arc::new(Database::open(data_dir.join("library.db"))?);
            let player = PlaybackManager::start(vec![Arc::new(TauriObserver { app: app.handle().clone() })]);
            app.manage(AppState { player, db, active_scan: Mutex::new(None) });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            library_list_tracks,
            library_count_tracks,
            library_search_tracks,
            library_list_albums,
            library_list_folders,
            library_add_folder,
            library_scan_folder,
            library_cancel_scan,
            player_play_tracks,
            player_play,
            player_pause,
            player_toggle,
            player_stop,
            player_next,
            player_previous,
            player_seek,
            player_set_volume,
            player_set_shuffle,
            player_set_repeat,
            player_play_item,
            player_remove_queue_item,
            player_clear_queue,
            player_get_state,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Anāhata");
}
