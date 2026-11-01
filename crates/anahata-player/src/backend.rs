use std::path::Path;

use anahata_audio::{AudioEngine, AudioResult, EngineEvent};

/// What the player needs from an audio engine. `AudioEngine` implements it; tests use a fake.
pub trait AudioBackend {
    /// Starts `path` from the beginning. On error the previous track must be left untouched.
    fn play_file(&mut self, path: &Path) -> AudioResult<()>;
    fn pause(&mut self) -> AudioResult<()>;
    fn resume(&mut self) -> AudioResult<()>;
    fn stop(&mut self) -> AudioResult<()>;
    fn seek(&mut self, time_seconds: f64) -> AudioResult<()>;
    fn set_volume(&mut self, volume: f32) -> AudioResult<()>;
    fn poll_event(&mut self) -> Option<EngineEvent>;
    fn position_secs(&self) -> f64;
    fn duration_secs(&self) -> Option<f64>;
    fn has_current_file(&self) -> bool;
}

impl AudioBackend for AudioEngine {
    fn play_file(&mut self, path: &Path) -> AudioResult<()> {
        AudioEngine::play_file(self, path)
    }
    fn pause(&mut self) -> AudioResult<()> {
        AudioEngine::pause(self)
    }
    fn resume(&mut self) -> AudioResult<()> {
        AudioEngine::resume(self)
    }
    fn stop(&mut self) -> AudioResult<()> {
        AudioEngine::stop(self)
    }
    fn seek(&mut self, time_seconds: f64) -> AudioResult<()> {
        AudioEngine::seek(self, time_seconds)
    }
    fn set_volume(&mut self, volume: f32) -> AudioResult<()> {
        AudioEngine::set_volume(self, volume)
    }
    fn poll_event(&mut self) -> Option<EngineEvent> {
        AudioEngine::poll_event(self)
    }
    fn position_secs(&self) -> f64 {
        AudioEngine::position_secs(self)
    }
    fn duration_secs(&self) -> Option<f64> {
        AudioEngine::duration_secs(self)
    }
    fn has_current_file(&self) -> bool {
        self.current_file().is_some()
    }
}
