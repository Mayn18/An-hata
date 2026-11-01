use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use anahata_audio::{AudioEngine, EngineEvent};

use crate::backend::AudioBackend;
use crate::queue::Queue;
use crate::types::{PlaybackPosition, PlaybackStatus, PlayerCommand, PlayerEvent, PlayerState, QueueItem};

/// Pressing Previous after this much of a track has played restarts it instead of going back.
const RESTART_THRESHOLD_SECS: f64 = 3.0;

pub trait PlayerObserver: Send + Sync {
    fn on_event(&self, event: &PlayerEvent);
}

/// The playback state machine. It owns the queue and talks to an [`AudioBackend`].
///
/// Rules it keeps:
/// - Queue position only moves to a track once that track has actually started; if a
///   user-requested change fails, the previous position is restored and old audio is untouched.
/// - Tracks that cannot be played are reported and skipped, never silently ignored.
/// - When the engine reports a track ended, the queue advances according to repeat mode.
pub struct Player<B: AudioBackend> {
    backend: B,
    queue: Queue,
    status: PlaybackStatus,
    volume: f32,
    last_error: Option<String>,
    last_position_report: u64,
    observers: Vec<Arc<dyn PlayerObserver>>,
    snapshot: Arc<Mutex<PlayerState>>,
}

impl<B: AudioBackend> Player<B> {
    pub fn new(backend: B, observers: Vec<Arc<dyn PlayerObserver>>) -> Self {
        Self::with_snapshot(backend, observers, Arc::new(Mutex::new(PlayerState::idle())))
    }

    pub fn with_snapshot(
        backend: B,
        observers: Vec<Arc<dyn PlayerObserver>>,
        snapshot: Arc<Mutex<PlayerState>>,
    ) -> Self {
        Self {
            backend,
            queue: Queue::new(),
            status: PlaybackStatus::Stopped,
            volume: 1.0,
            last_error: None,
            last_position_report: 0,
            observers,
            snapshot,
        }
    }

    pub fn backend(&self) -> &B {
        &self.backend
    }

    pub fn backend_mut(&mut self) -> &mut B {
        &mut self.backend
    }

    pub fn queue(&self) -> &Queue {
        &self.queue
    }

    pub fn status(&self) -> PlaybackStatus {
        self.status
    }

    pub fn state(&self) -> PlayerState {
        PlayerState {
            status: self.status,
            queue_revision: self.queue.revision(),
            current_queue_index: self.queue.current_queue_index(),
            current_item: self.queue.current_item().cloned(),
            position: self.position(),
            volume: self.volume,
            shuffle: self.queue.is_shuffle(),
            repeat: self.queue.repeat_mode(),
            error: self.last_error.clone(),
        }
    }

    fn position(&self) -> PlaybackPosition {
        PlaybackPosition {
            position_ms: (self.backend.position_secs() * 1000.0) as u64,
            duration_ms: self.backend.duration_secs().map(|d| (d * 1000.0) as u64),
            playing: self.status == PlaybackStatus::Playing,
        }
    }

    fn notify(&self, event: PlayerEvent) {
        for obs in &self.observers {
            obs.on_event(&event);
        }
    }

    fn emit_state(&self) {
        let state = self.state();
        if let Ok(mut snap) = self.snapshot.lock() {
            *snap = state.clone();
        }
        self.notify(PlayerEvent::StateChanged(state));
    }

    fn report_error(&mut self, message: String) {
        log::error!("{message}");
        self.last_error = Some(message.clone());
        self.notify(PlayerEvent::Error(message));
    }

    /// Stops the backend and settles into `status`.
    fn halt(&mut self, status: PlaybackStatus) {
        let _ = self.backend.stop();
        self.status = status;
        self.last_position_report = 0;
        self.emit_state();
    }

    /// Stops and tells observers that nothing is playing any more.
    fn finish_queue(&mut self) {
        self.halt(PlaybackStatus::Stopped);
        self.notify(PlayerEvent::TrackChanged(None));
    }

    /// Tries to start the current queue item. Success is the only way status becomes Playing.
    fn try_play_current(&mut self) -> Result<(), String> {
        let item: QueueItem = self.queue.current_item().cloned().ok_or_else(|| "Queue is empty".to_string())?;
        match self.backend.play_file(&item.path) {
            Ok(()) => {
                self.status = PlaybackStatus::Playing;
                self.last_error = None;
                self.last_position_report = 0;
                self.notify(PlayerEvent::TrackChanged(Some(item)));
                self.emit_state();
                Ok(())
            }
            Err(e) => {
                let msg = format!("Cannot play \"{}\": {e}", item.title);
                self.report_error(msg.clone());
                Err(msg)
            }
        }
    }

