use rusqlite::{Connection, Result as SqlResult};

pub const SCHEMA_V1: &str = r#"
CREATE TABLE IF NOT EXISTS library_folders (
  id            INTEGER PRIMARY KEY,
  kind          TEXT    NOT NULL DEFAULT 'path' CHECK (kind IN ('path','saf_tree')),
  location      TEXT    NOT NULL,
  display_name  TEXT    NOT NULL,
  is_enabled    INTEGER NOT NULL DEFAULT 1 CHECK (is_enabled IN (0,1)),
  watch         INTEGER NOT NULL DEFAULT 0 CHECK (watch IN (0,1)),
  availability  TEXT    NOT NULL DEFAULT 'unknown' CHECK (availability IN ('available','unavailable','unknown')),
  added_at      INTEGER NOT NULL,
  last_scan_at  INTEGER,
  UNIQUE (kind, location)
);

CREATE TABLE IF NOT EXISTS scan_history (
  id            INTEGER PRIMARY KEY,
  folder_id     INTEGER REFERENCES library_folders(id) ON DELETE CASCADE,
  mode          TEXT NOT NULL CHECK (mode IN ('full','incremental')),
  status        TEXT NOT NULL CHECK (status IN ('running','completed','cancelled','failed','interrupted')),
  started_at    INTEGER NOT NULL,
  finished_at   INTEGER,
  files_seen    INTEGER NOT NULL DEFAULT 0,
  added         INTEGER NOT NULL DEFAULT 0,
  updated       INTEGER NOT NULL DEFAULT 0,
  moved         INTEGER NOT NULL DEFAULT 0,
  missing       INTEGER NOT NULL DEFAULT 0,
  duplicates    INTEGER NOT NULL DEFAULT 0,
  skipped       INTEGER NOT NULL DEFAULT 0,
  errors        INTEGER NOT NULL DEFAULT 0,
  error_summary TEXT
);

