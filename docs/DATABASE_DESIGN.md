# DATABASE_DESIGN

Status: **Stage 3 draft** (2026-11-01). Labels: [Verified] [Limitation] [Assumption] [Unverified] [Recommendation] as in `ARCHITECTURE.md`.

---

## 1. Conventions

- Timestamps: `INTEGER` Unix epoch **milliseconds, UTC**. Booleans: `INTEGER` 0/1 with `CHECK`. Enums: `TEXT` with `CHECK (… IN (…))`.
- IDs: `INTEGER PRIMARY KEY` (SQLite rowid alias). Track ids are **stable**: a moved/renamed file keeps its id, so playlists and queue survive.
- `*_key` columns are normalised strings computed in Rust (Unicode lowercase, trim, collapse whitespace; **[Unverified]** whether NFKC normalisation via a crate is justified — decide in M4). They give case-insensitive identity and fast sort without SQL collations.
- Paths: a track's location is `(folder_id, rel_path)`. Never store absolute paths, so a folder can move (e.g. a drive mounted elsewhere) by updating one row. **[Limitation]** files whose path is not valid UTF-8 are skipped and counted in the scan report (v1).

## 2. Connection and concurrency strategy

- Engine: SQLite via `rusqlite` with the `bundled` feature on **all** platforms (own SQLite, FTS5 known-enabled; no dependence on the OS SQLite, which matters on Android). **[Verified]** `libsqlite3-sys` bundled builds compile with FTS5 enabled (build script flag seen in source; also reported working with rusqlite 0.40). **[Recommendation]** a CI test creates an FTS5 table on every target to prove it per build.
- **One writer thread** owns the only write connection. All writes are jobs sent over a bounded channel; scanner batches (~500 tracks) become one transaction.
- **Reader pool: 4 read-only connections** (small hand-rolled pool: `Mutex<Vec<Connection>>` + `Condvar`; no extra dependency). Used from `spawn_blocking`.
- PRAGMAs on every connection: `foreign_keys=ON`, `busy_timeout=5000`, `temp_store=MEMORY`. On the writer: `journal_mode=WAL` (persistent), `synchronous=NORMAL`, `wal_autocheckpoint=1000`. Readers: `query_only=ON`.
- Prepared statements via rusqlite's statement cache (`prepare_cached`).
- WAL lets the UI read while a scan writes. **[Limitation]** WAL needs a local filesystem; the DB lives in the app data dir, never on a network/removable drive.

## 3. Schema (migration `0001_init.sql`)

