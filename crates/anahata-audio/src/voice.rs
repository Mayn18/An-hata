use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use rtrb::{Consumer, Producer};

use crate::error::{AudioError, AudioResult};
use crate::ring::AudioRingBuffer;
use crate::source::Source;

pub enum VoiceWorkerCommand {
    Seek(f64),
    Stop,
}

pub struct VoiceSharedState {
    pub frames_played: AtomicU64,
    pub seek_base_frames: AtomicU64,
    pub eof: AtomicBool,
    pub failed: AtomicBool,
    pub prime_ready: AtomicBool,
    pub sample_rate: u32,
    pub duration_secs: Option<f64>,
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
    /// Creates and spawns a new Voice decode worker for the specified audio file.
    /// Ring buffer capacity is sized for ~2 seconds of stereo audio (e.g. 48000 * 2 * 2 = 192000 samples).
    pub fn spawn<P: AsRef<Path>>(id: u64, path: P) -> AudioResult<Self> {
        let path_buf = path.as_ref().to_path_buf();
        let source = Source::open(&path_buf)?;

        let sample_rate = source.sample_rate();
        let duration_secs = source.duration_secs();

        // 2 seconds buffer for stereo f32
        let ring_capacity = (sample_rate as usize * 2) * 2;
        let (producer, consumer) = AudioRingBuffer::new(ring_capacity);

        let shared = Arc::new(VoiceSharedState {
            frames_played: AtomicU64::new(0),
            seek_base_frames: AtomicU64::new(0),
            eof: AtomicBool::new(false),
            failed: AtomicBool::new(false),
            prime_ready: AtomicBool::new(false),
            sample_rate,
            duration_secs,
        });

        let (command_tx, command_rx) = mpsc::channel();

        let shared_clone = Arc::clone(&shared);
        let path_clone = path_buf.clone();

        let worker_handle = thread::Builder::new()
            .name(format!("voice-worker-{id}"))
            .spawn(move || {
                Self::worker_loop(path_clone, source, producer, ring_capacity, command_rx, shared_clone);
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

    pub fn seek(&self, time_seconds: f64) -> AudioResult<()> {
        self.command_tx
            .send(VoiceWorkerCommand::Seek(time_seconds))
            .map_err(|_| AudioError::ChannelClosed)
    }

    pub fn frames_played(&self) -> u64 {
        self.shared.frames_played.load(Ordering::Relaxed)
    }

    pub fn is_eof(&self) -> bool {
        self.shared.eof.load(Ordering::Relaxed)
    }

    pub fn is_failed(&self) -> bool {
        self.shared.failed.load(Ordering::Relaxed)
    }

    fn worker_loop(
        _path: PathBuf,
        mut source: Source,
        mut producer: Producer<f32>,
        ring_capacity: usize,
        command_rx: Receiver<VoiceWorkerCommand>,
        shared: Arc<VoiceSharedState>,
    ) {
        let mut is_eof = false;
        let prime_threshold_samples = (source.sample_rate() as usize / 4) * 2; // 250ms of stereo audio

        loop {
            // Check commands
            while let Ok(cmd) = command_rx.try_recv() {
                match cmd {
                    VoiceWorkerCommand::Stop => {
                        return;
                    }
                    VoiceWorkerCommand::Seek(time_secs) => {
                        if let Err(e) = source.seek(time_secs) {
                            log::error!("Seek failed in worker: {e}");
                        } else {
                            // Update base seek position in frames and reset frames_played counter
                            let base_frames = (time_secs * source.sample_rate() as f64).max(0.0) as u64;
                            shared.seek_base_frames.store(base_frames, Ordering::SeqCst);
                            shared.frames_played.store(0, Ordering::SeqCst);
                        }
                        is_eof = false;
                        shared.eof.store(false, Ordering::SeqCst);
                    }
                }
            }

            if is_eof {
                thread::sleep(Duration::from_millis(10));
                continue;
            }

            // If ring buffer has plenty of space, decode next packet
            if producer.slots() >= 1024 {
                match source.next_packet_stereo() {
                    Ok(Some(samples)) => {
                        let mut offset = 0;
                        while offset < samples.len() {
                            let chunk = &samples[offset..];
                            let to_write = chunk.len().min(producer.slots());
                            let written = if to_write > 0 {
                                if let Ok(mut out_chunk) = producer.write_chunk(to_write) {
                                    let (first, second) = out_chunk.as_mut_slices();
                                    let n1 = first.len();
                                    first.copy_from_slice(&chunk[..n1]);
                                    if !second.is_empty() {
                                        second.copy_from_slice(&chunk[n1..to_write]);
                                    }
                                    out_chunk.commit_all();
                                    to_write
                                } else {
                                    0
                                }
                            } else {
                                0
                            };

                            offset += written;

                            if offset < samples.len() {
                                // Check for stop command or consumer drop to avoid deadlock
                                if producer.is_abandoned() {
                                    return;
                                }
                                if let Ok(cmd) = command_rx.try_recv() {
                                    match cmd {
                                        VoiceWorkerCommand::Stop => return,
                                        VoiceWorkerCommand::Seek(time_secs) => {
                                            // Exit writing loop early; process seek at outer loop
                                            let base_frames = (time_secs * shared.sample_rate as f64).max(0.0) as u64;
                                            shared.seek_base_frames.store(base_frames, Ordering::SeqCst);
                                            shared.frames_played.store(0, Ordering::SeqCst);
                                            let _ = source.seek(time_secs);
                                            is_eof = false;
                                            shared.eof.store(false, Ordering::SeqCst);
                                            break;
                                        }
                                    }
                                }

                                // Wait briefly for consumer to drain
                                thread::sleep(Duration::from_millis(5));
                            }
                        }

                        if !shared.prime_ready.load(Ordering::Relaxed) {
                            let buffered = ring_capacity.saturating_sub(producer.slots());
                            if buffered >= prime_threshold_samples {
                                shared.prime_ready.store(true, Ordering::SeqCst);
                            }
                        }
                    }
                    Ok(None) => {
                        // Reached EOF
                        is_eof = true;
                        shared.eof.store(true, Ordering::SeqCst);
                        shared.prime_ready.store(true, Ordering::SeqCst);
                    }
                    Err(e) => {
                        log::error!("Decoder error in voice worker: {e}");
                        shared.failed.store(true, Ordering::SeqCst);
                        return;
                    }
                }
            } else {
                // Buffer is full enough; sleep briefly
                thread::sleep(Duration::from_millis(5));
            }
        }
    }
}

impl Drop for Voice {
    fn drop(&mut self) {
        let _ = self.command_tx.send(VoiceWorkerCommand::Stop);
        if let Some(handle) = self.worker_handle.take() {
            let _ = handle.join();
        }
    }
}
