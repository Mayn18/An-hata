use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use rtrb::Consumer;

use crate::voice::VoiceSharedState;

pub enum MixerCommand {
    SetVolume(f32),
    Pause,
    Resume,
    Stop,
}

pub struct ActiveVoice {
    pub id: u64,
    pub consumer: Consumer<f32>,
    pub shared: Arc<VoiceSharedState>,
}

pub struct Mixer {
    active_voice: Option<ActiveVoice>,
    volume: f32,
    target_volume: f32,
    is_paused: bool,
    gain_ramp: f32,
    underrun_counter: Arc<AtomicU32>,
    is_playing_state: Arc<AtomicBool>,
}

impl Mixer {
    pub fn new(underrun_counter: Arc<AtomicU32>, is_playing_state: Arc<AtomicBool>) -> Self {
        Self {
            active_voice: None,
            volume: 1.0,
            target_volume: 1.0,
            is_paused: false,
            gain_ramp: 1.0,
            underrun_counter,
            is_playing_state,
        }
    }

    pub fn set_voice(&mut self, voice: Option<ActiveVoice>) {
        self.active_voice = voice;
        self.gain_ramp = 1.0;
        self.update_playing_state();
    }

    pub fn set_volume(&mut self, volume: f32) {
        self.target_volume = volume.clamp(0.0, 1.0);
    }

    pub fn pause(&mut self) {
        self.is_paused = true;
        self.update_playing_state();
    }

    pub fn resume(&mut self) {
        self.is_paused = false;
        self.update_playing_state();
    }

    pub fn stop(&mut self) {
        self.active_voice = None;
        self.is_paused = false;
        self.update_playing_state();
    }

    fn update_playing_state(&mut self) {
        let playing = self.active_voice.is_some() && !self.is_paused;
        self.is_playing_state.store(playing, Ordering::Relaxed);
    }

    /// Real-time render callback. Zero allocations, zero locks, zero I/O.
    /// Fills the output slice (interleaved stereo f32 samples) from the active voice ring buffer.
    pub fn render(&mut self, output: &mut [f32]) {
        // Smooth master volume changes
        let vol_step = 0.001f32;
        if (self.volume - self.target_volume).abs() > vol_step {
            if self.volume < self.target_volume {
                self.volume = (self.volume + vol_step).min(self.target_volume);
            } else {
                self.volume = (self.volume - vol_step).max(self.target_volume);
            }
        } else {
            self.volume = self.target_volume;
        }

        // If paused or no voice, handle gain ramp down to silence
        if self.is_paused || self.active_voice.is_none() {
            if self.gain_ramp > 0.0 {
                let ramp_step = 1.0 / (output.len().max(1) as f32);
                for sample in output.iter_mut() {
                    *sample = 0.0;
                    self.gain_ramp = (self.gain_ramp - ramp_step).max(0.0);
                }
            } else {
                output.fill(0.0);
            }
            return;
        }

        // Smooth gain ramp up if resuming
        if self.gain_ramp < 1.0 {
            self.gain_ramp = (self.gain_ramp + 0.01).min(1.0);
        }

        let voice = self.active_voice.as_mut().unwrap();

        let needed_samples = output.len();
        let available = voice.consumer.slots();
        let to_read = needed_samples.min(available);
        let read_samples = if to_read > 0 {
            if let Ok(chunk) = voice.consumer.read_chunk(to_read) {
                let (first, second) = chunk.as_slices();
                let n1 = first.len();
                output[..n1].copy_from_slice(first);
                if !second.is_empty() {
                    output[n1..n1 + second.len()].copy_from_slice(second);
                }
                let total = n1 + second.len();
                chunk.commit_all();
                if total < needed_samples {
                    output[total..].fill(0.0);
                }
                total
            } else {
                output.fill(0.0);
                0
            }
        } else {
            output.fill(0.0);
            0
        };

        if read_samples < needed_samples {
            // Buffer underrun or EOF
            if !voice.shared.eof.load(Ordering::Relaxed) {
                self.underrun_counter.fetch_add(1, Ordering::Relaxed);
            }
        }

        // Apply gain, clamping, and advance frames_played
        let frames_read = (read_samples / 2) as u64;
        voice.shared.frames_played.fetch_add(frames_read, Ordering::Relaxed);

        let final_gain = self.volume * self.gain_ramp;
        for sample in output.iter_mut() {
            *sample = (*sample * final_gain).clamp(-1.0, 1.0);
        }
    }
}