    /// Plays the current item; if it cannot be played, hops forward (without wrapping) until
    /// something starts. Returns whether anything is now playing.
    fn play_current_or_skip_forward(&mut self) -> bool {
        if self.queue.current_item().is_none() {
            return false;
        }
        loop {
            if self.try_play_current().is_ok() {
                return true;
            }
            if self.queue.step_forward().is_none() {
                return false;
            }
        }
    }

    /// Starts the current item and, if nothing at all could be played, settles into Error.
    fn start_or_fail(&mut self) {
        if self.queue.current_item().is_some() && !self.play_current_or_skip_forward() {
            self.halt(PlaybackStatus::Error);
        }
    }

    fn restore_current(&mut self, id: Option<u64>) {
        if let Some(id) = id {
            self.queue.play_item(id);
        }
    }

    pub fn handle(&mut self, cmd: PlayerCommand) {
        match cmd {
            PlayerCommand::Shutdown => {
                let _ = self.backend.stop();
            }
            PlayerCommand::Play => {
                if self.status == PlaybackStatus::Paused && self.backend.has_current_file() {
                    let _ = self.backend.resume();
                    self.status = PlaybackStatus::Playing;
                    self.emit_state();
                } else {
                    self.start_or_fail();
                }
            }
            PlayerCommand::Pause => {
                if self.status == PlaybackStatus::Playing {
                    let _ = self.backend.pause();
                    self.status = PlaybackStatus::Paused;
                    self.emit_state();
                }
            }
            PlayerCommand::Toggle => match self.status {
                PlaybackStatus::Playing => self.handle(PlayerCommand::Pause),
                _ => self.handle(PlayerCommand::Play),
            },
            PlayerCommand::Stop => self.halt(PlaybackStatus::Stopped),
            PlayerCommand::Next => {
                let saved = self.queue.current_item_id();
                if self.queue.skip_next().is_none() {
                    self.finish_queue();
                } else if !self.play_current_or_skip_forward() {
                    // Nothing further could be played: keep the old track current and playing.
                    self.restore_current(saved);
                    self.emit_state();
                }
            }
            PlayerCommand::Previous => {
                if self.backend.has_current_file() && self.backend.position_secs() > RESTART_THRESHOLD_SECS {
                    self.handle(PlayerCommand::Seek(0));
                    return;
                }
                let saved = self.queue.current_item_id();
                if self.queue.previous().is_some() && self.try_play_current().is_err() {
                    self.restore_current(saved);
                    self.emit_state();
                }
            }
            PlayerCommand::Seek(pos_ms) => {
                if self.backend.has_current_file() {
                    if let Err(e) = self.backend.seek(pos_ms as f64 / 1000.0) {
                        self.report_error(format!("Seek failed: {e}"));
                    }
                    self.last_position_report = pos_ms;
                    self.notify(PlayerEvent::PositionAnchor(self.position()));
                }
            }
            PlayerCommand::SetVolume(vol) => {
                self.volume = vol.clamp(0.0, 1.0);
                let _ = self.backend.set_volume(self.volume);
                self.emit_state();
            }
            PlayerCommand::SetShuffle(shuffle) => {
                self.queue.set_shuffle(shuffle);
                self.emit_state();
            }
            PlayerCommand::SetRepeat(mode) => {
                self.queue.set_repeat(mode);
                self.emit_state();
            }
            PlayerCommand::PlayItem(id) => {
                let saved = self.queue.current_item_id();
                if self.queue.play_item(id).is_some() && self.try_play_current().is_err() {
                    self.restore_current(saved);
                    self.emit_state();
                }
            }
            PlayerCommand::LoadQueue { items, start_index } => {
                let len = items.len();
                self.queue.load(items, start_index);
                self.notify(PlayerEvent::QueueChanged { revision: self.queue.revision(), length: len });
                if self.queue.current_item().is_some() {
                    self.start_or_fail();
                } else {
                    self.finish_queue();
                }
            }
            PlayerCommand::ClearQueue => {
                self.queue.clear();
                self.notify(PlayerEvent::QueueChanged { revision: self.queue.revision(), length: 0 });
                self.finish_queue();
            }
            PlayerCommand::RemoveQueueItem(id) => {
                let outcome = self.queue.remove_item(id);
                if !outcome.removed {
                    return;
                }
                self.notify(PlayerEvent::QueueChanged { revision: self.queue.revision(), length: self.queue.len() });
                if !outcome.removed_current {
                    self.emit_state();
                } else if self.status == PlaybackStatus::Playing && self.queue.current_item().is_some() {
                    // The playing track was removed: move on to the one that followed it.
                    self.start_or_fail();
                } else {
                    self.finish_queue();
                }
            }
        }
    }

