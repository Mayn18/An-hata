use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use rtrb::{Consumer, Producer};

use crate::error::{AudioError, AudioResult};
use crate::resample::StereoResampler;
use crate::ring::AudioRingBuffer;
use crate::source::Source;

pub enum VoiceWorkerCommand {
    Stop,
}

/// State shared between the decode worker, the audio callback and the engine.
///
/// `sample_rate` is the **output** rate: `frames_played` and `seek_base_frames`
/// are both counted in output frames, so position = (base + played) / sample_rate.
pub struct VoiceSharedState {
    pub frames_played: AtomicU64,
    pub seek_base_frames: AtomicU64,
    /// The worker has written every sample of the file (after resampling) into the ring.
    pub eof: AtomicBool,
    /// The worker stopped because of a decode error or panic. See `failure_message`.
    pub failed: AtomicBool,
    /// Set by the mixer once the ring has been fully drained after `eof` or `failed`.
    pub finished: AtomicBool,
    pub prime_ready: AtomicBool,
    pub sample_rate: u32,
    pub duration_secs: Option<f64>,
    failure: Mutex<Option<String>>,
}

impl VoiceSharedState {
    pub fn failure_message(&self) -> Option<String> {
        self.failure.lock().ok().and_then(|g| g.clone())
    }

    fn fail(&self, message: String) {
        log::error!("Voice decode failure: {message}");
        if let Ok(mut g) = self.failure.lock() {
            *g = Some(message);
        }
        self.failed.store(true, Ordering::SeqCst);
    }
}

pub struct Voice {
    pub id: u64,
    pub path: PathBuf,
    pub shared: Arc<VoiceSharedState>,
    pub consumer: Option<Consumer<f32>>,
    command_tx: Sender<VoiceWorkerCommand>,
    worker_handle: Option<JoinHandle<()>>,
}

impl Voice {
    /// Spawns a decode worker for `path`, starting at `start_secs`, producing stereo f32
    /// at `output_rate`. Seeking is done by spawning a fresh voice, so no stale audio
    /// can survive in the ring.
    pub fn spawn<P: AsRef<Path>>(
        id: u64,
        path: P,
        output_rate: u32,
        start_secs: f64,
    ) -> AudioResult<Self> {
        let path_buf = path.as_ref().to_path_buf();
        let mut source = Source::open(&path_buf)?;
        let output_rate = output_rate.max(1);

        let duration_secs = source.duration_secs();
        let start_secs = match duration_secs {
            Some(d) => start_secs.clamp(0.0, d),
            None => start_secs.max(0.0),
        };
        if start_secs > 0.0 {
            source.seek(start_secs)?;
        }

        // ~2 seconds of stereo f32 at the output rate.
        let ring_capacity = (output_rate as usize * 2) * 2;
        let (producer, consumer) = AudioRingBuffer::new(ring_capacity);

        let shared = Arc::new(VoiceSharedState {
            frames_played: AtomicU64::new(0),
            seek_base_frames: AtomicU64::new((start_secs * output_rate as f64) as u64),
            eof: AtomicBool::new(false),
            failed: AtomicBool::new(false),
            finished: AtomicBool::new(false),
            prime_ready: AtomicBool::new(false),
            sample_rate: output_rate,
            duration_secs,
            failure: Mutex::new(None),
        });

        let (command_tx, command_rx) = mpsc::channel();
        let shared_clone = Arc::clone(&shared);
        let resampler = StereoResampler::new(source.sample_rate(), output_rate);

        let worker_handle = thread::Builder::new()
            .name(format!("voice-worker-{id}"))
            .spawn(move || {
                Self::worker_main(source, resampler, producer, ring_capacity, command_rx, shared_clone);
            })
            .map_err(|e| AudioError::Internal(format!("Failed to spawn voice worker thread: {e}")))?;

        Ok(Self {
            id,
            path: path_buf,
            shared,
            consumer: Some(consumer),
            command_tx,
            worker_handle: Some(worker_handle),
        })
    }

    pub fn frames_played(&self) -> u64 {
        self.shared.frames_played.load(Ordering::Relaxed)
    }

    pub fn is_eof(&self) -> bool {
        self.shared.eof.load(Ordering::Acquire)
    }

    pub fn is_failed(&self) -> bool {
        self.shared.failed.load(Ordering::Acquire)
    }

    /// True once the mixer has played out every buffered sample after end of file or failure.
    pub fn is_finished(&self) -> bool {
        self.shared.finished.load(Ordering::Acquire)
    }