```sql
CREATE TABLE library_folders (
  id            INTEGER PRIMARY KEY,
  kind          TEXT    NOT NULL DEFAULT 'path' CHECK (kind IN ('path','saf_tree')),
  location      TEXT    NOT NULL,            -- desktop: absolute path; Android: tree URI (kind='saf_tree')
  display_name  TEXT    NOT NULL,
  is_enabled    INTEGER NOT NULL DEFAULT 1 CHECK (is_enabled IN (0,1)),
  watch         INTEGER NOT NULL DEFAULT 0 CHECK (watch IN (0,1)),   -- folder monitoring (desktop only)
  availability  TEXT    NOT NULL DEFAULT 'unknown' CHECK (availability IN ('available','unavailable','unknown')),
  added_at      INTEGER NOT NULL,
  last_scan_at  INTEGER,
  UNIQUE (kind, location)
);

CREATE TABLE scan_history (
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
  skipped       INTEGER NOT NULL DEFAULT 0,   -- unsupported/non-UTF-8 path/etc.
  errors        INTEGER NOT NULL DEFAULT 0,
  error_summary TEXT                          -- first N error messages, length-limited
);

CREATE TABLE artwork (
  id           INTEGER PRIMARY KEY,
  content_hash TEXT NOT NULL UNIQUE,          -- hash of image bytes; dedupes identical covers
  source       TEXT NOT NULL CHECK (source IN ('embedded','local_file','online')),
  file_name    TEXT NOT NULL,                 -- relative to the artwork cache dir
  mime_type    TEXT NOT NULL,
  width        INTEGER,
  height       INTEGER,
  byte_size    INTEGER NOT NULL,
  created_at   INTEGER NOT NULL
);

CREATE TABLE artists (
  id          INTEGER PRIMARY KEY,
  name        TEXT NOT NULL,
  sort_key    TEXT NOT NULL,
  name_key    TEXT NOT NULL UNIQUE,
  album_count INTEGER NOT NULL DEFAULT 0,     -- maintained by scanner (§7)
  track_count INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE albums (
  id                INTEGER PRIMARY KEY,
  title             TEXT NOT NULL,
  sort_key          TEXT NOT NULL,
  album_key         TEXT NOT NULL UNIQUE,     -- identity: mb_release_id if present, else title_key|album_artist_key
  album_artist_id   INTEGER REFERENCES artists(id) ON DELETE SET NULL,
  year              INTEGER,
  mb_release_id     TEXT,
  artwork_id        INTEGER REFERENCES artwork(id) ON DELETE SET NULL,
  track_count       INTEGER NOT NULL DEFAULT 0,
  total_duration_ms INTEGER NOT NULL DEFAULT 0,
  last_added_at     INTEGER NOT NULL
);

CREATE TABLE genres (
  id          INTEGER PRIMARY KEY,
  name        TEXT NOT NULL,
  name_key    TEXT NOT NULL UNIQUE,
  track_count INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE tracks (
  id               INTEGER PRIMARY KEY,
  folder_id        INTEGER NOT NULL REFERENCES library_folders(id) ON DELETE CASCADE,
  rel_path         TEXT    NOT NULL,
  file_size        INTEGER NOT NULL,
  mtime_ms         INTEGER NOT NULL,
  fingerprint      TEXT,                      -- see §6
  format           TEXT    NOT NULL,          -- 'mp3','flac','wav','m4a','ogg','opus','aiff', ...
  codec            TEXT,                      -- 'aac','alac','vorbis','opus','pcm', ...
  title            TEXT    NOT NULL,          -- fallback: filename without extension
  title_sort       TEXT    NOT NULL,
  artist_id        INTEGER REFERENCES artists(id) ON DELETE SET NULL,
  artist_display   TEXT,                      -- raw tag text, e.g. "A feat. B"
  album_id         INTEGER REFERENCES albums(id)  ON DELETE SET NULL,   -- NULL = "Unknown Album"
  track_no         INTEGER, track_total INTEGER,
  disc_no          INTEGER, disc_total  INTEGER,
  year             INTEGER,
  composer         TEXT,
  duration_ms      INTEGER,
  bitrate          INTEGER,                   -- bits per second
  sample_rate      INTEGER,
  channels         INTEGER,
  bit_depth        INTEGER,
  artwork_id       INTEGER REFERENCES artwork(id) ON DELETE SET NULL,   -- this file's embedded art
  status           TEXT    NOT NULL DEFAULT 'ok' CHECK (status IN ('ok','missing','unreadable')),
  missing_since    INTEGER,
  duplicate_of     INTEGER REFERENCES tracks(id) ON DELETE SET NULL,
  is_visible       INTEGER NOT NULL DEFAULT 1 CHECK (is_visible IN (0,1)),  -- 1 iff status='ok' AND duplicate_of IS NULL
  added_at         INTEGER NOT NULL,
  updated_at       INTEGER NOT NULL,
  last_seen_scan_id INTEGER REFERENCES scan_history(id) ON DELETE SET NULL,
  UNIQUE (folder_id, rel_path)
);

CREATE TABLE track_genres (
  track_id INTEGER NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
  genre_id INTEGER NOT NULL REFERENCES genres(id) ON DELETE CASCADE,
  PRIMARY KEY (track_id, genre_id)
) WITHOUT ROWID;

CREATE TABLE playlists (
  id         INTEGER PRIMARY KEY,
  name       TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);

CREATE TABLE playlist_tracks (
  id          INTEGER PRIMARY KEY,           -- row id: the same track may appear twice
  playlist_id INTEGER NOT NULL REFERENCES playlists(id) ON DELETE CASCADE,
  track_id    INTEGER NOT NULL REFERENCES tracks(id)    ON DELETE CASCADE,
  position    INTEGER NOT NULL,              -- dense 0..n-1; renumbered in a transaction on move/remove
  added_at    INTEGER NOT NULL
);

CREATE TABLE artwork_lookup (                -- online lookups: prevents repeated requests
  album_id        INTEGER PRIMARY KEY REFERENCES albums(id) ON DELETE CASCADE,
  status          TEXT NOT NULL CHECK (status IN ('pending','found','not_found','error')),
  attempts        INTEGER NOT NULL DEFAULT 0,
  last_attempt_at INTEGER,
  retry_after     INTEGER,                   -- backoff for 'error'; long delay for 'not_found'
  mb_release_id   TEXT
);

CREATE TABLE settings (
  key        TEXT PRIMARY KEY,
  value      TEXT NOT NULL,                   -- JSON-encoded scalar
  updated_at INTEGER NOT NULL
);

CREATE TABLE playback_state (                -- single row
  id            INTEGER PRIMARY KEY CHECK (id = 1),
  current_pos   INTEGER,                     -- index into queue_items.position
  position_ms   INTEGER NOT NULL DEFAULT 0,
  volume        REAL    NOT NULL DEFAULT 1.0 CHECK (volume BETWEEN 0.0 AND 1.0),
  shuffle       INTEGER NOT NULL DEFAULT 0 CHECK (shuffle IN (0,1)),
  repeat_mode   TEXT    NOT NULL DEFAULT 'off' CHECK (repeat_mode IN ('off','all','one')),
  was_playing   INTEGER NOT NULL DEFAULT 0 CHECK (was_playing IN (0,1)),
  saved_at      INTEGER NOT NULL
);

CREATE TABLE queue_items (
  position   INTEGER PRIMARY KEY,            -- queue order
  track_id   INTEGER NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
  play_order INTEGER NOT NULL                -- shuffle permutation (= position when shuffle off)
);
```

