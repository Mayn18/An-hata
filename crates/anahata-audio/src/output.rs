use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{
    BufferSize, BuildStreamError, FromSample, OutputCallbackInfo, SampleFormat, SampleRate, SizedSample, Stream,
    StreamConfig, SupportedStreamConfig,
};
use rtrb::Consumer;

use crate::error::{AudioError, AudioResult};
use crate::mixer::{ActiveVoice, Mixer};

/// Samples of scratch space for converting to integer output formats. Larger callbacks
/// are processed in pieces, so the callback never allocates.
const SCRATCH_SAMPLES: usize = 16 * 1024;

pub enum AudioOutputCommand {
    SetVoice(Option<ActiveVoice>),
    SetVolume(f32),
    Pause,
    Resume,
    Stop,
    Shutdown,
}

pub struct AudioOutput {
    cmd_tx: Sender<AudioOutputCommand>,
    underrun_counter: Arc<AtomicU32>,
    is_playing_state: Arc<AtomicBool>,
    device_name: String,
    sample_rate: u32,
    channels: u16,
    worker_handle: Option<JoinHandle<()>>,
}

/// Everything the audio callback owns.
struct CallbackState {
    mixer: Mixer,
    commands: Consumer<AudioOutputCommand>,
    scratch: Vec<f32>,
}

impl CallbackState {
    fn drain_commands(&mut self) {
        while let Ok(cmd) = self.commands.pop() {
            match cmd {
                AudioOutputCommand::SetVoice(v) => self.mixer.set_voice(v),
                AudioOutputCommand::SetVolume(vol) => self.mixer.set_volume(vol),
                AudioOutputCommand::Pause => self.mixer.pause(),
                AudioOutputCommand::Resume => self.mixer.resume(),
                AudioOutputCommand::Stop | AudioOutputCommand::Shutdown => self.mixer.stop(),
            }
        }
    }

    fn fill<T: SizedSample + FromSample<f32>>(&mut self, data: &mut [T]) {
        self.drain_commands();
        let piece = self.scratch.len();
        for chunk in data.chunks_mut(piece) {
            let scratch = &mut self.scratch[..chunk.len()];
            self.mixer.render(scratch);
            for (dest, &src) in chunk.iter_mut().zip(scratch.iter()) {
                *dest = T::from_sample(src);
            }
        }
    }
}

fn build_stream<T>(device: &cpal::Device, config: &StreamConfig, mut state: CallbackState) -> Result<Stream, BuildStreamError>
where
    T: SizedSample + FromSample<f32> + Send + 'static,
{
    device.build_output_stream(
        config,
        move |data: &mut [T], _: &OutputCallbackInfo| state.fill(data),
        |err| log::error!("CPAL audio stream error: {err}"),
        None,
    )
}

impl AudioOutput {
    pub fn start() -> AudioResult<Self> {
        let underrun_counter = Arc::new(AtomicU32::new(0));
        let is_playing_state = Arc::new(AtomicBool::new(false));

        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or_else(|| AudioError::DeviceUnavailable("No default audio output device available".to_string()))?;

        let device_name = device.name().unwrap_or_else(|_| "Unknown Device".to_string());

        let supported_config: SupportedStreamConfig = device
            .default_output_config()
            .map_err(|e| AudioError::DeviceUnavailable(format!("Default output config error: {e}")))?;

        let sample_rate = supported_config.sample_rate().0;
        let channels = 2u16; // We enforce stereo

        let config = StreamConfig {
            channels,
            sample_rate: SampleRate(sample_rate),
            buffer_size: BufferSize::Default,
        };

        let (cmd_tx, cmd_rx) = mpsc::channel::<AudioOutputCommand>();
        let (ready_tx, ready_rx) = mpsc::channel::<Result<(), String>>();

        let underruns_clone = Arc::clone(&underrun_counter);
        let playing_clone = Arc::clone(&is_playing_state);
        let sample_format = supported_config.sample_format();

        let worker_handle = thread::Builder::new()
            .name("cpal-output-runner".to_string())
            .spawn(move || {
                Self::output_runner(device, config, sample_format, cmd_rx, underruns_clone, playing_clone, ready_tx);
            })
            .map_err(|e| AudioError::Internal(format!("Failed to spawn audio output thread: {e}")))?;

        // The stream is built on the runner thread (it may not be Send), so wait for the verdict.
        match ready_rx.recv_timeout(Duration::from_secs(10)) {
            Ok(Ok(())) => {}
            Ok(Err(msg)) => {
                let _ = worker_handle.join();
                return Err(AudioError::DeviceUnavailable(msg));
            }
            Err(_) => {
                return Err(AudioError::Internal("Timed out waiting for the audio stream to start".to_string()));
            }
        }

        Ok(Self {
            cmd_tx,
            underrun_counter,
            is_playing_state,
            device_name,
            sample_rate,
            channels,
            worker_handle: Some(worker_handle),
        })
    }

