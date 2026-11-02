//! Library scanner. Behaviour follows `docs/DATABASE_DESIGN.md` §6–7:
//!
//! - **Incremental:** a known path with the same size and mtime is not re-read.
//! - **States:** tracks that vanish become `missing` (and come back with the same id if the
//!   file returns); renamed files keep their id (`moved`); copies are hidden `duplicate_of`
//!   the original; unparseable files are recorded as `unreadable`.
//! - **Safe:** an unreachable root, a walk error or a cancelled scan never marks tracks missing.
//! - **Batched:** writes go through the single writer in transactions of up to [`BATCH_SIZE`] files.

use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use serde::Serialize;
use rusqlite::{named_params, params, Connection, OptionalExtension, Transaction};
use walkdir::WalkDir;

use crate::db::connection::Database;
use crate::error::{LibraryError, LibraryResult};
use crate::metadata::{extract_metadata, is_unsupported_codec, TrackMetadata};

const BATCH_SIZE: usize = 200;
const PURGE_AFTER_MS: i64 = 30 * 24 * 60 * 60 * 1000;
const SUPPORTED_EXTENSIONS: [&str; 6] = ["mp3", "flac", "wav", "m4a", "ogg", "aiff"];

#[derive(Debug, Default, Clone, Serialize)]
pub struct ScanSummary {
    pub scan_id: i64,
    pub files_seen: usize,
    pub added: usize,
    pub updated: usize,
    pub moved: usize,
    pub missing: usize,
    pub duplicates: usize,
    /// Unchanged files that were not re-read.
    pub skipped: usize,
    /// Files that were not indexed because they cannot be played or stored (Opus, non-UTF-8 names).
    pub unsupported: usize,
    pub errors: usize,
    /// Missing tracks permanently removed after 30 days.
    pub purged: usize,
    pub cancelled: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum ScanPhase {
    Walking,
    Indexing,
    Finishing,
}

#[derive(Debug, Clone, Serialize)]
pub struct ScanProgress {
    pub phase: ScanPhase,
    pub files_total: usize,
    pub files_done: usize,
}

type ProgressFn = Arc<dyn Fn(&ScanProgress) + Send + Sync>;

#[derive(Default, Clone)]
pub struct ScanOptions {
    /// Set to `true` from any thread to stop the scan after the current batch.
    pub cancel: Option<Arc<AtomicBool>>,
    pub on_progress: Option<ProgressFn>,
}

impl ScanOptions {
    fn cancelled(&self) -> bool {
        self.cancel.as_ref().map(|c| c.load(Ordering::Relaxed)).unwrap_or(false)
    }
    fn report(&self, phase: ScanPhase, files_total: usize, files_done: usize) {
        if let Some(cb) = &self.on_progress {
            cb(&ScanProgress { phase, files_total, files_done });
        }
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Case-folded, whitespace-collapsed key. Done in Rust because SQLite's `lower()` is ASCII-only.
pub fn norm_key(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

fn clean(s: Option<String>) -> Option<String> {
    s.map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
}

fn read_up_to(file: &mut File, buf: &mut [u8]) -> std::io::Result<usize> {
    let mut filled = 0;
    while filled < buf.len() {
        let n = file.read(&mut buf[filled..])?;
        if n == 0 {
            break;
        }
        filled += n;
    }
    Ok(filled)
}

/// Content fingerprint: BLAKE3 over file size + first 64 KiB + last 64 KiB, truncated to
/// 128 bits and hex-encoded (32 characters).
pub fn compute_fingerprint<P: AsRef<Path>>(path: P) -> std::io::Result<String> {
    const CHUNK: usize = 64 * 1024;
    let mut file = File::open(path)?;
    let size = file.metadata()?.len();

    let mut hasher = blake3::Hasher::new();
    hasher.update(&size.to_le_bytes());

    let mut buffer = vec![0u8; CHUNK];
    let n = read_up_to(&mut file, &mut buffer)?;
    hasher.update(&buffer[..n]);

    if size > CHUNK as u64 {
        file.seek(SeekFrom::Start(size - CHUNK as u64))?;
        let n = read_up_to(&mut file, &mut buffer)?;
        hasher.update(&buffer[..n]);
    }

    let hex = hasher.finalize().to_hex();
    Ok(hex.as_str()[..32].to_string())
}

struct ExistingTrack {
    id: i64,
    size: i64,
    mtime_ms: i64,
    status: String,
    fingerprint: Option<String>,
}

struct FileEntry {
    path: PathBuf,
    rel_path: String,
    size: i64,
    mtime_ms: i64,
    ext: String,
}

struct Prepared {
    rel_path: String,
    size: i64,
    mtime_ms: i64,
    fingerprint: Option<String>,
    ext: String,
    meta: TrackMetadata,
}

enum Op {
    /// Unchanged file: only record that it was seen.
    Touch(i64),
    Insert(Prepared),
    Update { id: i64, p: Prepared, moved: bool },
    Unreadable { existing: Option<i64>, rel_path: String, size: i64, mtime_ms: i64, ext: String, title: String },
}

#[derive(Default)]
struct BatchCounts {
    added: usize,
    updated: usize,
    moved: usize,
    duplicates: usize,
}

pub struct Scanner;

impl Scanner {
    /// Registers `path` as a library folder (idempotent) and returns its id.
    pub fn add_folder<P: AsRef<Path>>(db: &Database, path: P) -> LibraryResult<i64> {
        let root = path.as_ref();
        let location = root
            .to_str()
            .ok_or_else(|| LibraryError::Internal(format!("Folder path is not valid UTF-8: {}", root.display())))?
            .to_string();
        let display_name = root.file_name().and_then(|s| s.to_str()).unwrap_or("Music").to_string();
        let now = now_ms();

        let loc = location.clone();
        db.writer.execute(move |conn| {
            conn.execute(
                "INSERT INTO library_folders (kind, location, display_name, added_at, availability)
                 VALUES ('path', ?1, ?2, ?3, 'unknown')
                 ON CONFLICT(kind, location) DO NOTHING;",
                params![loc, display_name, now],
            )?;
            Ok(())
        })?;

        db.readers.with_connection(|conn| {
            Ok(conn.query_row(
                "SELECT id FROM library_folders WHERE kind = 'path' AND location = ?1;",
                params![location],
                |r| r.get(0),
            )?)
        })
    }

    /// Registers `path` and scans it with default options. Intended for trusted callers
    /// (tests, CLI). The UI layer should pick folders natively and scan by id.
    pub fn scan_directory<P: AsRef<Path>>(db: &Database, folder_path: P) -> LibraryResult<ScanSummary> {
        let id = Self::add_folder(db, folder_path)?;
        Self::scan_folder(db, id, &ScanOptions::default())
    }

    /// Scans a registered folder by id.
    pub fn scan_folder(db: &Database, folder_id: i64, opts: &ScanOptions) -> LibraryResult<ScanSummary> {
        let location: String = db.readers.with_connection(|conn| {
            conn.query_row("SELECT location FROM library_folders WHERE id = ?1;", params![folder_id], |r| r.get(0))
                .optional()?
                .ok_or_else(|| LibraryError::Internal(format!("Unknown library folder id {folder_id}")))
        })?;
        let root = PathBuf::from(&location);

        // Interrupted scans from a previous run, then a fresh history row.
        let started = now_ms();
        let scan_id = {
            let id_slot = Arc::new(Mutex::new(0i64));
            let slot = Arc::clone(&id_slot);
            db.writer.execute(move |conn| {
                conn.execute(
                    "UPDATE scan_history SET status = 'interrupted' WHERE folder_id = ?1 AND status = 'running';",
                    params![folder_id],
                )?;
                conn.execute(
                    "INSERT INTO scan_history (folder_id, mode, status, started_at) VALUES (?1, 'incremental', 'running', ?2);",
                    params![folder_id, started],
                )?;
                *slot.lock().unwrap() = conn.last_insert_rowid();
                Ok(())
            })?;
            let id = *id_slot.lock().unwrap();
            id
        };

        match Self::run_scan(db, folder_id, scan_id, &root, opts) {
            Ok(summary) => Ok(summary),
            Err(e) => {
                let msg = e.to_string();
                let unreachable = matches!(e, LibraryError::FileUnavailable(_));
                let _ = db.writer.execute(move |conn| {
                    conn.execute(
                        "UPDATE scan_history SET status = 'failed', finished_at = ?2, error_summary = ?3 WHERE id = ?1;",
                        params![scan_id, now_ms(), msg],
                    )?;
                    if unreachable {
                        conn.execute(
                            "UPDATE library_folders SET availability = 'unavailable' WHERE id = ?1;",
                            params![folder_id],
                        )?;
                    }
                    Ok(())
                });
                Err(e)
            }
        }
    }

    fn run_scan(
        db: &Database,
        folder_id: i64,
        scan_id: i64,
        root: &Path,
        opts: &ScanOptions,
    ) -> LibraryResult<ScanSummary> {
        let mut summary = ScanSummary { scan_id, ..Default::default() };

        // An unreachable root (e.g. unplugged drive) must not be mistaken for "all files deleted".
        if !root.is_dir() {
            return Err(LibraryError::FileUnavailable(root.to_path_buf()));
        }

        // 1. Walk the tree.
        opts.report(ScanPhase::Walking, 0, 0);
        let mut entries: Vec<FileEntry> = Vec::new();
        let mut walk_incomplete = false;
        for res in WalkDir::new(root).follow_links(false) {
            let entry = match res {
                Ok(e) => e,
                Err(e) => {
                    if e.depth() == 0 {
                        return Err(LibraryError::FileUnavailable(root.to_path_buf()));
                    }
                    log::warn!("Walk error: {e}");
                    walk_incomplete = true;
                    summary.errors += 1;
                    continue;
                }
            };
            if !entry.file_type().is_file() {
                continue;
            }
            let path = entry.path();
            let ext = match path.extension().and_then(|s| s.to_str()).map(|s| s.to_lowercase()) {
                Some(e) if SUPPORTED_EXTENSIONS.contains(&e.as_str()) || e == "opus" => e,
                _ => continue,
            };
            summary.files_seen += 1;

            // Paths that are not valid UTF-8 cannot be stored losslessly: skip them.
            let rel = path.strip_prefix(root).ok().and_then(|p| p.to_str());
            let Some(rel_path) = rel.map(str::to_string) else {
                log::warn!("Skipping path that is not valid UTF-8: {}", path.display());
                summary.unsupported += 1;
                continue;
            };
            let meta = match entry.metadata() {
                Ok(m) => m,
                Err(e) => {
                    log::warn!("Cannot stat {}: {e}", path.display());
                    summary.errors += 1;
                    continue;
                }
            };
            let mtime_ms = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as i64)
                .unwrap_or(0);
            entries.push(FileEntry { path: path.to_path_buf(), rel_path, size: meta.len() as i64, mtime_ms, ext });
        }

        let traversal_complete = !walk_incomplete;
        let seen: HashSet<&str> = entries.iter().map(|e| e.rel_path.as_str()).collect();

        // 2. Load what we already know about this folder.
        let existing: HashMap<String, ExistingTrack> = db.readers.with_connection(|conn| {
            let mut stmt = conn.prepare(
                "SELECT rel_path, id, file_size, mtime_ms, status, fingerprint FROM tracks WHERE folder_id = ?1;",
            )?;
            let rows = stmt.query_map(params![folder_id], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    ExistingTrack {
                        id: r.get(1)?,
                        size: r.get(2)?,
                        mtime_ms: r.get(3)?,
                        status: r.get(4)?,
                        fingerprint: r.get(5)?,
                    },
                ))
            })?;
            let mut map = HashMap::new();
            for row in rows {
                let (k, v) = row?;
                map.insert(k, v);
            }
            Ok(map)
        })?;

