//! Pure conversion helpers between the library and the player. No Tauri types here, so
//! they are unit-tested without a window.

use std::path::PathBuf;

use anahata_library::TrackRow;
use anahata_player::QueueItem;

/// Largest page the UI may request. Keeps a buggy or hostile frontend from asking for the whole library.
pub const MAX_PAGE: usize = 500;

pub fn clamp_limit(limit: usize) -> usize {
    limit.clamp(1, MAX_PAGE)
}

/// Builds queue items from library rows. Paths are resolved here from the database;
/// the frontend only ever names tracks by id.
///
/// `start_track_id` selects which row starts playing; if it is absent from `rows`, the first row starts.
pub fn queue_from_rows(rows: Vec<TrackRow>, start_track_id: Option<i64>) -> (Vec<QueueItem>, Option<usize>) {
    let start = start_track_id.and_then(|id| rows.iter().position(|r| r.id == id));
    let items = rows
        .into_iter()
        .map(|r| QueueItem {
            id: 0, // assigned by the queue
            track_id: r.id,
            path: PathBuf::from(&r.folder_location).join(&r.rel_path),
            title: r.title,
            artist: r.artist,
        })
        .collect();
    (items, start)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: i64, title: &str) -> TrackRow {
        TrackRow {
            id,
            title: title.into(),
            artist: "A".into(),
            album: "B".into(),
            duration_ms: 1000,
            rel_path: format!("sub/{title}.flac"),
            folder_location: "/music".into(),
        }
    }

    #[test]
    fn paths_come_from_the_database_rows() {
        let (items, _) = queue_from_rows(vec![row(7, "x")], None);
        assert_eq!(items[0].path, PathBuf::from("/music/sub/x.flac"));
        assert_eq!(items[0].track_id, 7);
        assert_eq!(items[0].id, 0);
    }

    #[test]
    fn start_index_follows_the_requested_track() {
        let rows = vec![row(1, "a"), row(2, "b"), row(3, "c")];
        assert_eq!(queue_from_rows(rows.clone(), Some(3)).1, Some(2));
        assert_eq!(queue_from_rows(rows.clone(), Some(99)).1, None);
        assert_eq!(queue_from_rows(rows, None).1, None);
    }

    #[test]
    fn page_size_is_bounded() {
        assert_eq!(clamp_limit(0), 1);
        assert_eq!(clamp_limit(50), 50);
        assert_eq!(clamp_limit(usize::MAX), MAX_PAGE);
    }
}