`queue_item_id` (the unique id the UI uses) is an in-memory counter, not persisted; it is reissued on restore.

### Settings keys (typed in Rust, defaults in code; unknown keys rejected)

| Key | Type | Default |
|---|---|---|
| `appearance.theme` | `light`/`dark`/`system` | `system` |
| `library.monitor_folders` | bool | false (desktop only; ignored on Android) |
| `library.rescan_on_launch` | bool | true |
| `playback.default_volume` | 0.0–1.0 | 0.8 (used on first run) |
| `playback.crossfade_enabled` | bool | **false** (required) |
| `playback.crossfade_seconds` | 1–12 | 6 |
| `playback.restore_queue` | bool | true |
| `playback.autoplay_on_launch` | bool | **false** (required: never surprise-play) |
| `artwork.online_lookup` | bool | **false** — owner decision pending (see log) |

"Shuffle behavior" (named in the requirements) is **not defined there**; a proposal is in the decision log awaiting the owner.

## 4. Indexes

```sql
-- browsing / sorting (partial: only visible tracks, so counts and scans stay small)
CREATE INDEX idx_tracks_title    ON tracks(title_sort, id)                  WHERE is_visible = 1;
CREATE INDEX idx_tracks_added    ON tracks(added_at DESC, id)               WHERE is_visible = 1;
CREATE INDEX idx_tracks_album    ON tracks(album_id, disc_no, track_no, id) WHERE is_visible = 1;
CREATE INDEX idx_tracks_artist   ON tracks(artist_id, title_sort)           WHERE is_visible = 1;
-- scanner
CREATE INDEX idx_tracks_scan     ON tracks(folder_id, last_seen_scan_id);
CREATE INDEX idx_tracks_fprint   ON tracks(fingerprint) WHERE fingerprint IS NOT NULL;
CREATE INDEX idx_tracks_status   ON tracks(status, missing_since) WHERE status <> 'ok';
-- other
CREATE INDEX idx_albums_sort     ON albums(sort_key, id);
CREATE INDEX idx_albums_artist   ON albums(album_artist_id, year);
CREATE INDEX idx_albums_added    ON albums(last_added_at DESC, id);
CREATE INDEX idx_artists_sort    ON artists(sort_key, id);
CREATE INDEX idx_genres_name     ON genres(name_key);
CREATE INDEX idx_trackgenres_g   ON track_genres(genre_id, track_id);
CREATE INDEX idx_playlist_pos    ON playlist_tracks(playlist_id, position);
CREATE INDEX idx_queue_order     ON queue_items(play_order);
```

