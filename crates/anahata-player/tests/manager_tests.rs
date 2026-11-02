use std::collections::{HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anahata_audio::{AudioError, AudioResult, EngineEvent};
use anahata_player::{
    AudioBackend, PlaybackStatus, Player, PlayerCommand, PlayerEvent, PlayerObserver, QueueItem, RepeatMode,
};

#[derive(Default)]
struct FakeBackend {
    broken: HashSet<PathBuf>,
    current: Option<PathBuf>,
    started: Vec<PathBuf>,
    events: VecDeque<EngineEvent>,
    paused: bool,
    position: f64,
    seeks: Vec<f64>,
}

impl FakeBackend {
    fn finish_current(&mut self) {
        let path = self.current.clone().expect("nothing playing");
        self.events.push_back(EngineEvent::TrackFinished { voice_id: 1, path });
    }
    fn fail_current(&mut self, message: &str) {
        let path = self.current.clone().expect("nothing playing");
        self.events.push_back(EngineEvent::TrackFailed { voice_id: 1, path, message: message.into() });
    }
}

impl AudioBackend for FakeBackend {
    fn play_file(&mut self, path: &Path) -> AudioResult<()> {
        if self.broken.contains(path) {
            return Err(AudioError::UnsupportedFormat("broken".into()));
        }
        self.current = Some(path.to_path_buf());
        self.started.push(path.to_path_buf());
        self.paused = false;
        self.position = 0.0;
        Ok(())
    }
    fn pause(&mut self) -> AudioResult<()> {
        self.paused = true;
        Ok(())
    }
    fn resume(&mut self) -> AudioResult<()> {
        self.paused = false;
        Ok(())
    }
    fn stop(&mut self) -> AudioResult<()> {
        self.current = None;
        Ok(())
    }
    fn seek(&mut self, t: f64) -> AudioResult<()> {
        self.seeks.push(t);
        self.position = t;
        Ok(())
    }
    fn set_volume(&mut self, _: f32) -> AudioResult<()> {
        Ok(())
    }
    fn poll_event(&mut self) -> Option<EngineEvent> {
        self.events.pop_front()
    }
    fn position_secs(&self) -> f64 {
        self.position
    }
    fn duration_secs(&self) -> Option<f64> {
        Some(180.0)
    }
    fn has_current_file(&self) -> bool {
        self.current.is_some()
    }
}

#[derive(Default)]
struct Recorder(Mutex<Vec<PlayerEvent>>);

impl PlayerObserver for Recorder {
    fn on_event(&self, event: &PlayerEvent) {
        self.0.lock().unwrap().push(event.clone());
    }
}

fn item(id: u64, name: &str) -> QueueItem {
    QueueItem { id, track_id: id as i64, path: PathBuf::from(format!("/m/{name}.flac")), title: name.into(), artist: "A".into() }
}

fn player(names: &[&str]) -> (Player<FakeBackend>, Arc<Recorder>) {
    let rec = Arc::new(Recorder::default());
    let mut p = Player::new(FakeBackend::default(), vec![rec.clone() as Arc<dyn PlayerObserver>]);
    let items = names.iter().enumerate().map(|(i, n)| item(i as u64 + 1, n)).collect();
    p.handle(PlayerCommand::LoadQueue { items, start_index: Some(0) });
    (p, rec)
}

fn current_title(p: &Player<FakeBackend>) -> String {
    p.queue().current_item().map(|i| i.title.clone()).unwrap_or_default()
}

fn errors(rec: &Recorder) -> usize {
    rec.0.lock().unwrap().iter().filter(|e| matches!(e, PlayerEvent::Error(_))).count()
}

#[test]
fn track_end_advances_to_next_track() {
    let (mut p, _) = player(&["a", "b", "c"]);
    p.backend_mut().finish_current();
    p.tick();
    assert_eq!(current_title(&p), "b");
    assert_eq!(p.backend().current, Some(PathBuf::from("/m/b.flac")));
    assert_eq!(p.status(), PlaybackStatus::Playing);
}

#[test]
fn end_of_queue_stops_with_repeat_off() {
    let (mut p, rec) = player(&["a", "b"]);
    for _ in 0..2 {
        p.backend_mut().finish_current();
        p.tick();
    }
    assert_eq!(p.status(), PlaybackStatus::Stopped);
    assert!(p.backend().current.is_none());
    assert!(rec.0.lock().unwrap().iter().any(|e| matches!(e, PlayerEvent::TrackChanged(None))));
}

#[test]
fn repeat_one_replays_and_repeat_all_wraps() {
    let (mut p, _) = player(&["a", "b"]);
    p.handle(PlayerCommand::SetRepeat(RepeatMode::One));
    p.backend_mut().finish_current();
    p.tick();
    assert_eq!(current_title(&p), "a");
    assert_eq!(p.backend().started.len(), 2);

    p.handle(PlayerCommand::SetRepeat(RepeatMode::All));
    p.handle(PlayerCommand::Next);
    p.backend_mut().finish_current();
    p.tick();
    assert_eq!(current_title(&p), "a", "repeat all wraps from the last track");
}

#[test]
fn manual_next_with_repeat_one_actually_advances() {
    let (mut p, _) = player(&["a", "b"]);
    p.handle(PlayerCommand::SetRepeat(RepeatMode::One));
    p.handle(PlayerCommand::Next);
    assert_eq!(current_title(&p), "b");
}

#[test]
fn unplayable_track_is_skipped_and_reported() {
    let (mut p, rec) = player(&["a", "b", "c"]);
    p.backend_mut().broken.insert(PathBuf::from("/m/b.flac"));
    p.handle(PlayerCommand::Next);
    assert_eq!(current_title(&p), "c", "must hop over the broken track");
    assert_eq!(p.backend().current, Some(PathBuf::from("/m/c.flac")));
    assert_eq!(errors(&rec), 1);
    assert_eq!(p.status(), PlaybackStatus::Playing);
}

#[test]
fn failed_next_keeps_old_track_current_and_playing() {
    let (mut p, rec) = player(&["a", "b"]);
    p.backend_mut().broken.insert(PathBuf::from("/m/b.flac"));
    p.handle(PlayerCommand::Next);
    assert_eq!(current_title(&p), "a", "UI position must not move to a track that never started");
    assert_eq!(p.backend().current, Some(PathBuf::from("/m/a.flac")), "old audio keeps playing");
    assert_eq!(p.status(), PlaybackStatus::Playing);
    assert_eq!(errors(&rec), 1);
}

#[test]
fn failed_play_item_restores_position_and_reports() {
    let (mut p, rec) = player(&["a", "b", "c"]);
    p.backend_mut().broken.insert(PathBuf::from("/m/c.flac"));
    p.handle(PlayerCommand::PlayItem(3));
    assert_eq!(current_title(&p), "a");
    assert_eq!(p.status(), PlaybackStatus::Playing);
    assert_eq!(errors(&rec), 1);
}

#[test]
fn load_queue_with_nothing_playable_ends_in_error_state() {
    let rec = Arc::new(Recorder::default());
    let mut p = Player::new(FakeBackend::default(), vec![rec.clone() as Arc<dyn PlayerObserver>]);
    p.backend_mut().broken.insert(PathBuf::from("/m/a.flac"));
    p.backend_mut().broken.insert(PathBuf::from("/m/b.flac"));
    p.handle(PlayerCommand::LoadQueue { items: vec![item(1, "a"), item(2, "b")], start_index: Some(0) });
    assert_eq!(p.status(), PlaybackStatus::Error);
    assert_eq!(errors(&rec), 2);
    assert!(p.state().error.is_some());
}

#[test]
fn mid_stream_decode_failure_is_reported_and_skipped() {
    let (mut p, rec) = player(&["a", "b"]);
    p.backend_mut().fail_current("corrupt frame");
    p.tick();
    assert_eq!(current_title(&p), "b");
    assert_eq!(p.status(), PlaybackStatus::Playing);
    assert_eq!(errors(&rec), 1);
}

#[test]
fn failure_on_last_track_ends_in_error() {
    let (mut p, _) = player(&["a"]);
    p.backend_mut().fail_current("corrupt frame");
    p.tick();
    assert_eq!(p.status(), PlaybackStatus::Error);
    assert!(p.state().error.unwrap().contains("corrupt frame"));
}

#[test]
fn removing_a_track_before_current_keeps_the_same_song() {
    let (mut p, _) = player(&["a", "b", "c", "d"]);
    p.handle(PlayerCommand::PlayItem(3)); // c
    p.handle(PlayerCommand::RemoveQueueItem(1)); // remove a
    assert_eq!(current_title(&p), "c");
    assert_eq!(p.backend().current, Some(PathBuf::from("/m/c.flac")));
    assert_eq!(p.backend().started.len(), 2, "removal must not restart playback");
    assert_eq!(p.state().current_queue_index, Some(1));
}

#[test]
fn removing_the_playing_track_moves_on_to_the_following_one() {
    let (mut p, _) = player(&["a", "b", "c"]);
    p.handle(PlayerCommand::RemoveQueueItem(1));
    assert_eq!(current_title(&p), "b");
    assert_eq!(p.backend().current, Some(PathBuf::from("/m/b.flac")));
    assert_eq!(p.status(), PlaybackStatus::Playing);
}

#[test]
fn removing_the_last_playing_track_stops() {
    let (mut p, _) = player(&["a", "b"]);
    p.handle(PlayerCommand::PlayItem(2));
    p.handle(PlayerCommand::RemoveQueueItem(2));
    assert_eq!(p.status(), PlaybackStatus::Stopped);
    assert!(p.backend().current.is_none());
}

#[test]
fn removing_with_shuffle_on_keeps_current_and_order() {
    let (mut p, _) = player(&["a", "b", "c", "d", "e"]);
    p.handle(PlayerCommand::SetShuffle(true));
    let playing = current_title(&p);
    let victim = if playing == "e" { 4 } else { 5 };
    p.handle(PlayerCommand::RemoveQueueItem(victim));
    assert_eq!(current_title(&p), playing);
    assert_eq!(p.queue().len(), 4);
}

#[test]
fn pause_resume_and_previous_restart() {
    let (mut p, _) = player(&["a", "b"]);
    p.handle(PlayerCommand::Pause);
    assert_eq!(p.status(), PlaybackStatus::Paused);
    assert!(p.backend().paused);
    p.handle(PlayerCommand::Play);
    assert_eq!(p.status(), PlaybackStatus::Playing);
    assert_eq!(p.backend().started.len(), 1, "Play after Pause resumes, it does not restart");

    p.handle(PlayerCommand::Next);
    p.backend_mut().position = 10.0;
    p.handle(PlayerCommand::Previous);
    assert_eq!(current_title(&p), "b", "past the threshold Previous restarts the track");
    assert_eq!(p.backend().seeks, vec![0.0]);

    p.backend_mut().position = 1.0;
    p.handle(PlayerCommand::Previous);
    assert_eq!(current_title(&p), "a");
}

#[test]
fn snapshot_tracks_latest_state() {
    let (mut p, _) = player(&["a", "b"]);
    p.handle(PlayerCommand::SetVolume(0.25));
    let snap = p.state();
    assert_eq!(snap.volume, 0.25);
    assert_eq!(snap.current_item.unwrap().title, "a");
}