        // Rows whose file is gone are move candidates, but only if we saw the whole tree.
        let mut move_candidates: HashMap<(String, i64), Vec<i64>> = HashMap::new();
        if traversal_complete {
            for (rel, ex) in &existing {
                if !seen.contains(rel.as_str()) && ex.status != "unreadable" {
                    if let Some(fp) = &ex.fingerprint {
                        move_candidates.entry((fp.clone(), ex.size)).or_default().push(ex.id);
                    }
                }
            }
        }

        // 3. Index in batches.
        let total = entries.len();
        let counts = Arc::new(Mutex::new(BatchCounts::default()));
        let mut ops: Vec<Op> = Vec::with_capacity(BATCH_SIZE);
        let mut done = 0usize;

        for entry in &entries {
            if opts.cancelled() {
                summary.cancelled = true;
                break;
            }
            done += 1;

            let known = existing.get(&entry.rel_path);
            if let Some(ex) = known {
                let unchanged = ex.size == entry.size && ex.mtime_ms == entry.mtime_ms;
                if unchanged && (ex.status == "ok" || ex.status == "unreadable") {
                    ops.push(Op::Touch(ex.id));
                    summary.skipped += 1;
                    Self::maybe_flush(db, folder_id, scan_id, &mut ops, &counts, opts, total, done)?;
                    continue;
                }
            }

            let fingerprint = compute_fingerprint(&entry.path).ok();
            match extract_metadata(&entry.path) {
                Ok(meta) if is_unsupported_codec(&meta) => {
                    log::info!("Not indexing {} (codec not playable by this build)", entry.path.display());
                    summary.unsupported += 1;
                    if let Some(ex) = known {
                        ops.push(Op::Unreadable {
                            existing: Some(ex.id),
                            rel_path: entry.rel_path.clone(),
                            size: entry.size,
                            mtime_ms: entry.mtime_ms,
                            ext: entry.ext.clone(),
                            title: meta.title.clone(),
                        });
                    }
                }
                Ok(meta) => {
                    let prepared = Prepared {
                        rel_path: entry.rel_path.clone(),
                        size: entry.size,
                        mtime_ms: entry.mtime_ms,
                        fingerprint: fingerprint.clone(),
                        ext: entry.ext.clone(),
                        meta,
                    };
                    match known {
                        Some(ex) => {
                            let _ = &ex.fingerprint;
                            ops.push(Op::Update { id: ex.id, p: prepared, moved: false });
                        }
                        None => {
                            let moved_from = fingerprint
                                .as_ref()
                                .and_then(|fp| move_candidates.get_mut(&(fp.clone(), entry.size)))
                                .and_then(|ids| ids.pop());
                            match moved_from {
                                Some(id) => ops.push(Op::Update { id, p: prepared, moved: true }),
                                None => ops.push(Op::Insert(prepared)),
                            }
                        }
                    }
                }
                Err(e) => {
                    log::warn!("Metadata extraction failed for {}: {e}", entry.path.display());
                    summary.errors += 1;
                    let title = entry
                        .path
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("Unknown Title")
                        .to_string();
                    ops.push(Op::Unreadable {
                        existing: known.map(|k| k.id),
                        rel_path: entry.rel_path.clone(),
                        size: entry.size,
                        mtime_ms: entry.mtime_ms,
                        ext: entry.ext.clone(),
                        title,
                    });
                }
            }
            Self::maybe_flush(db, folder_id, scan_id, &mut ops, &counts, opts, total, done)?;
        }
        Self::flush(db, folder_id, scan_id, &mut ops, &counts)?;
        opts.report(ScanPhase::Finishing, total, done);