**Rule for Jules:** any query served by a partial index must repeat `WHERE is_visible = 1` exactly, or SQLite will not use the index. Each list query must be checked with `EXPLAIN QUERY PLAN` in a test (no full table scan on 100k rows).

## 5. Search (FTS5)

```sql
CREATE VIRTUAL TABLE fts_tracks  USING fts5(title, artist, album, genre,
  tokenize = 'unicode61 remove_diacritics 2', prefix = '2 3');   -- rowid = tracks.id
CREATE VIRTUAL TABLE fts_albums  USING fts5(title, artist, tokenize='unicode61 remove_diacritics 2', prefix='2 3'); -- rowid = albums.id
CREATE VIRTUAL TABLE fts_artists USING fts5(name,          tokenize='unicode61 remove_diacritics 2', prefix='2 3'); -- rowid = artists.id
```

- Maintained **explicitly by the scanner in the same transaction** as the row change (not by triggers), because indexed text (artist/album names) lives in other tables.
- Query building: split user text into tokens, quote each (`"tok"`), add `*` to the last token for prefix matching, join with spaces (AND). **Never** pass raw user text to `MATCH`.
- Ranking: `bm25()`; search results join back to `tracks WHERE is_visible = 1`.
- **[Limitation]** `unicode61` does not segment CJK text. A `trigram` tokenizer (substring search) is available in recent SQLite **[Unverified]** for the bundled version; decide in M5 whether to add a CJK fallback table.
- **[Assumption]** FTS tables for ~100k tracks stay in the tens of MB; measured in M12.

## 6. Identity, fingerprints, moves and duplicates

