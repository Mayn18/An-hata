use std::fs::File;
use std::io::Write;
use std::path::Path;

use anahata_library::{Database, Queries, Scanner};

fn create_dummy_wav<P: AsRef<Path>>(path: P, fill: u8) {
    let mut file = File::create(path).unwrap();
    let num_samples: usize = 4410;
    let sample_rate: u32 = 44100;
    let num_channels: u16 = 2;
    let bits_per_sample: u16 = 16;
    let byte_rate = sample_rate * num_channels as u32 * (bits_per_sample / 8) as u32;
    let block_align = num_channels * (bits_per_sample / 8);
    let subchunk2_size = (num_samples * num_channels as usize * (bits_per_sample / 8) as usize) as u32;
    let chunk_size = 36 + subchunk2_size;

    file.write_all(b"RIFF").unwrap();
    file.write_all(&chunk_size.to_le_bytes()).unwrap();
    file.write_all(b"WAVEfmt ").unwrap();
    file.write_all(&16u32.to_le_bytes()).unwrap();
    file.write_all(&1u16.to_le_bytes()).unwrap();
    file.write_all(&num_channels.to_le_bytes()).unwrap();
    file.write_all(&sample_rate.to_le_bytes()).unwrap();
    file.write_all(&byte_rate.to_le_bytes()).unwrap();
    file.write_all(&block_align.to_le_bytes()).unwrap();
    file.write_all(&bits_per_sample.to_le_bytes()).unwrap();
    file.write_all(b"data").unwrap();
    file.write_all(&subchunk2_size.to_le_bytes()).unwrap();

    // Different fill per file: identical files would (correctly) be treated as duplicates.
    let silence = vec![fill; subchunk2_size as usize];
    file.write_all(&silence).unwrap();
}

#[test]
fn test_database_migrations_and_scanner() {
    let temp_dir = std::env::temp_dir().join(format!("anahata_test_{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
    std::fs::create_dir_all(&temp_dir).unwrap();

    let db_path = temp_dir.join("library.db");
    let music_dir = temp_dir.join("music");
    std::fs::create_dir_all(&music_dir).unwrap();

    let track1 = music_dir.join("song1.wav");
    let track2 = music_dir.join("song2.wav");
    create_dummy_wav(&track1, 0);
    create_dummy_wav(&track2, 1);

    let db = Database::open(&db_path).expect("Failed to open DB");

    // Scan directory
    let summary = Scanner::scan_directory(&db, &music_dir).expect("Scan failed");
    assert_eq!(summary.files_seen, 2);
    assert_eq!(summary.added, 2);

    // List tracks
    let tracks = Queries::list_tracks(&db, 10, 0).expect("Failed to list tracks");
    assert_eq!(tracks.len(), 2);

    // FTS search
    let search_results = Queries::search_tracks(&db, "song1", 10).expect("FTS search failed");
    assert_eq!(search_results.len(), 1);
    assert_eq!(search_results[0].title, "song1");

    let _ = std::fs::remove_dir_all(temp_dir);
}
