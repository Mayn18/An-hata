use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use anahata_library::{
    compute_fingerprint, is_unsupported_codec, Database, LibraryError, Queries, ScanOptions, Scanner, TrackMetadata,
};
use lofty::config::WriteOptions;
use lofty::tag::{Accessor, ItemKey, Tag, TagExt, TagType};

struct Fixture {
    root: PathBuf,
    music: PathBuf,
    db: Database,
}

impl Fixture {
    fn new(name: &str) -> Self {
        let root = std::env::temp_dir().join(format!("anahata_scan_{}_{}_{}", name, std::process::id(), nanos()));
        let music = root.join("music");
        std::fs::create_dir_all(&music).unwrap();
        let db = Database::open(root.join("library.db")).unwrap();
        Self { root, music, db }
    }

    fn scan(&self) -> anahata_library::ScanSummary {
        Scanner::scan_directory(&self.db, &self.music).expect("scan")
    }

    fn folder_id(&self) -> i64 {
        Scanner::add_folder(&self.db, &self.music).unwrap()
    }

    fn sql<T: rusqlite::types::FromSql>(&self, sql: &str) -> T {
        self.db.readers.with_connection(|c| Ok(c.query_row(sql, [], |r| r.get(0))?)).unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn nanos() -> u128 {
    SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_nanos()
}

/// A short WAV whose content depends on `seed`, so different seeds have different fingerprints.
fn wav(path: &Path, seed: i16) {
    let frames = 4410usize;
    let (rate, ch, bits) = (44100u32, 2u16, 16u16);
    let data_len = (frames * ch as usize * 2) as u32;
    let mut f = File::create(path).unwrap();
    f.write_all(b"RIFF").unwrap();
    f.write_all(&(36 + data_len).to_le_bytes()).unwrap();
    f.write_all(b"WAVEfmt ").unwrap();
    f.write_all(&16u32.to_le_bytes()).unwrap();
    f.write_all(&1u16.to_le_bytes()).unwrap();
    f.write_all(&ch.to_le_bytes()).unwrap();
    f.write_all(&rate.to_le_bytes()).unwrap();
    f.write_all(&(rate * ch as u32 * 2).to_le_bytes()).unwrap();
    f.write_all(&(ch * 2).to_le_bytes()).unwrap();
    f.write_all(&bits.to_le_bytes()).unwrap();
    f.write_all(b"data").unwrap();
    f.write_all(&data_len.to_le_bytes()).unwrap();
    for i in 0..frames {
        let s = ((i as i16).wrapping_mul(7)).wrapping_add(seed).to_le_bytes();
        f.write_all(&s).unwrap();
        f.write_all(&s).unwrap();
    }
}

#[allow(clippy::too_many_arguments)]
fn tag(path: &Path, title: &str, artist: &str, album: &str, album_artist: Option<&str>, genre: &str, track: u32, disc: u32, year: u32) {
    let mut t = Tag::new(TagType::Id3v2);
    t.set_title(title.to_string());
    t.set_artist(artist.to_string());
    t.set_album(album.to_string());
    t.set_genre(genre.to_string());
    t.set_track(track);
    t.set_disk(disc);
    t.set_year(year);
    if let Some(aa) = album_artist {
        t.insert_text(ItemKey::AlbumArtist, aa.to_string());
    }
    t.save_to_path(path, WriteOptions::default()).expect("write tag");
}

fn bump_mtime(path: &Path) {
    let f = File::options().write(true).open(path).unwrap();
    f.set_modified(SystemTime::now() + Duration::from_secs(120)).unwrap();
}

#[test]
fn all_tags_are_persisted_and_compilations_stay_together() {
    let fx = Fixture::new("tags");
    let a = fx.music.join("a.wav");
    let b = fx.music.join("b.wav");
    wav(&a, 1);
    wav(&b, 2);
    tag(&a, "One", "Artist X", "Hits", Some("Various Artists"), "Rock", 1, 1, 1999);
    tag(&b, "Two", "Artist Y", "Hits", Some("Various Artists"), "Rock", 2, 1, 2001);

    let s = fx.scan();
    assert_eq!((s.files_seen, s.added, s.errors), (2, 2, 0));

    assert_eq!(fx.sql::<i64>("SELECT COUNT(*) FROM albums"), 1, "compilation must not split per track artist");
    assert_eq!(fx.sql::<i64>("SELECT track_no FROM tracks WHERE title = 'Two'"), 2);
    assert_eq!(fx.sql::<i64>("SELECT disc_no FROM tracks WHERE title = 'One'"), 1);
    assert_eq!(fx.sql::<i64>("SELECT year FROM tracks WHERE title = 'One'"), 1999);
    assert_eq!(fx.sql::<String>("SELECT g.name FROM track_genres tg JOIN genres g ON g.id = tg.genre_id JOIN tracks t ON t.id = tg.track_id WHERE t.title = 'One'"), "Rock");
    assert_eq!(fx.sql::<String>("SELECT ar.name FROM albums al JOIN artists ar ON ar.id = al.album_artist_id"), "Various Artists");
    assert_eq!(fx.sql::<i64>("SELECT year FROM albums"), 1999, "album year is the earliest track year");

    let albums = Queries::list_albums(&fx.db, 10, 0).unwrap();
    assert_eq!(albums.len(), 1);
    assert_eq!(albums[0].track_count, 2, "album track_count must be maintained");
    assert_eq!(fx.sql::<i64>("SELECT total_duration_ms FROM albums") > 0, true);
}

#[test]
fn rescan_is_incremental_and_records_history() {
    let fx = Fixture::new("incr");
    for i in 0..3 {
        wav(&fx.music.join(format!("{i}.wav")), i);
    }
    let first = fx.scan();
    assert_eq!((first.added, first.skipped), (3, 0));

    let second = fx.scan();
    assert_eq!((second.added, second.updated, second.skipped, second.missing), (0, 0, 3, 0));
    assert_eq!(fx.sql::<i64>("SELECT COUNT(*) FROM scan_history WHERE status = 'completed'"), 2);
    assert_eq!(fx.sql::<i64>("SELECT skipped FROM scan_history ORDER BY id DESC LIMIT 1"), 3);
    assert!(fx.sql::<Option<i64>>("SELECT last_scan_at FROM library_folders").is_some());
}

#[test]
fn changed_file_is_reparsed() {
    let fx = Fixture::new("changed");
    let p = fx.music.join("song.wav");
    wav(&p, 1);
    tag(&p, "Old Title", "A", "B", None, "Pop", 1, 1, 2000);
    fx.scan();

    tag(&p, "New Title", "A", "B", None, "Pop", 1, 1, 2000);
    bump_mtime(&p);
    let s = fx.scan();
    assert_eq!((s.updated, s.added, s.skipped), (1, 0, 0));
    assert_eq!(Queries::search_tracks(&fx.db, "New", 10).unwrap().len(), 1);
    assert_eq!(Queries::search_tracks(&fx.db, "Old", 10).unwrap().len(), 0);
}

#[test]
fn deleted_file_goes_missing_and_returns_with_the_same_id() {
    let fx = Fixture::new("missing");
    let keep = fx.music.join("keep.wav");
    let gone = fx.music.join("gone.wav");
    wav(&keep, 1);
    wav(&gone, 2);
    fx.scan();
    let id: i64 = fx.sql("SELECT id FROM tracks WHERE rel_path = 'gone.wav'");

    let backup = fx.root.join("gone.bak");
    std::fs::rename(&gone, &backup).unwrap();
    let s = fx.scan();
    assert_eq!(s.missing, 1);
    assert_eq!(Queries::list_tracks(&fx.db, 10, 0).unwrap().len(), 1, "missing track must be hidden");
    assert_eq!(fx.sql::<String>("SELECT status FROM tracks WHERE id = (SELECT id FROM tracks WHERE rel_path = 'gone.wav')"), "missing");

    std::fs::rename(&backup, &gone).unwrap();
    let s = fx.scan();
    assert_eq!(s.missing, 0);
    assert_eq!(fx.sql::<i64>("SELECT id FROM tracks WHERE rel_path = 'gone.wav'"), id, "same id after reappearing");
    assert_eq!(fx.sql::<String>("SELECT status FROM tracks WHERE rel_path = 'gone.wav'"), "ok");
    assert_eq!(Queries::list_tracks(&fx.db, 10, 0).unwrap().len(), 2);
}

#[test]
fn renamed_file_keeps_its_track_id() {
    let fx = Fixture::new("moved");
    let old = fx.music.join("old name.wav");
    wav(&old, 5);
    tag(&old, "Keep Me", "A", "B", None, "Pop", 1, 1, 2000);
    fx.scan();
    let id: i64 = fx.sql("SELECT id FROM tracks");

    std::fs::create_dir_all(fx.music.join("sub")).unwrap();
    std::fs::rename(&old, fx.music.join("sub").join("new name.wav")).unwrap();
    let s = fx.scan();
    assert_eq!((s.moved, s.added, s.missing), (1, 0, 0));
    assert_eq!(fx.sql::<i64>("SELECT COUNT(*) FROM tracks"), 1);
    assert_eq!(fx.sql::<i64>("SELECT id FROM tracks"), id);
    assert!(fx.sql::<String>("SELECT rel_path FROM tracks").ends_with("new name.wav"));
}

#[test]
fn duplicates_are_hidden_and_promoted_when_the_original_disappears() {
    let fx = Fixture::new("dups");
    let orig = fx.music.join("orig.wav");
    wav(&orig, 9);
    tag(&orig, "Same", "A", "B", None, "Pop", 1, 1, 2000);
    fx.scan();
    std::fs::copy(&orig, fx.music.join("copy.wav")).unwrap();

    let s = fx.scan();
    assert_eq!((s.added, s.duplicates), (1, 1));
    let listed = Queries::list_tracks(&fx.db, 10, 0).unwrap();
    assert_eq!(listed.len(), 1, "copy must be hidden");
    assert_eq!(listed[0].rel_path, "orig.wav");
    assert_eq!(fx.sql::<i64>("SELECT COUNT(*) FROM tracks WHERE duplicate_of IS NOT NULL"), 1);

    std::fs::remove_file(&orig).unwrap();
    let s = fx.scan();
    assert_eq!(s.missing, 1);
    let listed = Queries::list_tracks(&fx.db, 10, 0).unwrap();
    assert_eq!(listed.len(), 1, "the copy takes over");
    assert_eq!(listed[0].rel_path, "copy.wav");
}

#[test]
fn unreachable_root_does_not_delete_or_hide_tracks() {
    let fx = Fixture::new("drive");
    wav(&fx.music.join("a.wav"), 1);
    fx.scan();
    let folder_id = fx.folder_id();

    let away = fx.root.join("music_unplugged");
    std::fs::rename(&fx.music, &away).unwrap();
    let err = Scanner::scan_folder(&fx.db, folder_id, &ScanOptions::default()).unwrap_err();
    assert!(matches!(err, LibraryError::FileUnavailable(_)));

    assert_eq!(Queries::list_tracks(&fx.db, 10, 0).unwrap().len(), 1, "tracks stay visible");
    assert_eq!(fx.sql::<String>("SELECT status FROM tracks"), "ok");
    assert_eq!(fx.sql::<String>("SELECT availability FROM library_folders"), "unavailable");
    assert_eq!(fx.sql::<String>("SELECT status FROM scan_history ORDER BY id DESC LIMIT 1"), "failed");

    std::fs::rename(&away, &fx.music).unwrap();
    let s = Scanner::scan_folder(&fx.db, folder_id, &ScanOptions::default()).unwrap();
    assert_eq!((s.missing, s.skipped), (0, 1));
    assert_eq!(fx.sql::<String>("SELECT availability FROM library_folders"), "available");
}

#[test]
fn cancelled_scan_never_marks_tracks_missing() {
    let fx = Fixture::new("cancel");
    wav(&fx.music.join("a.wav"), 1);
    wav(&fx.music.join("b.wav"), 2);
    fx.scan();
    std::fs::remove_file(fx.music.join("b.wav")).unwrap();

    let cancel = Arc::new(AtomicBool::new(true));
    let opts = ScanOptions { cancel: Some(cancel), on_progress: None };
    let s = Scanner::scan_folder(&fx.db, fx.folder_id(), &opts).unwrap();
    assert!(s.cancelled);
    assert_eq!(s.missing, 0);
    assert_eq!(fx.sql::<i64>("SELECT COUNT(*) FROM tracks WHERE status = 'ok'"), 2);
    assert_eq!(fx.sql::<String>("SELECT status FROM scan_history ORDER BY id DESC LIMIT 1"), "cancelled");

    let s = fx.scan();
    assert_eq!(s.missing, 1, "a complete scan afterwards does mark it");
}

#[test]
fn progress_is_reported() {
    let fx = Fixture::new("progress");
    for i in 0..5 {
        wav(&fx.music.join(format!("{i}.wav")), i);
    }
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let sink = Arc::clone(&seen);
    let opts = ScanOptions {
        cancel: None,
        on_progress: Some(Arc::new(move |p| sink.lock().unwrap().push((p.phase, p.files_total)))),
    };
    Scanner::scan_folder(&fx.db, fx.folder_id(), &opts).unwrap();
    let seen = seen.lock().unwrap();
    assert!(seen.iter().any(|(ph, _)| *ph == anahata_library::ScanPhase::Walking));
    assert!(seen.iter().any(|(ph, t)| *ph == anahata_library::ScanPhase::Finishing && *t == 5));
}

#[cfg(unix)]
#[test]
fn non_utf8_file_names_are_skipped_not_corrupted() {
    use std::os::unix::ffi::OsStrExt;
    let fx = Fixture::new("utf8");
    wav(&fx.music.join("fine.wav"), 1);
    let bad = fx.music.join(std::ffi::OsStr::from_bytes(b"bad\xff.wav"));
    wav(&bad, 2);

    let s = fx.scan();
    assert_eq!((s.files_seen, s.added, s.unsupported), (2, 1, 1));
    assert_eq!(fx.sql::<i64>("SELECT COUNT(*) FROM tracks"), 1);
}

#[test]
fn unreadable_file_is_recorded_once_and_not_listed() {
    let fx = Fixture::new("unreadable");
    wav(&fx.music.join("ok.wav"), 1);
    std::fs::write(fx.music.join("broken.mp3"), b"this is not audio at all").unwrap();

    let s = fx.scan();
    assert_eq!((s.added, s.errors), (1, 1));
    assert_eq!(Queries::list_tracks(&fx.db, 10, 0).unwrap().len(), 1);
    assert_eq!(fx.sql::<String>("SELECT status FROM tracks WHERE rel_path = 'broken.mp3'"), "unreadable");

    let s = fx.scan();
    assert_eq!((s.errors, s.skipped), (0, 2), "unchanged unreadable files are not re-parsed");
}

#[test]
fn names_that_differ_only_by_unicode_case_are_one_artist() {
    let fx = Fixture::new("unicode");
    let a = fx.music.join("a.wav");
    let b = fx.music.join("b.wav");
    wav(&a, 1);
    wav(&b, 2);
    tag(&a, "One", "Éric", "Album", None, "Pop", 1, 1, 2000);
    tag(&b, "Two", "éric", "Album", None, "Pop", 2, 1, 2000);
    fx.scan();
    assert_eq!(fx.sql::<i64>("SELECT COUNT(*) FROM artists"), 1);
    assert_eq!(fx.sql::<i64>("SELECT COUNT(*) FROM albums"), 1);
}

#[test]
fn fingerprint_is_128_bit_and_sensitive_to_head_and_tail() {
    let dir = std::env::temp_dir().join(format!("anahata_fp_{}_{}", std::process::id(), nanos()));
    std::fs::create_dir_all(&dir).unwrap();
    let base: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();

    let write = |name: &str, data: &[u8]| {
        let p = dir.join(name);
        std::fs::write(&p, data).unwrap();
        compute_fingerprint(&p).unwrap()
    };
    let f0 = write("a", &base);
    assert_eq!(f0.len(), 32);
    assert!(f0.chars().all(|c| c.is_ascii_hexdigit()));
    assert_eq!(f0, write("same", &base));

    let mut tail = base.clone();
    *tail.last_mut().unwrap() ^= 0xFF;
    assert_ne!(f0, write("tail", &tail));
    let mut head = base.clone();
    head[0] ^= 0xFF;
    assert_ne!(f0, write("head", &head));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn opus_codec_is_flagged_unsupported() {
    let opus = TrackMetadata { codec: Some("opus".into()), ..Default::default() };
    let flac = TrackMetadata { codec: Some("flac".into()), ..Default::default() };
    assert!(is_unsupported_codec(&opus));
    assert!(!is_unsupported_codec(&flac));
}

#[test]
fn tracks_by_ids_preserves_order_and_skips_unknown() {
    let fx = Fixture::new("byids");
    wav(&fx.music.join("a.wav"), 1);
    wav(&fx.music.join("b.wav"), 2);
    fx.scan();
    let ids: Vec<i64> = Queries::list_tracks(&fx.db, 10, 0).unwrap().iter().map(|t| t.id).collect();
    let got = Queries::tracks_by_ids(&fx.db, &[ids[1], 9999, ids[0]]).unwrap();
    assert_eq!(got.iter().map(|t| t.id).collect::<Vec<_>>(), vec![ids[1], ids[0]]);
    assert_eq!(Queries::count_tracks(&fx.db).unwrap(), 2);
    let folders = Queries::list_folders(&fx.db).unwrap();
    assert_eq!((folders.len(), folders[0].track_count), (1, 2));
}