        // 4. Finish: missing, duplicate promotion, purge, aggregates, history.
        let mark_missing = traversal_complete && !summary.cancelled;
        let finish_counts = Arc::new(Mutex::new((0usize, 0usize)));
        let fc = Arc::clone(&finish_counts);
        let cancelled = summary.cancelled;
        {
            let c = counts.lock().unwrap();
            summary.added = c.added;
            summary.updated = c.updated;
            summary.moved = c.moved;
            summary.duplicates = c.duplicates;
        }
        let snapshot = summary.clone();
        db.writer.execute(move |conn| {
            let tx = conn.transaction()?;
            let now = now_ms();
            let mut missing = 0usize;
            let mut purged = 0usize;

            if mark_missing {
                missing = tx.execute(
                    "UPDATE tracks SET status = 'missing', missing_since = COALESCE(missing_since, ?3), is_visible = 0
                     WHERE folder_id = ?1 AND status = 'ok'
                       AND (last_seen_scan_id IS NULL OR last_seen_scan_id <> ?2);",
                    params![folder_id, scan_id, now],
                )?;
                promote_duplicates(&tx)?;
                purged = purge_missing(&tx, folder_id, now - PURGE_AFTER_MS)?;
            }
            refresh_aggregates(&tx)?;

            tx.execute(
                "UPDATE scan_history SET status = ?2, finished_at = ?3, files_seen = ?4, added = ?5, updated = ?6,
                        moved = ?7, missing = ?8, duplicates = ?9, skipped = ?10, errors = ?11
                 WHERE id = ?1;",
                params![
                    scan_id,
                    if cancelled { "cancelled" } else { "completed" },
                    now,
                    snapshot.files_seen as i64,
                    snapshot.added as i64,
                    snapshot.updated as i64,
                    snapshot.moved as i64,
                    missing as i64,
                    snapshot.duplicates as i64,
                    snapshot.skipped as i64,
                    snapshot.errors as i64,
                ],
            )?;
            tx.execute(
                "UPDATE library_folders SET availability = 'available', last_scan_at = ?2 WHERE id = ?1;",
                params![folder_id, now],
            )?;
            tx.commit()?;
            *fc.lock().unwrap() = (missing, purged);
            Ok(())
        })?;