    /// Periodic maintenance: reacts to engine events and reports position. Call about every 50 ms.
    pub fn tick(&mut self) {
        while let Some(event) = self.backend.poll_event() {
            if self.status != PlaybackStatus::Playing {
                continue;
            }
            match event {
                EngineEvent::TrackFinished { .. } => self.on_track_finished(),
                EngineEvent::TrackFailed { message, .. } => self.on_track_failed(message),
            }
        }

        if self.status == PlaybackStatus::Playing {
            let pos = (self.backend.position_secs() * 1000.0) as u64;
            if pos.abs_diff(self.last_position_report) >= 1000 {
                self.last_position_report = pos;
                self.notify(PlayerEvent::PositionAnchor(self.position()));
            }
        }
    }

    fn on_track_finished(&mut self) {
        if self.queue.next().is_none() {
            self.finish_queue();
        } else if !self.play_current_or_skip_forward() {
            self.halt(PlaybackStatus::Error);
        }
    }

    fn on_track_failed(&mut self, message: String) {
        let title = self.queue.current_item().map(|i| i.title.clone()).unwrap_or_default();
        self.report_error(format!("Playback of \"{title}\" failed: {message}"));
        if self.queue.step_forward().is_some() && self.play_current_or_skip_forward() {
            return;
        }
        self.halt(PlaybackStatus::Error);
    }
}

pub struct PlaybackManager {
    cmd_tx: Sender<PlayerCommand>,
    snapshot: Arc<Mutex<PlayerState>>,
    worker_handle: Option<JoinHandle<()>>,
}

impl PlaybackManager {
    pub fn start(observers: Vec<Arc<dyn PlayerObserver>>) -> Self {
        let (cmd_tx, cmd_rx) = mpsc::channel();
        let snapshot = Arc::new(Mutex::new(PlayerState::idle()));
        let thread_snapshot = Arc::clone(&snapshot);

        let worker_handle = thread::Builder::new()
            .name("playback-manager".to_string())
            .spawn(move || {
                Self::manager_loop(cmd_rx, observers, thread_snapshot);
            })
            .expect("Failed to spawn playback manager thread");

        Self { cmd_tx, snapshot, worker_handle: Some(worker_handle) }
    }

    pub fn send(&self, cmd: PlayerCommand) {
        let _ = self.cmd_tx.send(cmd);
    }

    /// The most recently published player state (updated on every state change).
    pub fn state(&self) -> PlayerState {
        self.snapshot.lock().map(|s| s.clone()).unwrap_or_else(|_| PlayerState::idle())
    }

    fn manager_loop(
        cmd_rx: Receiver<PlayerCommand>,
        observers: Vec<Arc<dyn PlayerObserver>>,
        snapshot: Arc<Mutex<PlayerState>>,
    ) {
        let engine = match AudioEngine::new() {
            Ok(e) => e,
            Err(err) => {
                let err_msg = format!("Failed to initialize audio engine: {err}");
                log::error!("{err_msg}");
                for obs in &observers {
                    obs.on_event(&PlayerEvent::Error(err_msg.clone()));
                }
                return;
            }
        };

        let mut player = Player::with_snapshot(engine, observers, snapshot);

        loop {
            match cmd_rx.recv_timeout(Duration::from_millis(50)) {
                Ok(PlayerCommand::Shutdown) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                    player.handle(PlayerCommand::Shutdown);
                    break;
                }
                Ok(cmd) => player.handle(cmd),
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
            player.tick();
        }
    }
}

impl Drop for PlaybackManager {
    fn drop(&mut self) {
        let _ = self.cmd_tx.send(PlayerCommand::Shutdown);
        if let Some(h) = self.worker_handle.take() {
            let _ = h.join();
        }
    }
}
