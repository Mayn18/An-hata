use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use anahata_audio::AudioEngine;

use crate::queue::Queue;
use crate::types::{PlaybackPosition, PlaybackStatus, PlayerCommand, PlayerEvent, PlayerState};

pub trait PlayerObserver: Send + Sync {
    fn on_event(&self, event: &PlayerEvent);
}

pub struct PlaybackManager {
    cmd_tx: Sender<PlayerCommand>,
    worker_handle: Option<JoinHandle<()>>,
}

impl PlaybackManager {
    pub fn start(observers: Vec<Arc<dyn PlayerObserver>>) -> Self {
        let (cmd_tx, cmd_rx) = mpsc::channel();

        let worker_handle = thread::Builder::new()
            .name("playback-manager".to_string())
            .spawn(move || {
                Self::manager_loop(cmd_rx, observers);
            })
            .expect("Failed to spawn playback manager thread");

        Self {
            cmd_tx,
            worker_handle: Some(worker_handle),
        }
    }

    pub fn send(&self, cmd: PlayerCommand) {
        let _ = self.cmd_tx.send(cmd);
    }

    fn manager_loop(cmd_rx: Receiver<PlayerCommand>, observers: Vec<Arc<dyn PlayerObserver>>) {
        let mut engine = match AudioEngine::new() {
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

        let mut queue = Queue::new();
        let mut status = PlaybackStatus::Stopped;
        let mut volume = 1.0f32;
        let mut last_position_report = 0u64;

        let notify_observers = |event: PlayerEvent| {
            for obs in &observers {
                obs.on_event(&event);
            }
        };

        let get_state = |queue: &Queue, status: PlaybackStatus, volume: f32, engine: &AudioEngine| -> PlayerState {
            let pos_sec = engine.position_secs();
            let dur_sec = engine.duration_secs();
            PlayerState {
                status,
                queue_revision: queue.revision(),
                current_queue_index: queue.current_queue_index(),
                current_item: queue.current_item().cloned(),
                position: PlaybackPosition {
                    position_ms: (pos_sec * 1000.0) as u64,
                    duration_ms: dur_sec.map(|d| (d * 1000.0) as u64),
                    playing: status == PlaybackStatus::Playing,
                },
                volume,
                shuffle: queue.is_shuffle(),
                repeat: queue.repeat_mode(),
                error: None,
            }
        };

        loop {
            // Process commands with short timeout to poll engine status
            let cmd = cmd_rx.recv_timeout(Duration::from_millis(50));
            match cmd {
                Ok(PlayerCommand::Shutdown) => {
                    let _ = engine.stop();
                    break;
                }
                Ok(PlayerCommand::Play) => {
                    if let Some(item) = queue.current_item() {
                        if let Err(e) = engine.play_file(&item.path) {
                            let msg = format!("Failed to play file: {e}");
                            status = PlaybackStatus::Error;
                            let _ = get_state(&queue, status, volume, &engine);
                            notify_observers(PlayerEvent::Error(msg));
                        } else {
                            status = PlaybackStatus::Playing;
                            let state = get_state(&queue, status, volume, &engine);
                            notify_observers(PlayerEvent::TrackChanged(Some(item.clone())));
                            notify_observers(PlayerEvent::StateChanged(state));
                        }
                    }
                }
                Ok(PlayerCommand::Pause) => {
                    let _ = engine.pause();
                    status = PlaybackStatus::Paused;
                    let state = get_state(&queue, status, volume, &engine);
                    notify_observers(PlayerEvent::StateChanged(state));
                }
                Ok(PlayerCommand::Toggle) => {
                    if status == PlaybackStatus::Playing {
                        let _ = engine.pause();
                        status = PlaybackStatus::Paused;
                    } else if let Some(item) = queue.current_item() {
                        if engine.current_file().is_none() {
                            let _ = engine.play_file(&item.path);
                        } else {
                            let _ = engine.resume();
                        }
                        status = PlaybackStatus::Playing;
                    }
                    let state = get_state(&queue, status, volume, &engine);
                    notify_observers(PlayerEvent::StateChanged(state));
                }
                Ok(PlayerCommand::Stop) => {
                    let _ = engine.stop();
                    status = PlaybackStatus::Stopped;
                    let state = get_state(&queue, status, volume, &engine);
                    notify_observers(PlayerEvent::StateChanged(state));
                }
                Ok(PlayerCommand::Next) => {
                    if let Some(next_item) = queue.next() {
                        let next_path = next_item.path.clone();
                        let item_clone = next_item.clone();
                        if let Err(e) = engine.play_file(&next_path) {
                            log::error!("Failed to advance to next track: {e}");
                        } else {
                            status = PlaybackStatus::Playing;
                            let state = get_state(&queue, status, volume, &engine);
                            notify_observers(PlayerEvent::TrackChanged(Some(item_clone)));
                            notify_observers(PlayerEvent::StateChanged(state));
                        }
                    } else {
                        let _ = engine.stop();
                        status = PlaybackStatus::Stopped;
                        let state = get_state(&queue, status, volume, &engine);
                        notify_observers(PlayerEvent::TrackChanged(None));
                        notify_observers(PlayerEvent::StateChanged(state));
                    }
                }
                Ok(PlayerCommand::Previous) => {
                    if let Some(prev_item) = queue.previous() {
                        let prev_path = prev_item.path.clone();
                        let item_clone = prev_item.clone();
                        let _ = engine.play_file(&prev_path);
                        status = PlaybackStatus::Playing;
                        let state = get_state(&queue, status, volume, &engine);
                        notify_observers(PlayerEvent::TrackChanged(Some(item_clone)));
                        notify_observers(PlayerEvent::StateChanged(state));
                    }
                }
                Ok(PlayerCommand::Seek(pos_ms)) => {
                    let _ = engine.seek(pos_ms as f64 / 1000.0);
                    let state = get_state(&queue, status, volume, &engine);
                    notify_observers(PlayerEvent::PositionAnchor(state.position));
                }
                Ok(PlayerCommand::SetVolume(vol)) => {
                    volume = vol.clamp(0.0, 1.0);
                    let _ = engine.set_volume(volume);
                    let state = get_state(&queue, status, volume, &engine);
                    notify_observers(PlayerEvent::StateChanged(state));
                }
                Ok(PlayerCommand::SetShuffle(shuf)) => {
                    queue.set_shuffle(shuf);
                    let state = get_state(&queue, status, volume, &engine);
                    notify_observers(PlayerEvent::StateChanged(state));
                }
                Ok(PlayerCommand::SetRepeat(rep)) => {
                    queue.set_repeat(rep);
                    let state = get_state(&queue, status, volume, &engine);
                    notify_observers(PlayerEvent::StateChanged(state));
                }
                Ok(PlayerCommand::PlayItem(id)) => {
                    if let Some(item) = queue.play_item(id) {
                        let p = item.path.clone();
                        let item_clone = item.clone();
                        let _ = engine.play_file(&p);
                        status = PlaybackStatus::Playing;
                        let state = get_state(&queue, status, volume, &engine);
                        notify_observers(PlayerEvent::TrackChanged(Some(item_clone)));
                        notify_observers(PlayerEvent::StateChanged(state));
                    }
                }
                Ok(PlayerCommand::LoadQueue { items, start_index }) => {
                    let len = items.len();
                    queue.load(items, start_index);
                    notify_observers(
                        PlayerEvent::QueueChanged {
                            revision: queue.revision(),
                            length: len,
                        },
                    );
                    if let Some(cur) = queue.current_item() {
                        let path = cur.path.clone();
                        let item_clone = cur.clone();
                        let _ = engine.play_file(&path);
                        status = PlaybackStatus::Playing;
                        let state = get_state(&queue, status, volume, &engine);
                        notify_observers(PlayerEvent::TrackChanged(Some(item_clone)));
                        notify_observers(PlayerEvent::StateChanged(state));
                    }
                }
                Ok(PlayerCommand::ClearQueue) => {
                    queue.clear();
                    let _ = engine.stop();
                    status = PlaybackStatus::Stopped;
                    let state = get_state(&queue, status, volume, &engine);
                    notify_observers(
                        PlayerEvent::QueueChanged {
                            revision: queue.revision(),
                            length: 0,
                        },
                    );
                    notify_observers(PlayerEvent::TrackChanged(None));
                    notify_observers(PlayerEvent::StateChanged(state));
                }
                Ok(PlayerCommand::RemoveQueueItem(id)) => {
                    queue.remove_item(id);
                    let state = get_state(&queue, status, volume, &engine);
                    notify_observers(
                        PlayerEvent::QueueChanged {
                            revision: queue.revision(),
                            length: queue.len(),
                        },
                    );
                    notify_observers(PlayerEvent::StateChanged(state));
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    // Periodic maintenance / position heartbeat
                    if status == PlaybackStatus::Playing {
                        let pos = (engine.position_secs() * 1000.0) as u64;
                        if pos.abs_diff(last_position_report) >= 1000 {
                            last_position_report = pos;
                            let state = get_state(&queue, status, volume, &engine);
                            notify_observers(PlayerEvent::PositionAnchor(state.position));
                        }
                    }
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    break;
                }
            }
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
