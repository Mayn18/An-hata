use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use crate::error::{AudioError, AudioResult};
use crate::mixer::ActiveVoice;
use crate::output::{AudioOutput, AudioOutputCommand};
use crate::voice::Voice;

pub struct AudioEngine {
    output: Option<AudioOutput>,
    current_voice: Option<Voice>,
    current_path: Option<PathBuf>,
    volume: f32,
    voice_id_counter: AtomicU64,
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
        })
    }

    /// Plays a local audio file from the beginning.
    pub fn play_file<P: AsRef<Path>>(&mut self, path: P) -> AudioResult<()> {
        let path_ref = path.as_ref();
        let id = self.voice_id_counter.fetch_add(1, Ordering::Relaxed);

        let mut voice = Voice::spawn(id, path_ref)?;

        if let Some(ref output) = self.output {
            if let Some(consumer) = voice.consumer.take() {
                output.send_command(AudioOutputCommand::SetVoice(Some(ActiveVoice {
                    id: voice.id,
                    consumer,
                    shared: Arc::clone(&voice.shared),
                })))?;
                output.send_command(AudioOutputCommand::Resume)?;
            }
        }

        self.current_path = Some(path_ref.to_path_buf());
        self.current_voice = Some(voice);

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
        Ok(())
    }

    /// Seeks to a position in seconds in the currently playing file.
    pub fn seek(&mut self, time_seconds: f64) -> AudioResult<()> {
        if let Some(ref voice) = self.current_voice {
            voice.seek(time_seconds)?;
        }
        Ok(())
    }

    /// Sets playback volume (0.0 to 1.0).
    pub fn set_volume(&mut self, volume: f32) -> AudioResult<()> {
        self.volume = volume.clamp(0.0, 1.0);
        if let Some(ref output) = self.output {
            output.send_command(AudioOutputCommand::SetVolume(self.volume))?;
        }
        Ok(())
    }

    /// Returns the currently playing file path, if any.
    pub fn current_file(&self) -> Option<&Path> {
        self.current_path.as_deref()
    }

    /// Returns current playback position in seconds.
    pub fn position_secs(&self) -> f64 {
        if let Some(ref voice) = self.current_voice {
            let base_frames = voice.shared.seek_base_frames.load(Ordering::Relaxed);
            let frames = voice.frames_played();
            let rate = voice.shared.sample_rate.max(1);
            (base_frames + frames) as f64 / rate as f64
        } else {
            0.0
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

    /// Returns total count of buffer underruns.
    pub fn underruns(&self) -> u32 {
        self.output.as_ref().map(|o| o.underruns()).unwrap_or(0)
    }

    /// Returns output device name if active.
    pub fn device_name(&self) -> Option<&str> {
        self.output.as_ref().map(|o| o.device_name())
    }
}
