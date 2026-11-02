fn main() {
    // Every app command is listed here so each window must be granted it explicitly in
    // capabilities/default.json (Tauri 2 otherwise allows all custom commands).
    const COMMANDS: &[&str] = &[
        "library_list_tracks",
        "library_count_tracks",
        "library_search_tracks",
        "library_list_albums",
        "library_list_folders",
        "library_add_folder",
        "library_scan_folder",
        "library_cancel_scan",
        "player_play_tracks",
        "player_play",
        "player_pause",
        "player_toggle",
        "player_stop",
        "player_next",
        "player_previous",
        "player_seek",
        "player_set_volume",
        "player_set_shuffle",
        "player_set_repeat",
        "player_play_item",
        "player_remove_queue_item",
        "player_clear_queue",
        "player_get_state",
    ];
    tauri_build::try_build(
        tauri_build::Attributes::new().app_manifest(tauri_build::AppManifest::new().commands(COMMANDS)),
    )
    .expect("failed to run tauri-build");
}
