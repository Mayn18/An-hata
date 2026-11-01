use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use crate::error::{AudioError, AudioResult};
use crate::mixer::ActiveVoice;
use crate::output::{AudioOutput, AudioOutputCommand};
use crate::voice::Voice;

/// Sample rate used for decoding when no output device is available.
const HEADLESS_RATE: u32 = 48_000;

/// Something the player needs to react to. Poll with [`AudioEngine::poll_event`].
#[derive(Debug, Clone, PartialEq)]
pub enum EngineEvent {
    /// The current track played to its end.
    TrackFinished { voice_id: u64, path: PathBuf },
    /// The current track stopped early because it could not be decoded.
    TrackFailed { voice_id: u64, path: PathBuf, message: String },
}

pub struct AudioEngine {
    output: Option<AudioOutput>,
    current_voice: Option<Voice>,
    current_path: Option<PathBuf>,
    volume: f32,
    voice_id_counter: AtomicU64,
    event_reported: bool,
}

impl AudioEngine {
    /// Initializes the audio engine. Tries to open the default CPAL output device.
    /// If no audio device is found (e.g. headless CI), returns Ok with output = None.
    pub fn new() -> AudioResult<Self> {
        let output = match AudioOutput::start() {
            Ok(out) => Some(out),
            Err(AudioError::DeviceUnavailable(msg)) => {
                log::warn!("Audio device unavailable: {msg}. Running in headless/silent mode.");
                None
            }
            Err(e) => return Err(e),
        };

        Ok(Self {
            output,
            current_voice: None,
            current_path: None,
            volume: 1.0,
            voice_id_counter: AtomicU64::new(1),
            event_reported: false,
        })
    }

    fn output_rate(&self) -> u32 {
        self.output.as_ref().map(|o| o.sample_rate()).unwrap_or(HEADLESS_RATE)
    }

    fn next_voice_id(&self) -> u64 {
        self.voice_id_counter.fetch_add(1, Ordering::Relaxed)
    }

    /// Spawns a voice for `path` at `start_secs` and swaps it in. The previous voice is
    /// dropped only after the new one is installed. Paused/playing state is left alone.
    fn install_voice(&mut self, path: &Path, start_secs: f64) -> AudioResult<()> {
        let mut voice = Voice::spawn(self.next_voice_id(), path, self.output_rate(), start_secs)?;

        if let Some(ref output) = self.output {
            if let Some(consumer) = voice.consumer.take() {
                output.send_command(AudioOutputCommand::SetVoice(Some(ActiveVoice {
                    id: voice.id,
                    consumer,
                    shared: Arc::clone(&voice.shared),
                })))?;
            }
        }

        self.current_path = Some(path.to_path_buf());
        self.current_voice = Some(voice);
        self.event_reported = false;
        Ok(())
    }

    /// Plays a local audio file from the beginning.
    ///
    /// On error the previous track is left untouched, so the caller can decide what to do.
    pub fn play_file<P: AsRef<Path>>(&mut self, path: P) -> AudioResult<()> {
        self.install_voice(path.as_ref(), 0.0)?;
        if let Some(ref output) = self.output {
            output.send_command(AudioOutputCommand::Resume)?;
        }
        Ok(())
    }

    /// Pauses playback.
    pub fn pause(&mut self) -> AudioResult<()> {
        if let Some(ref output) = self.output {
            output.send_command(AudioOutputCommand::Pause)?;
        }
        Ok(())
    }

    /// Resumes playback.
    pub fn resume(&mut self) -> AudioResult<()> {
        if let Some(ref output) = self.output {
            output.send_command(AudioOutputCommand::Resume)?;
        }
        Ok(())
    }

    /// Stops playback and clears the active voice.
    pub fn stop(&mut self) -> AudioResult<()> {
        if let Some(ref output) = self.output {
            output.send_command(AudioOutputCommand::Stop)?;
        }
        self.current_voice = None;
        self.current_path = None;
        self.event_reported = false;
        Ok(())
    }

    /// Seeks by starting a fresh voice at the target time, so none of the old position's
    /// buffered audio can leak through. Keeps the current paused/playing state.
    pub fn seek(&mut self, time_seconds: f64) -> AudioResult<()> {
        let Some(path) = self.current_path.clone() else {
            return Ok(());
        };
        self.install_voice(&path, time_seconds)
    }

    /// Sets playback volume (0.0 to 1.0).
    pub fn set_volume(&mut self, volume: f32) -> AudioResult<()> {
        self.volume = volume.clamp(0.0, 1.0);
        if let Some(ref output) = self.output {
            output.send_command(AudioOutputCommand::SetVolume(self.volume))?;
        }
        Ok(())
    }

    /// Returns the next pending event, at most once per track.
    pub fn poll_event(&mut self) -> Option<EngineEvent> {
        if self.event_reported {
            return None;
        }
        let voice = self.current_voice.as_ref()?;
        if !voice.is_finished() {
            return None;
        }
        self.event_reported = true;
        let path = voice.path.clone();
        let voice_id = voice.id;
        if voice.is_failed() {
            let message = voice.shared.failure_message().unwrap_or_else(|| "decode failed".to_string());
            Some(EngineEvent::TrackFailed { voice_id, path, message })
        } else {
            Some(EngineEvent::TrackFinished { voice_id, path })
        }
    }

    /// Returns the currently playing file path, if any.
    pub fn current_file(&self) -> Option<&Path> {
        self.current_path.as_deref()
    }

    /// Returns current playback position in seconds.
    pub fn position_secs(&self) -> f64 {
        match self.current_voice {
            Some(ref voice) => {
                let base = voice.shared.seek_base_frames.load(Ordering::Relaxed);
                let played = voice.frames_played();
                let rate = voice.shared.sample_rate.max(1);
                let pos = (base + played) as f64 / rate as f64;
                match voice.shared.duration_secs {
                    Some(d) => pos.min(d),
                    None => pos,
                }
            }
            None => 0.0,
        }
    }

    /// Returns total track duration in seconds if known.
    pub fn duration_secs(&self) -> Option<f64> {
        self.current_voice.as_ref().and_then(|v| v.shared.duration_secs)
    }

    /// Returns true if currently playing.
    pub fn is_playing(&self) -> bool {
        self.output.as_ref().map(|o| o.is_playing()).unwrap_or(false)
    }

    /// Returns true when an output device is open (false in headless mode).
    pub fn has_output(&self) -> bool {
        self.output.is_some()
    }

    /// Returns total count of buffer underruns.
    pub fn underruns(&self) -> u32 {
        self.output.as_ref().map(|o| o.underruns()).unwrap_or(0)
    }

    /// Returns output device name if active.
    pub fn device_name(&self) -> Option<&str> {
        self.output.as_ref().map(|o| o.device_name())
    }

    /// Returns the output sample rate decoded audio is converted to.
    pub fn sample_rate(&self) -> u32 {
        self.output_rate()
    }
}
