use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::db::connection::Database;
use crate::error::LibraryResult;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackRow {
    pub id: i64,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration_ms: u64,
    pub rel_path: String,
    pub folder_location: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlbumRow {
    pub id: i64,
    pub title: String,
    pub artist: String,
    pub track_count: u32,
    pub year: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FolderRow {
    pub id: i64,
    pub location: String,
    pub display_name: String,
    pub availability: String,
    pub last_scan_at: Option<i64>,
    pub track_count: u64,
}

pub struct Queries;

fn map_track_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<TrackRow> {
    Ok(TrackRow {
        id: row.get(0)?,
        title: row.get(1)?,
        artist: row.get(2)?,
        album: row.get(3)?,
        duration_ms: row.get::<_, Option<i64>>(4)?.unwrap_or(0).max(0) as u64,
        rel_path: row.get(5)?,
        folder_location: row.get(6)?,
    })
}

impl Queries {
    pub fn list_tracks(db: &Database, limit: usize, offset: usize) -> LibraryResult<Vec<TrackRow>> {
        db.readers.with_connection(|conn| {
            let mut stmt = conn.prepare_cached(
                "SELECT t.id, t.title, COALESCE(t.artist_display, 'Unknown Artist'),
                        COALESCE(al.title, 'Unknown Album'), t.duration_ms, t.rel_path, f.location
                 FROM tracks t
                 JOIN library_folders f ON t.folder_id = f.id
                 LEFT JOIN albums al ON t.album_id = al.id
                 WHERE t.is_visible = 1
                 ORDER BY t.title_sort ASC, t.id ASC
                 LIMIT ?1 OFFSET ?2;"
            )?;

            let rows = stmt.query_map(params![limit as i64, offset as i64], |row| {
                Ok(TrackRow {
                    id: row.get(0)?,
                    title: row.get(1)?,
                    artist: row.get(2)?,
                    album: row.get(3)?,
                    duration_ms: row.get::<_, i64>(4).unwrap_or(0).max(0) as u64,
                    rel_path: row.get(5)?,
                    folder_location: row.get(6)?,
                })
            })?;

            let mut result = Vec::new();
            for r in rows {
                result.push(r?);
            }
            Ok(result)
        })
    }

    pub fn list_albums(db: &Database, limit: usize, offset: usize) -> LibraryResult<Vec<AlbumRow>> {
        db.readers.with_connection(|conn| {
            let mut stmt = conn.prepare_cached(
                "SELECT al.id, al.title, COALESCE(ar.name, 'Unknown Artist'),
                        al.track_count, al.year
                 FROM albums al
                 LEFT JOIN artists ar ON al.album_artist_id = ar.id
                 WHERE al.track_count > 0
                 ORDER BY al.sort_key ASC, al.id ASC
                 LIMIT ?1 OFFSET ?2;"
            )?;

            let rows = stmt.query_map(params![limit as i64, offset as i64], |row| {
                Ok(AlbumRow {
                    id: row.get(0)?,
                    title: row.get(1)?,
                    artist: row.get(2)?,
                    track_count: row.get(3)?,
                    year: row.get(4)?,
                })
            })?;

            let mut result = Vec::new();
            for r in rows {
                result.push(r?);
            }
            Ok(result)
        })
    }

    pub fn search_tracks(db: &Database, search_text: &str, limit: usize) -> LibraryResult<Vec<TrackRow>> {
        let tokens: Vec<String> = search_text
            .split_whitespace()
            .map(|tok| format!("\"{}\"*", tok.replace('"', "")))
            .collect();

        if tokens.is_empty() {
            return Ok(Vec::new());
        }

        let fts_query = tokens.join(" ");

        db.readers.with_connection(|conn| {
            let mut stmt = conn.prepare_cached(
                "SELECT t.id, t.title, COALESCE(t.artist_display, 'Unknown Artist'),
                        COALESCE(al.title, 'Unknown Album'), t.duration_ms, t.rel_path, f.location
                 FROM fts_tracks fts
                 JOIN tracks t ON fts.rowid = t.id
                 JOIN library_folders f ON t.folder_id = f.id
                 LEFT JOIN albums al ON t.album_id = al.id
                 WHERE fts_tracks MATCH ?1 AND t.is_visible = 1
                 ORDER BY bm25(fts_tracks)
                 LIMIT ?2;"
            )?;

            let rows = stmt.query_map(params![fts_query, limit as i64], |row| {
                Ok(TrackRow {
                    id: row.get(0)?,
                    title: row.get(1)?,
                    artist: row.get(2)?,
                    album: row.get(3)?,
                    duration_ms: row.get::<_, i64>(4).unwrap_or(0).max(0) as u64,
                    rel_path: row.get(5)?,
                    folder_location: row.get(6)?,
                })
            })?;

            let mut result = Vec::new();
            for r in rows {
                result.push(r?);
            }
            Ok(result)
        })
    }

    pub fn count_tracks(db: &Database) -> LibraryResult<u64> {
        db.readers.with_connection(|conn| {
            Ok(conn.query_row("SELECT COUNT(*) FROM tracks WHERE is_visible = 1;", [], |r| r.get::<_, i64>(0))?
                .max(0) as u64)
        })
    }

    pub fn list_folders(db: &Database) -> LibraryResult<Vec<FolderRow>> {
        db.readers.with_connection(|conn| {
            let mut stmt = conn.prepare_cached(
                "SELECT f.id, f.location, f.display_name, f.availability, f.last_scan_at,
                        (SELECT COUNT(*) FROM tracks t WHERE t.folder_id = f.id AND t.is_visible = 1)
                 FROM library_folders f WHERE f.is_enabled = 1 ORDER BY f.display_name ASC, f.id ASC;",
            )?;
            let rows = stmt.query_map([], |r| {
                Ok(FolderRow {
                    id: r.get(0)?,
                    location: r.get(1)?,
                    display_name: r.get(2)?,
                    availability: r.get(3)?,
                    last_scan_at: r.get(4)?,
                    track_count: r.get::<_, i64>(5)?.max(0) as u64,
                })
            })?;
            Ok(rows.collect::<Result<Vec<_>, _>>()?)
        })
    }

    /// Fetches tracks by id, in the order requested. Unknown or hidden ids are skipped.
    /// The UI refers to tracks by id only; paths are resolved here, never taken from the frontend.
    pub fn tracks_by_ids(db: &Database, ids: &[i64]) -> LibraryResult<Vec<TrackRow>> {
        db.readers.with_connection(|conn| {
            let mut stmt = conn.prepare_cached(
                "SELECT t.id, t.title, COALESCE(t.artist_display, 'Unknown Artist'),
                        COALESCE(al.title, 'Unknown Album'), t.duration_ms, t.rel_path, f.location
                 FROM tracks t
                 JOIN library_folders f ON t.folder_id = f.id
                 LEFT JOIN albums al ON t.album_id = al.id
                 WHERE t.id = ?1 AND t.is_visible = 1;",
            )?;
            let mut out = Vec::with_capacity(ids.len());
            for id in ids {
                if let Some(row) = stmt.query_row(params![id], map_track_row).optional()? {
                    out.push(row);
                }
            }
            Ok(out)
        })
    }
}
