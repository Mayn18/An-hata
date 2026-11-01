#![allow(dead_code)]
use std::fs::File;
use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU32};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anahata_audio::{ActiveVoice, Mixer, Voice};
use rtrb::Consumer;

/// Writes a 16-bit stereo PCM WAV where frame `i` has value `f(i)` on both channels.
pub fn write_wav<P: AsRef<Path>>(path: P, frames: usize, sample_rate: u32, f: impl Fn(usize) -> i16) {
    let mut file = File::create(path).expect("create wav");
    let channels: u16 = 2;
    let data_len = (frames * channels as usize * 2) as u32;
    file.write_all(b"RIFF").unwrap();
    file.write_all(&(36 + data_len).to_le_bytes()).unwrap();
    file.write_all(b"WAVEfmt ").unwrap();
    file.write_all(&16u32.to_le_bytes()).unwrap();
    file.write_all(&1u16.to_le_bytes()).unwrap();
    file.write_all(&channels.to_le_bytes()).unwrap();
    file.write_all(&sample_rate.to_le_bytes()).unwrap();
    file.write_all(&(sample_rate * channels as u32 * 2).to_le_bytes()).unwrap();
    file.write_all(&(channels * 2).to_le_bytes()).unwrap();
    file.write_all(&16u16.to_le_bytes()).unwrap();
    file.write_all(b"data").unwrap();
    file.write_all(&data_len.to_le_bytes()).unwrap();
    for i in 0..frames {
        let s = f(i).to_le_bytes();
        file.write_all(&s).unwrap();
        file.write_all(&s).unwrap();
    }
}

pub fn sine_wav<P: AsRef<Path>>(path: P, frames: usize, sample_rate: u32, freq: f32) {
    write_wav(path, frames, sample_rate, |i| {
        ((i as f32 / sample_rate as f32 * freq * std::f32::consts::TAU).sin() * 20000.0) as i16
    });
}

pub fn dc_wav<P: AsRef<Path>>(path: P, frames: usize, sample_rate: u32, value: i16) {
    write_wav(path, frames, sample_rate, |_| value);
}

pub struct Rig {
    pub mixer: Mixer,
    pub retired: Consumer<ActiveVoice>,
    pub voice: Voice,
}

/// Builds a Voice for `path` and a Mixer already playing it, with no audio device involved.
pub fn rig(path: &Path, out_rate: u32, start_secs: f64) -> Rig {
    let (mut mixer, retired) = Mixer::new(Arc::new(AtomicU32::new(0)), Arc::new(AtomicBool::new(false)), out_rate);
    let mut voice = Voice::spawn(1, path, out_rate, start_secs).expect("spawn voice");
    let consumer = voice.consumer.take().unwrap();
    mixer.set_voice(Some(ActiveVoice { id: voice.id, consumer, shared: Arc::clone(&voice.shared) }));
    mixer.resume();
    Rig { mixer, retired, voice }
}

/// Renders `block`-sample buffers until the voice reports finished. Returns every sample rendered.
pub fn render_to_end(rig: &mut Rig, block: usize) -> Vec<f32> {
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut all = Vec::new();
    let mut buf = vec![0.0f32; block];
    while !rig.voice.is_finished() {
        assert!(Instant::now() < deadline, "voice never finished");
        rig.mixer.render(&mut buf);
        all.extend_from_slice(&buf);
        std::thread::sleep(Duration::from_micros(200));
    }
    all
}

/// Renders until at least `frames` frames of real audio have been consumed from the voice.
pub fn render_frames(rig: &mut Rig, frames: u64, block: usize) -> Vec<f32> {
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut all = Vec::new();
    let mut buf = vec![0.0f32; block];
    while rig.voice.frames_played() < frames {
        assert!(Instant::now() < deadline, "timed out waiting for audio");
        rig.mixer.render(&mut buf);
        all.extend_from_slice(&buf);
        std::thread::sleep(Duration::from_micros(200));
    }
    all
}
