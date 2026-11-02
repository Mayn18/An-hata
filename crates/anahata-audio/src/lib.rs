//! Anāhata Audio Engine Core

pub mod channels;
pub mod engine;
pub mod error;
pub mod mixer;
pub mod output;
pub mod resample;
pub mod ring;
pub mod source;
pub mod voice;

pub use channels::downmix_to_stereo;
pub use engine::{AudioEngine, EngineEvent};
pub use error::{AudioError, AudioResult};
pub use mixer::{ActiveVoice, Mixer};
pub use output::{AudioOutput, AudioOutputCommand};
pub use resample::StereoResampler;
pub use ring::AudioRingBuffer;
pub use source::Source;
pub use voice::{Voice, VoiceSharedState, VoiceWorkerCommand};