- **Track identity** = `tracks.id`. Not the filename.
- **Fingerprint** (cheap, content-based): hash of `file_size` + first 64 KiB + last 64 KiB. Computed for new/changed files only (the first 64 KiB is read for tags anyway). **[Recommendation]** hash function chosen by Jules among verified options (a fast 128-bit+ hash); record the algorithm in `TECHNICAL_DECISIONS.md`. Stored as hex text.
- **Change detection** for a known path: `(file_size, mtime_ms)` equal → skip (no re-parse). Different → re-parse and recompute fingerprint.
- **Move/rename detection** (end of a scan, only if the folder was fully traversed): a *new* path whose fingerprint equals a track that is now *missing* in the same library → update `rel_path`/`folder_id` on the existing row (keeps id, playlist entries, queue) and count as `moved`.
- **Duplicates**: a new path whose fingerprint matches an **ok, visible** track elsewhere → inserted with `duplicate_of = <original id>`, `is_visible = 0` (hidden, retained so rescans don't reprocess it). If the original goes missing, the oldest duplicate is promoted (`duplicate_of = NULL`, `is_visible = 1`).
- **Limitation:** two *different* files with identical size+edges (extremely unlikely) would be mis-detected; **[Assumption]** acceptable.

## 7. Scanner reconciliation and aggregates

1. Create `scan_history` row (`running`). Check folder reachable → set `availability`. **If the root is unreachable: mark folder `unavailable`, do not touch its tracks** (external drive removed ≠ files deleted). Tracks of an unavailable folder remain visible but flagged unplayable in UI.
2. Walk the tree (no symlink following; skip hidden directories on Linux/macOS), batch results → writer (~500 per transaction). Each seen track gets `last_seen_scan_id = scan id`.
3. **Only if traversal completed without a root-level error and was not cancelled:** tracks in this folder with older `last_seen_scan_id` → `status='missing'`, `missing_since=now`, `is_visible=0`. Then run move detection (§6).
4. Missing tracks are purged (rows deleted → cascade removes playlist/queue entries) after **30 days** missing; until then they can reappear (drive remounted, file restored) with the same id.
5. Recompute denormalised counters (`albums.track_count`, `total_duration_ms`, `artists.*`, `genres.track_count`) for **touched** albums/artists/genres only, at the end of each batch.
6. Cancellation/crash: committed batches stay; the next incremental scan is naturally resumable. Rows left `running` at startup become `interrupted`.

**Album rules:** tracks with no album tag get `album_id = NULL` (shown as "Unknown Album" grouped by artist in the UI; not listed in the Albums grid). Album identity = `mb_release_id` if tagged, else `title_key|album_artist_key` (album artist falls back to track artist). **[Limitation]** compilations tagged without album artist may split by artist; v1 relies on the album-artist tag.

## 8. Query patterns and pagination

- **Offset pagination** (`ORDER BY <indexed key>, id LIMIT ? OFFSET ?`) because virtualised lists need random access to scroll positions. **[Assumption]** OFFSET stays fast at 100k rows on these indexes; verify with the 100k benchmark in M12 (switch to keyset + position index if over budget).
- Total counts via `COUNT(*)` on the partial index (cached per query until `library:changed`).
- Page size 100–200 rows; the frontend keeps only a window of pages in memory (react-query/virtual cache eviction).
- Play-from-source resolves ids in Rust (`SELECT id … ORDER BY …`); 100k ids ≈ 800 KB; never crosses IPC.
- Performance budgets (targets, to be measured): list page ≤ 30 ms, search ≤ 80 ms at 100k tracks, initial scan ≥ 100 files/s on SSD (metadata-bound), UI never blocked.

## 9. Migrations, backup and corruption recovery

- **Migrations:** numbered, forward-only SQL files embedded in the binary (`include_str!`/embed), applied in order, each in a transaction, version tracked with `PRAGMA user_version`. A ~60-line runner is acceptable; a maintained crate may be used if Jules verifies it (see decisions log). **Before applying any pending migration, copy the DB file to `backups/pre-migration-<version>.db`.** Never edit an applied migration; add a new one.
- **Backups:** after a successful startup, at most once per day, create `backups/library-YYYYMMDD.db` using `VACUUM INTO` (**[Assumption]** available in bundled SQLite; add test). Keep the latest 3. Rationale: the library is rebuildable by rescanning, but **playlists and settings are not**.
- **Corruption:** `PRAGMA quick_check` runs in the background after startup. On failure or on open/migrate failure: stop, rename the file to `*.corrupt-<timestamp>`, restore the newest good backup if any (else create a fresh DB), tell the user in plain language, and trigger a rescan. Never delete the corrupt file automatically.
- **Disk full / write errors:** writer reports `DatabaseError`; scanner pauses and surfaces the error; playback unaffected (playback reads only at track start).

## 10. Testing requirements for this module
- Migration tests: empty DB → latest; each step from a fixture DB.
- Scanner tests on generated fixture trees: add / change / delete / rename / duplicate / drive-missing / cancel-midway / non-UTF-8 name / unreadable file.
- `EXPLAIN QUERY PLAN` assertions for every list/search query.
- FTS tests: prefix, diacritics, punctuation/quote injection attempts (`"`, `*`, `AND`, `NEAR(`).
- Synthetic 5k / 20k / 100k library generator for benchmarks.
