use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;

use rtrb::{Consumer, Producer, RingBuffer};

use crate::voice::VoiceSharedState;

/// Capacity of the queue that carries finished voices off the audio thread.
const RETIRE_QUEUE: usize = 16;
/// Time to ramp master volume from 0 to 1.
const VOLUME_RAMP_SECS: f32 = 0.05;
/// Time to fade playback in or out on pause, resume and voice change.
const TRANSPORT_RAMP_SECS: f32 = 0.02;

pub struct ActiveVoice {
    pub id: u64,
    pub consumer: Consumer<f32>,
    pub shared: Arc<VoiceSharedState>,
}

/// Renders the active voice into the output buffer. Everything in `render` and the
/// methods the callback calls is allocation-free, lock-free and never frees memory:
/// replaced voices are pushed to a retire queue and dropped on another thread.
pub struct Mixer {
    active_voice: Option<ActiveVoice>,
    retire_tx: Producer<ActiveVoice>,
    volume: f32,
    target_volume: f32,
    volume_step: f32,
    is_paused: bool,
    transport_gain: f32,
    transport_step: f32,
    underrun_counter: Arc<AtomicU32>,
    is_playing_state: Arc<AtomicBool>,
}

impl Mixer {
    /// Returns the mixer and the consumer end of its retire queue. The owner of the
    /// consumer must drain it regularly from a non-real-time thread.
    pub fn new(
        underrun_counter: Arc<AtomicU32>,
        is_playing_state: Arc<AtomicBool>,
        sample_rate: u32,
    ) -> (Self, Consumer<ActiveVoice>) {
        let (retire_tx, retire_rx) = RingBuffer::<ActiveVoice>::new(RETIRE_QUEUE);
        let rate = sample_rate.max(1) as f32;
        let mixer = Self {
            active_voice: None,
            retire_tx,
            volume: 1.0,
            target_volume: 1.0,
            volume_step: 1.0 / (rate * VOLUME_RAMP_SECS),
            is_paused: false,
            transport_gain: 0.0,
            transport_step: 1.0 / (rate * TRANSPORT_RAMP_SECS),
            underrun_counter,
            is_playing_state,
        };
        (mixer, retire_rx)
    }

    /// Hands a replaced voice to the retire queue. If the queue is full (the owner
    /// stopped draining it) the voice is leaked rather than freed on this thread.
    fn retire(&mut self, voice: ActiveVoice) {
        if let Err(rtrb::PushError::Full(v)) = self.retire_tx.push(voice) {
            std::mem::forget(v);
        }
    }

    pub fn set_voice(&mut self, voice: Option<ActiveVoice>) {
        if let Some(old) = std::mem::replace(&mut self.active_voice, voice) {
            self.retire(old);
        }
        // Fade the new voice in from silence so a swap never clicks.
        self.transport_gain = 0.0;
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
        if let Some(old) = self.active_voice.take() {
            self.retire(old);
        }
        self.is_paused = false;
        self.update_playing_state();
    }

    pub fn volume(&self) -> f32 {
        self.volume
    }

    fn update_playing_state(&mut self) {
        let finished = self
            .active_voice
            .as_ref()
            .map(|v| v.shared.finished.load(Ordering::Acquire))
            .unwrap_or(true);
        let playing = self.active_voice.is_some() && !self.is_paused && !finished;
        self.is_playing_state.store(playing, Ordering::Relaxed);
    }

    /// Real-time render callback: interleaved stereo f32 in, nothing allocated.
    pub fn render(&mut self, output: &mut [f32]) {
        // Paused and fully faded out, or nothing to play: output silence and leave the ring alone.
        let audible = self.active_voice.is_some() && (!self.is_paused || self.transport_gain > 0.0);
        if !audible {
            output.fill(0.0);
            self.volume = self.target_volume;
            return;
        }

        let voice = self.active_voice.as_mut().unwrap();
        // Read `eof`/`failed` BEFORE the ring length. The worker publishes them with
        // Release after its final write, so if we see them set, the ring length below
        // already includes every sample the file will ever produce.
        let ended = voice.shared.eof.load(Ordering::Acquire) || voice.shared.failed.load(Ordering::Acquire);
        let available = voice.consumer.slots();
        let to_read = output.len().min(available) & !1;

        let mut read = 0;
        if to_read > 0 {
            if let Ok(chunk) = voice.consumer.read_chunk(to_read) {
                let (first, second) = chunk.as_slices();
                let n1 = first.len();
                output[..n1].copy_from_slice(first);
                output[n1..n1 + second.len()].copy_from_slice(second);
                read = n1 + second.len();
                chunk.commit_all();
            }
        }
        output[read..].fill(0.0);

        let drained = available - to_read < 2;
        if read < output.len() {
            if ended && drained {
                if !voice.shared.finished.swap(true, Ordering::AcqRel) {
                    self.is_playing_state.store(false, Ordering::Relaxed);
                }
            } else if !ended {
                self.underrun_counter.fetch_add(1, Ordering::Relaxed);
            }
        }
        voice.shared.frames_played.fetch_add((read / 2) as u64, Ordering::Relaxed);

        // Per-frame gain: master volume ramp times the pause/resume fade.
        let transport_target = if self.is_paused { 0.0 } else { 1.0 };
        let real_frames = read / 2;
        for (idx, frame) in output.chunks_exact_mut(2).enumerate() {
            if self.volume != self.target_volume {
                let d = self.target_volume - self.volume;
                self.volume = if d.abs() <= self.volume_step {
                    self.target_volume
                } else {
                    self.volume + self.volume_step.copysign(d)
                };
            }
            // The pause/resume/start fade only advances over real audio, so a start that
            // begins with a few underrun frames still fades in from silence. Fading out
            // for a pause always advances so it can never get stuck.
            let ramp_here = idx < real_frames || self.is_paused;
            if ramp_here && self.transport_gain != transport_target {
                let d = transport_target - self.transport_gain;
                self.transport_gain = if d.abs() <= self.transport_step {
                    transport_target
                } else {
                    self.transport_gain + self.transport_step.copysign(d)
                };
            }
            let g = self.volume * self.transport_gain;
            frame[0] = (frame[0] * g).clamp(-1.0, 1.0);
            frame[1] = (frame[1] * g).clamp(-1.0, 1.0);
        }
    }
}