        let (missing, purged) = *finish_counts.lock().unwrap();
        summary.missing = missing;
        summary.purged = purged;
        Ok(summary)
    }

    #[allow(clippy::too_many_arguments)]
    fn maybe_flush(
        db: &Database,
        folder_id: i64,
        scan_id: i64,
        ops: &mut Vec<Op>,
        counts: &Arc<Mutex<BatchCounts>>,
        opts: &ScanOptions,
        total: usize,
        done: usize,
    ) -> LibraryResult<()> {
        if ops.len() >= BATCH_SIZE {
            Self::flush(db, folder_id, scan_id, ops, counts)?;
            opts.report(ScanPhase::Indexing, total, done);
        }
        Ok(())
    }

    fn flush(
        db: &Database,
        folder_id: i64,
        scan_id: i64,
        ops: &mut Vec<Op>,
        counts: &Arc<Mutex<BatchCounts>>,
    ) -> LibraryResult<()> {
        if ops.is_empty() {
            return Ok(());
        }
        let batch = std::mem::take(ops);
        let counts = Arc::clone(counts);
        db.writer.execute(move |conn| apply_batch(conn, folder_id, scan_id, batch, &counts))
    }
}

fn apply_batch(
    conn: &mut Connection,
    folder_id: i64,
    scan_id: i64,
    ops: Vec<Op>,
    counts: &Arc<Mutex<BatchCounts>>,
) -> LibraryResult<()> {
    let tx = conn.transaction()?;
    let now = now_ms();
    let mut local = BatchCounts::default();

    for op in ops {
        match op {
            Op::Touch(id) => {
                tx.prepare_cached("UPDATE tracks SET last_seen_scan_id = ?1 WHERE id = ?2;")?
                    .execute(params![scan_id, id])?;
            }
            Op::Insert(p) => {
                let (_, dup) = write_track(&tx, folder_id, scan_id, None, &p, now)?;
                local.added += 1;
                if dup {
                    local.duplicates += 1;
                }
            }
            Op::Update { id, p, moved } => {
                let (_, dup) = write_track(&tx, folder_id, scan_id, Some(id), &p, now)?;
                if moved {
                    local.moved += 1;
                } else {
                    local.updated += 1;
                }
                if dup {
                    local.duplicates += 1;
                }
            }
            Op::Unreadable { existing, rel_path, size, mtime_ms, ext, title } => {
                let title_sort = norm_key(&title);
                match existing {
                    Some(id) => {
                        tx.execute(
                            "UPDATE tracks SET file_size = ?2, mtime_ms = ?3, status = 'unreadable', is_visible = 0,
                                    updated_at = ?4, last_seen_scan_id = ?5 WHERE id = ?1;",
                            params![id, size, mtime_ms, now, scan_id],
                        )?;
                    }
                    None => {
                        tx.execute(
                            "INSERT INTO tracks (folder_id, rel_path, file_size, mtime_ms, format, title, title_sort,
                                    status, is_visible, added_at, updated_at, last_seen_scan_id)
                             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'unreadable', 0, ?8, ?8, ?9);",
                            params![folder_id, rel_path, size, mtime_ms, ext, title, title_sort, now, scan_id],
                        )?;
                    }
                }
            }
        }
    }
    tx.commit()?;

    let mut c = counts.lock().unwrap();
    c.added += local.added;
    c.updated += local.updated;
    c.moved += local.moved;
    c.duplicates += local.duplicates;
    Ok(())
}