    /// Runs the decode loop and converts errors and panics into the `failed` flag.
    fn worker_main(
        source: Source,
        resampler: StereoResampler,
        producer: Producer<f32>,
        ring_capacity: usize,
        command_rx: Receiver<VoiceWorkerCommand>,
        shared: Arc<VoiceSharedState>,
    ) {
        let outcome = catch_unwind(AssertUnwindSafe(|| {
            Self::decode_loop(source, resampler, producer, ring_capacity, &command_rx, &shared)
        }));
        match outcome {
            Ok(Ok(())) => {}
            Ok(Err(message)) => shared.fail(message),
            Err(_) => shared.fail("decoder panicked".to_string()),
        }
    }

    fn decode_loop(
        mut source: Source,
        mut resampler: StereoResampler,
        mut producer: Producer<f32>,
        ring_capacity: usize,
        command_rx: &Receiver<VoiceWorkerCommand>,
        shared: &Arc<VoiceSharedState>,
    ) -> Result<(), String> {
        let prime_threshold = (shared.sample_rate as usize / 4) * 2; // 250 ms of stereo
        let mut resampled: Vec<f32> = Vec::new();

        loop {
            if producer.slots() < 1024 {
                match command_rx.try_recv() {
                    Ok(VoiceWorkerCommand::Stop) | Err(TryRecvError::Disconnected) => return Ok(()),
                    Err(TryRecvError::Empty) => {}
                }
                if producer.is_abandoned() {
                    return Ok(());
                }
                thread::sleep(Duration::from_millis(5));
                continue;
            }

            match source.next_packet_stereo() {
                Ok(Some(samples)) => {
                    let flow = if resampler.is_passthrough() {
                        push_samples(&mut producer, samples, command_rx)
                    } else {
                        resampled.clear();
                        resampler.process(samples, &mut resampled);
                        push_samples(&mut producer, &resampled, command_rx)
                    };
                    if flow == Flow::Stop {
                        return Ok(());
                    }
                    if !shared.prime_ready.load(Ordering::Relaxed) {
                        let buffered = ring_capacity.saturating_sub(producer.slots());
                        if buffered >= prime_threshold {
                            shared.prime_ready.store(true, Ordering::SeqCst);
                        }
                    }
                }
                Ok(None) => {
                    resampled.clear();
                    resampler.flush(&mut resampled);
                    if push_samples(&mut producer, &resampled, command_rx) == Flow::Stop {
                        return Ok(());
                    }
                    shared.prime_ready.store(true, Ordering::SeqCst);
                    // Release: everything written above is visible to whoever sees eof.
                    shared.eof.store(true, Ordering::Release);
                    return Ok(());
                }
                Err(e) => return Err(e.to_string()),
            }
        }
    }
}

#[derive(PartialEq, Eq)]
enum Flow {
    Continue,
    Stop,
}

/// Writes all of `samples` into the ring, waiting for the consumer to make room.
/// Returns `Flow::Stop` if the voice was told to stop or the consumer went away.
fn push_samples(
    producer: &mut Producer<f32>,
    samples: &[f32],
    command_rx: &Receiver<VoiceWorkerCommand>,
) -> Flow {
    let mut offset = 0;
    while offset < samples.len() {
        let to_write = (samples.len() - offset).min(producer.slots());
        if to_write > 0 {
            if let Ok(mut chunk) = producer.write_chunk(to_write) {
                let (first, second) = chunk.as_mut_slices();
                let n1 = first.len();
                first.copy_from_slice(&samples[offset..offset + n1]);
                if !second.is_empty() {
                    second.copy_from_slice(&samples[offset + n1..offset + to_write]);
                }
                chunk.commit_all();
                offset += to_write;
                continue;
            }
        }
        if producer.is_abandoned() {
            return Flow::Stop;
        }
        match command_rx.try_recv() {
            Ok(VoiceWorkerCommand::Stop) | Err(TryRecvError::Disconnected) => return Flow::Stop,
            Err(TryRecvError::Empty) => {}
        }
        thread::sleep(Duration::from_millis(5));
    }
    Flow::Continue
}

impl Drop for Voice {
    fn drop(&mut self) {
        let _ = self.command_tx.send(VoiceWorkerCommand::Stop);
        if let Some(handle) = self.worker_handle.take() {
            let _ = handle.join();
        }
    }
}
