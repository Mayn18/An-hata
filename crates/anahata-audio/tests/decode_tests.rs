use std::fs::File;
use std::io::Write;
use std::path::Path;

use anahata_audio::{AudioError, Source};

fn create_test_wav<P: AsRef<Path>>(path: P, num_samples: usize, sample_rate: u32) {
    let mut file = File::create(path).expect("failed to create wav file");

    let num_channels: u16 = 2;
    let bits_per_sample: u16 = 16;
    let byte_rate = sample_rate * num_channels as u32 * (bits_per_sample / 8) as u32;
    let block_align = num_channels * (bits_per_sample / 8);
    let subchunk2_size = (num_samples * num_channels as usize * (bits_per_sample / 8) as usize) as u32;
    let chunk_size = 36 + subchunk2_size;

    // RIFF Header
    file.write_all(b"RIFF").unwrap();
    file.write_all(&chunk_size.to_le_bytes()).unwrap();
    file.write_all(b"WAVE").unwrap();

    // fmt subchunk
    file.write_all(b"fmt ").unwrap();
    file.write_all(&16u32.to_le_bytes()).unwrap(); // Subchunk1Size (16 for PCM)
    file.write_all(&1u16.to_le_bytes()).unwrap();  // AudioFormat (1 = PCM)
    file.write_all(&num_channels.to_le_bytes()).unwrap();
    file.write_all(&sample_rate.to_le_bytes()).unwrap();
    file.write_all(&byte_rate.to_le_bytes()).unwrap();
    file.write_all(&block_align.to_le_bytes()).unwrap();
    file.write_all(&bits_per_sample.to_le_bytes()).unwrap();

    // data subchunk
    file.write_all(b"data").unwrap();
    file.write_all(&subchunk2_size.to_le_bytes()).unwrap();

    for i in 0..num_samples {
        let t = (i as f32) / (sample_rate as f32);
        let sample_val = ((t * 440.0 * 2.0 * std::f32::consts::PI).sin() * 30000.0) as i16;
        file.write_all(&sample_val.to_le_bytes()).unwrap(); // Left
        file.write_all(&sample_val.to_le_bytes()).unwrap(); // Right
    }
}

#[test]
fn test_decode_valid_wav_file() {
    let temp_dir = std::env::temp_dir();
    let wav_path = temp_dir.join("test_valid.wav");
    create_test_wav(&wav_path, 44100, 44100);

    let mut source = Source::open(&wav_path).expect("Failed to open valid wav file");
    assert_eq!(source.sample_rate(), 44100);
    assert_eq!(source.channels(), 2);

    let mut total_samples = 0;
    while let Ok(Some(packet)) = source.next_packet_stereo() {
        total_samples += packet.len();
    }

    assert!(total_samples > 0);

    // Test seeking back to beginning
    source.seek(0.0).expect("Seek failed");
    let packet_after_seek = source.next_packet_stereo().expect("Decode after seek failed");
    assert!(packet_after_seek.is_some());

    let _ = std::fs::remove_file(wav_path);
}

#[test]
fn test_corrupt_and_truncated_files() {
    let temp_dir = std::env::temp_dir();

    // 1. Non-existent file
    let missing_path = temp_dir.join("non_existent_audio_file.wav");
    match Source::open(&missing_path) {
        Err(AudioError::FileNotFound(_)) => (),
        Err(e) => panic!("Expected FileNotFound, got {:?}", e),
        Ok(_) => panic!("Expected error for non-existent file"),
    }

    // 2. Zero-byte file
    let empty_path = temp_dir.join("empty.wav");
    File::create(&empty_path).unwrap();
    match Source::open(&empty_path) {
        Err(AudioError::UnsupportedFormat(_)) => (),
        Err(e) => panic!("Expected UnsupportedFormat on empty file, got {:?}", e),
        Ok(_) => panic!("Expected error for empty file"),
    }
    let _ = std::fs::remove_file(empty_path);

    // 3. Garbage / corrupted file
    let corrupt_path = temp_dir.join("corrupt.wav");
    {
        let mut f = File::create(&corrupt_path).unwrap();
        f.write_all(b"RIFF\x24\x00\x00\x00WAVEfmt \x10\x00\x00\x00random_garbage_data_here_not_valid").unwrap();
    }
    let res = Source::open(&corrupt_path);
    assert!(res.is_err(), "Opening corrupt file must return error");
    let _ = std::fs::remove_file(corrupt_path);
}