CREATE TABLE IF NOT EXISTS artwork (
  id           INTEGER PRIMARY KEY,
  content_hash TEXT NOT NULL UNIQUE,
  source       TEXT NOT NULL CHECK (source IN ('embedded','local_file','online')),
  file_name    TEXT NOT NULL,
  mime_type    TEXT NOT NULL,
  width        INTEGER,
  height       INTEGER,
  byte_size    INTEGER NOT NULL,
  created_at   INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS artists (
  id          INTEGER PRIMARY KEY,
  name        TEXT NOT NULL,
  sort_key    TEXT NOT NULL,
  name_key    TEXT NOT NULL UNIQUE,
  album_count INTEGER NOT NULL DEFAULT 0,
  track_count INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS albums (
  id                INTEGER PRIMARY KEY,
  title             TEXT NOT NULL,
  sort_key          TEXT NOT NULL,
  album_key         TEXT NOT NULL UNIQUE,
  album_artist_id   INTEGER REFERENCES artists(id) ON DELETE SET NULL,
  year              INTEGER,
  mb_release_id     TEXT,
  artwork_id        INTEGER REFERENCES artwork(id) ON DELETE SET NULL,
  track_count       INTEGER NOT NULL DEFAULT 0,
  total_duration_ms INTEGER NOT NULL DEFAULT 0,
  last_added_at     INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS genres (
  id          INTEGER PRIMARY KEY,
  name        TEXT NOT NULL,
  name_key    TEXT NOT NULL UNIQUE,
  track_count INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS tracks (
  id               INTEGER PRIMARY KEY,
  folder_id        INTEGER NOT NULL REFERENCES library_folders(id) ON DELETE CASCADE,
  rel_path         TEXT    NOT NULL,
  file_size        INTEGER NOT NULL,
  mtime_ms         INTEGER NOT NULL,
  fingerprint      TEXT,
  format           TEXT    NOT NULL,
  codec            TEXT,
  title            TEXT    NOT NULL,
  title_sort       TEXT    NOT NULL,
  artist_id        INTEGER REFERENCES artists(id) ON DELETE SET NULL,
  artist_display   TEXT,
  album_id         INTEGER REFERENCES albums(id)  ON DELETE SET NULL,
  track_no         INTEGER, track_total INTEGER,
  disc_no          INTEGER, disc_total  INTEGER,
  year             INTEGER,
  composer         TEXT,
  duration_ms      INTEGER,
  bitrate          INTEGER,
  sample_rate      INTEGER,
  channels         INTEGER,
  bit_depth        INTEGER,
  artwork_id       INTEGER REFERENCES artwork(id) ON DELETE SET NULL,
  status           TEXT    NOT NULL DEFAULT 'ok' CHECK (status IN ('ok','missing','unreadable')),
  missing_since    INTEGER,
  duplicate_of     INTEGER REFERENCES tracks(id) ON DELETE SET NULL,
  is_visible       INTEGER NOT NULL DEFAULT 1 CHECK (is_visible IN (0,1)),
  added_at         INTEGER NOT NULL,
  updated_at       INTEGER NOT NULL,
  last_seen_scan_id INTEGER REFERENCES scan_history(id) ON DELETE SET NULL,
  UNIQUE (folder_id, rel_path)
);

CREATE TABLE IF NOT EXISTS track_genres (
  track_id INTEGER NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
  genre_id INTEGER NOT NULL REFERENCES genres(id) ON DELETE CASCADE,
  PRIMARY KEY (track_id, genre_id)
) WITHOUT ROWID;

CREATE TABLE IF NOT EXISTS playlists (
  id         INTEGER PRIMARY KEY,
  name       TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS playlist_tracks (
  id          INTEGER PRIMARY KEY,
  playlist_id INTEGER NOT NULL REFERENCES playlists(id) ON DELETE CASCADE,
  track_id    INTEGER NOT NULL REFERENCES tracks(id)    ON DELETE CASCADE,
  position    INTEGER NOT NULL,
  added_at    INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS settings (
  key        TEXT PRIMARY KEY,
  value      TEXT NOT NULL,
  updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS playback_state (
  id            INTEGER PRIMARY KEY CHECK (id = 1),
  current_pos   INTEGER,
  position_ms   INTEGER NOT NULL DEFAULT 0,
  volume        REAL    NOT NULL DEFAULT 1.0 CHECK (volume BETWEEN 0.0 AND 1.0),
  shuffle       INTEGER NOT NULL DEFAULT 0 CHECK (shuffle IN (0,1)),
  repeat_mode   TEXT    NOT NULL DEFAULT 'off' CHECK (repeat_mode IN ('off','all','one')),
  was_playing   INTEGER NOT NULL DEFAULT 0 CHECK (was_playing IN (0,1)),
  saved_at      INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS queue_items (
  position   INTEGER PRIMARY KEY,
  track_id   INTEGER NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
  play_order INTEGER NOT NULL
);

-- Partial indexes for browsing
CREATE INDEX IF NOT EXISTS idx_tracks_title    ON tracks(title_sort, id)                  WHERE is_visible = 1;
CREATE INDEX IF NOT EXISTS idx_tracks_added    ON tracks(added_at DESC, id)               WHERE is_visible = 1;
CREATE INDEX IF NOT EXISTS idx_tracks_album    ON tracks(album_id, disc_no, track_no, id) WHERE is_visible = 1;
CREATE INDEX IF NOT EXISTS idx_tracks_artist   ON tracks(artist_id, title_sort)           WHERE is_visible = 1;

-- Scanner indexes
CREATE INDEX IF NOT EXISTS idx_tracks_scan     ON tracks(folder_id, last_seen_scan_id);
CREATE INDEX IF NOT EXISTS idx_tracks_fprint   ON tracks(fingerprint) WHERE fingerprint IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_tracks_status   ON tracks(status, missing_since) WHERE status <> 'ok';

-- Browse indexes
CREATE INDEX IF NOT EXISTS idx_albums_sort     ON albums(sort_key, id);
CREATE INDEX IF NOT EXISTS idx_albums_artist   ON albums(album_artist_id, year);
CREATE INDEX IF NOT EXISTS idx_artists_sort    ON artists(sort_key, id);
CREATE INDEX IF NOT EXISTS idx_playlist_pos    ON playlist_tracks(playlist_id, position);
CREATE INDEX IF NOT EXISTS idx_queue_order     ON queue_items(play_order);

-- FTS5 Full Text Search
CREATE VIRTUAL TABLE IF NOT EXISTS fts_tracks USING fts5(
  title, artist, album,
  tokenize = 'unicode61 remove_diacritics 2',
  prefix = '2 3'
);
"#;

pub fn run_migrations(conn: &mut Connection) -> SqlResult<()> {
    let mut current_version: u32 = conn.query_row("PRAGMA user_version;", [], |row| row.get(0))?;

    if current_version == 0 {
        let tx = conn.transaction()?;
        tx.execute_batch(SCHEMA_V1)?;
        tx.pragma_update(None, "user_version", 1)?;
        tx.commit()?;
        current_version = 1;
    }

    log::info!("Database schema is up to date (version {current_version})");
    Ok(())
}
