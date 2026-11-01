mod common;

use std::sync::atomic::Ordering;

use common::*;

fn tmp(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("anahata_mixer_{}_{name}", std::process::id()))
}

#[test]
fn track_ends_and_is_reported_finished() {
    let path = tmp("end.wav");
    sine_wav(&path, 44_100, 44_100, 440.0);
    let mut r = rig(&path, 44_100, 0.0);

    let out = render_to_end(&mut r, 512);
    assert!(r.voice.is_finished());
    assert!(r.voice.is_eof());
    assert!(!r.voice.is_failed());
    assert_eq!(r.voice.frames_played(), 44_100, "every frame of the file must be played exactly once");
    assert!(out.len() >= 44_100 * 2);
    let _ = std::fs::remove_file(path);
}

#[test]
fn mismatched_rate_keeps_duration_and_pitch() {
    // A 1 s, 440 Hz file at 44.1 kHz played on a 48 kHz device must still last 1 s and stay at 440 Hz.
    let path = tmp("rate.wav");
    sine_wav(&path, 44_100, 44_100, 440.0);
    let mut r = rig(&path, 48_000, 0.0);

    let out = render_to_end(&mut r, 480);
    let played = r.voice.frames_played() as i64;
    assert!((played - 48_000).abs() <= 8, "played {played} frames, expected ~48000");

    // Skip any leading underrun silence before the worker had primed the ring.
    let first = out.chunks_exact(2).position(|f| f[0] != 0.0).expect("no audio rendered");
    let left: Vec<f32> = out.chunks_exact(2).skip(first).take(played as usize).map(|f| f[0]).collect();
    let cycles = left.windows(2).filter(|w| w[0] < 0.0 && w[1] >= 0.0).count();
    assert!((cycles as i64 - 440).abs() <= 3, "got {cycles} cycles, expected ~440");
    let _ = std::fs::remove_file(path);
}

#[test]
fn starting_mid_file_reports_position_and_plays_the_rest() {
    let path = tmp("seek.wav");
    sine_wav(&path, 48_000 * 2, 48_000, 440.0);
    let mut r = rig(&path, 48_000, 1.0);

    assert_eq!(r.voice.shared.seek_base_frames.load(Ordering::Relaxed), 48_000);
    render_to_end(&mut r, 512);
    let played = r.voice.frames_played() as i64;
    assert!((played - 48_000).abs() <= 2_048, "played {played}, expected about the second half");
    let _ = std::fs::remove_file(path);
}

#[test]
fn seek_never_replays_old_buffered_audio() {
    // First half of the file is +0.5, second half is -0.5. A voice started at 1.5 s must never emit positive samples.
    let path = tmp("stale.wav");
    write_wav(&path, 96_000, 48_000, |i| if i < 48_000 { 16384 } else { -16384 });
    let mut r = rig(&path, 48_000, 1.5);

    let out = render_to_end(&mut r, 512);
    let positives = out.iter().filter(|&&s| s > 0.01).count();
    assert_eq!(positives, 0, "stale audio from before the seek point leaked through");
    let _ = std::fs::remove_file(path);
}

#[test]
fn start_fades_in_and_reaches_full_level() {
    let path = tmp("fade.wav");
    dc_wav(&path, 48_000, 48_000, 16384);
    let mut r = rig(&path, 48_000, 0.0);

    let out = render_frames(&mut r, 4_000, 256);
    let left: Vec<f32> = out.chunks_exact(2).map(|f| f[0]).collect();
    // Skip leading silence from underruns before the worker primed the ring.
    let first = left.iter().position(|&s| s > 0.0).expect("no audio");
    assert!(left[first] < 0.05, "should start near silence, got {}", left[first]);
    let last = *left.last().unwrap();
    assert!((last - 0.5).abs() < 0.01, "should reach full level (0.5), got {last}");
    let _ = std::fs::remove_file(path);
}

#[test]
fn volume_changes_ramp_per_sample_not_per_callback() {
    let path = tmp("vol.wav");
    dc_wav(&path, 96_000, 48_000, 16384);
    let mut r = rig(&path, 48_000, 0.0);
    render_frames(&mut r, 4_000, 256); // reach steady state

    r.mixer.set_volume(0.0);
    // 50 ms ramp = 2400 frames at 48 kHz. After 3000 more frames the level must be exactly zero...
    let out = render_frames(&mut r, 4_000 + 3_000, 256);
    let left: Vec<f32> = out.chunks_exact(2).map(|f| f[0]).collect();
    assert_eq!(*left.last().unwrap(), 0.0);
    // ...and the ramp must be monotonic with no single-sample jump.
    let max_step = left.windows(2).map(|w| (w[1] - w[0]).abs()).fold(0.0f32, f32::max);
    assert!(max_step < 0.001, "volume stepped by {max_step} in one sample");
    let _ = std::fs::remove_file(path);
}

#[test]
fn pause_fades_out_and_stops_consuming_then_resume_continues() {
    let path = tmp("pause.wav");
    dc_wav(&path, 192_000, 48_000, 16384);
    let mut r = rig(&path, 48_000, 0.0);
    render_frames(&mut r, 4_000, 256);

    r.mixer.pause();
    let mut buf = vec![0.0f32; 2_048]; // 1024 frames > 20 ms fade
    r.mixer.render(&mut buf);
    let fade_tail = *buf.chunks_exact(2).last().unwrap().first().unwrap();
    assert_eq!(fade_tail, 0.0, "pause must fade fully to silence");
    let max_step = buf.chunks_exact(2).map(|f| f[0]).collect::<Vec<_>>().windows(2).map(|w| (w[1] - w[0]).abs()).fold(0.0f32, f32::max);
    assert!(max_step < 0.01, "pause was a hard cut ({max_step})");

    let frozen = r.voice.frames_played();
    for _ in 0..10 {
        r.mixer.render(&mut buf);
        assert!(buf.iter().all(|&s| s == 0.0));
    }
    assert_eq!(r.voice.frames_played(), frozen, "paused audio must not consume the ring");

    r.mixer.resume();
    render_frames(&mut r, frozen + 2_000, 256);
    assert!(r.voice.frames_played() > frozen);
    let _ = std::fs::remove_file(path);
}

#[test]
fn replaced_voice_is_retired_not_dropped_on_the_audio_thread() {
    let path = tmp("retire.wav");
    dc_wav(&path, 48_000, 48_000, 1000);
    let mut r = rig(&path, 48_000, 0.0);

    r.mixer.set_voice(None);
    let old = r.retired.pop().expect("old voice must be handed to the retire queue");
    assert_eq!(old.id, r.voice.id);

    // Stop retires too.
    let mut r2 = rig(&path, 48_000, 0.0);
    r2.mixer.stop();
    assert!(r2.retired.pop().is_ok());
    let _ = std::fs::remove_file(path);
}