    pub fn device_name(&self) -> &str {
        &self.device_name
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn channels(&self) -> u16 {
        self.channels
    }

    pub fn is_playing(&self) -> bool {
        self.is_playing_state.load(Ordering::Relaxed)
    }

    pub fn underruns(&self) -> u32 {
        self.underrun_counter.load(Ordering::Relaxed)
    }

    pub fn send_command(&self, cmd: AudioOutputCommand) -> AudioResult<()> {
        self.cmd_tx.send(cmd).map_err(|_| AudioError::ChannelClosed)
    }

    fn output_runner(
        device: cpal::Device,
        config: StreamConfig,
        sample_format: SampleFormat,
        cmd_rx: Receiver<AudioOutputCommand>,
        underruns: Arc<AtomicU32>,
        playing_state: Arc<AtomicBool>,
        ready_tx: Sender<Result<(), String>>,
    ) {
        let (mut mixer_producer, mixer_consumer) = rtrb::RingBuffer::<AudioOutputCommand>::new(64);
        let (mixer, mut retired) = Mixer::new(underruns, playing_state, config.sample_rate.0);
        let state = CallbackState {
            mixer,
            commands: mixer_consumer,
            scratch: vec![0.0; SCRATCH_SAMPLES],
        };

        let stream_res = match sample_format {
            SampleFormat::F32 => build_stream::<f32>(&device, &config, state),
            SampleFormat::I16 => build_stream::<i16>(&device, &config, state),
            SampleFormat::U16 => build_stream::<u16>(&device, &config, state),
            SampleFormat::I32 => build_stream::<i32>(&device, &config, state),
            other => {
                let _ = ready_tx.send(Err(format!("Unsupported cpal sample format: {other:?}")));
                return;
            }
        };

        let stream = match stream_res {
            Ok(s) => s,
            Err(e) => {
                let _ = ready_tx.send(Err(format!("Failed to build output stream: {e}")));
                return;
            }
        };

        if let Err(e) = stream.play() {
            let _ = ready_tx.send(Err(format!("Failed to start output stream: {e}")));
            return;
        }
        let _ = ready_tx.send(Ok(()));

        loop {
            // Voices replaced or stopped in the callback are freed here, never there.
            while let Ok(old) = retired.pop() {
                drop(old);
            }

            match cmd_rx.recv_timeout(Duration::from_millis(50)) {
                Ok(AudioOutputCommand::Shutdown) => {
                    let _ = mixer_producer.push(AudioOutputCommand::Shutdown);
                    let _ = stream.pause();
                    break;
                }
                Ok(mut cmd) => loop {
                    match mixer_producer.push(cmd) {
                        Ok(()) => break,
                        Err(rtrb::PushError::Full(returned)) => {
                            cmd = returned;
                            while let Ok(old) = retired.pop() {
                                drop(old);
                            }
                            thread::sleep(Duration::from_millis(1));
                        }
                    }
                },
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
    }
}

impl Drop for AudioOutput {
    fn drop(&mut self) {
        let _ = self.send_command(AudioOutputCommand::Shutdown);
        if let Some(handle) = self.worker_handle.take() {
            let _ = handle.join();
        }
    }
}