fn upsert_artist(tx: &Transaction, name: &str) -> LibraryResult<i64> {
    let key = norm_key(name);
    tx.execute(
        "INSERT INTO artists (name, sort_key, name_key) VALUES (?1, ?2, ?2) ON CONFLICT(name_key) DO NOTHING;",
        params![name, key],
    )?;
    Ok(tx.query_row("SELECT id FROM artists WHERE name_key = ?1;", params![key], |r| r.get(0))?)
}

/// Inserts or updates one track row with every extracted tag, plus its artist, album,
/// genre and search index entries. Returns the track id and whether it was stored as a duplicate.
fn write_track(
    tx: &Transaction,
    folder_id: i64,
    scan_id: i64,
    existing: Option<i64>,
    p: &Prepared,
    now: i64,
) -> LibraryResult<(i64, bool)> {
    let m = &p.meta;
    let track_artist = clean(m.artist.clone()).unwrap_or_else(|| "Unknown Artist".to_string());
    let album_title = clean(m.album.clone()).unwrap_or_else(|| "Unknown Album".to_string());
    // Albums belong to the album artist, so compilations stay in one album.
    let album_artist = clean(m.album_artist.clone()).unwrap_or_else(|| track_artist.clone());

    let artist_id = upsert_artist(tx, &track_artist)?;
    let album_artist_id = upsert_artist(tx, &album_artist)?;

    let album_key = format!("{}|{}", norm_key(&album_title), norm_key(&album_artist));
    tx.execute(
        "INSERT INTO albums (title, sort_key, album_key, album_artist_id, last_added_at)
         VALUES (?1, ?2, ?3, ?4, ?5) ON CONFLICT(album_key) DO NOTHING;",
        params![album_title, norm_key(&album_title), album_key, album_artist_id, now],
    )?;
    let album_id: i64 =
        tx.query_row("SELECT id FROM albums WHERE album_key = ?1;", params![album_key], |r| r.get(0))?;

    // A different, visible, ok track with the same content makes this one a hidden duplicate.
    let duplicate_of: Option<i64> = match &p.fingerprint {
        Some(fp) => tx
            .query_row(
                "SELECT id FROM tracks WHERE fingerprint = ?1 AND file_size = ?2 AND status = 'ok'
                   AND duplicate_of IS NULL AND id <> ?3 ORDER BY id LIMIT 1;",
                params![fp, p.size, existing.unwrap_or(-1)],
                |r| r.get(0),
            )
            .optional()?,
        None => None,
    };
    let is_visible = duplicate_of.is_none() as i64;
    let title = m.title.clone();
    let title_sort = norm_key(&title);

    let track_id = match existing {
        Some(id) => {
            tx.execute(
                "UPDATE tracks SET rel_path = :rel_path, file_size = :size, mtime_ms = :mtime, fingerprint = :fp,
                        format = :format, codec = :codec, title = :title, title_sort = :title_sort,
                        artist_id = :artist_id, artist_display = :artist, album_id = :album_id,
                        track_no = :track_no, track_total = :track_total, disc_no = :disc_no, disc_total = :disc_total,
                        year = :year, duration_ms = :duration, bitrate = :bitrate, sample_rate = :sample_rate,
                        channels = :channels, status = 'ok', missing_since = NULL, duplicate_of = :dup,
                        is_visible = :visible, updated_at = :now, last_seen_scan_id = :scan
                 WHERE id = :id;",
                named_params! {
                    ":id": id, ":rel_path": p.rel_path, ":size": p.size, ":mtime": p.mtime_ms, ":fp": p.fingerprint,
                    ":format": p.ext, ":codec": m.codec, ":title": title, ":title_sort": title_sort,
                    ":artist_id": artist_id, ":artist": track_artist, ":album_id": album_id,
                    ":track_no": m.track_no.map(i64::from), ":track_total": m.track_total.map(i64::from),
                    ":disc_no": m.disc_no.map(i64::from), ":disc_total": m.disc_total.map(i64::from),
                    ":year": m.year.map(i64::from), ":duration": m.duration_ms as i64,
                    ":bitrate": m.bitrate.map(i64::from), ":sample_rate": m.sample_rate.map(i64::from),
                    ":channels": m.channels.map(i64::from), ":dup": duplicate_of, ":visible": is_visible,
                    ":now": now, ":scan": scan_id,
                },
            )?;
            id
        }
        None => {
            tx.execute(
                "INSERT INTO tracks (folder_id, rel_path, file_size, mtime_ms, fingerprint, format, codec, title,
                        title_sort, artist_id, artist_display, album_id, track_no, track_total, disc_no, disc_total,
                        year, duration_ms, bitrate, sample_rate, channels, status, duplicate_of, is_visible,
                        added_at, updated_at, last_seen_scan_id)
                 VALUES (:folder, :rel_path, :size, :mtime, :fp, :format, :codec, :title, :title_sort, :artist_id,
                        :artist, :album_id, :track_no, :track_total, :disc_no, :disc_total, :year, :duration,
                        :bitrate, :sample_rate, :channels, 'ok', :dup, :visible, :now, :now, :scan);",
                named_params! {
                    ":folder": folder_id, ":rel_path": p.rel_path, ":size": p.size, ":mtime": p.mtime_ms,
                    ":fp": p.fingerprint, ":format": p.ext, ":codec": m.codec, ":title": title,
                    ":title_sort": title_sort, ":artist_id": artist_id, ":artist": track_artist,
                    ":album_id": album_id, ":track_no": m.track_no.map(i64::from),
                    ":track_total": m.track_total.map(i64::from), ":disc_no": m.disc_no.map(i64::from),
                    ":disc_total": m.disc_total.map(i64::from), ":year": m.year.map(i64::from),
                    ":duration": m.duration_ms as i64, ":bitrate": m.bitrate.map(i64::from),
                    ":sample_rate": m.sample_rate.map(i64::from), ":channels": m.channels.map(i64::from),
                    ":dup": duplicate_of, ":visible": is_visible, ":now": now, ":scan": scan_id,
                },
            )?;
            tx.last_insert_rowid()
        }
    };

    // Genre
    tx.execute("DELETE FROM track_genres WHERE track_id = ?1;", params![track_id])?;
    if let Some(genre) = clean(m.genre.clone()) {
        let key = norm_key(&genre);
        tx.execute(
            "INSERT INTO genres (name, name_key) VALUES (?1, ?2) ON CONFLICT(name_key) DO NOTHING;",
            params![genre, key],
        )?;
        let genre_id: i64 = tx.query_row("SELECT id FROM genres WHERE name_key = ?1;", params![key], |r| r.get(0))?;
        tx.execute(
            "INSERT OR IGNORE INTO track_genres (track_id, genre_id) VALUES (?1, ?2);",
            params![track_id, genre_id],
        )?;
    }

    // Search index (queries join back to tracks with is_visible = 1)
    tx.execute(
        "INSERT OR REPLACE INTO fts_tracks (rowid, title, artist, album) VALUES (?1, ?2, ?3, ?4);",
        params![track_id, title, track_artist, album_title],
    )?;

    Ok((track_id, duplicate_of.is_some()))
}

