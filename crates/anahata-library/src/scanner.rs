use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::time::SystemTime;

use rusqlite::params;
use walkdir::WalkDir;

use crate::db::connection::Database;
use crate::error::{LibraryError, LibraryResult};
use crate::metadata::extract_metadata;

#[derive(Debug, Default, Clone)]
pub struct ScanSummary {
    pub files_seen: usize,
    pub added: usize,
    pub updated: usize,
    pub moved: usize,
    pub missing: usize,
    pub duplicates: usize,
    pub skipped: usize,
    pub errors: usize,
}

/// Computes a fast content fingerprint (hash of size + first 64KB + last 64KB)
pub fn compute_fingerprint<P: AsRef<Path>>(path: P) -> std::io::Result<String> {
    let mut file = File::open(path)?;
    let metadata = file.metadata()?;
    let size = metadata.len();

    let mut hasher = crc32fast::Hasher::new();
    hasher.update(&size.to_le_bytes());

    let mut buffer = [0u8; 64 * 1024];

    // First 64KB
    let bytes_read = file.read(&mut buffer)?;
    hasher.update(&buffer[..bytes_read]);

    // Last 64KB
    if size > 64 * 1024 {
        let seek_pos = size.saturating_sub(64 * 1024);
        file.seek(SeekFrom::Start(seek_pos))?;
        let bytes_read = file.read(&mut buffer)?;
        hasher.update(&buffer[..bytes_read]);
    }

    Ok(format!("{:08x}", hasher.finalize()))
}

pub struct Scanner;

impl Scanner {
    pub fn scan_directory<P: AsRef<Path>>(
        db: &Database,
        folder_path: P,
    ) -> LibraryResult<ScanSummary> {
        let root = folder_path.as_ref();
        if !root.exists() {
            return Err(LibraryError::FileUnavailable(root.to_path_buf()));
        }

        let mut summary = ScanSummary::default();

        let supported_extensions = ["mp3", "flac", "wav", "m4a", "ogg", "opus", "aiff"];

        // 1. Register or find folder_id in DB
        let folder_location = root.to_string_lossy().to_string();
        let display_name = root
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("Music")
            .to_string();

        let folder_id: i64 = {
            let loc = folder_location.clone();
            let dname = display_name.clone();
            let now = SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_millis() as i64;

            db.writer.execute(move |conn| {
                conn.execute(
                    "INSERT INTO library_folders (kind, location, display_name, added_at, availability)
                     VALUES ('path', ?1, ?2, ?3, 'available')
                     ON CONFLICT(kind, location) DO UPDATE SET availability = 'available';",
                    params![loc, dname, now],
                )?;
                Ok(())
            })?;

            let loc2 = folder_location.clone();
            db.readers.with_connection(|conn| {
                let id: i64 = conn.query_row(
                    "SELECT id FROM library_folders WHERE kind = 'path' AND location = ?1;",
                    params![loc2],
                    |r| r.get(0),
                )?;
                Ok(id)
            })?
        };

        // 2. Walk directory and parse audio files
        for entry in WalkDir::new(root).follow_links(false).into_iter().filter_map(|e| e.ok()) {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }

            let ext = match path.extension().and_then(|s| s.to_str()) {
                Some(e) => e.to_lowercase(),
                None => continue,
            };

            if !supported_extensions.contains(&ext.as_str()) {
                continue;
            }

            summary.files_seen += 1;

            let rel_path = match path.strip_prefix(root) {
                Ok(p) => p.to_string_lossy().to_string(),
                Err(_) => continue,
            };

            let file_meta = match path.metadata() {
                Ok(m) => m,
                Err(_) => {
                    summary.errors += 1;
                    continue;
                }
            };

            let file_size = file_meta.len() as i64;
            let mtime_ms = file_meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as i64)
                .unwrap_or(0);

            let fingerprint = compute_fingerprint(path).ok();
            let meta = match extract_metadata(path) {
                Ok(m) => m,
                Err(e) => {
                    log::warn!("Metadata extraction failed for {}: {e}", path.display());
                    summary.errors += 1;
                    continue;
                }
            };

            // Insert or update into tracks table
            let now = SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_millis() as i64;

            let artist_name = meta.artist.unwrap_or_else(|| "Unknown Artist".to_string());
            let album_name = meta.album.unwrap_or_else(|| "Unknown Album".to_string());
            let title = meta.title;

            db.writer.execute(move |conn| {
                let tx = conn.transaction()?;

                // Artist
                tx.execute(
                    "INSERT INTO artists (name, sort_key, name_key)
                     VALUES (?1, lower(?1), lower(?1))
                     ON CONFLICT(name_key) DO NOTHING;",
                    params![artist_name],
                )?;
                let artist_id: i64 = tx.query_row(
                    "SELECT id FROM artists WHERE name_key = lower(?1);",
                    params![artist_name],
                    |r| r.get(0),
                )?;

                // Album
                tx.execute(
                    "INSERT INTO albums (title, sort_key, album_key, album_artist_id, last_added_at)
                     VALUES (?1, lower(?1), lower(?1) || '|' || lower(?2), ?3, ?4)
                     ON CONFLICT(album_key) DO NOTHING;",
                    params![album_name, artist_name, artist_id, now],
                )?;
                let album_id: i64 = tx.query_row(
                    "SELECT id FROM albums WHERE album_key = lower(?1) || '|' || lower(?2);",
                    params![album_name, artist_name],
                    |r| r.get(0),
                )?;

                // Track
                tx.execute(
                    "INSERT INTO tracks (
                        folder_id, rel_path, file_size, mtime_ms, fingerprint, format,
                        title, title_sort, artist_id, artist_display, album_id, duration_ms,
                        sample_rate, channels, bitrate, added_at, updated_at
                     ) VALUES (
                        ?1, ?2, ?3, ?4, ?5, ?6, ?7, lower(?7), ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?15
                     )
                     ON CONFLICT(folder_id, rel_path) DO UPDATE SET
                        file_size = excluded.file_size,
                        mtime_ms = excluded.mtime_ms,
                        fingerprint = excluded.fingerprint,
                        title = excluded.title,
                        updated_at = excluded.updated_at;",
                    params![
                        folder_id, rel_path, file_size, mtime_ms, fingerprint, ext,
                        title, artist_id, artist_name, album_id, meta.duration_ms as i64,
                        meta.sample_rate.map(|r| r as i64), meta.channels.map(|c| c as i64),
                        meta.bitrate.map(|b| b as i64), now
                    ],
                )?;

                let track_id: i64 = tx.query_row(
                    "SELECT id FROM tracks WHERE folder_id = ?1 AND rel_path = ?2;",
                    params![folder_id, rel_path],
                    |r| r.get(0),
                )?;

                // Index in FTS5 explicitly using track_id as rowid
                let _ = tx.execute(
                    "INSERT OR REPLACE INTO fts_tracks (rowid, title, artist, album) VALUES (?1, ?2, ?3, ?4);",
                    params![track_id, title, artist_name, album_name],
                );

                tx.commit()?;
                Ok(())
            })?;

            summary.added += 1;
        }

        Ok(summary)
    }
}
