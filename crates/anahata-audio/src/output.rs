use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{BufferSize, OutputCallbackInfo, SampleFormat, SampleRate, Stream, StreamConfig, SupportedStreamConfig};

use crate::error::{AudioError, AudioResult};
use crate::mixer::{ActiveVoice, Mixer};

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

        let underruns_clone = Arc::clone(&underrun_counter);
        let playing_clone = Arc::clone(&is_playing_state);

        let worker_handle = thread::Builder::new()
            .name("cpal-output-runner".to_string())
            .spawn(move || {
                Self::output_runner(device, config, supported_config.sample_format(), cmd_rx, underruns_clone, playing_clone);
            })
            .map_err(|e| AudioError::Internal(format!("Failed to spawn audio output thread: {e}")))?;

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
    ) {
        let (mixer_tx, mixer_rx) = rtrb::RingBuffer::<AudioOutputCommand>::new(64);
        let mut mixer = Mixer::new(underruns, playing_state);

        let err_fn = |err| {
            log::error!("CPAL audio stream error: {err}");
        };

        let mut mixer_consumer = mixer_rx;

        // Build stream depending on native sample format
        let stream_res: Result<Stream, cpal::BuildStreamError> = match sample_format {
            SampleFormat::F32 => {
                device.build_output_stream(
                    &config,
                    move |data: &mut [f32], _: &OutputCallbackInfo| {
                        while let Ok(cmd) = mixer_consumer.pop() {
                            match cmd {
                                AudioOutputCommand::SetVoice(v) => mixer.set_voice(v),
                                AudioOutputCommand::SetVolume(vol) => mixer.set_volume(vol),
                                AudioOutputCommand::Pause => mixer.pause(),
                                AudioOutputCommand::Resume => mixer.resume(),
                                AudioOutputCommand::Stop => mixer.stop(),
                                AudioOutputCommand::Shutdown => mixer.stop(),
                            }
                        }
                        mixer.render(data);
                    },
                    err_fn,
                    None,
                )
            }
            SampleFormat::I16 => {
                let mut scratch_f32 = Vec::new();
                device.build_output_stream(
                    &config,
                    move |data: &mut [i16], _: &OutputCallbackInfo| {
                        while let Ok(cmd) = mixer_consumer.pop() {
                            match cmd {
                                AudioOutputCommand::SetVoice(v) => mixer.set_voice(v),
                                AudioOutputCommand::SetVolume(vol) => mixer.set_volume(vol),
                                AudioOutputCommand::Pause => mixer.pause(),
                                AudioOutputCommand::Resume => mixer.resume(),
                                AudioOutputCommand::Stop => mixer.stop(),
                                AudioOutputCommand::Shutdown => mixer.stop(),
                            }
                        }
                        if scratch_f32.len() != data.len() {
                            scratch_f32.resize(data.len(), 0.0);
                        }
                        mixer.render(&mut scratch_f32);
                        for (dest, &src) in data.iter_mut().zip(scratch_f32.iter()) {
                            *dest = (src * 32767.0).clamp(-32768.0, 32767.0) as i16;
                        }
                    },
                    err_fn,
                    None,
                )
            }
            SampleFormat::U16 => {
                let mut scratch_f32 = Vec::new();
                device.build_output_stream(
                    &config,
                    move |data: &mut [u16], _: &OutputCallbackInfo| {
                        while let Ok(cmd) = mixer_consumer.pop() {
                            match cmd {
                                AudioOutputCommand::SetVoice(v) => mixer.set_voice(v),
                                AudioOutputCommand::SetVolume(vol) => mixer.set_volume(vol),
                                AudioOutputCommand::Pause => mixer.pause(),
                                AudioOutputCommand::Resume => mixer.resume(),
                                AudioOutputCommand::Stop => mixer.stop(),
                                AudioOutputCommand::Shutdown => mixer.stop(),
                            }
                        }
                        if scratch_f32.len() != data.len() {
                            scratch_f32.resize(data.len(), 0.0);
                        }
                        mixer.render(&mut scratch_f32);
                        for (dest, &src) in data.iter_mut().zip(scratch_f32.iter()) {
                            *dest = ((src * 0.5 + 0.5) * 65535.0).clamp(0.0, 65535.0) as u16;
                        }
                    },
                    err_fn,
                    None,
                )
            }
            _ => {
                log::error!("Unsupported cpal sample format: {:?}", sample_format);
                return;
            }
        };

        let stream = match stream_res {
            Ok(s) => s,
            Err(e) => {
                log::error!("Failed to build CPAL output stream: {e}");
                return;
            }
        };

        if let Err(e) = stream.play() {
            log::error!("Failed to play CPAL stream: {e}");
            return;
        }

        let mut mixer_producer = mixer_tx;

        while let Ok(cmd) = cmd_rx.recv() {
            match cmd {
                AudioOutputCommand::Shutdown => {
                    let _ = mixer_producer.push(AudioOutputCommand::Shutdown);
                    let _ = stream.pause();
                    break;
                }
                mut other => {
                    // Forward command to the audio callback via lock-free queue
                    loop {
                        match mixer_producer.push(other) {
                            Ok(()) => break,
                            Err(rtrb::PushError::Full(returned)) => {
                                other = returned;
                                thread::sleep(Duration::from_millis(1));
                            }
                        }
                    }
                }
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