/// If the original of a hidden duplicate went missing, the oldest remaining copy takes its place.
fn promote_duplicates(tx: &Transaction) -> LibraryResult<()> {
    let orphaned: Vec<i64> = {
        let mut stmt = tx.prepare(
            "SELECT DISTINCT d.duplicate_of FROM tracks d JOIN tracks o ON d.duplicate_of = o.id WHERE o.status <> 'ok';",
        )?;
        let rows = stmt.query_map([], |r| r.get(0))?;
        rows.collect::<Result<_, _>>()?
    };
    for original in orphaned {
        let promote: Option<i64> = tx
            .query_row(
                "SELECT id FROM tracks WHERE duplicate_of = ?1 AND status = 'ok' ORDER BY id LIMIT 1;",
                params![original],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(p) = promote {
            tx.execute("UPDATE tracks SET duplicate_of = NULL, is_visible = 1 WHERE id = ?1;", params![p])?;
            tx.execute(
                "UPDATE tracks SET duplicate_of = ?1 WHERE duplicate_of = ?2 AND id <> ?1;",
                params![p, original],
            )?;
        }
    }
    Ok(())
}

/// Deletes tracks that have been missing since before `cutoff_ms`, then tidies what they leave behind.
fn purge_missing(tx: &Transaction, folder_id: i64, cutoff_ms: i64) -> LibraryResult<usize> {
    tx.execute(
        "DELETE FROM fts_tracks WHERE rowid IN
           (SELECT id FROM tracks WHERE folder_id = ?1 AND status = 'missing' AND missing_since < ?2);",
        params![folder_id, cutoff_ms],
    )?;
    let purged = tx.execute(
        "DELETE FROM tracks WHERE folder_id = ?1 AND status = 'missing' AND missing_since < ?2;",
        params![folder_id, cutoff_ms],
    )?;
    if purged > 0 {
        // A duplicate whose original was deleted becomes the visible copy.
        tx.execute(
            "UPDATE tracks SET is_visible = 1 WHERE status = 'ok' AND duplicate_of IS NULL AND is_visible = 0;",
            [],
        )?;
    }
    Ok(purged)
}

/// Recomputes denormalised counters and drops albums/artists/genres nothing refers to.
fn refresh_aggregates(tx: &Transaction) -> LibraryResult<()> {
    tx.execute_batch(
        "DELETE FROM albums WHERE id NOT IN (SELECT DISTINCT album_id FROM tracks WHERE album_id IS NOT NULL);
         DELETE FROM artists WHERE id NOT IN (SELECT DISTINCT artist_id FROM tracks WHERE artist_id IS NOT NULL)
                               AND id NOT IN (SELECT DISTINCT album_artist_id FROM albums WHERE album_artist_id IS NOT NULL);
         DELETE FROM genres WHERE id NOT IN (SELECT DISTINCT genre_id FROM track_genres);

         UPDATE albums SET
           track_count = (SELECT COUNT(*) FROM tracks t WHERE t.album_id = albums.id AND t.is_visible = 1),
           total_duration_ms = COALESCE((SELECT SUM(t.duration_ms) FROM tracks t WHERE t.album_id = albums.id AND t.is_visible = 1), 0),
           year = (SELECT MIN(t.year) FROM tracks t WHERE t.album_id = albums.id AND t.is_visible = 1 AND t.year IS NOT NULL);

         UPDATE artists SET
           track_count = (SELECT COUNT(*) FROM tracks t WHERE t.artist_id = artists.id AND t.is_visible = 1),
           album_count = (SELECT COUNT(*) FROM albums a WHERE a.album_artist_id = artists.id AND a.track_count > 0);

         UPDATE genres SET
           track_count = (SELECT COUNT(*) FROM track_genres g JOIN tracks t ON t.id = g.track_id
                          WHERE g.genre_id = genres.id AND t.is_visible = 1);",
    )?;
    Ok(())
}
